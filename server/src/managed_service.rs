use std::{
    path::{Path, PathBuf},
    process::Stdio,
    str::FromStr,
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result};
use common::{
    message::ServiceStatus,
    structure::{HealthCheck, HealthCheckProbe, Service},
};
use reqwest::{Client, Url};
use tokio::{
    sync::{Mutex, watch},
    task::JoinHandle,
};

use crate::log::LogBuffer;

pub struct DependencyList(pub Vec<watch::Receiver<ServiceState>>);

impl DependencyList {
    pub async fn wait_for_healthy(&mut self) {
        for rx in self.0.iter_mut() {
            if *rx.borrow() != ServiceState::Healthy {
                while rx.changed().await.is_ok() {
                    if *rx.borrow() == ServiceState::Healthy {
                        break;
                    }
                }
            }
        }
    }
}

impl From<Vec<watch::Receiver<ServiceState>>> for DependencyList {
    fn from(value: Vec<watch::Receiver<ServiceState>>) -> Self {
        Self(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceState {
    Pending,
    Healthy,
    Unhealthy,
}

#[derive(Debug)]
pub struct Process {
    child: tokio::process::Child,
    stdout_thread: JoinHandle<()>,
    stderr_thread: JoinHandle<()>,
    health_check_thread: JoinHandle<Result<()>>,
}

impl Process {
    pub async fn start(
        service: Service,
        cwd: impl AsRef<Path>,
        health_tx: watch::Sender<ServiceState>,
    ) -> Result<Self> {
        let mut child = service
            .to_command()
            .current_dir(cwd)
            .kill_on_drop(true)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("Failed to start the command")?;

        let stdout = child.stdout.take().context("Failed to take stdout")?;
        let stderr = child.stderr.take().context("Failed to take stderr")?;

        let log_buffer = LogBuffer::new(1000);

        let buffer_clone = log_buffer.clone();
        let stdout_thread = tokio::spawn(async move {
            let reader = tokio::io::BufReader::new(stdout);
            let mut lines = tokio::io::AsyncBufReadExt::lines(reader);
            while let Ok(Some(line)) = lines.next_line().await {
                buffer_clone.push(line).await;
            }
        });

        let buffer_clone = log_buffer.clone();
        let stderr_thread = tokio::spawn(async move {
            let reader = tokio::io::BufReader::new(stderr);
            let mut lines = tokio::io::AsyncBufReadExt::lines(reader);
            while let Ok(Some(line)) = lines.next_line().await {
                buffer_clone.push(line).await;
            }
        });

        let health_check_thread =
            tokio::spawn(Self::monitor_health(service.health_check, health_tx));

        Ok(Self {
            child,
            stdout_thread,
            stderr_thread,
            health_check_thread,
        })
    }

    /// Stops all related tasks for this ManagedService.
    pub fn stop(&mut self) {
        // Kill the child process
        let _ = self.child.start_kill();

        // Kill all the worker threads
        self.stdout_thread.abort();
        self.stderr_thread.abort();
        self.health_check_thread.abort();
    }

    pub fn state(&mut self) -> bool {
        match self.child.try_wait() {
            Ok(None) => true,
            Ok(Some(_)) => false,
            Err(_) => false,
        }
    }

    async fn monitor_health(
        check: Option<HealthCheck>,
        health_tx: watch::Sender<ServiceState>,
    ) -> Result<()> {
        let check = match check {
            Some(check) => check,
            None => {
                tokio::time::sleep(Duration::from_secs(1)).await;
                let _ = health_tx.send(ServiceState::Healthy);
                return Ok(());
            }
        };
        match check.probe {
            HealthCheckProbe::Http {
                port,
                path,
                host,
                schema,
            } => {
                // let timeout = Instant::now().add(check.timeout.unwrap_or_else(|| Duration::MAX));
                let client = Client::builder().timeout(check.probe_timeout).build()?;
                let url = Url::from_str(&format!("{}://{}:{}", schema, host, port))?.join(&path)?;

                let mut successes: u32 = 0;
                let mut failures: u32 = 0;

                tokio::time::sleep(check.initial_delay).await;

                loop {
                    let res = client.get(url.clone()).send().await;

                    match res {
                        Ok(response) if response.status().is_success() => {
                            successes += 1;
                            failures = 0;

                            if successes >= check.success_threshold {
                                let _ = health_tx.send(ServiceState::Healthy);
                            }
                        }
                        _ => {
                            // Catches both network errors (Err) and bad HTTP statuses (Ok with 4xx/5xx)
                            failures += 1;
                            successes = 0;

                            if failures >= check.failure_threshold {
                                let _ = health_tx.send(ServiceState::Unhealthy);
                            }
                        }
                    }

                    // if let Some(timeout) = &check.timeout
                    //     && start.elapsed() >= *timeout
                    // {
                    //     anyhow::bail!("Failed to startup - timeout");
                    // }

                    tokio::time::sleep(check.period).await;
                }
            }
        }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        println!("Dropping process...");
        self.stop();
    }
}

#[derive(Debug)]
pub struct ManagedService {
    child: Arc<Mutex<Option<Process>>>, // Kept alive here so it doesn't drop/kill immediately
    log_buffer: LogBuffer,
    health_tx: watch::Sender<ServiceState>,
    health_rx: watch::Receiver<ServiceState>,
    startup_handle: Option<JoinHandle<Result<()>>>,
}

impl ManagedService {
    pub fn new(
        health_tx: watch::Sender<ServiceState>,
        health_rx: watch::Receiver<ServiceState>,
    ) -> Self {
        Self {
            child: Arc::new(Mutex::new(None)),
            log_buffer: LogBuffer::new(1000),
            health_tx,
            health_rx,
            startup_handle: None,
        }
    }

    /// Start a new ManagedService.
    pub fn start(
        &mut self,
        service: Service,
        mut dependencies: DependencyList,
        cwd: Arc<PathBuf>,
    ) -> Result<()> {
        let process_ref = self.child.clone();
        let health_tx = self.health_tx.clone();

        let handle = tokio::spawn(async move {
            dependencies.wait_for_healthy().await;
            let process = Process::start(service, &*cwd, health_tx).await;

            match process {
                Ok(p) => {
                    (*process_ref.lock().await) = Some(p);
                    Ok(())
                }
                Err(e) => {
                    let e = e.context("Failed to spawn process");

                    eprintln!("Failed to start service: {e:#?}");

                    Err(e)
                }
            }
        });

        self.startup_handle = Some(handle);

        Ok(())
    }

    /// Stops all related tasks for this ManagedService.
    pub async fn stop(&mut self) {
        self.startup_handle.as_mut().map(|h| h.abort());

        let mut lock = self.child.lock().await;

        lock.as_mut().map(|e| e.stop());
        *lock = None;
    }

    pub async fn status(&mut self) -> ServiceStatus {
        let running = self
            .child
            .lock()
            .await
            .as_mut()
            .map(|c| c.state())
            .unwrap_or(false);

        let lock = self.health_rx.borrow();
        let state = match *lock {
            ServiceState::Pending => common::message::ServiceState::Pending,
            ServiceState::Healthy => common::message::ServiceState::Healthy,
            ServiceState::Unhealthy => common::message::ServiceState::Unhealthy,
        };

        ServiceStatus { running, state }
    }

    pub fn log(&self) -> &LogBuffer {
        &self.log_buffer
    }

    pub async fn wait_for_healthy(&mut self) -> Result<()> {
        let rx = &mut self.health_rx;

        // Check if it's already healthy
        if *rx.borrow() == ServiceState::Healthy {
            return Ok(());
        }

        // Otherwise, wait for a change until it's healthy
        while rx.changed().await.is_ok() {
            if *rx.borrow() == ServiceState::Healthy {
                return Ok(());
            }
        }

        anyhow::bail!("Service failed to become healthy");
    }
}

impl Drop for ManagedService {
    fn drop(&mut self) {
        self.startup_handle.as_mut().map(|h| h.abort());
    }
}

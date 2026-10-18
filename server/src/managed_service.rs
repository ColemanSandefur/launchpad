use std::{path::Path, process::Stdio, str::FromStr, sync::Arc};

use anyhow::{Context, Result};
use common::{
    message::ServiceStatus,
    structure::{HealthCheck, HealthCheckProbe, Service},
};
use reqwest::{Client, Url};
use tokio::{sync::Mutex, task::JoinHandle};

use crate::log::LogBuffer;

#[derive(Debug, Clone, Copy)]
pub enum ServiceState {
    Pending,
    Healthy,
    Unhealthy,
}

#[derive(Debug)]
pub struct ManagedService {
    child: tokio::process::Child, // Kept alive here so it doesn't drop/kill immediately
    log_buffer: LogBuffer,
    state: Arc<Mutex<ServiceState>>,
    stdout_thread: JoinHandle<()>,
    stderr_thread: JoinHandle<()>,
    health_check_thread: Option<JoinHandle<Result<()>>>,
}

impl ManagedService {
    /// Start a new ManagedService.
    pub async fn start(service: Service, cwd: impl AsRef<Path>) -> Result<Self> {
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

        let state = Arc::new(Mutex::new(ServiceState::Pending));
        let health_check_thread = match service.health_check {
            Some(health_check) => Some(tokio::spawn(Self::monitor_health(
                health_check,
                state.clone(),
            ))),
            None => None,
        };

        Ok(ManagedService {
            child,
            log_buffer,
            state,
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
        self.health_check_thread.as_mut().map(|h| h.abort());
    }

    pub async fn status(&mut self) -> ServiceStatus {
        let running = match self.child.try_wait() {
            Ok(None) => true,
            Ok(Some(_)) => false,
            Err(_) => false,
        };

        let lock = self.state.lock().await;
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

    async fn monitor_health(check: HealthCheck, health: Arc<Mutex<ServiceState>>) -> Result<()> {
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
                                let mut lock = health.lock().await;
                                *lock = ServiceState::Healthy;
                            }
                        }
                        _ => {
                            // Catches both network errors (Err) and bad HTTP statuses (Ok with 4xx/5xx)
                            failures += 1;
                            successes = 0;

                            if failures >= check.failure_threshold {
                                let mut lock = health.lock().await;
                                *lock = ServiceState::Unhealthy;
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

impl Drop for ManagedService {
    fn drop(&mut self) {
        println!("Dropping service...");
        self.stop();
    }
}

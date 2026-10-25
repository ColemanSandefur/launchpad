use std::{
    ffi::OsStr,
    io::{BufRead, BufReader, Read},
    str::FromStr,
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result};
use launchpad_common::structure::{HealthCheck, HealthCheckProbe, Service};
use portable_pty::{ChildKiller, PtySize, native_pty_system};
use reqwest::{Client, Url};
use tokio::{
    sync::{Mutex, watch},
    task::JoinHandle,
};

use crate::{log::LogBuffer, service_state::ServiceState};

#[derive(Debug)]
pub struct Process {
    child: Arc<Mutex<Box<dyn portable_pty::Child + Send + Sync>>>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    log_thread: JoinHandle<()>,
    health_check_thread: JoinHandle<Result<()>>,
    exit_monitor_thread: JoinHandle<()>,
    health_tx: watch::Sender<ServiceState>,
}

impl Process {
    pub async fn start(
        service: Service,
        log_buffer: LogBuffer,
        cwd: impl AsRef<OsStr>,
        health_tx: watch::Sender<ServiceState>,
        health_rx: watch::Receiver<ServiceState>,
    ) -> Result<Self> {
        let _ = health_tx.send(ServiceState::Starting);
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("Failed to open PTY pair")?;

        let mut cmd = service.to_pty_command();
        cmd.cwd(cwd);

        let child = pair
            .slave
            .spawn_command(cmd)
            .context("Failed to start the command")?;

        let killer = child.clone_killer();

        let reader = pair
            .master
            .try_clone_reader()
            .context("Failed to clone PTY reader")?;

        let child = Arc::new(Mutex::new(child));

        let log_thread = {
            let log_buffer = log_buffer.clone();
            let health_rx = health_rx.clone();
            tokio::task::spawn_blocking(move || Self::monitor_logs(reader, log_buffer, health_rx))
        };

        let health_check_thread = tokio::spawn(Self::monitor_health(
            service.health_check,
            health_tx.clone(),
            health_rx.clone(),
        ));

        let exit_monitor_thread = tokio::spawn(Self::monitor_exit(
            child.clone(),
            health_tx.clone(),
            health_rx.clone(),
        ));

        Ok(Self {
            child,
            killer,
            log_thread,
            health_check_thread,
            exit_monitor_thread,
            health_tx,
        })
    }

    /// Stops all related tasks for this ManagedService.
    pub async fn stop(&mut self) {
        let _ = self.health_tx.send(ServiceState::Stopping);

        // Kill the child process
        let _ = self.child.lock().await.kill();

        let _ = self.health_tx.send(ServiceState::Stopped);
    }

    fn monitor_logs<R>(inner: R, buffer: LogBuffer, health_rx: watch::Receiver<ServiceState>)
    where
        R: Read + Send,
    {
        let buf_reader = BufReader::new(inner);
        let mut lines = buf_reader.lines();

        let rt = tokio::runtime::Handle::current();

        while !health_rx.borrow().is_dead() {
            match lines.next() {
                Some(Ok(line)) => {
                    rt.block_on(buffer.push(line));
                }
                _ => break,
            }
        }
        println!("Leaving monitor_logs (PTY)");
    }

    async fn monitor_health(
        check: Option<HealthCheck>,
        health_tx: watch::Sender<ServiceState>,
        health_rx: watch::Receiver<ServiceState>,
    ) -> Result<()> {
        let check = match check {
            Some(check) => check,
            None => {
                tokio::time::sleep(Duration::from_secs(10)).await;
                if !health_rx.borrow().is_dead() {
                    let _ = health_tx.send(ServiceState::Healthy);
                }
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

                while !health_rx.borrow().is_dead() {
                    let res = client.get(url.clone()).send().await;

                    match res {
                        Ok(response) if response.status().is_success() => {
                            successes += 1;
                            failures = 0;

                            if successes >= check.success_threshold && !health_rx.borrow().is_dead()
                            {
                                let _ = health_tx.send(ServiceState::Healthy);
                            }
                        }
                        _ => {
                            // Catches both network errors (Err) and bad HTTP statuses (Ok with 4xx/5xx)
                            failures += 1;
                            successes = 0;

                            if failures >= check.failure_threshold && !health_rx.borrow().is_dead()
                            {
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

                println!("Leaving monitor_health");

                Ok(())
            }
        }
    }

    async fn monitor_exit(
        child: Arc<Mutex<Box<dyn portable_pty::Child + Send + Sync>>>,
        health_tx: watch::Sender<ServiceState>,
        health_rx: watch::Receiver<ServiceState>,
    ) {
        while !health_rx.borrow().is_dead() {
            tokio::time::sleep(Duration::from_millis(500)).await;

            let mut guard = child.lock().await;
            match guard.try_wait() {
                Ok(Some(status)) => {
                    drop(guard);
                    let new_state = if status.success() {
                        ServiceState::Stopped
                    } else {
                        ServiceState::Failed
                    };
                    let _ = health_tx.send(new_state);
                    break;
                }
                Ok(None) => {
                    // Still running
                }
                Err(_) => {
                    drop(guard);
                    let _ = health_tx.send(ServiceState::Failed);
                    break;
                }
            }
        }

        println!("Leaving monitor_exit");
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        println!("Dropping process...");

        if let Err(e) = self.killer.kill() {
            eprintln!("Failed to kill PTY child on drop: {e}");
        }

        // Kill all the worker threads
        self.log_thread.abort();
        self.health_check_thread.abort();
        self.exit_monitor_thread.abort();
    }
}

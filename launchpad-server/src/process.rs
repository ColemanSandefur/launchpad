use std::{
    ffi::OsStr,
    io::{Read, Write},
    str::FromStr,
    sync::{Arc, Mutex as StdMutex},
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

pub struct Process {
    child: Arc<Mutex<Box<dyn portable_pty::Child + Send + Sync>>>,
    _pty_master: Box<dyn portable_pty::MasterPty + Send>,
    pty_writer: Arc<StdMutex<Option<Box<dyn Write + Send>>>>,
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
        drop(pair.slave);

        let killer = child.clone_killer();

        let reader = pair
            .master
            .try_clone_reader()
            .context("Failed to clone PTY reader")?;
        let writer = pair
            .master
            .take_writer()
            .context("Failed to take PTY writer")?;
        let pty_master = pair.master;
        let writer = Arc::new(StdMutex::new(Some(writer)));

        let child = Arc::new(Mutex::new(child));

        let log_thread = {
            let log_buffer = log_buffer.clone();
            let pty_writer = writer.clone();
            tokio::task::spawn_blocking(move || Self::monitor_logs(reader, pty_writer, log_buffer))
        };

        let health_check_thread = tokio::spawn(Self::monitor_health(
            service.health_check,
            health_tx.clone(),
            health_rx.clone(),
        ));

        let exit_monitor_thread = tokio::spawn(Self::monitor_exit(
            child.clone(),
            writer.clone(),
            health_tx.clone(),
            health_rx.clone(),
        ));

        Ok(Self {
            child,
            _pty_master: pty_master,
            pty_writer: writer,
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
        self.pty_writer.lock().unwrap().take();

        let _ = self.health_tx.send(ServiceState::Stopped);
    }

    fn monitor_logs<R>(
        mut reader: R,
        writer: Arc<StdMutex<Option<Box<dyn Write + Send>>>>,
        buffer: LogBuffer,
    ) where
        R: Read + Send,
    {
        let rt = tokio::runtime::Handle::current();
        let cursor_query = b"\x1b[6n";
        let cursor_response = b"\x1b[1;1R";
        let mut pending = Vec::new();
        let mut line = Vec::new();
        let mut chunk = [0; 8192];

        loop {
            let count = match reader.read(&mut chunk) {
                Ok(0) => break,
                Ok(count) => count,
                Err(error) => {
                    eprintln!("Failed to read PTY output: {error}");
                    break;
                }
            };

            for byte in &chunk[..count] {
                pending.push(*byte);
                while !cursor_query.starts_with(&pending) {
                    let byte = pending.remove(0);
                    if byte == b'\n' {
                        if line.last() == Some(&b'\r') {
                            line.pop();
                        }
                        rt.block_on(buffer.push(String::from_utf8_lossy(&line).into_owned()));
                        line.clear();
                    } else {
                        line.push(byte);
                    }
                }

                if pending == cursor_query {
                    let response_result = match writer.lock().unwrap().as_mut() {
                        Some(writer) => writer.write_all(cursor_response),
                        None => return,
                    };
                    if let Err(error) = response_result {
                        eprintln!("Failed to respond to PTY cursor-position query: {error}");
                        return;
                    }
                    pending.clear();
                }
            }
        }

        line.extend(pending);
        if !line.is_empty() {
            rt.block_on(buffer.push(String::from_utf8_lossy(&line).into_owned()));
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
        pty_writer: Arc<StdMutex<Option<Box<dyn Write + Send>>>>,
        health_tx: watch::Sender<ServiceState>,
        health_rx: watch::Receiver<ServiceState>,
    ) {
        while !health_rx.borrow().is_dead() {
            tokio::time::sleep(Duration::from_millis(500)).await;

            let mut guard = child.lock().await;
            match guard.try_wait() {
                Ok(Some(status)) => {
                    drop(guard);
                    pty_writer.lock().unwrap().take();
                    let new_state = if status.success() {
                        ServiceState::Stopped
                    } else {
                        println!("Child exited with status: {status}");
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
                    pty_writer.lock().unwrap().take();
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
        self.pty_writer.lock().unwrap().take();

        // Kill all the worker threads
        self.log_thread.abort();
        self.health_check_thread.abort();
        self.exit_monitor_thread.abort();
    }
}

use std::{path::PathBuf, sync::Arc};

use anyhow::Result;
use common::structure::Service;
use tokio::{
    sync::{Mutex, watch},
    task::JoinHandle,
};

use crate::{
    dependency_list::DependencyList, log::LogBuffer, process::Process, service_state::ServiceState,
};

#[derive(Debug)]
pub struct ManagedService {
    child: Arc<Mutex<Option<Process>>>,
    service: Box<Service>,
    dependencies: DependencyList,
    cwd: Arc<PathBuf>,
    log_buffer: LogBuffer,
    health_tx: watch::Sender<ServiceState>,
    health_rx: watch::Receiver<ServiceState>,
    startup_handle: Option<JoinHandle<Result<()>>>,
}

impl ManagedService {
    pub fn new(
        service: Service,
        dependencies: DependencyList,
        cwd: Arc<PathBuf>,
        health_tx: watch::Sender<ServiceState>,
        health_rx: watch::Receiver<ServiceState>,
    ) -> Self {
        Self {
            child: Arc::new(Mutex::new(None)),
            service: Box::new(service),
            dependencies,
            cwd,
            log_buffer: LogBuffer::new(2000),
            health_tx,
            health_rx,
            startup_handle: None,
        }
    }

    /// Start a new ManagedService.
    pub fn start(&mut self) -> Result<()> {
        let child = self.child.clone();
        let service = self.service.clone();
        let mut dependencies = self.dependencies.clone();
        let cwd = self.cwd.clone();
        let log_buffer = self.log_buffer.clone();
        let health_tx = self.health_tx.clone();
        let health_rx = self.health_rx.clone();

        let handle = tokio::spawn(async move {
            dependencies.wait_for_healthy().await;
            let process = Process::start(*service, log_buffer, &*cwd, health_tx, health_rx).await;

            match process {
                Ok(p) => {
                    (*child.lock().await) = Some(p);
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

        if let Some(mut child) = lock.take() {
            child.stop().await;
        }
    }

    pub fn status(&self) -> ServiceState {
        *self.health_rx.borrow()
    }

    pub fn log(&self) -> &LogBuffer {
        &self.log_buffer
    }

    pub async fn wait_for_healthy(&mut self) -> Result<()> {
        let rx = &mut self.health_rx;

        // Check if it's already healthy
        if rx.borrow().is_available() {
            return Ok(());
        }

        // Otherwise, wait for a change until it's healthy
        while rx.changed().await.is_ok() {
            if rx.borrow().is_available() {
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

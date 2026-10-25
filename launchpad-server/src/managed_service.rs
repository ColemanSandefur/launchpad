use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result};
use globset::{Glob, GlobSet, GlobSetBuilder};
use launchpad_common::structure::{Service, Watch};
use notify::{RecommendedWatcher, Watcher};
use tokio::{
    sync::{Mutex, watch},
    task::JoinHandle,
};

use crate::{
    dependency_list::DependencyList, log::LogBuffer, process::Process, service_state::ServiceState,
};

fn build_glob<I, S>(items: I) -> Result<GlobSet>
where
    I: Iterator<Item = S>,
    S: AsRef<str>,
{
    let mut builder = GlobSetBuilder::new();

    for item in items {
        let glob = Glob::new(item.as_ref())
            .with_context(|| anyhow::anyhow!("Invalid glob pattern: {}", item.as_ref()))?;

        builder.add(glob);
    }

    Ok(builder.build()?)
}

#[derive(Debug)]
struct WatcherData {
    watcher: RecommendedWatcher,
    rx: watch::Receiver<()>,
}

fn build_watcher(watch: &Watch) -> Result<WatcherData> {
    let extensions = watch.extensions.clone().into_iter().collect::<HashSet<_>>();
    let paths = watch.paths.clone();

    let ignore = build_glob(watch.ignore.iter())?;

    let (tx, rx) = watch::channel(());

    let tx_clone = tx.clone();

    let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let should_change: Box<dyn Fn(&Path) -> bool> = {
            match extensions.is_empty() {
                true => Box::new(|path: &Path| !ignore.is_match(path)),
                false => Box::new(|path: &Path| {
                    let str = path
                        .extension()
                        .and_then(|o| o.to_os_string().into_string().ok());

                    str.map(|s| extensions.contains(&s)).unwrap_or_default()
                        && !ignore.is_match(path)
                }),
            }
        };

        if let Ok(event) = res {
            if event.kind.is_modify() || event.kind.is_create() {
                println!("Detected change of file: {:?}", event.paths);
                let matches = event
                    .paths
                    .iter()
                    .flat_map(|p| paths.iter().filter_map(|base| p.strip_prefix(base).ok()))
                    .any(should_change);
                if matches {
                    println!("Triggering change!");
                    let _ = tx_clone.send(());
                }
            }
        }
    })?;

    Ok(WatcherData { watcher, rx })
}

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
    watcher: Option<WatcherData>,
    watch_handle: Option<JoinHandle<()>>,
}

impl ManagedService {
    pub fn new(
        service: Service,
        dependencies: DependencyList,
        cwd: Arc<PathBuf>,
        health_tx: watch::Sender<ServiceState>,
        health_rx: watch::Receiver<ServiceState>,
    ) -> Result<Self> {
        let mut watcher = match &service.watch {
            Some(w) => Some(build_watcher(w)?),
            None => None,
        };

        if let (Some(watcher), Some(w)) = (&mut watcher, &service.watch) {
            for path in &w.paths {
                println!("Watching path: {:?}", path.as_os_str().to_str());
                let _ = watcher
                    .watcher
                    .watch(path, notify::RecursiveMode::Recursive);
            }
        }

        let mut s = Self {
            child: Arc::new(Mutex::new(None)),
            service: Box::new(service),
            dependencies,
            cwd,
            log_buffer: LogBuffer::new(2000),
            health_tx,
            health_rx,
            startup_handle: None,
            watcher,
            watch_handle: None,
        };

        s.spawn_watch_loop();

        Ok(s)
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

    fn spawn_watch_loop(&mut self) {
        let watcher_data = match &self.watcher {
            Some(w) => w,
            None => return,
        };

        let mut rx = watcher_data.rx.clone();
        let child_ref = self.child.clone();
        let service = self.service.clone();
        let mut dependencies = self.dependencies.clone();
        let cwd = self.cwd.clone();
        let log_buffer = self.log_buffer.clone();
        let health_tx = self.health_tx.clone();
        let health_rx = self.health_rx.clone();

        let handle = tokio::spawn(async move {
            while let Ok(_) = rx.changed().await {
                println!("File change detected via watcher. Restarting service...");

                // 1. Stop the current process if it's running
                {
                    let mut lock = child_ref.lock().await;
                    if let Some(mut proc) = lock.take() {
                        proc.stop().await;
                    }
                }

                // Brief pause before restarting
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;

                // 2. Wait for dependencies again if necessary, then restart
                dependencies.wait_for_healthy().await;

                match Process::start(
                    *service.clone(),
                    log_buffer.clone(),
                    &*cwd,
                    health_tx.clone(),
                    health_rx.clone(),
                )
                .await
                {
                    Ok(p) => {
                        *child_ref.lock().await = Some(p);
                        println!("Service successfully restarted after file change.");
                    }
                    Err(e) => {
                        eprintln!("Failed to restart service after change: {e:#?}");
                    }
                }
            }
        });

        self.watch_handle = Some(handle);
    }
}

impl Drop for ManagedService {
    fn drop(&mut self) {
        self.startup_handle.as_mut().map(|h| h.abort());
    }
}

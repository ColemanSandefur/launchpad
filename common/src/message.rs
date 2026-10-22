use std::{collections::HashMap, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::structure::Structure;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum ServerRequest {
    Initial {
        id: String,
    },
    Status,
    Up {
        structure: Structure,
        cwd: PathBuf,
    },
    Down,
    ServiceLog {
        service_name: String,
        start_idx: Option<usize>,
    },
    Ping,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum ServerResponse {
    Initialized,
    Acknowledge,
    StatusResponse(StatusResponse),
    ServiceLogResponse(ServiceLogResponse),
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct StatusResponse {
    pub services: HashMap<String, ServiceStatus>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ServiceLogResponse {
    pub service_name: String,
    pub lines: Vec<String>,
    pub next_idx: usize,
    pub dropped_history: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ServiceStatus {
    pub state: ServiceState,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum ServiceState {
    /// Service has not yet started or is waiting for dependencies.
    Pending,
    /// Process has been spawned, but health checks have not yet passed.
    Starting,
    /// Process is running and health checks are fully passing.
    Healthy,
    /// Process is running, but health checks are failing.
    Unhealthy,
    /// Service is intentionally shutting down.
    Stopping,
    /// Service process has exited successfully or was stopped.
    Stopped,
    /// Service process crashed or failed to start.
    Failed,
}

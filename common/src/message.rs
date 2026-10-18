use std::{collections::HashMap, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::structure::Structure;

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
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
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
pub enum ServerResponse {
    Initialized,
    StatusResponse {
        services: HashMap<String, ServiceStatus>,
    },
    ServiceLogResponse {
        service_name: String,
        lines: Vec<String>,
        next_idx: usize,
        dropped_history: bool,
    },
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct ServiceStatus {
    pub running: bool,
    pub state: ServiceState,
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
pub enum ServiceState {
    Healthy,
    Unhealthy,
}

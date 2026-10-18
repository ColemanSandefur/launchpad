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
        services: HashMap<String, bool>,
    },
    ServiceLogResponse {
        service_name: String,
        lines: Vec<String>,
        next_idx: usize,
        dropped_history: bool,
    },
}

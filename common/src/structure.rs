use std::{collections::HashMap, time::Duration};

use serde::{Deserialize, Serialize};
use tokio::process::Command;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Structure {
    pub services: HashMap<String, Service>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Service {
    pub program: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub after: Vec<String>,
    pub health_check: Option<HealthCheck>,
}

impl Service {
    pub fn to_command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command.args(&self.args);
        command.envs(&self.env);

        command
    }
}

impl Into<Command> for Service {
    fn into(self) -> Command {
        self.to_command()
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct HealthCheck {
    /// The specific probe to use
    pub probe: HealthCheckProbe,

    /// Duration before first probe
    pub initial_delay: Duration,

    /// Duration between probes
    pub period: Duration,

    /// Duration before probe times out
    pub probe_timeout: Duration,

    /// Consecutive successes needed
    pub success_threshold: u32,

    /// Consecutive failures before unhealthy
    pub failure_threshold: u32,

    /// Overall deadline duration for the process to become ready.
    pub timeout: Option<Duration>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum HealthCheckProbe {
    Http {
        /// The port to connect to.
        ///
        /// The HTTP port to connect to - e.g. `8080`
        port: u16,

        /// The path to connect to.
        ///
        /// The path to connect to - e.g. `/health`
        path: String,

        /// The host to connect to.
        ///
        /// The host to connect to - e.g. `127.0.0.1`
        host: String,

        /// The schema to use.
        ///
        /// The schema to connect to - e.g. `http`
        schema: String,
    },
}

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use tokio::process::Command;

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct Structure {
    pub services: HashMap<String, Service>,
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct Service {
    pub program: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
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

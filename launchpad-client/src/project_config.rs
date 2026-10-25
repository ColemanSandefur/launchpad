use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use launchpad_common::structure::{Service, Structure};
use mlua::{Lua, LuaSerdeExt};
use path_absolutize::Absolutize;
use serde::{Deserialize, Serialize};
use tokio::fs;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ProjectConfig {
    pub services: HashMap<String, ServiceConfig>,
}

impl ProjectConfig {
    pub async fn load(path: impl AsRef<Path>) -> Result<ProjectConfig> {
        let lua = Lua::new();

        let file_content = fs::read_to_string(path)
            .await
            .context("Failed to load config.lua")?;

        let lua_val = lua.load(&file_content).eval()?;

        let config: ProjectConfig = lua.from_value(lua_val)?;

        config.validate().context("Failed to pass validation")?;

        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        let service_names: HashSet<&String> = self.services.keys().collect();

        // 1. Check that all dependencies exist
        for (name, service) in &self.services {
            for dep in &service.after {
                if !service_names.contains(dep) {
                    bail!(
                        "Configuration error: Service '{}' depends on '{}', but '{}' does not exist in the configuration.",
                        name,
                        dep,
                        dep
                    );
                }
            }
        }

        // 2. Check for circular dependencies (DAG validation)
        self.check_for_cycles()?;

        Ok(())
    }

    fn check_for_cycles(&self) -> Result<()> {
        #[derive(Clone, Copy)]
        enum Visited {
            /// Unvisited
            NotVisited,
            /// Current recursion stack
            Visiting,
            /// Fully Visited
            Visited,
        }

        let mut visited: HashMap<&str, Visited> = HashMap::new();

        fn dfs<'a>(
            name: &'a str,
            services: &'a HashMap<String, ServiceConfig>,
            visited: &mut HashMap<&'a str, Visited>,
        ) -> Result<()> {
            match visited.get(name).copied().unwrap_or(Visited::NotVisited) {
                Visited::Visiting => {
                    bail!("Circular dependency detected involving service: '{}'", name)
                }
                Visited::Visited => return Ok(()), // Already fully checked
                Visited::NotVisited => {}
            }

            visited.insert(name, Visited::Visiting);

            if let Some(service) = services.get(name) {
                for dep in &service.after {
                    dfs(dep, services, visited)?;
                }
            }

            visited.insert(name, Visited::Visited);
            Ok(())
        }

        for name in self.services.keys() {
            dfs(name, &self.services, &mut visited)?;
        }

        Ok(())
    }

    pub fn to_structure(self, config_path: &Path) -> Structure {
        Structure {
            services: self
                .services
                .into_iter()
                .map(|(name, service)| (name.clone(), service.to_service(config_path)))
                .collect(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ServiceConfig {
    pub command: String,

    #[serde(default)]
    pub args: Vec<String>,

    #[serde(default)]
    pub env_file: Option<String>,

    #[serde(default)]
    pub env: HashMap<String, String>,

    #[serde(default)]
    pub after: Vec<String>,

    #[serde(default)]
    pub health_check: Option<HealthCheck>,

    #[serde(default)]
    pub watch: Option<Watch>,
}

impl ServiceConfig {
    pub fn resolve_env(&self) -> HashMap<String, String> {
        let mut env = self.env.clone();

        if let Some(env_file) = &self.env_file {
            match dotenvy::from_filename_iter(env_file) {
                Ok(iter) => {
                    for item in iter {
                        match item {
                            Ok((key, val)) => {
                                env.entry(key).or_insert(val);
                            }
                            Err(e) => eprintln!("Parsing error in .env: {e}"),
                        }
                    }
                }
                Err(_) => panic!("Failed to find the file: {:?}", env_file),
            }
        }

        env
    }

    pub fn to_service(self, config_path: &Path) -> Service {
        Service {
            env: self.resolve_env(),
            program: self.command,
            args: self.args,
            after: self.after,
            health_check: self.health_check.map(|h| h.into()),
            watch: self.watch.map(|w| w.to_watch(config_path)),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct HealthCheck {
    /// The specific probe to use
    pub probe: HealthCheckProbe,

    /// Seconds before first probe (default: 0)
    #[serde(default = "default_initial_delay")]
    pub initial_delay: u32,

    /// Seconds between probes (default: 10)
    #[serde(default = "default_period")]
    pub period: u32,

    /// Seconds before probe times out (default: 1)
    #[serde(default = "default_probe_timeout")]
    pub probe_timeout: u32,

    /// Consecutive successes needed (default: 1)
    #[serde(default = "default_success_threshold")]
    pub success_threshold: u32,

    /// Consecutive failures before unhealthy (default: 3).
    #[serde(default = "default_failure_threshold")]
    pub failure_threshold: u32,

    /// Overall deadline in seconds for the process to become ready.
    #[serde(default)]
    pub timeout: Option<u32>,
}

fn default_initial_delay() -> u32 {
    0
}

fn default_period() -> u32 {
    10
}

fn default_probe_timeout() -> u32 {
    1
}

fn default_success_threshold() -> u32 {
    1
}

fn default_failure_threshold() -> u32 {
    3
}

impl From<HealthCheck> for launchpad_common::structure::HealthCheck {
    fn from(value: HealthCheck) -> Self {
        launchpad_common::structure::HealthCheck {
            probe: value.probe.into(),
            initial_delay: Duration::from_secs(value.initial_delay.into()),
            period: Duration::from_secs(value.period.into()),
            probe_timeout: Duration::from_secs(value.probe_timeout.into()),
            success_threshold: value.success_threshold,
            failure_threshold: value.failure_threshold,
            timeout: value.timeout.map(|t| Duration::from_secs(t.into())),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
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
        #[serde(default = "default_host")]
        host: String,

        /// The schema to use.
        ///
        /// The schema to connect to - e.g. `http`
        #[serde(default = "default_schema")]
        schema: String,
    },
}

fn default_host() -> String {
    "127.0.0.1".to_string()
}

fn default_schema() -> String {
    "http".to_string()
}

impl From<HealthCheckProbe> for launchpad_common::structure::HealthCheckProbe {
    fn from(value: HealthCheckProbe) -> Self {
        match value {
            HealthCheckProbe::Http {
                port,
                path,
                host,
                schema,
            } => launchpad_common::structure::HealthCheckProbe::Http {
                port,
                path,
                host,
                schema,
            },
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Watch {
    #[serde(default)]
    paths: Vec<String>,
    #[serde(default)]
    extensions: Vec<String>,
    #[serde(default)]
    ignore: Vec<String>,
}

impl Watch {
    pub fn to_watch(self, config_path: &Path) -> launchpad_common::structure::Watch {
        let root_dir = config_path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("Failed to get the parent directory"))
            .unwrap();

        launchpad_common::structure::Watch {
            paths: self
                .paths
                .iter()
                .map(PathBuf::from)
                .map(|p| PathBuf::from(p.absolutize_from(root_dir)))
                .collect(),
            extensions: self.extensions,
            ignore: self.ignore,
        }
    }
}

use std::{
    collections::HashMap,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use base64::prelude::*;
use clap::{Parser, Subcommand};
use common::{
    Channel,
    message::{ServerRequest, ServerResponse},
    structure::{Service, Structure},
};
use interprocess::local_socket::{
    GenericNamespaced,
    tokio::{Stream, prelude::*},
};
use mlua::{Lua, LuaSerdeExt};
use serde::Deserialize;
use tokio::fs;

#[derive(Deserialize, Debug)]
struct ProjectConfig {
    services: HashMap<String, ServiceConfig>,
}

#[derive(Deserialize, Debug)]
struct ServiceConfig {
    command: String,
    args: Vec<String>,

    #[serde(default)]
    env_file: Option<String>,

    #[serde(default)]
    env: HashMap<String, String>,
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
}

#[derive(Parser, Debug)]
struct Args {
    /// Path to the service configuration file
    #[arg(short, long, default_value = "./config.lua")]
    config: PathBuf,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Start up all the services
    Up,
    /// Stop all the services
    Down,
    /// Get the status of the services
    Status,
    /// Get the logs for a specific service
    Logs {
        name: String,
        start_idx: Option<usize>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    let current_dir = std::env::current_dir()?;

    let config_path = args.config.canonicalize()?;

    let config = load_config(&config_path).await?;

    let mut chan = init_connection(config_path.as_os_str().as_bytes()).await?;

    match args.command {
        Commands::Up => {
            let structure = Structure {
                services: config
                    .services
                    .iter()
                    .map(|(name, service)| {
                        (
                            name.clone(),
                            Service {
                                program: service.command.clone(),
                                args: service.args.clone(),
                                env: service.resolve_env(),
                            },
                        )
                    })
                    .collect(),
            };
            chan.send(&ServerRequest::Up {
                structure,
                cwd: current_dir,
            })
            .await
            .unwrap();
            chan.receive::<ServerResponse>().await.unwrap().unwrap();
        }
        Commands::Down => {
            chan.send(&ServerRequest::Down).await.unwrap();
            chan.receive::<ServerResponse>().await.unwrap().unwrap();
        }
        Commands::Status => {
            chan.send(&ServerRequest::Status).await?;

            let ServerResponse::StatusResponse { services } =
                chan.receive::<ServerResponse>().await.unwrap().unwrap()
            else {
                panic!()
            };

            println!("Status: {:?}", services);
        }
        Commands::Logs { name, start_idx } => {
            chan.send(&ServerRequest::ServiceLog {
                service_name: name,
                start_idx,
            })
            .await
            .unwrap();

            let ServerResponse::ServiceLogResponse { lines, .. } =
                chan.receive::<ServerResponse>().await.unwrap().unwrap()
            else {
                panic!()
            };

            println!("{}", lines.join("\n"));
        }
    }

    Ok(())
}

async fn init_connection(id: impl AsRef<[u8]>) -> Result<Channel> {
    let printname = "example.sock";
    let name = printname.to_ns_name::<GenericNamespaced>()?;

    let conn = Stream::connect(name).await?;

    let mut chan = Channel::from_stream(conn);

    let message = ServerRequest::Initial {
        id: BASE64_STANDARD.encode(id),
    };

    chan.send(&message).await?;

    let ServerResponse::Initialized = chan.receive::<ServerResponse>().await.unwrap().unwrap()
    else {
        anyhow::bail!("Server unreachable")
    };

    Ok(chan)
}

async fn load_config(path: impl AsRef<Path>) -> Result<ProjectConfig> {
    let lua = Lua::new();

    let file_content = fs::read_to_string(path)
        .await
        .context("Failed to load config.lua")?;

    let lua_val = lua.load(&file_content).eval()?;

    Ok(lua.from_value(lua_val)?)
}

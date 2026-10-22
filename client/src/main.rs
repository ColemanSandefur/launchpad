mod api;
mod channel_id;
mod connection_manager;
pub mod project_config;
pub mod ui;

use std::{os::unix::ffi::OsStrExt, path::PathBuf};

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::{api::Api, channel_id::ChannelId, project_config::ProjectConfig};

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
    Up {
        /// Start the services without attaching to the TUI
        #[arg(short, long)]
        detach: bool,
    },
    /// Stop all the services
    Down,
    /// Get the status of the services
    Status,
    /// Get the logs for a specific service
    Logs {
        name: String,
        start_idx: Option<usize>,
    },
    /// Checks if the given config is valid
    Validate,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    let current_dir = std::env::current_dir()?;

    let config_path = args.config.canonicalize()?;

    let config = ProjectConfig::load(&config_path).await?;

    let id = ChannelId::encode(config_path.as_os_str().as_bytes());
    let mut api = Api::connect(id.clone()).await?;

    match args.command {
        Commands::Up { detach } => {
            let is_running = !api.get_status().await?.services.is_empty();

            match is_running {
                true => println!(
                    "The service was already running.\nIf you want to publish new changes, take the services down and back up."
                ),
                false => api.up(config.into(), current_dir).await?,
            }

            if !detach {
                crate::ui::run(id).await?;
            }
        }
        Commands::Down => {
            api.down().await?;
        }
        Commands::Status => {
            let services = api.get_status().await?.services;

            println!("Status: {:#?}", services);
        }
        Commands::Logs { name, start_idx } => {
            let lines = api.logs(name, start_idx).await?.lines;

            println!("{}", lines.join("\n"));
        }
        Commands::Validate => {
            println!("Valid!");
            println!("{config:#?}");
        }
    }

    Ok(())
}

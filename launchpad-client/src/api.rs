use std::path::PathBuf;

use interprocess::local_socket::{
    GenericNamespaced,
    tokio::{Stream, prelude::*},
};
use launchpad_common::{
    Channel, ReceiveError, SendError,
    message::{ServerRequest, ServerResponse, ServiceLogResponse, StatusResponse},
    structure::Structure,
};
use thiserror::Error;

use crate::channel_id::ChannelId;

#[derive(Debug)]
pub struct Api {
    /// The acutal identifier sent through the channel
    id: ChannelId,

    /// The full channel
    channel: Channel,
}

impl Api {
    pub async fn connect(id: impl Into<ChannelId>) -> anyhow::Result<Self> {
        let id = id.into();

        let printname = "example.sock";
        let name = printname.to_ns_name::<GenericNamespaced>()?;

        let conn = match Stream::connect(name.clone()).await {
            Ok(c) => c,
            Err(_e) => {
                println!("Server not running. Launching launchpad-server...");

                // Spawn the server binary in the background
                #[cfg(debug_assertions)]
                {
                    use std::process::Stdio;

                    std::process::Command::new("../target/debug/launchpad-server")
                        .stdin(Stdio::null())
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .spawn()?;
                }
                #[cfg(not(debug_assertions))]
                {
                    use std::process::Stdio;

                    std::process::Command::new("launchpad-server")
                        .stdin(Stdio::null())
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .spawn()?;
                }

                // Retry connecting while the server spins up and binds the socket
                let mut connected_stream = None;
                for _ in 0..10 {
                    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                    if let Ok(c) = Stream::connect(name.clone()).await {
                        connected_stream = Some(c);
                        break;
                    }
                }

                connected_stream.ok_or_else(|| {
                    anyhow::anyhow!("Failed to connect to launchpad-server after auto-launching")
                })?
            }
        };

        let mut channel = Channel::from_stream(conn);

        let message = ServerRequest::Initial {
            id: id.clone().into(),
        };

        channel.send(&message).await?;

        let ServerResponse::Initialized =
            channel.receive::<ServerResponse>().await.unwrap().unwrap()
        else {
            anyhow::bail!("Server unreachable")
        };

        Ok(Self { id, channel })
    }

    pub async fn clone(&self) -> anyhow::Result<Self> {
        Self::connect(self.id.clone()).await
    }

    pub async fn ping(&mut self) -> Result<(), ApiError> {
        self.channel.send(&ServerRequest::Ping).await?;

        let response = match self.channel.receive::<ServerResponse>().await {
            Some(val) => val?,
            None => {
                return Err(ApiError::ConnectionClosed(anyhow::anyhow!(
                    "Connection closed"
                )));
            }
        };

        match response {
            ServerResponse::Acknowledge => Ok(()),
            x => Err(ApiError::InvalidResponse(anyhow::anyhow!(
                "Invalid response received: {x:?}"
            ))),
        }
    }

    pub async fn get_status(&mut self) -> Result<StatusResponse, ApiError> {
        self.channel.send(&ServerRequest::Status).await?;

        let response = match self.channel.receive::<ServerResponse>().await {
            Some(val) => val?,
            None => {
                return Err(ApiError::ConnectionClosed(anyhow::anyhow!(
                    "Connection closed"
                )));
            }
        };

        match response {
            ServerResponse::StatusResponse(status) => Ok(status),
            x => Err(ApiError::InvalidResponse(anyhow::anyhow!(
                "Invalid response received: {x:?}"
            ))),
        }
    }

    pub async fn up(
        &mut self,
        structure: Structure,
        cwd: impl Into<PathBuf>,
    ) -> Result<(), ApiError> {
        self.channel
            .send(&ServerRequest::Up {
                structure,
                cwd: cwd.into(),
            })
            .await?;

        let response = match self.channel.receive::<ServerResponse>().await {
            Some(val) => val?,
            None => {
                return Err(ApiError::ConnectionClosed(anyhow::anyhow!(
                    "Connection closed"
                )));
            }
        };

        match response {
            ServerResponse::Acknowledge => Ok(()),
            x => Err(ApiError::InvalidResponse(anyhow::anyhow!(
                "Invalid response received: {x:?}"
            ))),
        }
    }

    pub async fn down(&mut self) -> Result<(), ApiError> {
        self.channel.send(&ServerRequest::Down).await?;

        let response = match self.channel.receive::<ServerResponse>().await {
            Some(val) => val?,
            None => {
                return Err(ApiError::ConnectionClosed(anyhow::anyhow!(
                    "Connection closed"
                )));
            }
        };

        match response {
            ServerResponse::Acknowledge => Ok(()),
            x => Err(ApiError::InvalidResponse(anyhow::anyhow!(
                "Invalid response received: {x:?}"
            ))),
        }
    }

    pub async fn logs(
        &mut self,
        service_name: impl Into<String>,
        start_idx: Option<usize>,
    ) -> Result<ServiceLogResponse, ApiError> {
        self.channel
            .send(&ServerRequest::ServiceLog {
                service_name: service_name.into(),
                start_idx,
            })
            .await?;

        let response = match self.channel.receive::<ServerResponse>().await {
            Some(val) => val?,
            None => {
                return Err(ApiError::ConnectionClosed(anyhow::anyhow!(
                    "Connection closed"
                )));
            }
        };

        match response {
            ServerResponse::ServiceLogResponse(response) => Ok(response),
            x => Err(ApiError::InvalidResponse(anyhow::anyhow!(
                "Invalid response received: {x:?}"
            ))),
        }
    }

    pub async fn start(&mut self, service_name: impl Into<String>) -> Result<(), ApiError> {
        self.channel
            .send(&ServerRequest::Start {
                service_name: service_name.into(),
            })
            .await?;

        let response = match self.channel.receive::<ServerResponse>().await {
            Some(val) => val?,
            None => {
                return Err(ApiError::ConnectionClosed(anyhow::anyhow!(
                    "Connection closed"
                )));
            }
        };

        match response {
            ServerResponse::Acknowledge => Ok(()),
            x => Err(ApiError::InvalidResponse(anyhow::anyhow!(
                "Invalid response received: {x:?}"
            ))),
        }
    }

    pub async fn stop(&mut self, service_name: impl Into<String>) -> Result<(), ApiError> {
        self.channel
            .send(&ServerRequest::Stop {
                service_name: service_name.into(),
            })
            .await?;

        let response = match self.channel.receive::<ServerResponse>().await {
            Some(val) => val?,
            None => {
                return Err(ApiError::ConnectionClosed(anyhow::anyhow!(
                    "Connection closed"
                )));
            }
        };

        match response {
            ServerResponse::Acknowledge => Ok(()),
            x => Err(ApiError::InvalidResponse(anyhow::anyhow!(
                "Invalid response received: {x:?}"
            ))),
        }
    }
}

#[derive(Error, Debug)]
pub enum ApiError {
    #[error("Failed to send: {0:?}")]
    Send(#[from] SendError),
    #[error("Failed to receive: {0:?}")]
    Receive(#[from] ReceiveError),
    #[error("Invalid response: {0:?}")]
    InvalidResponse(anyhow::Error),
    #[error("Connection closed: {0:?}")]
    ConnectionClosed(anyhow::Error),
}

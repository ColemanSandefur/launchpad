use std::path::PathBuf;

use base64::prelude::*;
use common::{
    Channel, ReceiveError, SendError,
    message::{ServerRequest, ServerResponse, ServiceLogResponse, StatusResponse},
    structure::Structure,
};
use interprocess::local_socket::{
    GenericNamespaced,
    tokio::{Stream, prelude::*},
};
use thiserror::Error;

#[derive(Debug)]
pub struct Api {
    channel: Channel,
}

impl Api {
    pub async fn connect(id: impl AsRef<[u8]>) -> anyhow::Result<Self> {
        let printname = "example.sock";
        let name = printname.to_ns_name::<GenericNamespaced>()?;

        let conn = Stream::connect(name).await?;

        let mut channel = Channel::from_stream(conn);

        let message = ServerRequest::Initial {
            id: BASE64_STANDARD.encode(id),
        };

        channel.send(&message).await?;

        let ServerResponse::Initialized =
            channel.receive::<ServerResponse>().await.unwrap().unwrap()
        else {
            anyhow::bail!("Server unreachable")
        };

        Ok(Self { channel })
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
            ServerResponse::Initialized => Ok(()),
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
            ServerResponse::Initialized => Ok(()),
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

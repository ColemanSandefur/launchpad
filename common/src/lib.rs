#![feature(core_io)]

use futures::{SinkExt, StreamExt};
use interprocess::local_socket::tokio::Stream;
use serde::de::DeserializeOwned;
use thiserror::Error;
use tokio_util::codec::{Framed, LengthDelimitedCodec};

pub mod message;
pub mod structure;

#[derive(Error, Debug)]
pub enum SendError {
    #[error("A postcard error occurred: {0}")]
    Postcard(#[from] postcard::Error),

    #[error("An IO error occurred: {0}")]
    Io(#[from] core::io::Error),
}

pub struct Channel {
    channel: Framed<Stream, LengthDelimitedCodec>,
}

impl Channel {
    pub fn from_stream(stream: Stream) -> Self {
        let channel = LengthDelimitedCodec::builder().new_framed(stream);

        Self { channel }
    }

    pub async fn send<T>(&mut self, data: &T) -> Result<(), SendError>
    where
        T: serde::Serialize,
    {
        let bytes = postcard::to_stdvec(data)?;

        println!("Sending {} bytes", bytes.len());

        self.channel.send(bytes.as_slice()).await?;

        Ok(())
    }

    pub async fn receive<T>(&mut self) -> Option<Result<T, SendError>>
    where
        T: DeserializeOwned,
    {
        if let Some(frame_result) = self.channel.next().await {
            let bytes = match frame_result {
                Ok(b) => b,
                Err(err) => return Some(Err(err.into())),
            };

            println!("Received {} bytes", bytes.len());

            let msg = match postcard::from_bytes(&bytes) {
                Ok(val) => val,
                Err(err) => return Some(Err(err.into())),
            };

            Some(Ok(msg))
        } else {
            None
        }
    }
}

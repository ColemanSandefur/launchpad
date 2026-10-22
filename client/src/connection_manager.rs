use bb8::ManageConnection;

use crate::{
    api::{Api, ApiError},
    channel_id::ChannelId,
};

pub struct ConnectionManager {
    channel_id: ChannelId,
}

impl ConnectionManager {
    pub fn new(channel_id: ChannelId) -> Self {
        Self {
            channel_id: channel_id,
        }
    }
}

impl ManageConnection for ConnectionManager {
    type Connection = Api;
    type Error = ApiError;

    async fn connect(&self) -> Result<Self::Connection, Self::Error> {
        // Calls your existing Api::connect method
        Api::connect(self.channel_id.clone())
            .await
            .map_err(|e| ApiError::ConnectionClosed(e))
    }

    async fn is_valid(&self, conn: &mut Self::Connection) -> Result<(), Self::Error> {
        // Perform a lightweight health check using your status method.
        // If this fails, bb8 will discard the connection and open a new one.
        conn.ping().await.map(|_| ())
    }

    fn has_broken(&self, _conn: &mut Self::Connection) -> bool {
        // Return true if you can explicitly tell the connection is dead
        false
    }
}

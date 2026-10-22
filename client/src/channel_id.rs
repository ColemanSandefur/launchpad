use base64::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ChannelId(String);

impl ChannelId {
    pub fn encode(id: impl AsRef<[u8]>) -> Self {
        Self(BASE64_STANDARD.encode(id))
    }
}

impl Into<String> for ChannelId {
    fn into(self) -> String {
        self.0
    }
}

impl<T> From<T> for ChannelId
where
    T: AsRef<[u8]>,
{
    fn from(value: T) -> Self {
        ChannelId::encode(value)
    }
}

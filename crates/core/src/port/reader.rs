use jiff::Zoned;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct CardId(pub String); // FeliCa IDm の hex 表現

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReaderStatus {
    Disconnected,
    Connecting,
    Ready,
    Error(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardScanned {
    pub card_id: CardId,
    pub scanned_at: Zoned, // Asia/Tokyo aware
}

#[derive(Debug, thiserror::Error)]
pub enum ReaderError {
    #[error("reader not connected")]
    NotConnected,
    #[error("pcsc error: {0}")]
    Pcsc(String),
    #[error("other: {0}")]
    Other(String),
}

#[async_trait::async_trait]
pub trait ReaderBackend: Send + Sync {
    async fn start(&self) -> Result<(), ReaderError>;
    async fn stop(&self) -> Result<(), ReaderError>;
    fn status(&self) -> ReaderStatus;
    fn subscribe(&self) -> tokio::sync::broadcast::Receiver<CardScanned>;
}

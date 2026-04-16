use async_trait::async_trait;
use jiff::Zoned;
use uuid::Uuid;

use crate::port::reader::CardId;

#[derive(Debug, Clone)]
pub enum NotifyEvent {
    UnregisteredCardDetected {
        card_id: CardId,
        at: Zoned,
    },
    MissingPunchSuspected {
        employee_id: Uuid,
        at: Zoned,
    },
    AdminCorrectionApplied {
        actor: Uuid,
        target_punch: Uuid,
    },
    DailyClosingResult {
        date: jiff::civil::Date,
        summary: String,
    },
    ShiftPublished {
        target_month: crate::domain::time::YearMonth,
    },
    LineworksResponse {
        user_id: String,
        text: String,
    },
    // 将来拡張
}

#[derive(Debug, thiserror::Error)]
pub enum NotifyError {
    #[error("network error: {0}")]
    Network(String),
    #[error("api error: {0}")]
    Api(String),
    #[error("other: {0}")]
    Other(String),
}

#[async_trait]
pub trait Notifier: Send + Sync {
    /// **非同期 fire-and-forget**。この関数のエラーは打刻処理を失敗させてはならない。
    async fn notify(&self, event: NotifyEvent) -> Result<(), NotifyError>;
}

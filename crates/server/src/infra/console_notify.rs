use async_trait::async_trait;
use pasori_core::port::notify::{NotifyError, NotifyEvent, Notifier};

pub struct ConsoleNotifier;

#[async_trait]
impl Notifier for ConsoleNotifier {
    async fn notify(&self, event: NotifyEvent) -> Result<(), NotifyError> {
        tracing::info!(?event, "Notification event (ConsoleNotifier)");
        Ok(())
    }
}

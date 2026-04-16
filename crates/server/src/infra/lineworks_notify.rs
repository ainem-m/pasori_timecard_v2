use async_trait::async_trait;
use pasori_core::port::notify::{Notifier, NotifyError, NotifyEvent};
use serde_json::json;

pub struct LineworksNotifier {
    client: reqwest::Client,
    bot_id: String,
    api_token: String,
}

impl LineworksNotifier {
    pub fn new(bot_id: String, api_token: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            bot_id,
            api_token,
        }
    }
}

#[async_trait]
impl Notifier for LineworksNotifier {
    async fn notify(&self, event: NotifyEvent) -> Result<(), NotifyError> {
        match event {
            NotifyEvent::LineworksResponse { user_id, text } => {
                let url = format!(
                    "https://www.worksapis.com/v1.0/bots/{}/users/{}/messages",
                    self.bot_id, user_id
                );

                let payload = json!({
                    "content": {
                        "type": "text",
                        "text": text
                    }
                });

                let response = self
                    .client
                    .post(&url)
                    .bearer_auth(&self.api_token)
                    .json(&payload)
                    .send()
                    .await
                    .map_err(|e| NotifyError::Network(e.to_string()))?;

                if !response.status().is_success() {
                    let status = response.status();
                    let body = response.text().await.unwrap_or_default();
                    return Err(NotifyError::Api(format!(
                        "LINE WORKS API error: {} - {}",
                        status, body
                    )));
                }

                Ok(())
            }
            _ => {
                // TODO: 他の通知イベントのメッセージング実装
                tracing::warn!(
                    "Notification not yet implemented for LINE WORKS: {:?}",
                    event
                );
                Ok(())
            }
        }
    }
}

use async_trait::async_trait;
use jiff::Zoned;
use pasori_core::port::notify::{Notifier, NotifyError, NotifyEvent};
use pasori_core::port::repo::ExternalAccountRepository;
use serde_json::json;
use std::sync::Arc;

pub struct LineworksNotifier {
    client: reqwest::Client,
    bot_id: String,
    api_token: String,
    admin_channel_id: Option<String>,
    external_repo: Arc<dyn ExternalAccountRepository>,
}

impl LineworksNotifier {
    pub fn new(
        bot_id: String,
        api_token: String,
        external_repo: Arc<dyn ExternalAccountRepository>,
    ) -> Self {
        let admin_channel_id = std::env::var("LINEWORKS_ADMIN_CHANNEL_ID").ok();
        Self::new_with_admin_channel(bot_id, api_token, admin_channel_id, external_repo)
    }

    pub fn new_with_admin_channel(
        bot_id: String,
        api_token: String,
        admin_channel_id: Option<String>,
        external_repo: Arc<dyn ExternalAccountRepository>,
    ) -> Self {
        Self {
            client: reqwest::Client::new(),
            bot_id,
            api_token,
            admin_channel_id,
            external_repo,
        }
    }
}

#[async_trait]
impl Notifier for LineworksNotifier {
    async fn notify(&self, event: NotifyEvent) -> Result<(), NotifyError> {
        let result = build_delivery_request(&event, self.admin_channel_id.as_deref());
        let (target, text) = match result {
            Some((target, text)) => (target, text),
            None => {
                let resolved = self.resolve_recipient(&event).await;
                match resolved {
                    Some((target, text)) => (target, text),
                    None => {
                        tracing::warn!(
                            ?event,
                            "LINE WORKS notification skipped due to unresolved recipient"
                        );
                        return Ok(());
                    }
                }
            }
        };

        let url = match target {
            DeliveryTarget::User(user_id) => format!(
                "https://www.worksapis.com/v1.0/bots/{}/users/{}/messages",
                self.bot_id, user_id
            ),
            DeliveryTarget::Channel(channel_id) => format!(
                "https://www.worksapis.com/v1.0/bots/{}/channels/{}/messages",
                self.bot_id, channel_id
            ),
        };

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
}

impl LineworksNotifier {
    async fn resolve_recipient(&self, event: &NotifyEvent) -> Option<(DeliveryTarget, String)> {
        match event {
            NotifyEvent::MissingPunchSuspected { employee_id, at } => {
                let account = self
                    .external_repo
                    .find_by_employee_id("lineworks", *employee_id)
                    .await
                    .ok()
                    .flatten();
                if let Some(acc) = account {
                    Some((
                        DeliveryTarget::User(acc.external_user_id.clone()),
                        format!(
                            "【打刻漏れの疑い】\n{} 時点で打刻が確認されていません。管理者に確認してください。",
                            at.strftime("%Y-%m-%d %H:%M")
                        ),
                    ))
                } else {
                    self.admin_channel_id.as_deref().map(|channel_id| {
                        (
                            DeliveryTarget::Channel(channel_id.to_string()),
                            format!(
                                "【打刻漏れの疑れ】\n従業員 {} の {} 時点での打刻が確認されていません。",
                                employee_id,
                                at.strftime("%Y-%m-%d %H:%M")
                            ),
                        )
                    })
                }
            }
            NotifyEvent::ShiftPublished { target_month } => {
                let admin_channel_id = self.admin_channel_id.as_deref()?;
                Some((
                    DeliveryTarget::Channel(admin_channel_id.to_string()),
                    format!(
                        "【シフト公開】\n{}月のシフトが公開されました。",
                        target_month.month()
                    ),
                ))
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DeliveryTarget {
    User(String),
    Channel(String),
}

fn build_delivery_request(
    event: &NotifyEvent,
    admin_channel_id: Option<&str>,
) -> Option<(DeliveryTarget, String)> {
    match event {
        NotifyEvent::LineworksResponse { user_id, text } => {
            Some((DeliveryTarget::User(user_id.clone()), text.clone()))
        }
        NotifyEvent::UnregisteredCardDetected { card_id, at } => Some((
            DeliveryTarget::Channel(admin_channel_id?.to_string()),
            format_unregistered_card_message(card_id.0.as_str(), at),
        )),
        NotifyEvent::DailyClosingResult { date, summary } => Some((
            DeliveryTarget::Channel(admin_channel_id?.to_string()),
            format!("【日次締め結果】\n対象日: {date}\n{summary}"),
        )),
        NotifyEvent::AdminCorrectionApplied {
            actor,
            target_punch,
        } => Some((
            DeliveryTarget::Channel(admin_channel_id?.to_string()),
            format!(
                "【勤怠修正反映】\n管理者 {actor} が打刻 {target_punch} に修正を反映しました。"
            ),
        )),
        NotifyEvent::MissingPunchSuspected { .. } | NotifyEvent::ShiftPublished { .. } => None,
    }
}

fn format_unregistered_card_message(card_id: &str, at: &Zoned) -> String {
    format!(
        "【未登録カード検出】\n未登録カード ({}) が {} にスキャンされました。",
        mask_card_id(card_id),
        at.strftime("%Y-%m-%d %H:%M")
    )
}

fn mask_card_id(card_id: &str) -> String {
    let prefix: String = card_id.chars().take(4).collect();
    if card_id.chars().count() <= 4 {
        prefix
    } else {
        format!("{prefix}...")
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DeliveryTarget, build_delivery_request, format_unregistered_card_message, mask_card_id,
    };
    use jiff::{Timestamp, tz::TimeZone};
    use pasori_core::domain::time::YearMonth;
    use pasori_core::port::notify::NotifyEvent;
    use pasori_core::port::reader::CardId;
    use uuid::Uuid;

    #[test]
    // 未登録カード検出は管理者 channel 向けの通知文面に変換する。
    fn builds_channel_message_for_unregistered_card_detection() {
        let at = Timestamp::from_second(1_776_734_400)
            .expect("timestamp")
            .to_zoned(TimeZone::get("Asia/Tokyo").expect("timezone"));
        let event = NotifyEvent::UnregisteredCardDetected {
            card_id: CardId("02020212A91B9843".to_string()),
            at,
        };

        let (target, text) =
            build_delivery_request(&event, Some("admin-channel")).expect("delivery request");

        assert_eq!(target, DeliveryTarget::Channel("admin-channel".to_string()));
        assert!(text.contains("未登録カード"));
        assert!(text.contains("0202..."));
    }

    #[test]
    // 返信系イベントは本人 user 向けに配送する。
    fn builds_user_message_for_direct_response() {
        let event = NotifyEvent::LineworksResponse {
            user_id: "user-1".to_string(),
            text: "返信".to_string(),
        };

        let (target, text) =
            build_delivery_request(&event, Some("admin-channel")).expect("delivery request");

        assert_eq!(target, DeliveryTarget::User("user-1".to_string()));
        assert_eq!(text, "返信");
    }

    #[test]
    // 宛先解決情報がない本人通知イベントはこの層では配送を組み立てない。
    fn skips_events_without_resolvable_lineworks_recipient() {
        let event = NotifyEvent::ShiftPublished {
            target_month: YearMonth::new(2026, 5).expect("year month"),
        };

        assert!(build_delivery_request(&event, Some("admin-channel")).is_none());
    }

    #[test]
    // 管理者 channel が未設定なら管理者向け通知は配送しない。
    fn skips_admin_notifications_without_channel_configuration() {
        let at = Timestamp::from_second(1_776_734_400)
            .expect("timestamp")
            .to_zoned(TimeZone::get("Asia/Tokyo").expect("timezone"));
        let event = NotifyEvent::UnregisteredCardDetected {
            card_id: CardId("02020212A91B9843".to_string()),
            at,
        };

        assert!(build_delivery_request(&event, None).is_none());
    }

    #[test]
    // カード ID は先頭 4 文字だけ見せて残りを伏せる。
    fn masks_card_id_after_first_four_characters() {
        assert_eq!(mask_card_id("02020212A91B9843"), "0202...");
        assert_eq!(mask_card_id("ABCD"), "ABCD");
    }

    #[test]
    // 未登録カード文面は JST の日時を含む。
    fn formats_unregistered_card_message_with_jst_timestamp() {
        let at = Timestamp::from_second(1_776_734_400)
            .expect("timestamp")
            .to_zoned(TimeZone::get("Asia/Tokyo").expect("timezone"));

        let text = format_unregistered_card_message("02020212A91B9843", &at);

        assert!(text.contains("0202..."));
        assert!(text.contains("2026-04-21"));
    }

    #[test]
    // 管理者修正反映は管理者 channel 向けの文面に変換する。
    fn builds_channel_message_for_admin_correction_applied() {
        let actor = Uuid::now_v7();
        let target_punch = Uuid::now_v7();
        let event = NotifyEvent::AdminCorrectionApplied {
            actor,
            target_punch,
        };

        let (target, text) =
            build_delivery_request(&event, Some("admin-channel")).expect("delivery request");

        assert_eq!(target, DeliveryTarget::Channel("admin-channel".to_string()));
        assert!(text.contains(&actor.to_string()));
        assert!(text.contains(&target_punch.to_string()));
    }
}

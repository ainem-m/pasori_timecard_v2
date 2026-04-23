use axum::{
    Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::post,
};
use base64::Engine;
use hmac::{Hmac, Mac};
use jiff::{Timestamp, Zoned, tz::TimeZone};
use pasori_core::application::lineworks::{
    LineworksCommand, LineworksUseCase, decide_lineworks_request_status, parse_lineworks_command,
};
use pasori_core::domain::request::AttendanceRequestStatus;
use serde::Deserialize;
use sha2::Sha256;
use std::sync::Arc;
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;
const DEFAULT_MINOR_CORRECTION_THRESHOLD_MINUTES: i64 = 120;

#[derive(Clone)]
pub struct LineworksAppState {
    bot_secret: Arc<[u8]>,
    use_case: Arc<LineworksUseCase>,
}

impl LineworksAppState {
    pub fn new(bot_secret: impl Into<Arc<[u8]>>, use_case: Arc<LineworksUseCase>) -> Self {
        Self {
            bot_secret: bot_secret.into(),
            use_case,
        }
    }
}

pub fn router(bot_secret: impl Into<Arc<[u8]>>, use_case: Arc<LineworksUseCase>) -> Router {
    Router::new()
        .route("/lineworks/callback", post(callback))
        .with_state(LineworksAppState::new(bot_secret, use_case))
}

pub fn verify_lineworks_signature(body: &[u8], signature: &str, secret: &[u8]) -> bool {
    let provided_signature = match base64::engine::general_purpose::STANDARD.decode(signature) {
        Ok(decoded) => decoded,
        Err(_) => return false,
    };

    let mut mac = match HmacSha256::new_from_slice(secret) {
        Ok(mac) => mac,
        Err(_) => return false,
    };

    mac.update(body);
    let expected_signature = mac.finalize().into_bytes();

    expected_signature.ct_eq(&provided_signature).into()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterpretedLineworksEvent {
    pub user_id: String,
    pub command: LineworksCommand,
    pub request_status: Option<AttendanceRequestStatus>,
    pub text: String,
}

#[derive(Debug, Deserialize)]
pub struct LineworksCallbackPayload {
    pub events: Vec<LineworksEvent>,
}

#[derive(Debug, Deserialize)]
pub struct LineworksEvent {
    pub source: LineworksSource,
    pub content: LineworksContent,
}

#[derive(Debug, Deserialize)]
pub struct LineworksSource {
    #[serde(rename = "userId")]
    pub user_id: String,
}

#[derive(Debug, Deserialize)]
pub struct LineworksContent {
    #[serde(default)]
    pub text: Option<String>,
}

pub fn interpret_callback_payload(
    payload: LineworksCallbackPayload,
    requested_at: &Zoned,
) -> Vec<InterpretedLineworksEvent> {
    payload
        .events
        .into_iter()
        .filter_map(|event| {
            let text = event.content.text?;
            let trimmed = text.trim();

            if trimmed.is_empty() {
                return None;
            }

            Some(InterpretedLineworksEvent {
                user_id: event.source.user_id,
                command: parse_lineworks_command(trimmed),
                request_status: decide_lineworks_request_status(
                    &parse_lineworks_command(trimmed),
                    requested_at,
                    false,
                    DEFAULT_MINOR_CORRECTION_THRESHOLD_MINUTES,
                    None,
                ),
                text: trimmed.to_string(),
            })
        })
        .collect()
}

async fn callback(
    State(state): State<LineworksAppState>,
    headers: HeaderMap,
    body: Bytes,
) -> StatusCode {
    let Some(signature) = headers
        .get("X-WORKS-Signature")
        .and_then(|value| value.to_str().ok())
    else {
        return StatusCode::UNAUTHORIZED;
    };

    if !verify_lineworks_signature(&body, signature, &state.bot_secret) {
        return StatusCode::UNAUTHORIZED;
    }

    let payload = match serde_json::from_slice::<LineworksCallbackPayload>(&body) {
        Ok(payload) => payload,
        Err(_) => return StatusCode::BAD_REQUEST,
    };

    let requested_at = match current_tokyo_time() {
        Ok(requested_at) => requested_at,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR,
    };

    let interpreted = interpret_callback_payload(payload, &requested_at);
    for event in interpreted {
        tracing::info!(
            lineworks_user_id = %event.user_id,
            command = ?event.command,
            request_status = ?event.request_status,
            "processing lineworks callback"
        );

        if let Err(e) = state
            .use_case
            .process_event(&event.user_id, event.command, &requested_at)
            .await
        {
            tracing::error!(error = %e, "failed to process lineworks event");
            // NOTE: We still return 204 to LINE WORKS as they will retry on error,
            // but we might want to return 500 in some cases.
            // For now, follow fire-and-forget for notifications but use_case itself might fail.
        }
    }

    StatusCode::NO_CONTENT
}

fn current_tokyo_time() -> Result<Zoned, jiff::Error> {
    Ok(Timestamp::now().to_zoned(TimeZone::get("Asia/Tokyo")?))
}

#[cfg(test)]
mod tests {
    use super::{
        HmacSha256, LineworksCallbackPayload, LineworksUseCase, current_tokyo_time,
        interpret_callback_payload, router, verify_lineworks_signature,
    };
    use axum::{body::Body, http::Request};
    use base64::Engine;
    use hmac::Mac;
    use jiff::Zoned;
    use pasori_core::application::lineworks::LineworksCommand;
    use std::sync::Arc;
    use tower::ServiceExt;

    fn mock_use_case() -> Arc<LineworksUseCase> {
        struct Mock;
        #[async_trait::async_trait]
        impl pasori_core::port::repo::ExternalAccountRepository for Mock {
            async fn find_by_external_id(
                &self,
                _: &str,
                _: &str,
            ) -> Result<
                Option<pasori_core::domain::employee::ExternalAccount>,
                pasori_core::port::repo::RepoError,
            > {
                Ok(None)
            }
            async fn find_by_employee_id(
                &self,
                _: &str,
                _: uuid::Uuid,
            ) -> Result<
                Option<pasori_core::domain::employee::ExternalAccount>,
                pasori_core::port::repo::RepoError,
            > {
                Ok(None)
            }
            async fn bind(
                &self,
                _: uuid::Uuid,
                _: &str,
                _: &str,
            ) -> Result<
                pasori_core::domain::employee::ExternalAccount,
                pasori_core::port::repo::RepoError,
            > {
                unimplemented!()
            }
        }
        #[async_trait::async_trait]
        impl pasori_core::port::repo::AttendanceRequestRepository for Mock {
            async fn create(
                &self,
                _: pasori_core::domain::request::NewAttendanceRequest,
            ) -> Result<
                pasori_core::domain::request::AttendanceRequest,
                pasori_core::port::repo::RepoError,
            > {
                unimplemented!()
            }
            async fn find(
                &self,
                _: uuid::Uuid,
            ) -> Result<
                Option<pasori_core::domain::request::AttendanceRequest>,
                pasori_core::port::repo::RepoError,
            > {
                unimplemented!()
            }
            async fn update_status(
                &self,
                _: uuid::Uuid,
                _: pasori_core::domain::request::AttendanceRequestStatus,
                _: Option<uuid::Uuid>,
            ) -> Result<
                pasori_core::domain::request::AttendanceRequest,
                pasori_core::port::repo::RepoError,
            > {
                unimplemented!()
            }
        }
        #[async_trait::async_trait]
        impl pasori_core::port::repo::PunchRepository for Mock {
            async fn insert(
                &self,
                _: pasori_core::domain::punch::NewPunchEvent,
            ) -> Result<pasori_core::domain::punch::PunchEvent, pasori_core::port::repo::RepoError>
            {
                unimplemented!()
            }
            async fn recent_for_employee(
                &self,
                _: uuid::Uuid,
                _: usize,
            ) -> Result<
                Vec<pasori_core::domain::punch::PunchEvent>,
                pasori_core::port::repo::RepoError,
            > {
                unimplemented!()
            }
            async fn list_in_range(
                &self,
                _: uuid::Uuid,
                _: &Zoned,
                _: &Zoned,
            ) -> Result<
                Vec<pasori_core::domain::punch::PunchEvent>,
                pasori_core::port::repo::RepoError,
            > {
                unimplemented!()
            }
            async fn update(
                &self,
                _: uuid::Uuid,
                _: pasori_core::domain::punch::PunchPatch,
                _: String,
            ) -> Result<pasori_core::domain::punch::PunchEvent, pasori_core::port::repo::RepoError>
            {
                unimplemented!()
            }
            async fn soft_delete(
                &self,
                _: uuid::Uuid,
                _: String,
            ) -> Result<(), pasori_core::port::repo::RepoError> {
                unimplemented!()
            }
        }
        #[async_trait::async_trait]
        impl pasori_core::port::repo::ShiftRepository for Mock {
            async fn list_for_month(
                &self,
                _: uuid::Uuid,
                _: pasori_core::domain::time::YearMonth,
            ) -> Result<
                Vec<pasori_core::domain::shift::ShiftAssignment>,
                pasori_core::port::repo::RepoError,
            > {
                unimplemented!()
            }
            async fn list_types(
                &self,
            ) -> Result<
                Vec<pasori_core::domain::shift::ShiftType>,
                pasori_core::port::repo::RepoError,
            > {
                unimplemented!()
            }
        }
        #[async_trait::async_trait]
        impl pasori_core::port::notify::Notifier for Mock {
            async fn notify(
                &self,
                _: pasori_core::port::notify::NotifyEvent,
            ) -> Result<(), pasori_core::port::notify::NotifyError> {
                Ok(())
            }
        }

        let m = Arc::new(Mock);
        Arc::new(LineworksUseCase::new(
            m.clone(),
            m.clone(),
            m.clone(),
            m.clone(),
            m.clone(),
        ))
    }

    #[tokio::test]
    // 正しい署名は検証に成功する。
    async fn accepts_valid_signature() {
        let body = br#"{"type":"message"}"#;
        let secret = b"secret";
        let signature = signature_for(body, secret);

        assert!(verify_lineworks_signature(body, &signature, secret));
    }

    #[tokio::test]
    // ボディが改ざんされると検証に失敗する。
    async fn rejects_tampered_body() {
        let body = br#"{"type":"message"}"#;
        let secret = b"secret";
        let signature = signature_for(body, secret);

        assert!(!verify_lineworks_signature(
            br#"{"type":"other"}"#,
            &signature,
            secret
        ));
    }

    #[tokio::test]
    // 署名文字列が Base64 でなければ検証に失敗する。
    async fn rejects_invalid_base64_signature() {
        let body = br#"{"type":"message"}"#;

        assert!(!verify_lineworks_signature(body, "not-base64", b"secret"));
    }

    #[tokio::test]
    // 別の secret で作られた署名は受け付けない。
    async fn rejects_signature_with_wrong_secret() {
        let body = br#"{"type":"message"}"#;
        let signature = signature_for(body, b"secret-a");

        assert!(!verify_lineworks_signature(body, &signature, b"secret-b"));
    }

    #[tokio::test]
    // callback は正しい署名があれば 204 を返す。
    async fn accepts_callback_with_valid_signature() {
        let body = r#"{
            "events": [
                {
                    "source": { "userId": "user-1" },
                    "content": { "text": "今日の勤怠" }
                }
            ]
        }"#;
        let secret = b"secret";
        let signature = signature_for(body.as_bytes(), secret);

        let request: Request<Body> = Request::builder()
            .method("POST")
            .uri("/lineworks/callback")
            .header("X-WORKS-Signature", signature)
            .body(Body::from(body))
            .expect("request should be built");

        let response = router(secret.to_vec(), mock_use_case())
            .oneshot(request)
            .await
            .expect("response should be returned");

        assert_eq!(response.status(), axum::http::StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    // callback は署名が無ければ 401 を返す。
    async fn rejects_callback_without_signature() {
        let body = r#"{
            "events": [
                {
                    "source": { "userId": "user-1" },
                    "content": { "text": "今日の勤怠" }
                }
            ]
        }"#;
        let secret = b"secret";

        let request: Request<Body> = Request::builder()
            .method("POST")
            .uri("/lineworks/callback")
            .body(Body::from(body))
            .expect("request should be built");

        let response = router(secret.to_vec(), mock_use_case())
            .oneshot(request)
            .await
            .expect("response should be returned");

        assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    // callback は署名不一致なら 401 を返す。
    async fn rejects_callback_with_invalid_signature() {
        let body = r#"{
            "events": [
                {
                    "source": { "userId": "user-1" },
                    "content": { "text": "今日の勤怠" }
                }
            ]
        }"#;
        let secret = b"secret";
        let signature = signature_for(body.as_bytes(), b"other-secret");

        let request: Request<Body> = Request::builder()
            .method("POST")
            .uri("/lineworks/callback")
            .header("X-WORKS-Signature", signature)
            .body(Body::from(body))
            .expect("request should be built");

        let response = router(secret.to_vec(), mock_use_case())
            .oneshot(request)
            .await
            .expect("response should be returned");

        assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
    }

    #[test]
    // callback payload の text message は command に解釈される。
    fn interprets_text_message_events() {
        let payload: LineworksCallbackPayload = serde_json::from_str(
            r#"{
                "events": [
                    {
                        "source": { "userId": "user-1" },
                        "content": { "text": "今日の勤怠" }
                    }
                ]
            }"#,
        )
        .expect("payload should deserialize");

        let requested_at = tokyo_datetime(2026, 4, 16, 10, 0);
        let interpreted = interpret_callback_payload(payload, &requested_at);

        assert_eq!(interpreted.len(), 1);
        assert_eq!(interpreted[0].user_id, "user-1");
        assert_eq!(interpreted[0].command, LineworksCommand::TodayAttendance);
    }

    #[test]
    // 空テキストや text を持たない event は無視する。
    fn ignores_non_text_or_empty_events() {
        let payload: LineworksCallbackPayload = serde_json::from_str(
            r#"{
                "events": [
                    {
                        "source": { "userId": "user-1" },
                        "content": { "type": "image" }
                    },
                    {
                        "source": { "userId": "user-2" },
                        "content": { "text": "   " }
                    }
                ]
            }"#,
        )
        .expect("payload should deserialize");

        let requested_at = tokyo_datetime(2026, 4, 16, 10, 0);
        let interpreted = interpret_callback_payload(payload, &requested_at);

        assert!(interpreted.is_empty());
    }

    #[test]
    // 打刻漏れ申請は当日なら自動承認候補として解釈される。
    fn interprets_missing_in_with_request_status() {
        let payload: LineworksCallbackPayload = serde_json::from_str(
            r#"{
                "events": [
                    {
                        "source": { "userId": "user-1" },
                        "content": { "text": "出勤忘れ 08:30" }
                    }
                ]
            }"#,
        )
        .expect("payload should deserialize");

        let requested_at = tokyo_datetime(2026, 4, 16, 10, 0);
        let interpreted = interpret_callback_payload(payload, &requested_at);

        assert_eq!(interpreted.len(), 1);
        assert_eq!(
            interpreted[0].request_status,
            Some(pasori_core::domain::request::AttendanceRequestStatus::AutoApproved)
        );
    }

    #[test]
    // 現在時刻は Asia/Tokyo の Zoned として取得する。
    fn returns_current_time_in_tokyo_timezone() {
        let current = current_tokyo_time().expect("current Tokyo time should be available");

        assert_eq!(current.time_zone().iana_name(), Some("Asia/Tokyo"));
    }

    #[tokio::test]
    // callback は不正 JSON を 400 として拒否する。
    async fn rejects_invalid_json_body() {
        let body = br#"{"events":["broken"]}"#;
        let secret = b"secret";
        let signature = signature_for(body, secret);

        let request: Request<Body> = Request::builder()
            .method("POST")
            .uri("/lineworks/callback")
            .header("X-WORKS-Signature", signature)
            .body(Body::from(body.as_slice()))
            .expect("request should be built");

        let response = router(secret.to_vec(), mock_use_case())
            .oneshot(request)
            .await
            .expect("response should be returned");

        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    fn signature_for(body: &[u8], secret: &[u8]) -> String {
        let mut mac = HmacSha256::new_from_slice(secret).expect("secret should be valid");
        mac.update(body);
        base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
    }

    fn tokyo_datetime(year: i16, month: i8, day: i8, hour: i8, minute: i8) -> Zoned {
        jiff::civil::date(year, month, day)
            .at(hour, minute, 0, 0)
            .in_tz("Asia/Tokyo")
            .expect("Asia/Tokyo datetime should be valid")
    }
}

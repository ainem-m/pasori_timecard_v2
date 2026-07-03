use crate::infra::sqlite::{AuthenticatedTerminal, SqliteRepository};
use axum::{
    Json, Router,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use pasori_core::application::attendance::RegisteredCardScan;
use pasori_core::application::attendance::{PunchUseCase, ResolvedCardScan};
use pasori_core::domain::card::Card;
use pasori_core::domain::punch::{NewPunchEvent, PunchEvent};
use pasori_core::port::policy::PunchEventType;
use pasori_core::port::reader::CardId;
use pasori_core::port::repo::{CardRepository, EmployeeRepository, RepoError};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::{Uuid, Version};

#[derive(Clone)]
pub struct TerminalAppState {
    pub punch_use_case: Arc<PunchUseCase>,
    pub repo: Arc<SqliteRepository>,
}

pub fn router(punch_use_case: Arc<PunchUseCase>, repo: Arc<SqliteRepository>) -> Router {
    Router::new()
        .route("/health", get(health_check))
        .route("/terminals/me/card_scanned", get(card_scanned))
        .route("/terminals/me/employees", get(list_active_employees))
        .route("/terminals/me/cards/bind", post(bind_card))
        .route("/terminals/me/punches", post(sync_punch))
        .route("/punches", post(submit_punch))
        .with_state(TerminalAppState {
            punch_use_case,
            repo,
        })
}

async fn health_check() -> impl IntoResponse {
    let mut headers = HeaderMap::new();
    let now = jiff::Zoned::now().to_string();
    if let Ok(val) = now.parse() {
        headers.insert("Server-Time", val);
    }
    (StatusCode::OK, headers)
}

#[derive(Deserialize)]
struct CardScannedQuery {
    card_id: String,
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum CardScannedResponse {
    Registered(Box<RegisteredCardScanResponse>),
    Unregistered { card_id: String },
}

#[derive(Serialize)]
struct RegisteredCardScanResponse {
    employee: pasori_core::domain::employee::Employee,
    recent_events: Vec<PunchEvent>,
    suggested_type: PunchEventType,
}

#[derive(Deserialize)]
struct BindCardRequest {
    card_id: String,
    employee_id: uuid::Uuid,
}

#[derive(Serialize)]
struct TerminalEmployee {
    id: uuid::Uuid,
    display_name: String,
}

#[derive(Serialize)]
struct BindCardResponse {
    card: Card,
    employee: TerminalEmployee,
}

async fn card_scanned(
    headers: HeaderMap,
    Query(query): Query<CardScannedQuery>,
    State(state): State<TerminalAppState>,
) -> impl IntoResponse {
    let _terminal = match authenticate_terminal_request(&state, &headers).await {
        Ok(terminal) => terminal,
        Err(status) => return Err(status),
    };
    let now = jiff::Zoned::now();
    let card_id = CardId(query.card_id);

    match state.punch_use_case.resolve_card_scan(&card_id, &now).await {
        Ok(ResolvedCardScan::Registered(scan)) => {
            let RegisteredCardScan {
                employee,
                recent_events,
                suggested_type,
                ..
            } = *scan;
            Ok(Json(CardScannedResponse::Registered(Box::new(
                RegisteredCardScanResponse {
                    employee,
                    recent_events,
                    suggested_type,
                },
            ))))
        }
        Ok(ResolvedCardScan::Unregistered { card_id }) => {
            Ok(Json(CardScannedResponse::Unregistered {
                card_id: card_id.0,
            }))
        }
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn list_active_employees(
    headers: HeaderMap,
    State(state): State<TerminalAppState>,
) -> Result<Json<Vec<TerminalEmployee>>, StatusCode> {
    let _terminal = authenticate_terminal_request(&state, &headers).await?;

    state
        .repo
        .list_active()
        .await
        .map(|employees| {
            Json(
                employees
                    .into_iter()
                    .map(|employee| TerminalEmployee {
                        id: employee.id,
                        display_name: employee.display_name,
                    })
                    .collect(),
            )
        })
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn bind_card(
    headers: HeaderMap,
    State(state): State<TerminalAppState>,
    Json(payload): Json<BindCardRequest>,
) -> Result<(StatusCode, Json<BindCardResponse>), StatusCode> {
    let terminal = authenticate_terminal_request(&state, &headers).await?;
    let card_id = CardId(payload.card_id);
    if let Some(existing) = CardRepository::find(&*state.repo, &card_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    {
        if existing.is_active {
            return Err(StatusCode::CONFLICT);
        }
    }

    let employee = EmployeeRepository::find(&*state.repo, payload.employee_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .filter(|employee| employee.is_active)
        .ok_or(StatusCode::NOT_FOUND)?;

    let card = state
        .repo
        .bind_new_card_from_terminal(&card_id, employee.id, terminal.id)
        .await
        .map_err(map_repo_error)?;

    Ok((
        StatusCode::CREATED,
        Json(BindCardResponse {
            card,
            employee: TerminalEmployee {
                id: employee.id,
                display_name: employee.display_name,
            },
        }),
    ))
}

async fn submit_punch() -> StatusCode {
    StatusCode::GONE
}

#[derive(Deserialize)]
pub struct TerminalSubmitPunchRequest {
    pub punch_id: Uuid,
    pub card_id: String,
    pub event_type: PunchEventType,
    pub occurred_at: jiff::Zoned,
    pub source: String,
}

async fn sync_punch(
    headers: HeaderMap,
    State(state): State<TerminalAppState>,
    Json(payload): Json<TerminalSubmitPunchRequest>,
) -> impl IntoResponse {
    let _terminal = match authenticate_terminal_request(&state, &headers).await {
        Ok(terminal) => terminal,
        Err(status) => return Err(status),
    };
    if !is_uuid_v7(payload.punch_id) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let source = PunchSource::parse(&payload.source)?;
    let now = tokyo_now().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let occurred_at = resolve_occurred_at(source, &payload.occurred_at, &now)?;
    let card_id = CardId(payload.card_id);

    match state.punch_use_case.resolve_card_scan(&card_id, &now).await {
        Ok(ResolvedCardScan::Registered(scan)) => {
            let event = NewPunchEvent {
                id: payload.punch_id,
                employee_id: scan.employee.id,
                card_id: scan.card_id,
                event_type: payload.event_type,
                occurred_at,
                source: source.as_str().to_string(),
            };
            match state.punch_use_case.submit_punch(event).await {
                Ok(punch) => Ok((StatusCode::CREATED, Json(punch))),
                Err(error) => handle_submit_error(&state, payload.punch_id, error).await,
            }
        }
        _ => Err(StatusCode::NOT_FOUND),
    }
}

#[derive(Debug, Clone, Copy)]
enum PunchSource {
    Nfc,
    LocalCached,
}

impl PunchSource {
    fn parse(value: &str) -> Result<Self, StatusCode> {
        match value {
            "nfc" => Ok(Self::Nfc),
            "local_cached" => Ok(Self::LocalCached),
            _ => Err(StatusCode::BAD_REQUEST),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Nfc => "nfc",
            Self::LocalCached => "local_cached",
        }
    }
}

fn is_uuid_v7(id: Uuid) -> bool {
    id.get_version() == Some(Version::SortRand)
}

fn tokyo_now() -> Result<jiff::Zoned, jiff::Error> {
    Ok(jiff::Timestamp::now().to_zoned(jiff::tz::TimeZone::get("Asia/Tokyo")?))
}

fn resolve_occurred_at(
    source: PunchSource,
    requested_at: &jiff::Zoned,
    server_now: &jiff::Zoned,
) -> Result<jiff::Zoned, StatusCode> {
    match source {
        PunchSource::Nfc => truncate_to_minute(server_now),
        PunchSource::LocalCached => {
            let max_allowed = server_now
                .clone()
                .checked_add(jiff::SignedDuration::from_secs(10))
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            if requested_at.timestamp() > max_allowed.timestamp() {
                return Err(StatusCode::BAD_REQUEST);
            }
            Ok(requested_at.clone())
        }
    }
}

fn truncate_to_minute(zoned: &jiff::Zoned) -> Result<jiff::Zoned, StatusCode> {
    let dt = zoned.datetime();
    let truncated = jiff::civil::DateTime::new(
        dt.year(),
        dt.month(),
        dt.day(),
        dt.hour(),
        dt.minute(),
        0,
        0,
    )
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    truncated
        .in_tz("Asia/Tokyo")
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn handle_submit_error(
    state: &TerminalAppState,
    punch_id: uuid::Uuid,
    error: anyhow::Error,
) -> Result<(StatusCode, Json<PunchEvent>), StatusCode> {
    if let Some(RepoError::Conflict(_)) = error.downcast_ref::<RepoError>() {
        let existing = state
            .repo
            .find_punch_by_id(punch_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;

        return Ok((StatusCode::OK, Json(existing)));
    }

    Err(StatusCode::INTERNAL_SERVER_ERROR)
}

fn map_repo_error(error: RepoError) -> StatusCode {
    match error {
        RepoError::Conflict(_) => StatusCode::CONFLICT,
        RepoError::NotFound => StatusCode::NOT_FOUND,
        RepoError::Db(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

async fn authenticate_terminal_request(
    state: &TerminalAppState,
    headers: &HeaderMap,
) -> Result<AuthenticatedTerminal, StatusCode> {
    let token = extract_bearer_token(headers).ok_or(StatusCode::UNAUTHORIZED)?;
    let terminal = state
        .repo
        .authenticate_terminal_token(token)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::UNAUTHORIZED)?;

    state
        .repo
        .touch_terminal(terminal.id, &jiff::Zoned::now())
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(terminal)
}

fn extract_bearer_token(headers: &HeaderMap) -> Option<&str> {
    let header = headers.get(axum::http::header::AUTHORIZATION)?;
    let value = header.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;

    if scheme.eq_ignore_ascii_case("Bearer") && !token.is_empty() {
        Some(token)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{router, tokyo_now, truncate_to_minute};
    use crate::infra::{console_notify::ConsoleNotifier, sqlite::SqliteRepository};
    use argon2::{
        Argon2,
        password_hash::{PasswordHasher, SaltString},
    };
    use axum::{body::Body, http::Request, http::StatusCode};
    use pasori_core::{application::attendance::PunchUseCase, port::policy::DefaultPunchPolicy};
    use sqlx::{Row, sqlite::SqlitePoolOptions};
    use tower::ServiceExt;
    use uuid::Uuid;
    const TEST_CARD_ID: &str = "02020212A91B9843";

    #[tokio::test]
    // Terminal API は Bearer token なしでは利用できない。
    async fn rejects_terminal_request_without_bearer_token() {
        let (app, _pool) = test_app("terminal-secret").await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/terminals/me/card_scanned?card_id={TEST_CARD_ID}"))
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    // Terminal token があれば有効な従業員一覧を取得できる。
    async fn lists_active_employees_for_terminal_binding() {
        let (app, _pool) = test_app("terminal-secret").await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/terminals/me/employees")
                    .header(axum::http::header::AUTHORIZATION, "Bearer terminal-secret")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("json body");
        let employees = json.as_array().expect("employees array");
        assert!(
            employees
                .iter()
                .any(|employee| employee["display_name"] == "テスト 太郎")
        );
        assert!(
            employees
                .iter()
                .all(|employee| employee.get("employment_type").is_none())
        );
    }

    #[tokio::test]
    // Terminal は未登録カードを有効従業員に紐付け、terminal actor の監査ログを残す。
    async fn binds_unregistered_card_from_terminal_and_records_audit() {
        let (app, pool) = test_app("terminal-secret").await;
        let body = serde_json::json!({
            "card_id": "9999999999999999",
            "employee_id": "0196273c-8b3e-7b92-92a7-d0ddf4828a10"
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/terminals/me/cards/bind")
                    .header(axum::http::header::AUTHORIZATION, "Bearer terminal-secret")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.to_string()))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::CREATED);

        let card = sqlx::query("SELECT employee_id FROM card WHERE card_identifier = ?")
            .bind("9999999999999999")
            .fetch_one(&pool)
            .await
            .expect("card row");
        assert_eq!(
            card.get::<String, _>("employee_id"),
            "0196273c-8b3e-7b92-92a7-d0ddf4828a10"
        );

        let audit = sqlx::query(
            "SELECT actor_type, actor_id, action, metadata_json FROM audit_log WHERE action = 'card.bind' ORDER BY created_at DESC LIMIT 1",
        )
        .fetch_one(&pool)
        .await
        .expect("audit row");
        assert_eq!(audit.get::<String, _>("actor_type"), "terminal");
        assert_eq!(
            audit.get::<String, _>("actor_id"),
            "0196273c-8b3e-7b92-92a7-d0ddf4828a12"
        );
        assert!(
            audit
                .get::<String, _>("metadata_json")
                .contains("terminal_unregistered_card_flow")
        );
        assert!(
            !audit
                .get::<String, _>("metadata_json")
                .contains("9999999999999999")
        );

        let after_json = sqlx::query("SELECT after_json FROM audit_log WHERE action = 'card.bind' ORDER BY created_at DESC LIMIT 1")
            .fetch_one(&pool)
            .await
            .expect("audit after row")
            .get::<String, _>("after_json");
        assert!(!after_json.contains("9999999999999999"));
    }

    #[tokio::test]
    // Terminal から既存カードの付け替えはできない。
    async fn rejects_terminal_rebind_for_existing_active_card() {
        let (app, _pool) = test_app("terminal-secret").await;
        let body = serde_json::json!({
            "card_id": TEST_CARD_ID,
            "employee_id": "0196273c-8b3e-7b92-92a7-d0ddf4828a10"
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/terminals/me/cards/bind")
                    .header(axum::http::header::AUTHORIZATION, "Bearer terminal-secret")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.to_string()))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    // Terminal は無効化済みカードを未登録カードフローで再紐付けできる。
    async fn reactivates_inactive_card_from_terminal_binding() {
        let (app, pool) = test_app("terminal-secret").await;
        sqlx::query("UPDATE card SET is_active = 0 WHERE card_identifier = ?")
            .bind(TEST_CARD_ID)
            .execute(&pool)
            .await
            .expect("deactivate card");
        let body = serde_json::json!({
            "card_id": TEST_CARD_ID,
            "employee_id": "0196273c-8b3e-7b92-92a7-d0ddf4828a10"
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/terminals/me/cards/bind")
                    .header(axum::http::header::AUTHORIZATION, "Bearer terminal-secret")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.to_string()))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::CREATED);

        let card = sqlx::query("SELECT is_active FROM card WHERE card_identifier = ?")
            .bind(TEST_CARD_ID)
            .fetch_one(&pool)
            .await
            .expect("card row");
        assert_eq!(card.get::<i64, _>("is_active"), 1);
    }

    #[tokio::test]
    // 旧 punch API は安全側に無効化する。
    async fn rejects_legacy_punch_api_as_gone() {
        let (app, pool) = test_app("terminal-secret").await;
        sqlx::query("UPDATE employee SET is_active = 0 WHERE id = ?")
            .bind("0196273c-8b3e-7b92-92a7-d0ddf4828a10")
            .execute(&pool)
            .await
            .expect("deactivate employee");
        let body = serde_json::json!({
            "punch_id": Uuid::now_v7(),
            "employee_id": "0196273c-8b3e-7b92-92a7-d0ddf4828a10",
            "card_id": null,
            "event_type": "clock_in",
            "occurred_at": "2026-04-17T09:00:00+09:00[Asia/Tokyo]",
            "source": "nfc"
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/punches")
                    .header(axum::http::header::AUTHORIZATION, "Bearer terminal-secret")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.to_string()))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::GONE);
    }

    #[tokio::test]
    // 同じ未登録カードへの同時紐付けは片方だけ成功し、付け替えを起こさない。
    async fn rejects_concurrent_terminal_bind_for_same_card() {
        let (app, pool) = test_app("terminal-secret").await;
        let now = jiff::Zoned::now().to_string();
        sqlx::query(
            r#"
            INSERT INTO employee (id, display_name, employment_type, is_active, created_at, updated_at)
            VALUES (?, ?, ?, 1, ?, ?)
            "#,
        )
        .bind("0196273c-8b3e-7b92-92a7-d0ddf4828a13")
        .bind("テスト 花子")
        .bind("part_time")
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("insert employee");

        let body_a = serde_json::json!({
            "card_id": "AAAAAAAAAAAAAAAA",
            "employee_id": "0196273c-8b3e-7b92-92a7-d0ddf4828a10"
        });
        let body_b = serde_json::json!({
            "card_id": "AAAAAAAAAAAAAAAA",
            "employee_id": "0196273c-8b3e-7b92-92a7-d0ddf4828a13"
        });

        let request_a = Request::builder()
            .method("POST")
            .uri("/terminals/me/cards/bind")
            .header(axum::http::header::AUTHORIZATION, "Bearer terminal-secret")
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .body(Body::from(body_a.to_string()))
            .expect("request a");
        let request_b = Request::builder()
            .method("POST")
            .uri("/terminals/me/cards/bind")
            .header(axum::http::header::AUTHORIZATION, "Bearer terminal-secret")
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .body(Body::from(body_b.to_string()))
            .expect("request b");

        let (response_a, response_b) =
            tokio::join!(app.clone().oneshot(request_a), app.oneshot(request_b));
        let statuses = [
            response_a.expect("response a").status(),
            response_b.expect("response b").status(),
        ];

        assert!(statuses.contains(&StatusCode::CREATED));
        assert!(statuses.contains(&StatusCode::CONFLICT));

        let cards = sqlx::query("SELECT employee_id FROM card WHERE card_identifier = ?")
            .bind("AAAAAAAAAAAAAAAA")
            .fetch_all(&pool)
            .await
            .expect("card rows");
        assert_eq!(cards.len(), 1);
    }

    #[tokio::test]
    // オフライン再送された過去打刻は source=local_cached と occurred_at をそのまま保存する。
    async fn preserves_local_cached_source_and_occurred_at_for_synced_punch() {
        let (app, pool) = test_app("terminal-secret").await;
        let occurred_at = "2026-04-17T09:00:00+09:00[Asia/Tokyo]";
        let body = serde_json::json!({
            "punch_id": Uuid::now_v7(),
            "card_id": TEST_CARD_ID,
            "event_type": "clock_in",
            "occurred_at": occurred_at,
            "source": "local_cached"
        });

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/terminals/me/punches")
                    .header(axum::http::header::AUTHORIZATION, "Bearer terminal-secret")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.to_string()))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::CREATED);

        let row = sqlx::query(
            "SELECT source, occurred_at FROM punch_event ORDER BY created_at DESC LIMIT 1",
        )
        .fetch_one(&pool)
        .await
        .expect("stored punch");

        assert_eq!(row.get::<String, _>("source"), "local_cached");
        assert_eq!(row.get::<String, _>("occurred_at"), occurred_at);
    }

    #[tokio::test]
    // Terminal 打刻 API は未知の source を拒否する。
    async fn rejects_unknown_punch_source() {
        let (app, _pool) = test_app("terminal-secret").await;
        let body = serde_json::json!({
            "punch_id": Uuid::now_v7(),
            "card_id": TEST_CARD_ID,
            "event_type": "clock_in",
            "occurred_at": "2026-04-17T09:00:00+09:00[Asia/Tokyo]",
            "source": "manual"
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/terminals/me/punches")
                    .header(axum::http::header::AUTHORIZATION, "Bearer terminal-secret")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.to_string()))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    // オンライン打刻はリクエストの occurred_at ではなく Server 時刻で保存する。
    async fn uses_server_time_for_nfc_punch() {
        let (app, pool) = test_app("terminal-secret").await;
        let punch_id = Uuid::now_v7();
        let before_request = tokyo_now().expect("Tokyo now before request");
        let body = serde_json::json!({
            "punch_id": punch_id,
            "card_id": TEST_CARD_ID,
            "event_type": "clock_in",
            "occurred_at": "2020-01-01T00:00:00+09:00[Asia/Tokyo]",
            "source": "nfc"
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/terminals/me/punches")
                    .header(axum::http::header::AUTHORIZATION, "Bearer terminal-secret")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.to_string()))
                    .expect("request"),
            )
            .await
            .expect("response");
        let after_response = tokyo_now().expect("Tokyo now after response");

        assert_eq!(response.status(), StatusCode::CREATED);

        let row = sqlx::query("SELECT occurred_at FROM punch_event WHERE id = ?")
            .bind(punch_id.to_string())
            .fetch_one(&pool)
            .await
            .expect("stored punch");
        let occurred_at = row.get::<String, _>("occurred_at");
        let stored_at = occurred_at.parse::<jiff::Zoned>().expect("stored zoned");
        let earliest = truncate_to_minute(&before_request).expect("earliest minute");
        let latest = truncate_to_minute(&after_response).expect("latest minute");

        assert_ne!(occurred_at, "2020-01-01T00:00:00+09:00[Asia/Tokyo]");
        assert_eq!(stored_at.second(), 0);
        assert_eq!(stored_at.subsec_nanosecond(), 0);
        assert!(
            stored_at.timestamp() >= earliest.timestamp(),
            "stored_at={stored_at} earliest={earliest}"
        );
        assert!(
            stored_at.timestamp() <= latest.timestamp(),
            "stored_at={stored_at} latest={latest}"
        );
    }

    #[tokio::test]
    // オフライン再送は Server 時刻 +10 秒を超える未来 occurred_at を拒否する。
    async fn rejects_local_cached_punch_from_future() {
        let (app, _pool) = test_app("terminal-secret").await;
        let future_occurred_at = tokyo_now()
            .expect("Tokyo now")
            .checked_add(jiff::SignedDuration::from_secs(60))
            .expect("future occurred_at")
            .to_string();
        let body = serde_json::json!({
            "punch_id": Uuid::now_v7(),
            "card_id": TEST_CARD_ID,
            "event_type": "clock_in",
            "occurred_at": future_occurred_at,
            "source": "local_cached"
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/terminals/me/punches")
                    .header(axum::http::header::AUTHORIZATION, "Bearer terminal-secret")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.to_string()))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    // Terminal 打刻 API は UUID v7 ではない punch_id を拒否する。
    async fn rejects_non_uuid_v7_punch_id() {
        let (app, _pool) = test_app("terminal-secret").await;
        let body = serde_json::json!({
            "punch_id": Uuid::nil(),
            "card_id": TEST_CARD_ID,
            "event_type": "clock_in",
            "occurred_at": "2026-04-17T09:00:00+09:00[Asia/Tokyo]",
            "source": "local_cached"
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/terminals/me/punches")
                    .header(axum::http::header::AUTHORIZATION, "Bearer terminal-secret")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.to_string()))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    // 同じ打刻を再送しても既存レコードを返して同期完了できる。
    async fn accepts_duplicate_replay_for_existing_punch() {
        let (app, pool) = test_app("terminal-secret").await;
        let punch_id = Uuid::now_v7();
        let body = serde_json::json!({
            "punch_id": punch_id,
            "card_id": TEST_CARD_ID,
            "event_type": "clock_in",
            "occurred_at": "2026-04-17T09:00:00+09:00[Asia/Tokyo]",
            "source": "local_cached"
        })
        .to_string();

        let first_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/terminals/me/punches")
                    .header(axum::http::header::AUTHORIZATION, "Bearer terminal-secret")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.clone()))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(first_response.status(), StatusCode::CREATED);

        let second_response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/terminals/me/punches")
                    .header(axum::http::header::AUTHORIZATION, "Bearer terminal-secret")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(second_response.status(), StatusCode::OK);

        let row = sqlx::query("SELECT COUNT(*) as count FROM punch_event WHERE id = ?")
            .bind(punch_id.to_string())
            .fetch_one(&pool)
            .await
            .expect("punch count");
        assert_eq!(row.get::<i64, _>("count"), 1);
    }

    async fn test_pool() -> sqlx::SqlitePool {
        let database_url = format!("sqlite:file:{}?mode=memory&cache=shared", Uuid::now_v7());
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&database_url)
            .await
            .expect("sqlite pool");

        sqlx::migrate!("../../migrations")
            .run(&pool)
            .await
            .expect("migrate");

        pool
    }

    async fn test_app(token: &str) -> (axum::Router, sqlx::SqlitePool) {
        let pool = test_pool().await;
        let repo = std::sync::Arc::new(SqliteRepository::new(pool.clone()));
        let now = "2026-04-17T00:00:00+09:00[Asia/Tokyo]";
        let terminal_hash = hash_token(token);

        sqlx::query(
            "INSERT INTO employee (id, display_name, employment_type, affiliation, is_active, created_at, updated_at)
             VALUES (?, ?, ?, ?, 1, ?, ?)",
        )
        .bind("0196273c-8b3e-7b92-92a7-d0ddf4828a10")
        .bind("テスト 太郎")
        .bind("regular")
        .bind("開発部")
        .bind(now)
        .bind(now)
        .execute(&pool)
        .await
        .expect("insert employee");

        sqlx::query(
            "INSERT INTO card (id, employee_id, card_identifier, card_label, is_active, created_at, updated_at)
             VALUES (?, ?, ?, ?, 1, ?, ?)",
        )
        .bind("0196273c-8b3e-7b92-92a7-d0ddf4828a11")
        .bind("0196273c-8b3e-7b92-92a7-d0ddf4828a10")
        .bind(TEST_CARD_ID)
        .bind("テストカード")
        .bind(now)
        .bind(now)
        .execute(&pool)
        .await
        .expect("insert card");

        sqlx::query(
            "INSERT INTO terminal (id, name, api_token_hash, is_active, created_at, updated_at)
             VALUES (?, ?, ?, 1, ?, ?)",
        )
        .bind("0196273c-8b3e-7b92-92a7-d0ddf4828a12")
        .bind("受付端末")
        .bind(terminal_hash)
        .bind(now)
        .bind(now)
        .execute(&pool)
        .await
        .expect("insert terminal");

        let notifier: std::sync::Arc<dyn pasori_core::port::notify::Notifier> =
            std::sync::Arc::new(ConsoleNotifier);
        let punch_use_case = std::sync::Arc::new(PunchUseCase::new(
            repo.clone(),
            repo.clone(),
            repo.clone(),
            repo.clone(),
            notifier,
            std::sync::Arc::new(DefaultPunchPolicy),
        ));

        (router(punch_use_case, repo), pool)
    }

    fn hash_token(token: &str) -> String {
        let salt =
            SaltString::from_b64("dGVzdF9zYWx0X3ZhbHVl").expect("static salt should be valid");
        Argon2::default()
            .hash_password(token.as_bytes(), &salt)
            .expect("hash token")
            .to_string()
    }
}

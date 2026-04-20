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
use pasori_core::domain::punch::{NewPunchEvent, PunchEvent};
use pasori_core::port::policy::PunchEventType;
use pasori_core::port::reader::CardId;
use pasori_core::port::repo::RepoError;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone)]
pub struct TerminalAppState {
    pub punch_use_case: Arc<PunchUseCase>,
    pub repo: Arc<SqliteRepository>,
}

pub fn router(punch_use_case: Arc<PunchUseCase>, repo: Arc<SqliteRepository>) -> Router {
    Router::new()
        .route("/health", get(health_check))
        .route("/terminals/me/card_scanned", get(card_scanned))
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

#[derive(Deserialize)]
struct SubmitPunchRequest {
    punch_id: uuid::Uuid,
    employee_id: uuid::Uuid,
    card_id: Option<uuid::Uuid>,
    event_type: PunchEventType,
    occurred_at: jiff::Zoned,
    source: String,
}

async fn submit_punch(
    headers: HeaderMap,
    State(state): State<TerminalAppState>,
    Json(payload): Json<SubmitPunchRequest>,
) -> impl IntoResponse {
    let _terminal = match authenticate_terminal_request(&state, &headers).await {
        Ok(terminal) => terminal,
        Err(status) => return Err(status),
    };
    let event = NewPunchEvent {
        id: payload.punch_id,
        employee_id: payload.employee_id,
        card_id: payload.card_id,
        event_type: payload.event_type,
        occurred_at: payload.occurred_at,
        source: payload.source,
    };

    match state.punch_use_case.submit_punch(event).await {
        Ok(punch) => Ok((StatusCode::CREATED, Json(punch))),
        Err(error) => handle_submit_error(&state, payload.punch_id, error).await,
    }
}

#[derive(Deserialize)]
pub struct TerminalSubmitPunchRequest {
    pub punch_id: uuid::Uuid,
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
    let now = jiff::Zoned::now();
    let card_id = CardId(payload.card_id);

    // Resolve card first
    match state.punch_use_case.resolve_card_scan(&card_id, &now).await {
        Ok(ResolvedCardScan::Registered(scan)) => {
            let event = NewPunchEvent {
                id: payload.punch_id,
                employee_id: scan.employee.id,
                card_id: scan.card_id,
                event_type: payload.event_type,
                occurred_at: payload.occurred_at,
                source: payload.source,
            };
            match state.punch_use_case.submit_punch(event).await {
                Ok(punch) => Ok((StatusCode::CREATED, Json(punch))),
                Err(error) => handle_submit_error(&state, payload.punch_id, error).await,
            }
        }
        _ => Err(StatusCode::NOT_FOUND),
    }
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
    use super::router;
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
    // オフライン再送された打刻は source=local_cached のまま保存される。
    async fn preserves_local_cached_source_for_synced_punch() {
        let (app, pool) = test_app("terminal-secret").await;
        let body = serde_json::json!({
            "punch_id": Uuid::now_v7(),
            "card_id": TEST_CARD_ID,
            "event_type": "clock_in",
            "occurred_at": "2026-04-17T09:00:00+09:00[Asia/Tokyo]",
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

        let row = sqlx::query("SELECT source FROM punch_event ORDER BY created_at DESC LIMIT 1")
            .fetch_one(&pool)
            .await
            .expect("stored punch");

        assert_eq!(row.get::<String, _>("source"), "local_cached");
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

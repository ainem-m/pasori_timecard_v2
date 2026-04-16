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
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone)]
pub struct TerminalAppState {
    pub punch_use_case: Arc<PunchUseCase>,
}

pub fn router(punch_use_case: Arc<PunchUseCase>) -> Router {
    Router::new()
        .route("/api/health", get(health_check))
        .route("/api/terminals/me/card_scanned", get(card_scanned))
        .route("/api/punches", post(submit_punch))
        .with_state(TerminalAppState { punch_use_case })
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
    Query(query): Query<CardScannedQuery>,
    State(state): State<TerminalAppState>,
) -> impl IntoResponse {
    let now = jiff::Zoned::now();
    let card_id = CardId(query.card_id);

    match state.punch_use_case.resolve_card_scan(&card_id, &now).await {
        Ok(ResolvedCardScan::Registered(scan)) => {
            let RegisteredCardScan {
                employee,
                recent_events,
                suggested_type,
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
    State(state): State<TerminalAppState>,
    Json(payload): Json<SubmitPunchRequest>,
) -> impl IntoResponse {
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
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

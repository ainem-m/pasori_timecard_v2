use crate::infra::sqlite::{AdminAuthenticationResult, AuthenticatedAdmin, SqliteRepository};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::IntoResponse,
    routing::{delete, get, post},
};
use pasori_core::domain::audit::NewAuditLog;
use pasori_core::domain::employee::{EmployeePatch, NewEmployee};
use pasori_core::domain::punch::{NewPunchEvent, PunchEvent, PunchPatch};
use pasori_core::domain::request::{
    AttendanceRequest, AttendanceRequestStatus, AttendanceRequestType,
};
use pasori_core::port::policy::PunchEventType;
use pasori_core::port::repo::{
    AttendanceRequestRepository, AuditLogRepository, CardRepository, EmployeeRepository,
    PunchRepository, RepoError,
};
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

mod attendance;
mod terminal_cards;
use attendance::{day_end_in_tokyo, day_start_in_tokyo, get_monthly_attendance};
use terminal_cards::{
    bind_card, create_terminal, deactivate_terminal, list_terminals, rotate_terminal_token,
    unbind_card,
};

#[derive(Clone)]
pub struct AdminAppState {
    pub repo: Arc<SqliteRepository>,
}

pub fn router(repo: Arc<SqliteRepository>) -> Router {
    Router::new()
        .route("/admin/login", post(login))
        .route("/admin/logout", post(logout))
        .route(
            "/admin/employees",
            get(list_employees).post(create_employee),
        )
        .route(
            "/admin/employees/:id",
            get(get_employee)
                .put(update_employee)
                .delete(deactivate_employee),
        )
        .route(
            "/admin/terminals",
            get(list_terminals).post(create_terminal),
        )
        .route("/admin/terminals/:id", delete(deactivate_terminal))
        .route(
            "/admin/terminals/:id/rotate_token",
            post(rotate_terminal_token),
        )
        .route("/admin/attendance_requests", get(list_attendance_requests))
        .route(
            "/admin/attendance_requests/:id/approve",
            post(approve_attendance_request),
        )
        .route(
            "/admin/attendance_requests/:id/reject",
            post(reject_attendance_request),
        )
        .route("/admin/attendance/monthly", get(get_monthly_attendance))
        .route("/admin/punches", get(list_punches))
        .route("/admin/audit_logs", get(list_audit_logs))
        .route("/admin/cards/bind", post(bind_card))
        .route("/admin/cards/unbind", post(unbind_card))
        .with_state(AdminAppState { repo })
}

#[derive(serde::Deserialize)]
struct LoginRequest {
    username: String,
    password: String,
}

#[derive(serde::Serialize)]
struct LoginResponse {
    display_name: String,
}

#[derive(serde::Deserialize)]
struct AttendanceRequestListQuery {
    status: Option<AttendanceRequestStatus>,
}

#[derive(Deserialize)]
struct AttendanceRequestReviewInput {
    review_note: Option<String>,
}

async fn login(
    headers: HeaderMap,
    State(state): State<AdminAppState>,
    Json(payload): Json<LoginRequest>,
) -> impl IntoResponse {
    let metadata_json = login_metadata_json(&headers, &payload.username);
    match state
        .repo
        .verify_admin_credentials(&payload.username, &payload.password)
        .await
    {
        Ok(AdminAuthenticationResult::Authenticated(admin)) => {
            let (session_id, expires_at) = match state.repo.create_admin_session(admin.id).await {
                Ok(session) => session,
                Err(_) => return Err(StatusCode::INTERNAL_SERVER_ERROR),
            };

            if let Err(e) = state
                .repo
                .append(NewAuditLog {
                    actor_type: "admin".to_string(),
                    actor_id: Some(admin.id.to_string()),
                    action: "admin.login_success".to_string(),
                    target_type: "admin_user".to_string(),
                    target_id: Some(admin.id.to_string()),
                    before_json: None,
                    after_json: None,
                    metadata_json: Some(merge_login_metadata(
                        metadata_json,
                        serde_json::json!({
                            "session_id": session_id,
                            "expires_at": expires_at.to_string(),
                        }),
                    )),
                })
                .await
            {
                tracing::error!(error = %e, action = "admin.login_success", "audit log append failed");
            }

            let cookie = match build_admin_session_cookie(&session_id) {
                Ok(cookie) => cookie,
                Err(_) => return Err(StatusCode::INTERNAL_SERVER_ERROR),
            };

            Ok((
                [(header::SET_COOKIE, cookie)],
                Json(LoginResponse {
                    display_name: admin.display_name,
                }),
            ))
        }
        Ok(AdminAuthenticationResult::InvalidCredentials) => {
            if let Err(e) = state
                .repo
                .append(NewAuditLog {
                    actor_type: "system".to_string(),
                    actor_id: None,
                    action: "admin.login_failure".to_string(),
                    target_type: "admin_user".to_string(),
                    target_id: None,
                    before_json: None,
                    after_json: None,
                    metadata_json: Some(metadata_json),
                })
                .await
            {
                tracing::error!(error = %e, action = "admin.login_failure", "audit log append failed");
            }

            Err(StatusCode::UNAUTHORIZED)
        }
        Ok(AdminAuthenticationResult::Locked { locked_until }) => {
            if let Err(e) = state
                .repo
                .append(NewAuditLog {
                    actor_type: "system".to_string(),
                    actor_id: None,
                    action: "admin.login_locked".to_string(),
                    target_type: "admin_user".to_string(),
                    target_id: None,
                    before_json: None,
                    after_json: None,
                    metadata_json: Some(merge_login_metadata(
                        metadata_json,
                        serde_json::json!({
                            "locked_until": locked_until.to_string(),
                        }),
                    )),
                })
                .await
            {
                tracing::error!(error = %e, action = "admin.login_locked", "audit log append failed");
            }

            Err(StatusCode::LOCKED)
        }
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn logout(
    headers: HeaderMap,
    State(state): State<AdminAppState>,
) -> Result<impl IntoResponse, StatusCode> {
    let session_id = extract_admin_session_cookie(&headers).ok_or(StatusCode::UNAUTHORIZED)?;
    let admin_user_id = state
        .repo
        .delete_admin_session(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if let Some(admin_user_id) = admin_user_id {
        if let Err(e) = state
            .repo
            .append(NewAuditLog {
                actor_type: "admin".to_string(),
                actor_id: Some(admin_user_id.to_string()),
                action: "admin.logout".to_string(),
                target_type: "admin_user".to_string(),
                target_id: Some(admin_user_id.to_string()),
                before_json: None,
                after_json: None,
                metadata_json: Some(login_metadata_json(&headers, "")),
            })
            .await
        {
            tracing::error!(error = %e, action = "admin.logout", "audit log append failed");
        }
    }

    Ok((
        [(header::SET_COOKIE, clear_admin_session_cookie())],
        StatusCode::NO_CONTENT,
    ))
}

async fn list_employees(
    headers: HeaderMap,
    State(state): State<AdminAppState>,
) -> impl IntoResponse {
    let _admin = match authenticate_admin_request(&state, &headers).await {
        Ok(admin) => admin,
        Err(status) => return Err(status),
    };

    match state.repo.list_active().await {
        Ok(employees) => Ok(Json(employees)),
        Err(e) => {
            tracing::error!(error = ?e, "list_employees error");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

async fn create_employee(
    headers: HeaderMap,
    State(state): State<AdminAppState>,
    Json(input): Json<NewEmployee>,
) -> impl IntoResponse {
    let admin = match authenticate_admin_request(&state, &headers).await {
        Ok(admin) => admin,
        Err(status) => return Err(status),
    };

    match EmployeeRepository::create(&*state.repo, input).await {
        Ok(employee) => {
            if let Err(e) = state
                .repo
                .append(NewAuditLog {
                    actor_type: "admin".to_string(),
                    actor_id: Some(admin.id.to_string()),
                    action: "employee.create".to_string(),
                    target_type: "employee".to_string(),
                    target_id: Some(employee.id.to_string()),
                    before_json: None,
                    after_json: serde_json::to_string(&employee).ok(),
                    metadata_json: None,
                })
                .await
            {
                tracing::error!(error = %e, action = "employee.create", "audit log append failed");
            }
            Ok((StatusCode::CREATED, Json(employee)))
        }
        Err(e) => {
            tracing::error!(error = ?e, "create_employee error");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

async fn get_employee(
    headers: HeaderMap,
    State(state): State<AdminAppState>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let _admin = match authenticate_admin_request(&state, &headers).await {
        Ok(admin) => admin,
        Err(status) => return Err(status),
    };

    match EmployeeRepository::find(&*state.repo, id).await {
        Ok(Some(employee)) => Ok(Json(employee)),
        Ok(None) => Err(StatusCode::NOT_FOUND),
        Err(e) => {
            tracing::error!(error = ?e, id = ?id, "get_employee error");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

async fn update_employee(
    headers: HeaderMap,
    State(state): State<AdminAppState>,
    Path(id): Path<Uuid>,
    Json(patch): Json<EmployeePatch>,
) -> impl IntoResponse {
    let admin = match authenticate_admin_request(&state, &headers).await {
        Ok(admin) => admin,
        Err(status) => return Err(status),
    };

    let before = match EmployeeRepository::find(&*state.repo, id).await {
        Ok(Some(employee)) => employee,
        Ok(None) => return Err(StatusCode::NOT_FOUND),
        Err(_) => return Err(StatusCode::INTERNAL_SERVER_ERROR),
    };

    match EmployeeRepository::update(&*state.repo, id, patch).await {
        Ok(employee) => {
            if let Err(e) = state
                .repo
                .append(NewAuditLog {
                    actor_type: "admin".to_string(),
                    actor_id: Some(admin.id.to_string()),
                    action: "employee.update".to_string(),
                    target_type: "employee".to_string(),
                    target_id: Some(id.to_string()),
                    before_json: serde_json::to_string(&before).ok(),
                    after_json: serde_json::to_string(&employee).ok(),
                    metadata_json: None,
                })
                .await
            {
                tracing::error!(error = %e, action = "employee.update", "audit log append failed");
            }
            Ok(Json(employee))
        }
        Err(e) => {
            tracing::error!(error = ?e, id = ?id, "update_employee error");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

async fn deactivate_employee(
    headers: HeaderMap,
    State(state): State<AdminAppState>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let admin = match authenticate_admin_request(&state, &headers).await {
        Ok(admin) => admin,
        Err(status) => return Err(status),
    };

    match state.repo.deactivate(id).await {
        Ok(_) => {
            if let Err(e) = state
                .repo
                .append(NewAuditLog {
                    actor_type: "admin".to_string(),
                    actor_id: Some(admin.id.to_string()),
                    action: "employee.deactivate".to_string(),
                    target_type: "employee".to_string(),
                    target_id: Some(id.to_string()),
                    before_json: None,
                    after_json: None,
                    metadata_json: None,
                })
                .await
            {
                tracing::error!(error = %e, action = "employee.deactivate", "audit log append failed");
            }
            Ok(StatusCode::NO_CONTENT)
        }
        Err(e) => {
            tracing::error!(error = ?e, id = ?id, "deactivate_employee error");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

async fn list_punches(
    headers: HeaderMap,
    State(state): State<AdminAppState>,
) -> Result<Json<Vec<pasori_core::domain::punch::PunchEvent>>, StatusCode> {
    let _admin = authenticate_admin_request(&state, &headers).await?;

    match state.repo.list_recent_punches(100).await {
        Ok(punches) => Ok(Json(punches)),
        Err(e) => {
            tracing::error!(error = ?e, "list_punches error");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

async fn list_attendance_requests(
    headers: HeaderMap,
    State(state): State<AdminAppState>,
    axum::extract::Query(query): axum::extract::Query<AttendanceRequestListQuery>,
) -> Result<Json<Vec<AttendanceRequest>>, StatusCode> {
    let _admin = authenticate_admin_request(&state, &headers).await?;

    state
        .repo
        .list_attendance_requests(query.status)
        .await
        .map(Json)
        .map_err(repo_error_to_status)
}

async fn approve_attendance_request(
    headers: HeaderMap,
    State(state): State<AdminAppState>,
    Path(id): Path<Uuid>,
    Json(input): Json<AttendanceRequestReviewInput>,
) -> Result<Json<AttendanceRequest>, StatusCode> {
    let admin = authenticate_admin_request(&state, &headers).await?;
    let request = AttendanceRequestRepository::find(&*state.repo, id)
        .await
        .map_err(repo_error_to_status)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let reviewed = state
        .repo
        .review_attendance_request(
            id,
            admin.id,
            AttendanceRequestStatus::Approved,
            input.review_note.clone(),
        )
        .await
        .map_err(repo_error_to_status)?;

    let applied_event_id = match request.request_type {
        AttendanceRequestType::Correction => {
            let (before_punch, updated_punch) =
                apply_correction_request(&state.repo, &request).await?;
            append_punch_update_audit(&state, &admin, &before_punch, &updated_punch, id).await;
            updated_punch.id
        }
        AttendanceRequestType::MissingIn | AttendanceRequestType::MissingOut => {
            let punch = create_missing_punch(&state.repo, &request).await?;
            if let Err(e) = state
                .repo
                .append(NewAuditLog {
                    actor_type: "admin".to_string(),
                    actor_id: Some(admin.id.to_string()),
                    action: "punch.create_manual".to_string(),
                    target_type: "punch_event".to_string(),
                    target_id: Some(punch.id.to_string()),
                    before_json: None,
                    after_json: serde_json::to_string(&punch).ok(),
                    metadata_json: Some(
                        serde_json::json!({
                            "request_type": format!("{:?}", request.request_type),
                            "attendance_request_id": id,
                        })
                        .to_string(),
                    ),
                })
                .await
            {
                tracing::error!(error = %e, action = "punch.create_manual", "audit log append failed");
            }
            punch.id
        }
        _ => return Err(StatusCode::CONFLICT),
    };

    let applied = AttendanceRequestRepository::update_status(
        &*state.repo,
        id,
        AttendanceRequestStatus::Applied,
        Some(applied_event_id),
    )
    .await
    .map_err(repo_error_to_status)?;

    append_attendance_request_audit(
        &state,
        &admin,
        "request.approved",
        &request,
        &reviewed,
        serde_json::json!({
            "review_note": input.review_note,
            "applied_event_id": applied_event_id,
        }),
    )
    .await;

    Ok(Json(applied))
}

async fn reject_attendance_request(
    headers: HeaderMap,
    State(state): State<AdminAppState>,
    Path(id): Path<Uuid>,
    Json(input): Json<AttendanceRequestReviewInput>,
) -> Result<Json<AttendanceRequest>, StatusCode> {
    let admin = authenticate_admin_request(&state, &headers).await?;
    let request = AttendanceRequestRepository::find(&*state.repo, id)
        .await
        .map_err(repo_error_to_status)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let rejected = state
        .repo
        .review_attendance_request(
            id,
            admin.id,
            AttendanceRequestStatus::Rejected,
            input.review_note.clone(),
        )
        .await
        .map_err(repo_error_to_status)?;

    append_attendance_request_audit(
        &state,
        &admin,
        "request.rejected",
        &request,
        &rejected,
        serde_json::json!({
            "review_note": input.review_note,
        }),
    )
    .await;

    Ok(Json(rejected))
}

async fn list_audit_logs(
    headers: HeaderMap,
    State(state): State<AdminAppState>,
) -> impl IntoResponse {
    let _admin = match authenticate_admin_request(&state, &headers).await {
        Ok(admin) => admin,
        Err(status) => return Err(status),
    };

    let filter = pasori_core::domain::audit::AuditLogFilter::default();
    match state.repo.list(filter).await {
        Ok(logs) => Ok(Json(logs)),
        Err(e) => {
            tracing::error!(error = ?e, "list_audit_logs error");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

pub(super) async fn authenticate_admin_request(
    state: &AdminAppState,
    headers: &HeaderMap,
) -> Result<AuthenticatedAdmin, StatusCode> {
    let session_id = extract_admin_session_cookie(headers).ok_or(StatusCode::UNAUTHORIZED)?;
    state
        .repo
        .authenticate_admin_session(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::UNAUTHORIZED)
}

pub(super) fn repo_error_to_status(error: RepoError) -> StatusCode {
    match error {
        RepoError::NotFound => StatusCode::NOT_FOUND,
        RepoError::Conflict(_) => StatusCode::CONFLICT,
        RepoError::Db(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

fn extract_admin_session_cookie(headers: &HeaderMap) -> Option<&str> {
    let cookie_header = headers.get(axum::http::header::COOKIE)?.to_str().ok()?;

    cookie_header.split(';').find_map(|entry| {
        let (name, value) = entry.trim().split_once('=')?;
        if name == "admin_session" && !value.is_empty() {
            Some(value)
        } else {
            None
        }
    })
}

fn build_admin_session_cookie(
    session_id: &str,
) -> Result<HeaderValue, axum::http::header::InvalidHeaderValue> {
    let secure = if cfg!(debug_assertions) {
        ""
    } else {
        "; Secure"
    };
    HeaderValue::from_str(&format!(
        "admin_session={}; Path=/; HttpOnly; SameSite=Strict{}; Max-Age=86400",
        session_id, secure
    ))
}

fn clear_admin_session_cookie() -> HeaderValue {
    let secure = if cfg!(debug_assertions) {
        ""
    } else {
        "; Secure"
    };
    HeaderValue::from_static(match secure {
        "" => "admin_session=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0",
        _ => "admin_session=; Path=/; HttpOnly; SameSite=Strict; Secure; Max-Age=0",
    })
}

fn login_metadata_json(headers: &HeaderMap, username: &str) -> String {
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let ip = headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
        .or_else(|| {
            headers
                .get("x-real-ip")
                .and_then(|value| value.to_str().ok())
                .map(str::to_string)
        });

    serde_json::json!({
        "username": username,
        "ip": ip,
        "user_agent": user_agent,
    })
    .to_string()
}

fn merge_login_metadata(base_json: String, extra: serde_json::Value) -> String {
    let mut base = serde_json::from_str::<serde_json::Value>(&base_json)
        .unwrap_or_else(|_| serde_json::json!({}));
    if let (Some(base_obj), Some(extra_obj)) = (base.as_object_mut(), extra.as_object()) {
        for (key, value) in extra_obj {
            base_obj.insert(key.clone(), value.clone());
        }
    }
    base.to_string()
}

async fn apply_correction_request(
    repo: &SqliteRepository,
    request: &AttendanceRequest,
) -> Result<(PunchEvent, PunchEvent), StatusCode> {
    let payload: CorrectionRequestPayload = serde_json::from_str(&request.requested_payload_json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let target_date = parse_date(&payload.date).ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
    let correction_time =
        parse_hhmm_time(&payload.time).ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
    let target_event_type = match payload.target.as_str() {
        "clock_in" => PunchEventType::ClockIn,
        "clock_out" => PunchEventType::ClockOut,
        _ => return Err(StatusCode::INTERNAL_SERVER_ERROR),
    };

    let from = day_start_in_tokyo(target_date).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let to = day_end_in_tokyo(target_date).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let punches = repo
        .list_in_range(request.employee_id, &from, &to)
        .await
        .map_err(repo_error_to_status)?;
    let before_punch = punches
        .into_iter()
        .find(|punch| punch.event_type == target_event_type)
        .ok_or(StatusCode::CONFLICT)?;
    let corrected_at = before_punch
        .occurred_at
        .date()
        .at(correction_time.0, correction_time.1, 0, 0)
        .in_tz("Asia/Tokyo")
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let updated_punch = PunchRepository::update(
        repo,
        before_punch.id,
        PunchPatch {
            event_type: Some(target_event_type),
            occurred_at: Some(corrected_at),
        },
        "admin approved correction".to_string(),
    )
    .await
    .map_err(repo_error_to_status)?;

    Ok((before_punch, updated_punch))
}

#[derive(Deserialize)]
struct MissingPunchPayload {
    time: String,
}

async fn create_missing_punch(
    repo: &SqliteRepository,
    request: &AttendanceRequest,
) -> Result<PunchEvent, StatusCode> {
    let payload: MissingPunchPayload = serde_json::from_str(&request.requested_payload_json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let time = parse_hhmm_time(&payload.time).ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;

    let event_type = match request.request_type {
        AttendanceRequestType::MissingIn => PunchEventType::ClockIn,
        AttendanceRequestType::MissingOut => PunchEventType::ClockOut,
        _ => return Err(StatusCode::INTERNAL_SERVER_ERROR),
    };

    let card_id = CardRepository::find_by_employee(repo, request.employee_id)
        .await
        .map_err(repo_error_to_status)?
        .map(|c| c.id);

    let occurred_at = request
        .requested_at
        .date()
        .at(time.0, time.1, 0, 0)
        .in_tz("Asia/Tokyo")
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let event = PunchRepository::insert(
        repo,
        NewPunchEvent {
            id: uuid::Uuid::now_v7(),
            employee_id: request.employee_id,
            card_id,
            event_type,
            occurred_at,
            source: "lineworks".to_string(),
        },
    )
    .await
    .map_err(repo_error_to_status)?;

    Ok(event)
}

async fn append_attendance_request_audit(
    state: &AdminAppState,
    admin: &AuthenticatedAdmin,
    action: &str,
    before: &AttendanceRequest,
    after: &AttendanceRequest,
    metadata: serde_json::Value,
) {
    if let Err(e) = state
        .repo
        .append(NewAuditLog {
            actor_type: "admin".to_string(),
            actor_id: Some(admin.id.to_string()),
            action: action.to_string(),
            target_type: "attendance_request".to_string(),
            target_id: Some(after.id.to_string()),
            before_json: serde_json::to_string(before).ok(),
            after_json: serde_json::to_string(after).ok(),
            metadata_json: Some(metadata.to_string()),
        })
        .await
    {
        tracing::error!(error = %e, action = action, "audit log append failed");
    }
}

async fn append_punch_update_audit(
    state: &AdminAppState,
    admin: &AuthenticatedAdmin,
    before: &PunchEvent,
    after: &PunchEvent,
    request_id: Uuid,
) {
    if let Err(e) = state
        .repo
        .append(NewAuditLog {
            actor_type: "admin".to_string(),
            actor_id: Some(admin.id.to_string()),
            action: "punch.update".to_string(),
            target_type: "punch_event".to_string(),
            target_id: Some(after.id.to_string()),
            before_json: serde_json::to_string(before).ok(),
            after_json: serde_json::to_string(after).ok(),
            metadata_json: Some(
                serde_json::json!({
                    "reason": "lineworks request approved",
                    "attendance_request_id": request_id,
                })
                .to_string(),
            ),
        })
        .await
    {
        tracing::error!(error = %e, action = "punch.update", "audit log append failed");
    }
}

#[derive(Deserialize)]
struct CorrectionRequestPayload {
    date: String,
    target: String,
    time: String,
}

fn parse_date(input: &str) -> Option<jiff::civil::Date> {
    let (year, rest) = input.split_once('-')?;
    let (month, day) = rest.split_once('-')?;
    let year = year.parse::<i16>().ok()?;
    let month = month.parse::<i8>().ok()?;
    let day = day.parse::<i8>().ok()?;

    jiff::civil::Date::new(year, month, day).ok()
}

fn parse_hhmm_time(input: &str) -> Option<(i8, i8)> {
    let (hour, minute) = input.split_once(':')?;
    let hour = hour.parse::<i8>().ok()?;
    let minute = minute.parse::<i8>().ok()?;
    if !(0..24).contains(&hour) || !(0..60).contains(&minute) {
        return None;
    }
    Some((hour, minute))
}

#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;

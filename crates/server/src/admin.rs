use crate::infra::sqlite::{AdminAuthenticationResult, AuthenticatedAdmin, SqliteRepository};
use argon2::{
    Argon2,
    password_hash::{PasswordHasher, SaltString},
};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::IntoResponse,
    routing::{delete, get, post},
};
use base64::Engine;
use pasori_core::domain::audit::NewAuditLog;
use pasori_core::domain::employee::{EmployeePatch, NewEmployee};
use pasori_core::domain::punch::{
    AttendanceDay, AttendanceDayStatus, NewPunchEvent, PunchEvent, PunchPatch,
};
use pasori_core::domain::request::{
    AttendanceRequest, AttendanceRequestStatus, AttendanceRequestType,
};
use pasori_core::domain::time::{CutoffDay, CutoffRule, MonthlyTimesheet, YearMonth};
use pasori_core::port::policy::NoRounding;
use pasori_core::port::policy::PunchEventType;
use pasori_core::port::repo::{
    AttendanceRequestRepository, AuditLogRepository, CardRepository, EmployeeRepository,
    PunchRepository, RepoError,
};
use rand::{RngCore, rngs::OsRng};
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

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
struct MonthlyAttendanceQuery {
    employee_id: Uuid,
    year: i16,
    month: i8,
}

#[derive(serde::Deserialize)]
struct AttendanceRequestListQuery {
    status: Option<AttendanceRequestStatus>,
}

#[derive(Deserialize)]
struct AttendanceRequestReviewInput {
    review_note: Option<String>,
}

#[derive(Deserialize)]
struct CreateTerminalRequest {
    name: String,
}

#[derive(serde::Serialize)]
struct CreateTerminalResponse {
    terminal: crate::infra::sqlite::TerminalRecord,
    api_token: String,
}

#[derive(serde::Serialize)]
struct RotateTerminalTokenResponse {
    terminal: crate::infra::sqlite::TerminalRecord,
    api_token: String,
}

#[derive(serde::Serialize)]
struct MonthlyAttendanceResponse {
    employee_id: Uuid,
    year_month: MonthlyAttendanceYearMonth,
    days: Vec<AttendanceDayResponse>,
    total_work_minutes: i64,
    cutoff_rule: CutoffRuleResponse,
    period_start: String,
    period_end: String,
}

#[derive(serde::Serialize)]
struct MonthlyAttendanceYearMonth {
    year: i16,
    month: i8,
}

#[derive(serde::Serialize)]
struct AttendanceDayResponse {
    date: String,
    events: Vec<PunchEvent>,
    work_minutes: i64,
    has_inconsistency: bool,
    status: AttendanceDayStatus,
}

#[derive(serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum CutoffRuleResponse {
    DayOfMonth { day: i8 },
    EndOfMonth,
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

async fn list_terminals(
    headers: HeaderMap,
    State(state): State<AdminAppState>,
) -> Result<Json<Vec<crate::infra::sqlite::TerminalRecord>>, StatusCode> {
    let _admin = authenticate_admin_request(&state, &headers).await?;

    match state.repo.list_terminals().await {
        Ok(terminals) => Ok(Json(terminals)),
        Err(e) => {
            tracing::error!(error = ?e, "list_terminals error");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

async fn create_terminal(
    headers: HeaderMap,
    State(state): State<AdminAppState>,
    Json(input): Json<CreateTerminalRequest>,
) -> Result<(StatusCode, Json<CreateTerminalResponse>), StatusCode> {
    let admin = authenticate_admin_request(&state, &headers).await?;
    let api_token = generate_terminal_api_token();
    let api_token_hash =
        hash_terminal_token(&api_token).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let terminal = state
        .repo
        .create_terminal(&input.name, &api_token_hash)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if let Err(e) = state
        .repo
        .append(NewAuditLog {
            actor_type: "admin".to_string(),
            actor_id: Some(admin.id.to_string()),
            action: "terminal.registered".to_string(),
            target_type: "terminal".to_string(),
            target_id: Some(terminal.id.to_string()),
            before_json: None,
            after_json: serde_json::to_string(&terminal).ok(),
            metadata_json: Some(
                serde_json::json!({
                    "name": terminal.name,
                })
                .to_string(),
            ),
        })
        .await
    {
        tracing::error!(error = %e, action = "terminal.registered", "audit log append failed");
    }

    Ok((
        StatusCode::CREATED,
        Json(CreateTerminalResponse {
            terminal,
            api_token,
        }),
    ))
}

async fn rotate_terminal_token(
    headers: HeaderMap,
    State(state): State<AdminAppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<RotateTerminalTokenResponse>, StatusCode> {
    let admin = authenticate_admin_request(&state, &headers).await?;
    let before = state
        .repo
        .find_terminal(id)
        .await
        .map_err(repo_error_to_status)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let api_token = generate_terminal_api_token();
    let api_token_hash =
        hash_terminal_token(&api_token).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let terminal = state
        .repo
        .rotate_terminal_token(id, &api_token_hash)
        .await
        .map_err(repo_error_to_status)?;

    if let Err(e) = state
        .repo
        .append(NewAuditLog {
            actor_type: "admin".to_string(),
            actor_id: Some(admin.id.to_string()),
            action: "terminal.token_rotated".to_string(),
            target_type: "terminal".to_string(),
            target_id: Some(terminal.id.to_string()),
            before_json: serde_json::to_string(&before).ok(),
            after_json: serde_json::to_string(&terminal).ok(),
            metadata_json: Some(
                serde_json::json!({
                    "name": terminal.name,
                })
                .to_string(),
            ),
        })
        .await
    {
        tracing::error!(error = %e, action = "terminal.token_rotated", "audit log append failed");
    }

    Ok(Json(RotateTerminalTokenResponse {
        terminal,
        api_token,
    }))
}

async fn deactivate_terminal(
    headers: HeaderMap,
    State(state): State<AdminAppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, StatusCode> {
    let admin = authenticate_admin_request(&state, &headers).await?;
    let before = state
        .repo
        .find_terminal(id)
        .await
        .map_err(repo_error_to_status)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let terminal = state
        .repo
        .deactivate_terminal(id)
        .await
        .map_err(repo_error_to_status)?;

    if let Err(e) = state
        .repo
        .append(NewAuditLog {
            actor_type: "admin".to_string(),
            actor_id: Some(admin.id.to_string()),
            action: "terminal.deactivated".to_string(),
            target_type: "terminal".to_string(),
            target_id: Some(terminal.id.to_string()),
            before_json: serde_json::to_string(&before).ok(),
            after_json: serde_json::to_string(&terminal).ok(),
            metadata_json: Some(
                serde_json::json!({
                    "name": terminal.name,
                })
                .to_string(),
            ),
        })
        .await
    {
        tracing::error!(error = %e, action = "terminal.deactivated", "audit log append failed");
    }

    Ok(StatusCode::NO_CONTENT)
}

async fn get_monthly_attendance(
    headers: HeaderMap,
    State(state): State<AdminAppState>,
    axum::extract::Query(query): axum::extract::Query<MonthlyAttendanceQuery>,
) -> Result<Json<MonthlyAttendanceResponse>, StatusCode> {
    let _admin = authenticate_admin_request(&state, &headers).await?;

    let year_month =
        YearMonth::new(query.year, query.month).map_err(|_| StatusCode::BAD_REQUEST)?;
    let cutoff_rule =
        CutoffRule::DayOfMonth(CutoffDay::new(15).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?);
    let period = year_month
        .attendance_period(cutoff_rule)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let from =
        day_start_in_tokyo(period.period_start).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let to = day_end_in_tokyo(period.period_end).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let punches = state
        .repo
        .list_in_range(query.employee_id, &from, &to)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let timesheet = build_monthly_attendance(query.employee_id, year_month, cutoff_rule, punches)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(to_monthly_attendance_response(timesheet)))
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

async fn authenticate_admin_request(
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

fn repo_error_to_status(error: RepoError) -> StatusCode {
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
    session_id: &Uuid,
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

fn generate_terminal_api_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn hash_terminal_token(token: &str) -> Result<String, argon2::password_hash::Error> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(token.as_bytes(), &salt)
        .map(|hash| hash.to_string())
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

fn build_monthly_attendance(
    employee_id: Uuid,
    year_month: YearMonth,
    cutoff_rule: CutoffRule,
    punches: Vec<PunchEvent>,
) -> Result<MonthlyTimesheet, pasori_core::domain::time::TimeDomainError> {
    let mut grouped: std::collections::BTreeMap<jiff::civil::Date, Vec<PunchEvent>> =
        std::collections::BTreeMap::new();

    for punch in punches {
        grouped
            .entry(punch.occurred_at.date())
            .or_default()
            .push(punch);
    }

    let days = grouped
        .into_iter()
        .map(|(date, events)| {
            pasori_core::application::attendance::build_attendance_day(
                date,
                events,
                AttendanceDayStatus::Confirmed,
                &NoRounding,
            )
        })
        .collect();

    pasori_core::application::attendance::build_monthly_timesheet(
        employee_id,
        year_month,
        cutoff_rule,
        days,
    )
}

fn to_monthly_attendance_response(timesheet: MonthlyTimesheet) -> MonthlyAttendanceResponse {
    MonthlyAttendanceResponse {
        employee_id: timesheet.employee_id,
        year_month: MonthlyAttendanceYearMonth {
            year: timesheet.year_month.year(),
            month: timesheet.year_month.month(),
        },
        days: timesheet
            .days
            .into_iter()
            .map(to_attendance_day_response)
            .collect(),
        total_work_minutes: timesheet.total_work_minutes,
        cutoff_rule: match timesheet.cutoff_rule {
            CutoffRule::DayOfMonth(day) => CutoffRuleResponse::DayOfMonth { day: day.value() },
            CutoffRule::EndOfMonth => CutoffRuleResponse::EndOfMonth,
        },
        period_start: timesheet.period_start.to_string(),
        period_end: timesheet.period_end.to_string(),
    }
}

fn to_attendance_day_response(day: AttendanceDay) -> AttendanceDayResponse {
    AttendanceDayResponse {
        date: day.date.to_string(),
        events: day.events,
        work_minutes: day.work_minutes,
        has_inconsistency: day.has_inconsistency,
        status: day.status,
    }
}

fn day_start_in_tokyo(date: jiff::civil::Date) -> Result<jiff::Zoned, jiff::Error> {
    format!("{date}T00:00:00+09:00[Asia/Tokyo]").parse()
}

fn day_end_in_tokyo(date: jiff::civil::Date) -> Result<jiff::Zoned, jiff::Error> {
    format!("{date}T23:59:59+09:00[Asia/Tokyo]").parse()
}

#[cfg(test)]
mod tests {
    use super::router;
    use crate::infra::sqlite::SqliteRepository;
    use argon2::{
        Argon2,
        password_hash::{PasswordHasher, SaltString},
    };
    use axum::{body::Body, http::Request, http::StatusCode};
    use serde_json::Value;
    use sqlx::Row;
    use sqlx::sqlite::SqlitePoolOptions;
    use std::sync::Arc;
    use tower::ServiceExt;
    use uuid::Uuid;

    #[tokio::test]
    // Admin API は session cookie なしでは利用できない。
    async fn rejects_admin_request_without_session_cookie() {
        let app = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/admin/employees")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    // 正しい資格情報でログインすると session cookie を返す。
    async fn logs_in_and_sets_admin_session_cookie() {
        let app = test_app().await;
        let body = serde_json::json!({
            "username": "admin",
            "password": "correct horse battery staple",
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/admin/login")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.to_string()))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        let cookie = response
            .headers()
            .get(axum::http::header::SET_COOKIE)
            .and_then(|value| value.to_str().ok())
            .expect("set-cookie header");
        assert!(cookie.contains("admin_session="));
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("SameSite=Strict"));
    }

    #[tokio::test]
    // 連続失敗でロックされた管理者は 423 を返す。
    async fn rejects_locked_admin_login() {
        let app = test_app().await;
        let body = serde_json::json!({
            "username": "admin",
            "password": "wrong-password",
        });

        for _ in 0..4 {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/admin/login")
                        .header(axum::http::header::CONTENT_TYPE, "application/json")
                        .body(Body::from(body.to_string()))
                        .expect("request"),
                )
                .await
                .expect("response");

            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }

        let locked = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/admin/login")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.to_string()))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(locked.status(), StatusCode::LOCKED);
    }

    #[tokio::test]
    // logout は session を破棄して cookie を失効させる。
    async fn logs_out_and_clears_admin_session_cookie() {
        let app = test_app().await;
        let login_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/admin/login")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "username": "admin",
                            "password": "correct horse battery staple",
                        })
                        .to_string(),
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");
        let cookie = login_response
            .headers()
            .get(axum::http::header::SET_COOKIE)
            .and_then(|value| value.to_str().ok())
            .expect("set-cookie header")
            .split(';')
            .next()
            .expect("cookie pair")
            .to_string();

        let logout_response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/admin/logout")
                    .header(axum::http::header::COOKIE, cookie)
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(logout_response.status(), StatusCode::NO_CONTENT);
        let cleared_cookie = logout_response
            .headers()
            .get(axum::http::header::SET_COOKIE)
            .and_then(|value| value.to_str().ok())
            .expect("cleared set-cookie header");
        assert!(cleared_cookie.contains("Max-Age=0"));
    }

    #[tokio::test]
    // 月次勤怠 API は従業員と年月を受けて締め期間内の日次勤怠を返す。
    async fn returns_monthly_timesheet_for_employee_and_year_month() {
        let (app, pool) = test_app_with_pool().await;
        let employee_id = Uuid::now_v7();
        let card_id = Uuid::now_v7();

        sqlx::query(
            "INSERT INTO employee (id, display_name, employment_type, affiliation, is_active, note, created_at, updated_at)
             VALUES (?, ?, ?, NULL, 1, NULL, ?, ?)",
        )
        .bind(employee_id.to_string())
        .bind("山田太郎")
        .bind("regular")
        .bind("2026-04-20T00:00:00+09:00[Asia/Tokyo]")
        .bind("2026-04-20T00:00:00+09:00[Asia/Tokyo]")
        .execute(&pool)
        .await
        .expect("insert employee");

        sqlx::query(
            "INSERT INTO card (id, employee_id, card_identifier, card_label, is_active, created_at, updated_at)
             VALUES (?, ?, ?, NULL, 1, ?, ?)",
        )
        .bind(card_id.to_string())
        .bind(employee_id.to_string())
        .bind("02020212A91B9843")
        .bind("2026-04-20T00:00:00+09:00[Asia/Tokyo]")
        .bind("2026-04-20T00:00:00+09:00[Asia/Tokyo]")
        .execute(&pool)
        .await
        .expect("insert card");

        insert_punch(
            &pool,
            employee_id,
            card_id,
            "clock_in",
            "2026-03-16T09:00:00+09:00[Asia/Tokyo]",
        )
        .await;
        insert_punch(
            &pool,
            employee_id,
            card_id,
            "clock_out",
            "2026-03-16T18:00:00+09:00[Asia/Tokyo]",
        )
        .await;
        insert_punch(
            &pool,
            employee_id,
            card_id,
            "clock_in",
            "2026-04-15T09:30:00+09:00[Asia/Tokyo]",
        )
        .await;
        insert_punch(
            &pool,
            employee_id,
            card_id,
            "clock_out",
            "2026-04-15T18:00:00+09:00[Asia/Tokyo]",
        )
        .await;
        insert_punch(
            &pool,
            employee_id,
            card_id,
            "clock_in",
            "2026-04-16T09:00:00+09:00[Asia/Tokyo]",
        )
        .await;

        let login_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/admin/login")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "username": "admin",
                            "password": "correct horse battery staple",
                        })
                        .to_string(),
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");
        let cookie = login_response
            .headers()
            .get(axum::http::header::SET_COOKIE)
            .and_then(|value| value.to_str().ok())
            .expect("set-cookie header")
            .split(';')
            .next()
            .expect("cookie pair")
            .to_string();

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/admin/attendance/monthly?employee_id={employee_id}&year=2026&month=4"
                    ))
                    .header(axum::http::header::COOKIE, cookie)
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        let json: Value = serde_json::from_slice(&body).expect("json body");

        assert_eq!(json["employee_id"], employee_id.to_string());
        assert_eq!(json["year_month"]["year"], 2026);
        assert_eq!(json["year_month"]["month"], 4);
        assert_eq!(json["period_start"], "2026-03-16");
        assert_eq!(json["period_end"], "2026-04-15");
        assert_eq!(json["days"].as_array().expect("days array").len(), 2);
        assert_eq!(json["total_work_minutes"], 1050);
    }

    #[tokio::test]
    // 管理者は requested の修正申請を承認すると打刻を更新し、申請を applied に進める。
    async fn approves_requested_correction_and_applies_punch_update() {
        let (app, pool) = test_app_with_pool().await;
        let employee_id = Uuid::now_v7();
        let card_id = Uuid::now_v7();
        let request_id = Uuid::now_v7();

        insert_employee(&pool, employee_id).await;
        insert_card(&pool, card_id, employee_id).await;
        insert_punch(
            &pool,
            employee_id,
            card_id,
            "clock_in",
            "2026-04-15T09:00:00+09:00[Asia/Tokyo]",
        )
        .await;
        insert_attendance_request(
            &pool,
            request_id,
            employee_id,
            "correction",
            r#"{"date":"2026-04-15","target":"clock_in","time":"08:32"}"#,
            "requested",
        )
        .await;

        let cookie = login_and_extract_cookie(app.clone()).await;
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/admin/attendance_requests/{request_id}/approve"))
                    .header(axum::http::header::COOKIE, cookie)
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "review_note": "承認して反映",
                        })
                        .to_string(),
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);

        let request_row = sqlx::query(
            "SELECT status, reviewed_by_admin_user_id, reviewed_at, review_note, applied_event_id FROM attendance_request WHERE id = ?",
        )
        .bind(request_id.to_string())
        .fetch_one(&pool)
        .await
        .expect("attendance request row");
        assert_eq!(request_row.get::<String, _>("status"), "applied");
        assert!(
            request_row
                .get::<Option<String>, _>("reviewed_by_admin_user_id")
                .is_some()
        );
        assert!(
            request_row
                .get::<Option<String>, _>("reviewed_at")
                .is_some()
        );
        assert_eq!(
            request_row.get::<Option<String>, _>("review_note"),
            Some("承認して反映".to_string())
        );
        assert!(
            request_row
                .get::<Option<String>, _>("applied_event_id")
                .is_some()
        );

        let punch_row = sqlx::query(
            "SELECT occurred_at, correction_reason FROM punch_event WHERE employee_id = ? AND event_type = 'clock_in'",
        )
        .bind(employee_id.to_string())
        .fetch_one(&pool)
        .await
        .expect("punch row");
        assert_eq!(
            punch_row.get::<String, _>("occurred_at"),
            "2026-04-15T08:32:00+09:00[Asia/Tokyo]"
        );
        assert_eq!(
            punch_row.get::<Option<String>, _>("correction_reason"),
            Some("admin approved correction".to_string())
        );

        let audit_rows =
            sqlx::query("SELECT action FROM audit_log WHERE target_id = ? ORDER BY created_at ASC")
                .bind(request_id.to_string())
                .fetch_all(&pool)
                .await
                .expect("audit rows");
        let actions: Vec<String> = audit_rows
            .into_iter()
            .map(|row| row.get::<String, _>("action"))
            .collect();
        assert!(actions.contains(&"request.approved".to_string()));
    }

    #[tokio::test]
    // 管理者は requested の修正申請を却下でき、audit_log に却下を残す。
    async fn rejects_requested_correction_and_records_audit() {
        let (app, pool) = test_app_with_pool().await;
        let employee_id = Uuid::now_v7();
        let request_id = Uuid::now_v7();

        insert_employee(&pool, employee_id).await;
        insert_attendance_request(
            &pool,
            request_id,
            employee_id,
            "correction",
            r#"{"date":"2026-04-15","target":"clock_out","time":"18:05"}"#,
            "requested",
        )
        .await;

        let cookie = login_and_extract_cookie(app.clone()).await;
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/admin/attendance_requests/{request_id}/reject"))
                    .header(axum::http::header::COOKIE, cookie)
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "review_note": "証跡不足のため差し戻し",
                        })
                        .to_string(),
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);

        let request_row = sqlx::query(
            "SELECT status, review_note, applied_event_id FROM attendance_request WHERE id = ?",
        )
        .bind(request_id.to_string())
        .fetch_one(&pool)
        .await
        .expect("attendance request row");
        assert_eq!(request_row.get::<String, _>("status"), "rejected");
        assert_eq!(
            request_row.get::<Option<String>, _>("review_note"),
            Some("証跡不足のため差し戻し".to_string())
        );
        assert!(
            request_row
                .get::<Option<String>, _>("applied_event_id")
                .is_none()
        );

        let audit_row = sqlx::query(
            "SELECT action, metadata_json FROM audit_log WHERE target_id = ? AND action = 'request.rejected'",
        )
        .bind(request_id.to_string())
        .fetch_one(&pool)
        .await
        .expect("request.rejected audit row");
        let metadata: Value = serde_json::from_str(&audit_row.get::<String, _>("metadata_json"))
            .expect("metadata json");
        assert_eq!(metadata["review_note"], "証跡不足のため差し戻し");
    }

    #[tokio::test]
    // 管理者は status 指定で修正申請一覧を絞り込める。
    async fn filters_attendance_requests_by_status() {
        let (app, pool) = test_app_with_pool().await;
        let employee_id = Uuid::now_v7();

        insert_employee(&pool, employee_id).await;
        insert_attendance_request(
            &pool,
            Uuid::now_v7(),
            employee_id,
            "correction",
            r#"{"date":"2026-04-15","target":"clock_in","time":"08:32"}"#,
            "requested",
        )
        .await;
        insert_attendance_request(
            &pool,
            Uuid::now_v7(),
            employee_id,
            "correction",
            r#"{"date":"2026-04-14","target":"clock_out","time":"18:05"}"#,
            "rejected",
        )
        .await;

        let cookie = login_and_extract_cookie(app.clone()).await;
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/admin/attendance_requests?status=requested")
                    .header(axum::http::header::COOKIE, cookie)
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        let json: Value = serde_json::from_slice(&body).expect("json body");
        assert_eq!(json.as_array().expect("array").len(), 1);
        assert_eq!(json[0]["status"], "requested");
    }

    #[tokio::test]
    // 管理者は登録済み terminal 一覧を取得できる。
    async fn lists_registered_terminals() {
        let (app, pool) = test_app_with_pool().await;
        let terminal_id = Uuid::now_v7();

        insert_terminal(&pool, terminal_id, "受付端末", "terminal-secret").await;

        let cookie = login_and_extract_cookie(app.clone()).await;
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/admin/terminals")
                    .header(axum::http::header::COOKIE, cookie)
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        let json: Value = serde_json::from_slice(&body).expect("json body");
        assert_eq!(json.as_array().expect("terminals array").len(), 1);
        assert_eq!(json[0]["id"], terminal_id.to_string());
        assert_eq!(json[0]["name"], "受付端末");
        assert_eq!(json[0]["is_active"], true);
    }

    #[tokio::test]
    // 管理者は terminal を登録すると平文 token を一度だけ受け取り、監査ログが残る。
    async fn registers_terminal_and_returns_plaintext_token() {
        let (app, pool) = test_app_with_pool().await;
        let cookie = login_and_extract_cookie(app.clone()).await;

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/admin/terminals")
                    .header(axum::http::header::COOKIE, cookie)
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "name": "受付端末",
                        })
                        .to_string(),
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::CREATED);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        let json: Value = serde_json::from_slice(&body).expect("json body");
        assert_eq!(json["terminal"]["name"], "受付端末");
        assert_eq!(json["terminal"]["is_active"], true);
        assert!(
            json["api_token"]
                .as_str()
                .is_some_and(|token| !token.is_empty())
        );

        let terminal_id = json["terminal"]["id"].as_str().expect("terminal id");
        let terminal_row =
            sqlx::query("SELECT name, api_token_hash, is_active FROM terminal WHERE id = ?")
                .bind(terminal_id)
                .fetch_one(&pool)
                .await
                .expect("terminal row");
        assert_eq!(terminal_row.get::<String, _>("name"), "受付端末");
        assert_ne!(
            terminal_row.get::<String, _>("api_token_hash"),
            json["api_token"].as_str().expect("api token")
        );
        assert_eq!(terminal_row.get::<i64, _>("is_active"), 1);

        let audit_row = sqlx::query(
            "SELECT action, target_type, target_id FROM audit_log WHERE action = 'terminal.registered'",
        )
        .fetch_one(&pool)
        .await
        .expect("terminal.registered audit row");
        assert_eq!(audit_row.get::<String, _>("target_type"), "terminal");
        assert_eq!(audit_row.get::<String, _>("target_id"), terminal_id);
    }

    #[tokio::test]
    // 管理者が terminal token を再発行すると旧 token は使えなくなり、新 token だけが有効になる。
    async fn rotates_terminal_token_and_invalidates_previous_token() {
        let (app, pool) = test_app_with_pool().await;
        let terminal_id = Uuid::now_v7();
        insert_terminal(&pool, terminal_id, "受付端末", "terminal-secret").await;
        let cookie = login_and_extract_cookie(app.clone()).await;

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/admin/terminals/{terminal_id}/rotate_token"))
                    .header(axum::http::header::COOKIE, cookie)
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        let json: Value = serde_json::from_slice(&body).expect("json body");
        let new_token = json["api_token"].as_str().expect("new api token");
        assert!(!new_token.is_empty());

        let repo = SqliteRepository::new(pool.clone());
        let old_authenticated = repo
            .authenticate_terminal_token("terminal-secret")
            .await
            .expect("authenticate old token");
        assert!(old_authenticated.is_none());

        let new_authenticated = repo
            .authenticate_terminal_token(new_token)
            .await
            .expect("authenticate new token");
        assert_eq!(
            new_authenticated.expect("new token should authenticate").id,
            terminal_id
        );

        let audit_row = sqlx::query(
            "SELECT action, target_id FROM audit_log WHERE action = 'terminal.token_rotated'",
        )
        .fetch_one(&pool)
        .await
        .expect("terminal.token_rotated audit row");
        assert_eq!(
            audit_row.get::<String, _>("target_id"),
            terminal_id.to_string()
        );
    }

    #[tokio::test]
    // 管理者が terminal を無効化すると terminal token 認証に使えなくなる。
    async fn deactivates_terminal_and_revokes_token_authentication() {
        let (app, pool) = test_app_with_pool().await;
        let terminal_id = Uuid::now_v7();
        insert_terminal(&pool, terminal_id, "受付端末", "terminal-secret").await;
        let cookie = login_and_extract_cookie(app.clone()).await;

        let response = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/admin/terminals/{terminal_id}"))
                    .header(axum::http::header::COOKIE, cookie)
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::NO_CONTENT);

        let row = sqlx::query("SELECT is_active FROM terminal WHERE id = ?")
            .bind(terminal_id.to_string())
            .fetch_one(&pool)
            .await
            .expect("terminal row");
        assert_eq!(row.get::<i64, _>("is_active"), 0);

        let repo = SqliteRepository::new(pool.clone());
        let authenticated = repo
            .authenticate_terminal_token("terminal-secret")
            .await
            .expect("authenticate token");
        assert!(authenticated.is_none());
    }

    async fn test_app() -> axum::Router {
        let (app, _pool) = test_app_with_pool().await;
        app
    }

    async fn test_app_with_pool() -> (axum::Router, sqlx::SqlitePool) {
        let database_url = format!(
            "sqlite:file:{}?mode=memory&cache=shared",
            uuid::Uuid::now_v7()
        );
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&database_url)
            .await
            .expect("sqlite pool");

        sqlx::migrate!("../../migrations")
            .run(&pool)
            .await
            .expect("migrate");

        let now = "2026-04-20T00:00:00+09:00[Asia/Tokyo]";
        let admin_hash = hash_password("correct horse battery staple");
        sqlx::query(
            "INSERT INTO admin_user (id, username, password_hash, display_name, is_active, created_at, updated_at)
             VALUES (?, ?, ?, ?, 1, ?, ?)",
        )
        .bind(Uuid::now_v7().to_string())
        .bind("admin")
        .bind(admin_hash)
        .bind("管理者")
        .bind(now)
        .bind(now)
        .execute(&pool)
        .await
        .expect("insert admin user");

        let app = router(Arc::new(SqliteRepository::new(pool.clone())));
        (app, pool)
    }

    async fn login_and_extract_cookie(app: axum::Router) -> String {
        let login_response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/admin/login")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "username": "admin",
                            "password": "correct horse battery staple",
                        })
                        .to_string(),
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");

        login_response
            .headers()
            .get(axum::http::header::SET_COOKIE)
            .and_then(|value| value.to_str().ok())
            .expect("set-cookie header")
            .split(';')
            .next()
            .expect("cookie pair")
            .to_string()
    }

    async fn insert_employee(pool: &sqlx::SqlitePool, employee_id: Uuid) {
        sqlx::query(
            "INSERT INTO employee (id, display_name, employment_type, affiliation, is_active, note, created_at, updated_at)
             VALUES (?, ?, ?, NULL, 1, NULL, ?, ?)",
        )
        .bind(employee_id.to_string())
        .bind("山田太郎")
        .bind("regular")
        .bind("2026-04-20T00:00:00+09:00[Asia/Tokyo]")
        .bind("2026-04-20T00:00:00+09:00[Asia/Tokyo]")
        .execute(pool)
        .await
        .expect("insert employee");
    }

    async fn insert_card(pool: &sqlx::SqlitePool, card_id: Uuid, employee_id: Uuid) {
        sqlx::query(
            "INSERT INTO card (id, employee_id, card_identifier, card_label, is_active, created_at, updated_at)
             VALUES (?, ?, ?, NULL, 1, ?, ?)",
        )
        .bind(card_id.to_string())
        .bind(employee_id.to_string())
        .bind("02020212A91B9843")
        .bind("2026-04-20T00:00:00+09:00[Asia/Tokyo]")
        .bind("2026-04-20T00:00:00+09:00[Asia/Tokyo]")
        .execute(pool)
        .await
        .expect("insert card");
    }

    async fn insert_terminal(pool: &sqlx::SqlitePool, terminal_id: Uuid, name: &str, token: &str) {
        let now = "2026-04-20T00:00:00+09:00[Asia/Tokyo]";
        sqlx::query(
            "INSERT INTO terminal (id, name, api_token_hash, is_active, created_at, updated_at)
             VALUES (?, ?, ?, 1, ?, ?)",
        )
        .bind(terminal_id.to_string())
        .bind(name)
        .bind(hash_token(token))
        .bind(now)
        .bind(now)
        .execute(pool)
        .await
        .expect("insert terminal");
    }

    fn hash_password(password: &str) -> String {
        let salt =
            SaltString::from_b64("dGVzdF9hZG1pbl9zYWx0").expect("static salt should be valid");
        Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .expect("hash password")
            .to_string()
    }

    fn hash_token(token: &str) -> String {
        let salt =
            SaltString::from_b64("dGVzdF90ZXJtaW5hbF9zYWx0").expect("static salt should be valid");
        Argon2::default()
            .hash_password(token.as_bytes(), &salt)
            .expect("hash token")
            .to_string()
    }

    async fn insert_punch(
        pool: &sqlx::SqlitePool,
        employee_id: Uuid,
        card_id: Uuid,
        event_type: &str,
        occurred_at: &str,
    ) {
        let id = Uuid::now_v7().to_string();
        sqlx::query(
            "INSERT INTO punch_event (id, employee_id, card_id, event_type, occurred_at, server_recorded_at, source, correction_reason, deleted_at, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, 'nfc', NULL, NULL, ?, ?)",
        )
        .bind(id)
        .bind(employee_id.to_string())
        .bind(card_id.to_string())
        .bind(event_type)
        .bind(occurred_at)
        .bind(occurred_at)
        .bind(occurred_at)
        .bind(occurred_at)
        .execute(pool)
        .await
        .expect("insert punch event");

        let row = sqlx::query("SELECT COUNT(*) AS count FROM punch_event WHERE employee_id = ?")
            .bind(employee_id.to_string())
            .fetch_one(pool)
            .await
            .expect("count punch event");
        assert!(row.get::<i64, _>("count") >= 1);
    }

    async fn insert_attendance_request(
        pool: &sqlx::SqlitePool,
        request_id: Uuid,
        employee_id: Uuid,
        request_type: &str,
        requested_payload_json: &str,
        status: &str,
    ) {
        sqlx::query(
            "INSERT INTO attendance_request (id, employee_id, request_type, requested_payload_json, status, requested_via, requested_at, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, 'lineworks', '2026-04-16T10:00:00+09:00[Asia/Tokyo]', '2026-04-16T10:00:00+09:00[Asia/Tokyo]', '2026-04-16T10:00:00+09:00[Asia/Tokyo]')",
        )
        .bind(request_id.to_string())
        .bind(employee_id.to_string())
        .bind(request_type)
        .bind(requested_payload_json)
        .bind(status)
        .execute(pool)
        .await
        .expect("insert attendance request");
    }
}

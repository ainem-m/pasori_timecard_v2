use crate::infra::sqlite::{AdminAuthenticationResult, AuthenticatedAdmin, SqliteRepository};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::IntoResponse,
    routing::{get, post},
};
use pasori_core::domain::audit::NewAuditLog;
use pasori_core::domain::employee::{EmployeePatch, NewEmployee};
use pasori_core::domain::punch::{AttendanceDay, AttendanceDayStatus, PunchEvent};
use pasori_core::domain::time::{CutoffDay, CutoffRule, MonthlyTimesheet, YearMonth};
use pasori_core::port::policy::NoRounding;
use pasori_core::port::repo::{AuditLogRepository, EmployeeRepository, PunchRepository};
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

            let _ = state
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
                .await;

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
            let _ = state
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
                .await;

            Err(StatusCode::UNAUTHORIZED)
        }
        Ok(AdminAuthenticationResult::Locked { locked_until }) => {
            let _ = state
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
                .await;

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
        let _ = state
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
            .await;
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
    let _admin = match authenticate_admin_request(&state, &headers).await {
        Ok(admin) => admin,
        Err(status) => return Err(status),
    };

    match state.repo.create(input).await {
        Ok(employee) => Ok((StatusCode::CREATED, Json(employee))),
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

    match state.repo.find(id).await {
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
    let _admin = match authenticate_admin_request(&state, &headers).await {
        Ok(admin) => admin,
        Err(status) => return Err(status),
    };

    match EmployeeRepository::update(&*state.repo, id, patch).await {
        Ok(employee) => Ok(Json(employee)),
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
    let _admin = match authenticate_admin_request(&state, &headers).await {
        Ok(admin) => admin,
        Err(status) => return Err(status),
    };

    match state.repo.deactivate(id).await {
        Ok(_) => Ok(StatusCode::NO_CONTENT),
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

async fn get_monthly_attendance(
    headers: HeaderMap,
    State(state): State<AdminAppState>,
    axum::extract::Query(query): axum::extract::Query<MonthlyAttendanceQuery>,
) -> Result<Json<MonthlyAttendanceResponse>, StatusCode> {
    let _admin = authenticate_admin_request(&state, &headers).await?;

    let year_month =
        YearMonth::new(query.year, query.month).map_err(|_| StatusCode::BAD_REQUEST)?;
    let cutoff_rule = CutoffRule::DayOfMonth(
        CutoffDay::new(15).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
    );
    let period = year_month
        .attendance_period(cutoff_rule)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let from = day_start_in_tokyo(period.period_start).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
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

fn build_monthly_attendance(
    employee_id: Uuid,
    year_month: YearMonth,
    cutoff_rule: CutoffRule,
    punches: Vec<PunchEvent>,
) -> Result<MonthlyTimesheet, pasori_core::domain::time::TimeDomainError> {
    let mut grouped: std::collections::BTreeMap<jiff::civil::Date, Vec<PunchEvent>> =
        std::collections::BTreeMap::new();

    for punch in punches {
        grouped.entry(punch.occurred_at.date()).or_default().push(punch);
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
        days: timesheet.days.into_iter().map(to_attendance_day_response).collect(),
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

    fn hash_password(password: &str) -> String {
        let salt =
            SaltString::from_b64("dGVzdF9hZG1pbl9zYWx0").expect("static salt should be valid");
        Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .expect("hash password")
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
}

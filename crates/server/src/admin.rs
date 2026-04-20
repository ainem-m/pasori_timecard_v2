use crate::infra::sqlite::{AuthenticatedAdmin, SqliteRepository};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::IntoResponse,
    routing::{get, post},
};
use pasori_core::domain::audit::NewAuditLog;
use pasori_core::domain::employee::{EmployeePatch, NewEmployee};
use pasori_core::port::repo::{AuditLogRepository, EmployeeRepository};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Clone)]
pub struct AdminAppState {
    pub repo: Arc<SqliteRepository>,
}

pub fn router(repo: Arc<SqliteRepository>) -> Router {
    Router::new()
        .route("/admin/login", post(login))
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

async fn login(
    State(state): State<AdminAppState>,
    Json(payload): Json<LoginRequest>,
) -> impl IntoResponse {
    match state
        .repo
        .verify_admin_credentials(&payload.username, &payload.password)
        .await
    {
        Ok(Some(admin)) => {
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
                    metadata_json: Some(
                        serde_json::json!({
                            "session_id": session_id,
                            "expires_at": expires_at.to_string(),
                        })
                        .to_string(),
                    ),
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
        Ok(None) => {
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
                    metadata_json: Some(
                        serde_json::json!({
                            "username": payload.username,
                        })
                        .to_string(),
                    ),
                })
                .await;

            Err(StatusCode::UNAUTHORIZED)
        }
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
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

    match state.repo.update(id, patch).await {
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

#[cfg(test)]
mod tests {
    use super::router;
    use crate::infra::sqlite::SqliteRepository;
    use argon2::{
        Argon2,
        password_hash::{PasswordHasher, SaltString},
    };
    use axum::{body::Body, http::Request, http::StatusCode};
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

    async fn test_app() -> axum::Router {
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

        router(Arc::new(SqliteRepository::new(pool)))
    }

    fn hash_password(password: &str) -> String {
        let salt =
            SaltString::from_b64("dGVzdF9hZG1pbl9zYWx0").expect("static salt should be valid");
        Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .expect("hash password")
            .to_string()
    }
}

use crate::infra::sqlite::{AuthenticatedAdmin, SqliteRepository};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::get,
};
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

#[cfg(test)]
mod tests {
    use super::router;
    use crate::infra::sqlite::SqliteRepository;
    use axum::{body::Body, http::Request, http::StatusCode};
    use sqlx::sqlite::SqlitePoolOptions;
    use std::sync::Arc;
    use tower::ServiceExt;

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

        router(Arc::new(SqliteRepository::new(pool)))
    }
}

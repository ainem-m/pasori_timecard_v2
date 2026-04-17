use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
};
use pasori_core::domain::employee::{EmployeePatch, NewEmployee};
use pasori_core::port::repo::{AuditLogRepository, EmployeeRepository, PunchRepository};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Clone)]
pub struct AdminAppState {
    pub employee_repo: Arc<dyn EmployeeRepository>,
    pub punch_repo: Arc<dyn PunchRepository>,
    pub audit_repo: Arc<dyn AuditLogRepository>,
}

pub fn router(
    employee_repo: Arc<dyn EmployeeRepository>,
    punch_repo: Arc<dyn PunchRepository>,
    audit_repo: Arc<dyn AuditLogRepository>,
) -> Router {
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
        .with_state(AdminAppState {
            employee_repo,
            punch_repo,
            audit_repo,
        })
}

async fn list_employees(State(state): State<AdminAppState>) -> impl IntoResponse {
    match state.employee_repo.list_active().await {
        Ok(employees) => Ok(Json(employees)),
        Err(e) => {
            tracing::error!(error = ?e, "list_employees error");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

async fn create_employee(
    State(state): State<AdminAppState>,
    Json(input): Json<NewEmployee>,
) -> impl IntoResponse {
    match state.employee_repo.create(input).await {
        Ok(employee) => Ok((StatusCode::CREATED, Json(employee))),
        Err(e) => {
            tracing::error!(error = ?e, "create_employee error");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

async fn get_employee(
    State(state): State<AdminAppState>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match state.employee_repo.find(id).await {
        Ok(Some(employee)) => Ok(Json(employee)),
        Ok(None) => Err(StatusCode::NOT_FOUND),
        Err(e) => {
            tracing::error!(error = ?e, id = ?id, "get_employee error");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

async fn update_employee(
    State(state): State<AdminAppState>,
    Path(id): Path<Uuid>,
    Json(patch): Json<EmployeePatch>,
) -> impl IntoResponse {
    match state.employee_repo.update(id, patch).await {
        Ok(employee) => Ok(Json(employee)),
        Err(e) => {
            tracing::error!(error = ?e, id = ?id, "update_employee error");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

async fn deactivate_employee(
    State(state): State<AdminAppState>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match state.employee_repo.deactivate(id).await {
        Ok(_) => Ok(StatusCode::NO_CONTENT),
        Err(e) => {
            tracing::error!(error = ?e, id = ?id, "deactivate_employee error");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

async fn list_punches(State(state): State<AdminAppState>) -> Result<Json<Vec<pasori_core::domain::punch::PunchEvent>>, StatusCode> {
    // Note: MVP simple list. In production, we'd add filters and pagination.
    // For now, we list recent punches for all employees if possible.
    match state.punch_repo.list_in_range(Uuid::nil(), &jiff::Zoned::now(), &jiff::Zoned::now()).await {
        Ok(punches) => Ok(Json(punches)),
        Err(e) => {
            tracing::error!(error = ?e, "list_punches error");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

async fn list_audit_logs(State(state): State<AdminAppState>) -> impl IntoResponse {
    let filter = pasori_core::domain::audit::AuditLogFilter::default();
    match state.audit_repo.list(filter).await {
        Ok(logs) => Ok(Json(logs)),
        Err(e) => {
            tracing::error!(error = ?e, "list_audit_logs error");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

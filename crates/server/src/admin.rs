use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
};
use pasori_core::domain::employee::{EmployeePatch, NewEmployee};
use pasori_core::port::repo::EmployeeRepository;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Clone)]
pub struct AdminAppState {
    pub employee_repo: Arc<dyn EmployeeRepository>,
}

pub fn router(employee_repo: Arc<dyn EmployeeRepository>) -> Router {
    Router::new()
        .route(
            "/api/admin/employees",
            get(list_employees).post(create_employee),
        )
        .route(
            "/api/admin/employees/:id",
            get(get_employee)
                .put(update_employee)
                .delete(deactivate_employee),
        )
        .with_state(AdminAppState { employee_repo })
}

async fn list_employees(State(state): State<AdminAppState>) -> impl IntoResponse {
    match state.employee_repo.list_active().await {
        Ok(employees) => Ok(Json(employees)),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn create_employee(
    State(state): State<AdminAppState>,
    Json(input): Json<NewEmployee>,
) -> impl IntoResponse {
    match state.employee_repo.create(input).await {
        Ok(employee) => Ok((StatusCode::CREATED, Json(employee))),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn get_employee(
    State(state): State<AdminAppState>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match state.employee_repo.find(id).await {
        Ok(Some(employee)) => Ok(Json(employee)),
        Ok(None) => Err(StatusCode::NOT_FOUND),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn update_employee(
    State(state): State<AdminAppState>,
    Path(id): Path<Uuid>,
    Json(patch): Json<EmployeePatch>,
) -> impl IntoResponse {
    match state.employee_repo.update(id, patch).await {
        Ok(employee) => Ok(Json(employee)),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn deactivate_employee(
    State(state): State<AdminAppState>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match state.employee_repo.deactivate(id).await {
        Ok(_) => Ok(StatusCode::NO_CONTENT),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

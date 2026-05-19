use argon2::{
    Argon2,
    password_hash::{PasswordHasher, SaltString},
};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use base64::Engine;
use pasori_core::domain::audit::NewAuditLog;
use pasori_core::port::reader::CardId;
use pasori_core::port::repo::{AuditLogRepository, CardRepository};
use rand::{RngCore, rngs::OsRng};
use serde::Deserialize;
use uuid::Uuid;

use super::{AdminAppState, authenticate_admin_request, repo_error_to_status};

#[derive(Deserialize)]
pub(super) struct CreateTerminalRequest {
    name: String,
}

#[derive(serde::Serialize)]
pub(super) struct CreateTerminalResponse {
    terminal: crate::infra::sqlite::TerminalRecord,
    api_token: String,
}

#[derive(serde::Serialize)]
pub(super) struct RotateTerminalTokenResponse {
    terminal: crate::infra::sqlite::TerminalRecord,
    api_token: String,
}

#[derive(Deserialize)]
pub(super) struct BindCardRequest {
    card_identifier: String,
    employee_id: Uuid,
}

#[derive(Deserialize)]
pub(super) struct UnbindCardRequest {
    card_identifier: String,
}

pub(super) async fn list_terminals(
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

pub(super) async fn create_terminal(
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

pub(super) async fn rotate_terminal_token(
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

pub(super) async fn deactivate_terminal(
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

pub(super) async fn bind_card(
    headers: HeaderMap,
    State(state): State<AdminAppState>,
    Json(input): Json<BindCardRequest>,
) -> Result<(StatusCode, Json<pasori_core::domain::card::Card>), StatusCode> {
    let admin = authenticate_admin_request(&state, &headers).await?;
    let card_id = CardId(input.card_identifier);
    let before = CardRepository::find(&*state.repo, &card_id)
        .await
        .map_err(repo_error_to_status)?;
    let card = CardRepository::bind(&*state.repo, &card_id, input.employee_id)
        .await
        .map_err(repo_error_to_status)?;
    let action = match &before {
        Some(existing) if existing.employee_id != input.employee_id => "card.rebind",
        _ => "card.bind",
    };

    if let Err(e) = state
        .repo
        .append(NewAuditLog {
            actor_type: "admin".to_string(),
            actor_id: Some(admin.id.to_string()),
            action: action.to_string(),
            target_type: "card".to_string(),
            target_id: Some(card.id.to_string()),
            before_json: before
                .as_ref()
                .and_then(|card| serde_json::to_string(card).ok()),
            after_json: serde_json::to_string(&card).ok(),
            metadata_json: None,
        })
        .await
    {
        tracing::error!(error = %e, action = action, "audit log append failed");
    }

    Ok((StatusCode::CREATED, Json(card)))
}

pub(super) async fn unbind_card(
    headers: HeaderMap,
    State(state): State<AdminAppState>,
    Json(input): Json<UnbindCardRequest>,
) -> Result<StatusCode, StatusCode> {
    let admin = authenticate_admin_request(&state, &headers).await?;
    let card_id = CardId(input.card_identifier);
    let before = CardRepository::find(&*state.repo, &card_id)
        .await
        .map_err(repo_error_to_status)?;

    CardRepository::unbind(&*state.repo, &card_id)
        .await
        .map_err(repo_error_to_status)?;

    if let Err(e) = state
        .repo
        .append(NewAuditLog {
            actor_type: "admin".to_string(),
            actor_id: Some(admin.id.to_string()),
            action: "card.unbind".to_string(),
            target_type: "card".to_string(),
            target_id: before.as_ref().map(|card| card.id.to_string()),
            before_json: before
                .as_ref()
                .and_then(|card| serde_json::to_string(card).ok()),
            after_json: None,
            metadata_json: None,
        })
        .await
    {
        tracing::error!(error = %e, action = "card.unbind", "audit log append failed");
    }

    Ok(StatusCode::NO_CONTENT)
}

fn generate_terminal_api_token() -> String {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn hash_terminal_token(token: &str) -> Result<String, argon2::password_hash::Error> {
    let salt = SaltString::generate(&mut OsRng);
    Ok(Argon2::default()
        .hash_password(token.as_bytes(), &salt)?
        .to_string())
}

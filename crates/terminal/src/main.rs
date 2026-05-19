mod api_client;
mod card_binding;
mod clock;
mod offline;
mod punch;
mod rcs380;
mod reader;

use anyhow::Result;
use pasori_core::port::reader::ReaderBackend;
use std::sync::Arc;
use tauri::{Emitter, State};
use uuid::Uuid;

#[derive(serde::Serialize)]
pub struct ClockStatus {
    pub is_synced: bool,
    pub server_time: jiff::Zoned,
    pub local_time: jiff::Zoned,
    pub offset_seconds: i64,
}

struct AppState {
    backend: Arc<dyn ReaderBackend>,
    api_client: api_client::ApiClient,
    offline_repo: Arc<offline::OfflineRepository>,
}

#[tauri::command]
async fn get_reader_status(state: State<'_, AppState>) -> Result<String, String> {
    Ok(format!("{:?}", state.backend.status()))
}

#[tauri::command]
async fn resolve_card(
    state: State<'_, AppState>,
    card_id: String,
) -> Result<api_client::CardScannedResponse, String> {
    match state.api_client.resolve_card(&card_id).await {
        Ok(response) => {
            if let api_client::CardScannedResponse::Registered(ref data) = response {
                let cached = offline::CachedCard {
                    card_id: card_id.clone(),
                    employee_id: data.employee.id,
                    employee_name: data.employee.display_name.clone(),
                    suggested_type: format!("{:?}", data.suggested_type),
                    recent_events_json: serde_json::to_string(&data.recent_events)
                        .unwrap_or_else(|_| "[]".to_string()),
                    cached_at: jiff::Zoned::now(),
                };
                let _ = state.offline_repo.cache_card(&cached).await;
            }
            Ok(response)
        }
        Err(_) => {
            tracing::info!(card_id = %card_id, "server unreachable, falling back to local cache");
            match state.offline_repo.find_cached_card(&card_id).await {
                Ok(Some(cached)) => {
                    let recent_events: Vec<pasori_core::domain::punch::PunchEvent> =
                        serde_json::from_str(&cached.recent_events_json).unwrap_or_default();
                    let suggested_type = match cached.suggested_type.as_str() {
                        "ClockIn" => pasori_core::port::policy::PunchEventType::ClockIn,
                        "ClockOut" => pasori_core::port::policy::PunchEventType::ClockOut,
                        _ => pasori_core::port::policy::PunchEventType::ClockIn,
                    };
                    Ok(api_client::CardScannedResponse::Registered(Box::new(
                        api_client::RegisteredCardScanResponse {
                            employee: pasori_core::domain::employee::Employee {
                                id: cached.employee_id,
                                display_name: cached.employee_name,
                                employment_type: String::new(),
                                affiliation: None,
                                is_active: true,
                                note: None,
                                created_at: cached.cached_at.clone(),
                                updated_at: cached.cached_at,
                            },
                            recent_events,
                            suggested_type,
                        },
                    )))
                }
                Ok(None) => Ok(api_client::CardScannedResponse::Unregistered {
                    card_id: card_id.clone(),
                }),
                Err(e) => {
                    tracing::warn!(error = %e, "local cache lookup failed");
                    Err(
                        "サーバーに接続できず、ローカルキャッシュにも情報がありません。"
                            .to_string(),
                    )
                }
            }
        }
    }
}

#[tauri::command]
async fn list_active_employees(
    state: State<'_, AppState>,
) -> Result<Vec<api_client::TerminalEmployee>, String> {
    state
        .api_client
        .list_active_employees()
        .await
        .map_err(|e| e.to_string())
}

#[derive(serde::Deserialize)]
pub struct BindUnregisteredCardParams {
    pub card_id: String,
    pub employee_id: Uuid,
}

#[tauri::command]
async fn bind_unregistered_card(
    state: State<'_, AppState>,
    params: BindUnregisteredCardParams,
) -> Result<api_client::BindUnregisteredCardResponse, String> {
    let response = state
        .api_client
        .bind_unregistered_card(api_client::BindUnregisteredCardRequest {
            card_id: params.card_id.clone(),
            employee_id: params.employee_id,
        })
        .await
        .map_err(|e| e.to_string())?;

    if let Err(e) = card_binding::cache_bound_card(
        &state.offline_repo,
        &params.card_id,
        response.employee.id,
        &response.employee.display_name,
    )
    .await
    {
        tracing::warn!(error = %e, "server card binding succeeded but local card cache update failed");
    }

    Ok(response)
}

#[derive(serde::Deserialize)]
pub struct SubmitPunchParams {
    pub card_id: String,
    pub event_type: pasori_core::port::policy::PunchEventType,
}

#[tauri::command]
async fn submit_punch(
    state: State<'_, AppState>,
    params: SubmitPunchParams,
) -> Result<pasori_core::domain::punch::PunchEvent, String> {
    let req = punch::create_punch_request(params.card_id.clone(), params.event_type);
    let punch_id = req.punch_id;

    // 1. Save to local cache first (robustness)
    state
        .offline_repo
        .save_punch(
            punch_id,
            &req.card_id,
            &format!("{:?}", req.event_type),
            &req.occurred_at,
            "local_cached",
        )
        .await
        .map_err(|e| format!("failed to save local cache: {}", e))?;

    // 2. Try to submit to server with source "nfc"
    match state.api_client.submit_punch(req.clone()).await {
        Ok(event) => {
            // Success! Mark as synced
            let _ = state
                .offline_repo
                .mark_as_synced(punch_id, &jiff::Zoned::now())
                .await;
            Ok(event)
        }
        Err(e) => {
            // Failed (offline?), but it's okay because it's in the cache.
            tracing::warn!(error = %e, "failed to submit punch to server, will retry later");

            let employee_id = state
                .offline_repo
                .find_cached_card(&req.card_id)
                .await
                .ok()
                .flatten()
                .map(|c| c.employee_id)
                .unwrap_or(Uuid::nil());

            Ok(pasori_core::domain::punch::PunchEvent {
                id: punch_id,
                employee_id,
                card_id: None,
                event_type: req.event_type,
                occurred_at: req.occurred_at.clone(),
                server_recorded_at: req.occurred_at.clone(),
                source: "local_cached".to_string(),
                correction_reason: None,
                deleted_at: None,
                created_at: jiff::Zoned::now(),
                updated_at: jiff::Zoned::now(),
            })
        }
    }
}

#[tauri::command]
async fn check_clock_sync(state: State<'_, AppState>) -> Result<ClockStatus, String> {
    let os_clock = clock::check_os_clock_sync();
    let server_time = state
        .api_client
        .health_check()
        .await
        .map_err(|e| e.to_string())?;
    let local_time = jiff::Zoned::now();

    let offset_seconds =
        (server_time.timestamp().as_second() - local_time.timestamp().as_second()).abs();

    Ok(ClockStatus {
        is_synced: os_clock.is_synced && clock::is_server_offset_synced(offset_seconds),
        server_time,
        local_time,
        offset_seconds,
    })
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();

    tracing::info!("PaSoRi terminal starting...");

    // 1. Api Client
    let api_url =
        std::env::var("SERVER_API_URL").unwrap_or_else(|_| "http://localhost:8080/api".to_string());
    let api_token = std::env::var("TERMINAL_API_TOKEN").ok();
    let api_client = api_client::ApiClient::new(api_url, api_token);

    // 2. Reader Backend
    let backend: Arc<dyn ReaderBackend> =
        Arc::from(reader::detect_and_create().map_err(|e| anyhow::anyhow!(e))?);

    // Start backend
    backend.start().await.map_err(|e| anyhow::anyhow!(e))?;

    // 3. Offline Cache
    let app_data_dir = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string()); // fallback
    let db_path = format!("sqlite://{}/.pasori-terminal.db", app_data_dir);
    let offline_repo = Arc::new(offline::OfflineRepository::new(&db_path).await?);
    tracing::info!(db_path, "Offline repository initialized");

    // 4. Tauri App
    let offline_repo_for_setup = offline_repo.clone();
    let api_client_for_setup = api_client.clone();

    tauri::Builder::default()
        .manage(AppState {
            backend: backend.clone(),
            api_client,
            offline_repo,
        })
        .invoke_handler(tauri::generate_handler![
            get_reader_status,
            resolve_card,
            list_active_employees,
            bind_unregistered_card,
            submit_punch,
            check_clock_sync
        ])
        .setup(move |app| {
            let reader_handle = app.handle().clone();
            let mut rx = backend.subscribe();

            // Bridge broadcast channel to Tauri events
            tauri::async_runtime::spawn(async move {
                while let Ok(scanned) = rx.recv().await {
                    tracing::info!(card_id = %scanned.card_id.0, "bridge: emitting card-scanned event");
                    let _ = reader_handle.emit("card-scanned", scanned.card_id.0);
                }
            });

            // Background sync task
            tauri::async_runtime::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
                loop {
                    interval.tick().await;
                    if let Ok(unsynced) = offline_repo_for_setup.get_unsynced_punches().await {
                        if !unsynced.is_empty() {
                            tracing::info!(count = unsynced.len(), "retrying sync for unsynced punches");
                            for punch in unsynced {
                                let event_type = match punch.event_type.as_str() {
                                    "ClockIn" => pasori_core::port::policy::PunchEventType::ClockIn,
                                    "ClockOut" => pasori_core::port::policy::PunchEventType::ClockOut,
                                    _ => continue, // Skip unknown for now
                                };
                                let req = punch::create_offline_punch_request(
                                    punch.id,
                                    punch.card_id,
                                    event_type,
                                    punch.occurred_at,
                                );
                                match api_client_for_setup.submit_punch(req).await {
                                    Ok(_) => {
                                        let _ = offline_repo_for_setup.mark_as_synced(punch.id, &jiff::Zoned::now()).await;
                                        tracing::info!(id = %punch.id, "sync success");
                                    }
                                    Err(e) => {
                                        tracing::debug!(id = %punch.id, error = %e, "sync retry failed");
                                    }
                                }
                            }
                        }
                    }
                }
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");

    Ok(())
}

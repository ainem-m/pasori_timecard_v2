mod api_client;
mod offline;
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
    state
        .api_client
        .resolve_card(&card_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn submit_punch(
    state: State<'_, AppState>,
    req: api_client::SubmitPunchRequest,
) -> Result<pasori_core::domain::punch::PunchEvent, String> {
    // 1. Save to local cache first (robustness)
    let punch_id = req.punch_id;
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

    // 2. Try to submit to server
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

            // Return a "pending" event so UI can proceed
            Ok(pasori_core::domain::punch::PunchEvent {
                id: punch_id,
                employee_id: Uuid::nil(), // Unknown yet
                card_id: None,
                event_type: req.event_type,
                occurred_at: req.occurred_at.clone(),
                server_recorded_at: req.occurred_at.clone(), // Placeholder
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
    let server_time = state
        .api_client
        .health_check()
        .await
        .map_err(|e| e.to_string())?;
    let local_time = jiff::Zoned::now();

    let offset_seconds =
        (server_time.timestamp().as_second() - local_time.timestamp().as_second()).abs();

    Ok(ClockStatus {
        is_synced: offset_seconds <= 10,
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
                                let req = api_client::SubmitPunchRequest {
                                    punch_id: punch.id,
                                    card_id: punch.card_id,
                                    event_type,
                                    occurred_at: punch.occurred_at,
                                    source: punch.source,
                                };
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

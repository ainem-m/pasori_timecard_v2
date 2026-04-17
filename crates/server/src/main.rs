use anyhow::{Context, Result};
use pasori_core::WORKSPACE_STATUS;
use pasori_core::application::attendance::PunchUseCase;
use pasori_core::application::lineworks::LineworksUseCase;
use pasori_core::port::policy::DefaultPunchPolicy;
use server::infra::lineworks_notify::LineworksNotifier;
use server::infra::sqlite::SqliteRepository;
use server::lineworks;
use sqlx::sqlite::SqlitePoolOptions;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    tracing::info!("server workspace status = {}", WORKSPACE_STATUS);

    let database_url =
        std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite:pasori_timecard.db?mode=rwc".to_string());

    tracing::info!(database_url, "connecting to database...");
    
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .with_context(|| format!("failed to connect to SQLite at {}", database_url))?;

    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .context("failed to run migrations")?;

    let repo = Arc::new(SqliteRepository::new(pool));

    let notifier: Arc<dyn pasori_core::port::notify::Notifier> =
        match (std::env::var("LINEWORKS_BOT_ID"), std::env::var("LINEWORKS_API_TOKEN")) {
            (Ok(bot_id), Ok(bot_token)) => {
                tracing::info!("LINE WORKS notifier initialized");
                Arc::new(LineworksNotifier::new(bot_id, bot_token))
            }
            _ => {
                tracing::warn!("LINE WORKS credentials missing, falling back to ConsoleNotifier");
                Arc::new(server::infra::console_notify::ConsoleNotifier)
            }
        };

    let bot_secret = std::env::var("LINEWORKS_BOT_SECRET").unwrap_or_else(|_| "dummy_secret".to_string());

    let punch_policy = Arc::new(DefaultPunchPolicy);

    let punch_use_case = Arc::new(PunchUseCase::new(
        repo.clone(),
        repo.clone(),
        repo.clone(),
        repo.clone(),
        notifier.clone(),
        punch_policy,
    ));

    let lineworks_use_case = Arc::new(LineworksUseCase::new(
        repo.clone(),
        repo.clone(),
        repo.clone(),
        repo.clone(),
        notifier.clone(),
    ));

    let api_router = axum::Router::new()
        .merge(lineworks::router(bot_secret.into_bytes(), lineworks_use_case))
        .merge(server::terminal::router(punch_use_case))
        .merge(server::admin::router(repo.clone(), repo.clone(), repo.clone()));

    let app = axum::Router::new()
        .nest("/api", api_router)
        .fallback(server::web_assets::static_handler);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080")
        .await
        .context("failed to bind server socket")?;

    tracing::info!("server listening on 0.0.0.0:8080");
    axum::serve(listener, app)
        .await
        .context("server failed to run")?;

    Ok(())
}

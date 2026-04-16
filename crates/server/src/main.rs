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
        std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite:pasori_timecard.db".to_string());

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .context("failed to connect to SQLite")?;

    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .context("failed to run migrations")?;

    let repo = Arc::new(SqliteRepository::new(pool));

    let bot_id = std::env::var("LINEWORKS_BOT_ID").context("LINEWORKS_BOT_ID is required")?;
    let bot_secret =
        std::env::var("LINEWORKS_BOT_SECRET").context("LINEWORKS_BOT_SECRET is required")?;
    let bot_token =
        std::env::var("LINEWORKS_API_TOKEN").context("LINEWORKS_API_TOKEN is required")?;

    let notifier = Arc::new(LineworksNotifier::new(bot_id, bot_token));
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

    // Combine use cases as needed.
    let app = lineworks::router(bot_secret.into_bytes(), lineworks_use_case)
        .merge(server::terminal::router(punch_use_case))
        .merge(server::admin::router(repo.clone()));

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080")
        .await
        .context("failed to bind server socket")?;

    tracing::info!("server listening on 0.0.0.0:8080");
    axum::serve(listener, app)
        .await
        .context("server failed to run")?;

    Ok(())
}

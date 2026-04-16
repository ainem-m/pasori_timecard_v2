use anyhow::{Context, Result};
use pasori_core::WORKSPACE_STATUS;
use server::lineworks;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    tracing::info!("server workspace status = {}", WORKSPACE_STATUS);
    let bot_secret = std::env::var("LINEWORKS_BOT_SECRET")
        .context("LINEWORKS_BOT_SECRET is required to start the server")?;
    let app = lineworks::router(bot_secret.into_bytes());
    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080")
        .await
        .context("failed to bind server socket")?;

    tracing::info!("server listening on 0.0.0.0:8080");
    axum::serve(listener, app)
        .await
        .context("server failed to run")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use pasori_core::WORKSPACE_STATUS;

    #[test]
    // server クレートは core に依存できる。
    fn server_can_depend_on_core() {
        assert_eq!(WORKSPACE_STATUS, "ready");
    }
}

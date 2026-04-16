mod reader;
mod rcs380;

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();

    tracing::info!("PaSoRi reader starting...");

    let backend = reader::detect_and_create()?;
    backend.start().await?;

    tracing::info!("reader status: {:?}", backend.status());

    let mut rx = backend.subscribe();

    tracing::info!("カードをタッチしてください (Ctrl+C で終了)");

    loop {
        match rx.recv().await {
            Ok(scanned) => {
                println!(
                    "[{}] card_id = {}",
                    scanned.scanned_at.strftime("%H:%M:%S"),
                    scanned.card_id.0,
                );
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                tracing::warn!("broadcast lagged by {n} messages");
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                tracing::info!("reader stopped");
                break;
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use pasori_core::WORKSPACE_STATUS;

    #[test]
    // terminal クレートは core に依存できる。
    fn terminal_can_depend_on_core() {
        assert_eq!(WORKSPACE_STATUS, "ready");
    }
}

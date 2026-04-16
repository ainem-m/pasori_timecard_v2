use anyhow::Result;
use pasori_core::WORKSPACE_STATUS;

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    tracing::info!("terminal workspace status = {}", WORKSPACE_STATUS);
    Ok(())
}

#[cfg(test)]
mod tests {
    use pasori_core::WORKSPACE_STATUS;

    #[test]
    fn terminalはcoreに依存できる() {
        assert_eq!(WORKSPACE_STATUS, "ready");
    }
}

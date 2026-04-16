use anyhow::Result;
use pasori_core::WORKSPACE_STATUS;

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    tracing::info!("server workspace status = {}", WORKSPACE_STATUS);
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

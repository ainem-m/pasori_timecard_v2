use anyhow::Result;
use pasori_core::WORKSPACE_STATUS;

fn main() -> Result<()> {
    println!("import_v1 workspace status = {}", WORKSPACE_STATUS);
    Ok(())
}

#[cfg(test)]
mod tests {
    use pasori_core::WORKSPACE_STATUS;

    #[test]
    fn import_v1はcoreに依存できる() {
        assert_eq!(WORKSPACE_STATUS, "ready");
    }
}

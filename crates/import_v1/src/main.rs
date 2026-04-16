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
    // import_v1 クレートは core に依存できる。
    fn import_v1_can_depend_on_core() {
        assert_eq!(WORKSPACE_STATUS, "ready");
    }
}

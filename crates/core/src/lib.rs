pub mod application;
pub mod domain;
pub mod error;
pub mod port;

pub const WORKSPACE_STATUS: &str = "ready";

#[cfg(test)]
mod tests {
    use crate::WORKSPACE_STATUS;

    #[test]
    // ワークスペース準備状態は ready を返す。
    fn returns_ready_for_workspace_status() {
        assert_eq!(WORKSPACE_STATUS, "ready");
    }
}

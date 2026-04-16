pub mod application;
pub mod domain;
pub mod error;
pub mod port;

pub const WORKSPACE_STATUS: &str = "ready";

#[cfg(test)]
mod tests {
    use crate::WORKSPACE_STATUS;

    #[test]
    fn workspace準備状態はreadyを返す() {
        assert_eq!(WORKSPACE_STATUS, "ready");
    }
}

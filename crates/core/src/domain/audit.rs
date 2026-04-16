use jiff::Zoned;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuditLog {
    pub id: Uuid,
    pub actor_type: String, // 'admin' / 'employee' / 'system' / 'terminal'
    pub actor_id: Option<String>,
    pub action: String,
    pub target_type: String,
    pub target_id: Option<String>,
    pub before_json: Option<String>,
    pub after_json: Option<String>,
    pub metadata_json: Option<String>,
    pub created_at: Zoned,
}

#[derive(Debug, Clone)]
pub struct NewAuditLog {
    pub actor_type: String,
    pub actor_id: Option<String>,
    pub action: String,
    pub target_type: String,
    pub target_id: Option<String>,
    pub before_json: Option<String>,
    pub after_json: Option<String>,
    pub metadata_json: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct AuditLogFilter {
    pub actor_id: Option<String>,
    pub action: Option<String>,
    pub target_id: Option<String>,
    pub from: Option<Zoned>,
    pub to: Option<Zoned>,
}

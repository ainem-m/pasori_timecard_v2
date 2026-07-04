use jiff::{Zoned, civil::Date};
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
    pub prev_hash: Option<String>,
    pub entry_hash: Option<String>,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuditLogChainVerification {
    pub is_valid: bool,
    pub checked_entries: usize,
    pub legacy_entries: usize,
    pub first_invalid_audit_log_id: Option<Uuid>,
    pub first_invalid_action: Option<String>,
    pub last_valid_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuditDigest {
    pub date: Date,
    pub entry_count: usize,
    pub first_entry_hash: Option<String>,
    pub last_entry_hash: Option<String>,
    pub digest_hash: String,
}

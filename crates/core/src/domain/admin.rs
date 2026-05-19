use jiff::Zoned;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdminUser {
    pub id: Uuid,
    pub username: String,
    pub display_name: String,
    pub is_active: bool,
    pub created_at: Zoned,
    pub updated_at: Zoned,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdminSession {
    pub id: Uuid,
    pub admin_user_id: Uuid,
    pub expires_at: Zoned,
    pub last_active_at: Zoned,
    pub created_at: Zoned,
}

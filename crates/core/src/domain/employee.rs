use jiff::Zoned;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Employee {
    pub id: Uuid,
    pub display_name: String,
    pub employment_type: String,
    pub affiliation: Option<String>,
    pub is_active: bool,
    pub note: Option<String>,
    pub created_at: Zoned,
    pub updated_at: Zoned,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExternalAccount {
    pub id: Uuid,
    pub employee_id: Uuid,
    pub provider: String, // 'lineworks'
    pub external_user_id: String,
    pub external_domain_id: Option<String>,
    pub is_verified: bool,
    pub created_at: Zoned,
    pub updated_at: Zoned,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewEmployee {
    pub display_name: String,
    pub employment_type: String,
    pub affiliation: Option<String>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct EmployeePatch {
    pub display_name: Option<String>,
    pub employment_type: Option<String>,
    pub affiliation: Option<Option<String>>,
    pub is_active: Option<bool>,
    pub note: Option<Option<String>>,
}

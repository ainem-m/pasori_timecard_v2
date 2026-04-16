use jiff::Zoned;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::port::reader::CardId;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Card {
    pub id: Uuid,
    pub employee_id: Uuid,
    pub card_identifier: CardId,
    pub card_label: Option<String>,
    pub is_active: bool,
    pub created_at: Zoned,
    pub updated_at: Zoned,
}

use jiff::Zoned;
use jiff::civil::Date;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShiftType {
    pub id: Uuid,
    pub code: String,
    pub display_name: String,
    pub planned_start_time: Option<String>, // 'HH:MM'
    pub planned_end_time: Option<String>,
    pub default_break_minutes: Option<i32>,
    pub color: String,
    pub is_active: bool,
    pub created_at: Zoned,
    pub updated_at: Zoned,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShiftAssignment {
    pub id: Uuid,
    pub employee_id: Uuid,
    pub date: Date,
    pub shift_type_id: Uuid,
    pub planned_start_at: Option<Zoned>,
    pub planned_end_at: Option<Zoned>,
    pub note: Option<String>,
    pub status: ShiftAssignmentStatus,
    pub created_at: Zoned,
    pub updated_at: Zoned,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ShiftAssignmentStatus {
    Draft,
    Published,
    Finalized,
}

impl ShiftAssignmentStatus {
    pub fn transition_to(self, next: Self) -> Result<Self, ShiftTransitionError> {
        if self.can_transition_to(next) {
            Ok(next)
        } else {
            Err(ShiftTransitionError::InvalidTransition {
                from: self,
                to: next,
            })
        }
    }

    pub fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (
                ShiftAssignmentStatus::Draft,
                ShiftAssignmentStatus::Published
            ) | (
                ShiftAssignmentStatus::Published,
                ShiftAssignmentStatus::Draft
            ) | (
                ShiftAssignmentStatus::Published,
                ShiftAssignmentStatus::Finalized
            )
        )
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ShiftTransitionError {
    #[error("invalid shift assignment status transition: {from:?} -> {to:?}")]
    InvalidTransition {
        from: ShiftAssignmentStatus,
        to: ShiftAssignmentStatus,
    },
}

#[cfg(test)]
mod tests {
    use super::{ShiftAssignmentStatus, ShiftTransitionError};
    use proptest::prelude::*;

    #[test]
    // 下書きから公開への遷移は許可される。
    fn allows_draft_to_published_transition() {
        let next = ShiftAssignmentStatus::Draft
            .transition_to(ShiftAssignmentStatus::Published)
            .expect("transition should be allowed");

        assert_eq!(next, ShiftAssignmentStatus::Published);
    }

    #[test]
    // 公開後は締め済みへ遷移できる。
    fn allows_published_to_finalized_transition() {
        let next = ShiftAssignmentStatus::Published
            .transition_to(ShiftAssignmentStatus::Finalized)
            .expect("transition should be allowed");

        assert_eq!(next, ShiftAssignmentStatus::Finalized);
    }

    #[test]
    // 公開後は下書きへ差し戻せる。
    fn allows_published_to_draft_transition() {
        let next = ShiftAssignmentStatus::Published
            .transition_to(ShiftAssignmentStatus::Draft)
            .expect("transition should be allowed");

        assert_eq!(next, ShiftAssignmentStatus::Draft);
    }

    #[test]
    // 下書きから締め済みへの直接遷移は拒否される。
    fn rejects_draft_to_finalized_transition() {
        let error = ShiftAssignmentStatus::Draft
            .transition_to(ShiftAssignmentStatus::Finalized)
            .expect_err("transition should be rejected");

        assert_eq!(
            error,
            ShiftTransitionError::InvalidTransition {
                from: ShiftAssignmentStatus::Draft,
                to: ShiftAssignmentStatus::Finalized,
            }
        );
    }

    #[test]
    // 締め済み状態からの遷移は拒否される。
    fn rejects_any_transition_from_finalized() {
        assert!(matches!(
            ShiftAssignmentStatus::Finalized.transition_to(ShiftAssignmentStatus::Draft),
            Err(ShiftTransitionError::InvalidTransition { .. })
        ));
    }

    proptest! {
        #[test]
        // 無効な遷移はすべてエラーになる。
        fn rejects_invalid_status_transitions(
            from in prop_oneof![
                Just(ShiftAssignmentStatus::Draft),
                Just(ShiftAssignmentStatus::Published),
                Just(ShiftAssignmentStatus::Finalized),
            ],
            to in prop_oneof![
                Just(ShiftAssignmentStatus::Draft),
                Just(ShiftAssignmentStatus::Published),
                Just(ShiftAssignmentStatus::Finalized),
            ],
        ) {
            let transition = from.transition_to(to);

            let expected_allowed = matches!(
                (from, to),
                (ShiftAssignmentStatus::Draft, ShiftAssignmentStatus::Published)
                    | (ShiftAssignmentStatus::Published, ShiftAssignmentStatus::Draft)
                    | (ShiftAssignmentStatus::Published, ShiftAssignmentStatus::Finalized)
            );

            prop_assert_eq!(transition.is_ok(), expected_allowed);
        }
    }
}

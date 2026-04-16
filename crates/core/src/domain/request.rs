use jiff::Zoned;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AttendanceRequestType {
    Correction,
    MissingIn,
    MissingOut,
    QueryAttendance,
    QueryShift,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AttendanceRequestSource {
    LineWorks,
    Ui,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AttendanceRequestStatus {
    Requested,
    AutoApproved,
    Approved,
    Rejected,
    Applied,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AttendanceRequest {
    pub id: Uuid,
    pub employee_id: Uuid,
    pub request_type: AttendanceRequestType,
    pub requested_payload_json: String,
    pub status: AttendanceRequestStatus,
    pub requested_via: AttendanceRequestSource,
    pub requested_at: Zoned,
    pub reviewed_by_admin_user_id: Option<Uuid>,
    pub reviewed_at: Option<Zoned>,
    pub review_note: Option<String>,
    pub applied_event_id: Option<Uuid>,
}

#[derive(Debug, Clone)]
pub struct NewAttendanceRequest {
    pub employee_id: Uuid,
    pub request_type: AttendanceRequestType,
    pub requested_payload_json: String,
    pub requested_via: AttendanceRequestSource,
    pub requested_at: Zoned,
}

impl AttendanceRequestStatus {
    pub fn transition_to(self, next: Self) -> Result<Self, AttendanceRequestTransitionError> {
        if self.can_transition_to(next) {
            Ok(next)
        } else {
            Err(AttendanceRequestTransitionError::InvalidTransition {
                from: self,
                to: next,
            })
        }
    }

    pub fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (
                AttendanceRequestStatus::Requested,
                AttendanceRequestStatus::AutoApproved
            ) | (
                AttendanceRequestStatus::Requested,
                AttendanceRequestStatus::Approved
            ) | (
                AttendanceRequestStatus::Requested,
                AttendanceRequestStatus::Rejected
            ) | (
                AttendanceRequestStatus::Requested,
                AttendanceRequestStatus::Cancelled
            ) | (
                AttendanceRequestStatus::AutoApproved,
                AttendanceRequestStatus::Applied
            ) | (
                AttendanceRequestStatus::Approved,
                AttendanceRequestStatus::Applied
            )
        )
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AttendanceRequestTransitionError {
    #[error("invalid attendance request status transition: {from:?} -> {to:?}")]
    InvalidTransition {
        from: AttendanceRequestStatus,
        to: AttendanceRequestStatus,
    },
}

#[cfg(test)]
mod tests {
    use super::{AttendanceRequestStatus, AttendanceRequestTransitionError};
    use proptest::prelude::*;

    #[test]
    // 申請直後の状態から自動承認へ遷移できる。
    fn allows_requested_to_auto_approved_transition() {
        let next = AttendanceRequestStatus::Requested
            .transition_to(AttendanceRequestStatus::AutoApproved)
            .expect("transition should be allowed");

        assert_eq!(next, AttendanceRequestStatus::AutoApproved);
    }

    #[test]
    // 申請直後の状態から管理者承認へ遷移できる。
    fn allows_requested_to_approved_transition() {
        let next = AttendanceRequestStatus::Requested
            .transition_to(AttendanceRequestStatus::Approved)
            .expect("transition should be allowed");

        assert_eq!(next, AttendanceRequestStatus::Approved);
    }

    #[test]
    // 自動承認済みの申請は反映済みに遷移できる。
    fn allows_auto_approved_to_applied_transition() {
        let next = AttendanceRequestStatus::AutoApproved
            .transition_to(AttendanceRequestStatus::Applied)
            .expect("transition should be allowed");

        assert_eq!(next, AttendanceRequestStatus::Applied);
    }

    #[test]
    // 管理者承認済みの申請は反映済みに遷移できる。
    fn allows_approved_to_applied_transition() {
        let next = AttendanceRequestStatus::Approved
            .transition_to(AttendanceRequestStatus::Applied)
            .expect("transition should be allowed");

        assert_eq!(next, AttendanceRequestStatus::Applied);
    }

    #[test]
    // 申請直後の状態から却下へ遷移できる。
    fn allows_requested_to_rejected_transition() {
        let next = AttendanceRequestStatus::Requested
            .transition_to(AttendanceRequestStatus::Rejected)
            .expect("transition should be allowed");

        assert_eq!(next, AttendanceRequestStatus::Rejected);
    }

    #[test]
    // 申請直後の状態から取消へ遷移できる。
    fn allows_requested_to_cancelled_transition() {
        let next = AttendanceRequestStatus::Requested
            .transition_to(AttendanceRequestStatus::Cancelled)
            .expect("transition should be allowed");

        assert_eq!(next, AttendanceRequestStatus::Cancelled);
    }

    #[test]
    // 却下済み申請は反映済みに遷移できない。
    fn rejects_rejected_to_applied_transition() {
        let error = AttendanceRequestStatus::Rejected
            .transition_to(AttendanceRequestStatus::Applied)
            .expect_err("transition should be rejected");

        assert_eq!(
            error,
            AttendanceRequestTransitionError::InvalidTransition {
                from: AttendanceRequestStatus::Rejected,
                to: AttendanceRequestStatus::Applied,
            }
        );
    }

    #[test]
    // 反映済み状態からの遷移は拒否される。
    fn rejects_any_transition_from_applied() {
        assert!(matches!(
            AttendanceRequestStatus::Applied.transition_to(AttendanceRequestStatus::Approved),
            Err(AttendanceRequestTransitionError::InvalidTransition { .. })
        ));
    }

    proptest! {
        #[test]
        // 許可した遷移だけが成功し、それ以外は失敗する。
        fn respects_request_status_transition_matrix(
            from in prop_oneof![
                Just(AttendanceRequestStatus::Requested),
                Just(AttendanceRequestStatus::AutoApproved),
                Just(AttendanceRequestStatus::Approved),
                Just(AttendanceRequestStatus::Rejected),
                Just(AttendanceRequestStatus::Applied),
                Just(AttendanceRequestStatus::Cancelled),
            ],
            to in prop_oneof![
                Just(AttendanceRequestStatus::Requested),
                Just(AttendanceRequestStatus::AutoApproved),
                Just(AttendanceRequestStatus::Approved),
                Just(AttendanceRequestStatus::Rejected),
                Just(AttendanceRequestStatus::Applied),
                Just(AttendanceRequestStatus::Cancelled),
            ],
        ) {
            let transition = from.transition_to(to);

            let expected_allowed = matches!(
                (from, to),
                (AttendanceRequestStatus::Requested, AttendanceRequestStatus::AutoApproved)
                    | (AttendanceRequestStatus::Requested, AttendanceRequestStatus::Approved)
                    | (AttendanceRequestStatus::Requested, AttendanceRequestStatus::Rejected)
                    | (AttendanceRequestStatus::Requested, AttendanceRequestStatus::Cancelled)
                    | (AttendanceRequestStatus::AutoApproved, AttendanceRequestStatus::Applied)
                    | (AttendanceRequestStatus::Approved, AttendanceRequestStatus::Applied)
            );

            prop_assert_eq!(transition.is_ok(), expected_allowed);
        }
    }
}

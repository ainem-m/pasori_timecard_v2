use async_trait::async_trait;
use jiff::Zoned;
use uuid::Uuid;

use crate::domain::audit::{AuditLog, AuditLogFilter, NewAuditLog};
use crate::domain::card::Card;
use crate::domain::employee::{Employee, EmployeePatch, ExternalAccount, NewEmployee};
use crate::domain::punch::{NewPunchEvent, PunchEvent, PunchPatch};
use crate::domain::request::{AttendanceRequest, AttendanceRequestStatus, NewAttendanceRequest};
use crate::domain::shift::{ShiftAssignment, ShiftType};
use crate::domain::time::YearMonth;
use crate::port::reader::CardId;

#[async_trait]
pub trait EmployeeRepository: Send + Sync {
    async fn list_active(&self) -> Result<Vec<Employee>, RepoError>;
    async fn find(&self, id: Uuid) -> Result<Option<Employee>, RepoError>;
    async fn find_by_card(&self, card_id: &CardId) -> Result<Option<Employee>, RepoError>;
    async fn create(&self, input: NewEmployee) -> Result<Employee, RepoError>;
    async fn update(&self, id: Uuid, patch: EmployeePatch) -> Result<Employee, RepoError>;
    async fn deactivate(&self, id: Uuid) -> Result<(), RepoError>;
}

#[async_trait]
pub trait CardRepository: Send + Sync {
    async fn find(&self, card_id: &CardId) -> Result<Option<Card>, RepoError>;
    async fn bind(&self, card_id: &CardId, employee_id: Uuid) -> Result<Card, RepoError>;
    async fn unbind(&self, card_id: &CardId) -> Result<(), RepoError>;
}

#[async_trait]
pub trait PunchRepository: Send + Sync {
    async fn insert(&self, event: NewPunchEvent) -> Result<PunchEvent, RepoError>;
    async fn recent_for_employee(
        &self,
        employee_id: Uuid,
        limit: usize,
    ) -> Result<Vec<PunchEvent>, RepoError>;
    async fn list_in_range(
        &self,
        employee_id: Uuid,
        from: &Zoned,
        to: &Zoned,
    ) -> Result<Vec<PunchEvent>, RepoError>;
    async fn update(
        &self,
        id: Uuid,
        patch: PunchPatch,
        reason: String,
    ) -> Result<PunchEvent, RepoError>;
    async fn soft_delete(&self, id: Uuid, reason: String) -> Result<(), RepoError>;
}

#[async_trait]
pub trait ShiftRepository: Send + Sync {
    async fn list_for_month(
        &self,
        employee_id: Uuid,
        year_month: YearMonth,
    ) -> Result<Vec<ShiftAssignment>, RepoError>;
    async fn list_types(&self) -> Result<Vec<ShiftType>, RepoError>;
}

#[async_trait]
pub trait AuditLogRepository: Send + Sync {
    async fn append(&self, entry: NewAuditLog) -> Result<(), RepoError>;
    async fn list(&self, filter: AuditLogFilter) -> Result<Vec<AuditLog>, RepoError>;
}

#[async_trait]
pub trait ExternalAccountRepository: Send + Sync {
    async fn find_by_external_id(
        &self,
        provider: &str,
        external_user_id: &str,
    ) -> Result<Option<ExternalAccount>, RepoError>;
    async fn bind(
        &self,
        employee_id: Uuid,
        provider: &str,
        external_user_id: &str,
    ) -> Result<ExternalAccount, RepoError>;
}

#[async_trait]
pub trait AttendanceRequestRepository: Send + Sync {
    async fn create(&self, input: NewAttendanceRequest) -> Result<AttendanceRequest, RepoError>;
    async fn find(&self, id: Uuid) -> Result<Option<AttendanceRequest>, RepoError>;
    async fn update_status(
        &self,
        id: Uuid,
        status: AttendanceRequestStatus,
        applied_event_id: Option<Uuid>,
    ) -> Result<AttendanceRequest, RepoError>;
}

#[derive(Debug, thiserror::Error)]
pub enum RepoError {
    #[error("not found")]
    NotFound,
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("db error: {0}")]
    Db(String),
}

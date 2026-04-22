use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordVerifier},
};
use async_trait::async_trait;
use jiff::Zoned;
use pasori_core::domain::audit::{AuditLog, AuditLogFilter, NewAuditLog};
use pasori_core::domain::card::Card;
use pasori_core::domain::employee::{Employee, EmployeePatch, ExternalAccount, NewEmployee};
use pasori_core::domain::punch::{NewPunchEvent, PunchEvent, PunchPatch};
use pasori_core::domain::request::{
    AttendanceRequest, AttendanceRequestStatus, NewAttendanceRequest,
};
use pasori_core::domain::shift::{ShiftAssignment, ShiftType};
use pasori_core::domain::time::YearMonth;
use pasori_core::port::reader::CardId;
use pasori_core::port::repo::{
    AttendanceRequestRepository, AuditLogRepository, CardRepository, EmployeeRepository,
    ExternalAccountRepository, PunchRepository, RepoError, ShiftRepository,
};
use sqlx::{Row, sqlite::SqlitePool};
use uuid::Uuid;

pub struct SqliteRepository {
    pool: SqlitePool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatedTerminal {
    pub id: Uuid,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatedAdmin {
    pub id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TerminalRecord {
    pub id: Uuid,
    pub name: String,
    pub is_active: bool,
    pub last_seen_at: Option<Zoned>,
    pub created_at: Zoned,
    pub updated_at: Zoned,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminUserRecord {
    pub id: Uuid,
    pub display_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdminAuthenticationResult {
    Authenticated(AdminUserRecord),
    InvalidCredentials,
    Locked { locked_until: Zoned },
}

impl SqliteRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn authenticate_terminal_token(
        &self,
        token: &str,
    ) -> Result<Option<AuthenticatedTerminal>, RepoError> {
        let rows = sqlx::query(
            r#"
            SELECT id, name, api_token_hash
            FROM terminal
            WHERE is_active = 1
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(to_repo_error)?;

        for row in rows {
            let id = row.try_get::<String, _>("id").map_err(to_repo_error)?;
            let name = row.try_get::<String, _>("name").map_err(to_repo_error)?;
            let api_token_hash = row
                .try_get::<String, _>("api_token_hash")
                .map_err(to_repo_error)?;
            let parsed_hash = PasswordHash::new(&api_token_hash)
                .map_err(|e| RepoError::Db(format!("invalid terminal token hash: {e}")))?;

            if Argon2::default()
                .verify_password(token.as_bytes(), &parsed_hash)
                .is_ok()
            {
                return Ok(Some(AuthenticatedTerminal {
                    id: Uuid::parse_str(&id).map_err(|e| RepoError::Db(e.to_string()))?,
                    name,
                }));
            }
        }

        Ok(None)
    }

    pub async fn touch_terminal(
        &self,
        terminal_id: Uuid,
        seen_at: &Zoned,
    ) -> Result<(), RepoError> {
        sqlx::query(
            r#"
            UPDATE terminal
            SET last_seen_at = ?, updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(seen_at.to_string())
        .bind(seen_at.to_string())
        .bind(terminal_id.to_string())
        .execute(&self.pool)
        .await
        .map_err(to_repo_error)?;

        Ok(())
    }

    pub async fn authenticate_admin_session(
        &self,
        session_id: &str,
    ) -> Result<Option<AuthenticatedAdmin>, RepoError> {
        let now = Zoned::now();
        let next_expiry = session_expiry_from(&now)?;
        let row = sqlx::query(
            r#"
            SELECT s.admin_user_id
            FROM admin_session s
            JOIN admin_user u ON u.id = s.admin_user_id
            WHERE s.id = ?
              AND s.expires_at > ?
              AND u.is_active = 1
            "#,
        )
        .bind(session_id)
        .bind(now.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(to_repo_error)?;

        let admin = row
            .map(|row| {
                row.try_get::<String, _>("admin_user_id")
                    .map_err(to_repo_error)
                    .and_then(|id| {
                        Uuid::parse_str(&id)
                            .map(|id| AuthenticatedAdmin { id })
                            .map_err(|e| RepoError::Db(e.to_string()))
                    })
            })
            .transpose()?;

        if admin.is_some() {
            sqlx::query(
                r#"
                UPDATE admin_session
                SET expires_at = ?, last_active_at = ?
                WHERE id = ?
                "#,
            )
            .bind(next_expiry.to_string())
            .bind(now.to_string())
            .bind(session_id)
            .execute(&self.pool)
            .await
            .map_err(to_repo_error)?;
        }

        Ok(admin)
    }

    pub async fn list_recent_punches(&self, limit: usize) -> Result<Vec<PunchEvent>, RepoError> {
        let rows = sqlx::query(
            r#"
            SELECT * FROM punch_event
            WHERE deleted_at IS NULL
            ORDER BY occurred_at DESC
            LIMIT ?
            "#,
        )
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(to_repo_error)?;

        rows.into_iter().map(map_punch_row).collect()
    }

    pub async fn list_terminals(&self) -> Result<Vec<TerminalRecord>, RepoError> {
        let rows = sqlx::query(
            r#"
            SELECT id, name, is_active, last_seen_at, created_at, updated_at
            FROM terminal
            ORDER BY created_at DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(to_repo_error)?;

        rows.into_iter().map(map_terminal_row).collect()
    }

    pub async fn create_terminal(
        &self,
        name: &str,
        api_token_hash: &str,
    ) -> Result<TerminalRecord, RepoError> {
        let now = Zoned::now();
        let terminal = TerminalRecord {
            id: Uuid::now_v7(),
            name: name.to_string(),
            is_active: true,
            last_seen_at: None,
            created_at: now.clone(),
            updated_at: now.clone(),
        };

        sqlx::query(
            r#"
            INSERT INTO terminal (id, name, api_token_hash, is_active, created_at, updated_at)
            VALUES (?, ?, ?, 1, ?, ?)
            "#,
        )
        .bind(terminal.id.to_string())
        .bind(&terminal.name)
        .bind(api_token_hash)
        .bind(terminal.created_at.to_string())
        .bind(terminal.updated_at.to_string())
        .execute(&self.pool)
        .await
        .map_err(to_repo_error)?;

        Ok(terminal)
    }

    pub async fn find_terminal(&self, id: Uuid) -> Result<Option<TerminalRecord>, RepoError> {
        let row = sqlx::query(
            r#"
            SELECT id, name, is_active, last_seen_at, created_at, updated_at
            FROM terminal
            WHERE id = ?
            "#,
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(to_repo_error)?;

        row.map(map_terminal_row).transpose()
    }

    pub async fn rotate_terminal_token(
        &self,
        id: Uuid,
        api_token_hash: &str,
    ) -> Result<TerminalRecord, RepoError> {
        let now = Zoned::now();
        let result = sqlx::query(
            r#"
            UPDATE terminal
            SET api_token_hash = ?, updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(api_token_hash)
        .bind(now.to_string())
        .bind(id.to_string())
        .execute(&self.pool)
        .await
        .map_err(to_repo_error)?;

        if result.rows_affected() == 0 {
            return Err(RepoError::NotFound);
        }

        self.find_terminal(id).await?.ok_or(RepoError::NotFound)
    }

    pub async fn deactivate_terminal(&self, id: Uuid) -> Result<TerminalRecord, RepoError> {
        let before = self.find_terminal(id).await?.ok_or(RepoError::NotFound)?;
        let now = Zoned::now();
        let result = sqlx::query(
            r#"
            UPDATE terminal
            SET is_active = 0, updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(now.to_string())
        .bind(id.to_string())
        .execute(&self.pool)
        .await
        .map_err(to_repo_error)?;

        if result.rows_affected() == 0 {
            return Err(RepoError::NotFound);
        }

        Ok(TerminalRecord {
            is_active: false,
            updated_at: now,
            ..before
        })
    }

    pub async fn list_attendance_requests(
        &self,
        status: Option<AttendanceRequestStatus>,
    ) -> Result<Vec<AttendanceRequest>, RepoError> {
        let rows = if let Some(status) = status {
            let status_str = serde_json::to_string(&status)
                .map_err(|e| RepoError::Db(e.to_string()))?
                .replace('"', "");
            sqlx::query(
                r#"
                SELECT *
                FROM attendance_request
                WHERE status = ?
                ORDER BY requested_at DESC
                "#,
            )
            .bind(status_str)
            .fetch_all(&self.pool)
            .await
            .map_err(to_repo_error)?
        } else {
            sqlx::query(
                r#"
                SELECT *
                FROM attendance_request
                ORDER BY requested_at DESC
                "#,
            )
            .fetch_all(&self.pool)
            .await
            .map_err(to_repo_error)?
        };

        rows.into_iter().map(map_attendance_request_row).collect()
    }

    pub async fn review_attendance_request(
        &self,
        id: Uuid,
        reviewed_by_admin_user_id: Uuid,
        next_status: AttendanceRequestStatus,
        review_note: Option<String>,
    ) -> Result<AttendanceRequest, RepoError> {
        let existing = AttendanceRequestRepository::find(self, id)
            .await?
            .ok_or(RepoError::NotFound)?;
        existing
            .status
            .transition_to(next_status)
            .map_err(|e| RepoError::Conflict(e.to_string()))?;

        let reviewed_at = Zoned::now();
        let status_str = serde_json::to_string(&next_status)
            .map_err(|e| RepoError::Db(e.to_string()))?
            .replace('"', "");

        sqlx::query(
            r#"
            UPDATE attendance_request
            SET status = ?, reviewed_by_admin_user_id = ?, reviewed_at = ?, review_note = ?, updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(status_str)
        .bind(reviewed_by_admin_user_id.to_string())
        .bind(reviewed_at.to_string())
        .bind(review_note.clone())
        .bind(reviewed_at.to_string())
        .bind(id.to_string())
        .execute(&self.pool)
        .await
        .map_err(to_repo_error)?;

        Ok(AttendanceRequest {
            status: next_status,
            reviewed_by_admin_user_id: Some(reviewed_by_admin_user_id),
            reviewed_at: Some(reviewed_at),
            review_note,
            ..existing
        })
    }

    pub async fn find_punch_by_id(&self, id: Uuid) -> Result<Option<PunchEvent>, RepoError> {
        let row = sqlx::query("SELECT * FROM punch_event WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await
            .map_err(to_repo_error)?;

        row.map(map_punch_row).transpose()
    }

    pub async fn verify_admin_credentials(
        &self,
        username: &str,
        password: &str,
    ) -> Result<AdminAuthenticationResult, RepoError> {
        let now = Zoned::now();
        let row = sqlx::query(
            r#"
            SELECT id, display_name, password_hash, failed_login_attempts, locked_until
            FROM admin_user
            WHERE username = ? AND is_active = 1
            "#,
        )
        .bind(username)
        .fetch_optional(&self.pool)
        .await
        .map_err(to_repo_error)?;

        let Some(row) = row else {
            return Ok(AdminAuthenticationResult::InvalidCredentials);
        };

        let admin_id = Uuid::parse_str(&row.try_get::<String, _>("id").map_err(to_repo_error)?)
            .map_err(|e| RepoError::Db(e.to_string()))?;
        let display_name = row
            .try_get::<String, _>("display_name")
            .map_err(to_repo_error)?;
        let failed_login_attempts = row
            .try_get::<i64, _>("failed_login_attempts")
            .map_err(to_repo_error)?;
        let locked_until = row
            .try_get::<Option<String>, _>("locked_until")
            .map_err(to_repo_error)?
            .map(|value| parse_zoned(&value))
            .transpose()?;

        if let Some(locked_until) = locked_until {
            if locked_until > now {
                return Ok(AdminAuthenticationResult::Locked { locked_until });
            }

            sqlx::query(
                r#"
                UPDATE admin_user
                SET failed_login_attempts = 0, locked_until = NULL, updated_at = ?
                WHERE id = ?
                "#,
            )
            .bind(now.to_string())
            .bind(admin_id.to_string())
            .execute(&self.pool)
            .await
            .map_err(to_repo_error)?;
        }

        let password_hash = row
            .try_get::<String, _>("password_hash")
            .map_err(to_repo_error)?;
        let parsed_hash =
            PasswordHash::new(&password_hash).map_err(|e| RepoError::Db(e.to_string()))?;

        if Argon2::default()
            .verify_password(password.as_bytes(), &parsed_hash)
            .is_err()
        {
            let next_failed_attempts = failed_login_attempts + 1;
            let locked_until = if next_failed_attempts >= 5 {
                Some(admin_lockout_until(&now)?)
            } else {
                None
            };

            sqlx::query(
                r#"
                UPDATE admin_user
                SET failed_login_attempts = ?, locked_until = ?, updated_at = ?
                WHERE id = ?
                "#,
            )
            .bind(next_failed_attempts)
            .bind(locked_until.as_ref().map(ToString::to_string))
            .bind(now.to_string())
            .bind(admin_id.to_string())
            .execute(&self.pool)
            .await
            .map_err(to_repo_error)?;

            return Ok(match locked_until {
                Some(locked_until) => AdminAuthenticationResult::Locked { locked_until },
                None => AdminAuthenticationResult::InvalidCredentials,
            });
        }

        sqlx::query(
            r#"
            UPDATE admin_user
            SET failed_login_attempts = 0, locked_until = NULL, updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(now.to_string())
        .bind(admin_id.to_string())
        .execute(&self.pool)
        .await
        .map_err(to_repo_error)?;

        Ok(AdminAuthenticationResult::Authenticated(AdminUserRecord {
            id: admin_id,
            display_name,
        }))
    }

    pub async fn create_admin_session(
        &self,
        admin_user_id: Uuid,
    ) -> Result<(Uuid, Zoned), RepoError> {
        let session_id = Uuid::now_v7();
        let now = Zoned::now();
        let expires_at = session_expiry_from(&now)?;

        sqlx::query(
            r#"
            INSERT INTO admin_session (id, admin_user_id, expires_at, created_at, last_active_at)
            VALUES (?, ?, ?, ?, ?)
            "#,
        )
        .bind(session_id.to_string())
        .bind(admin_user_id.to_string())
        .bind(expires_at.to_string())
        .bind(now.to_string())
        .bind(now.to_string())
        .execute(&self.pool)
        .await
        .map_err(to_repo_error)?;

        Ok((session_id, expires_at))
    }

    pub async fn delete_admin_session(&self, session_id: &str) -> Result<Option<Uuid>, RepoError> {
        let row = sqlx::query(
            r#"
            SELECT admin_user_id
            FROM admin_session
            WHERE id = ?
            "#,
        )
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(to_repo_error)?;

        let Some(row) = row else {
            return Ok(None);
        };

        let admin_user_id = Uuid::parse_str(
            &row.try_get::<String, _>("admin_user_id")
                .map_err(to_repo_error)?,
        )
        .map_err(|e| RepoError::Db(e.to_string()))?;

        sqlx::query(
            r#"
            DELETE FROM admin_session
            WHERE id = ?
            "#,
        )
        .bind(session_id)
        .execute(&self.pool)
        .await
        .map_err(to_repo_error)?;

        Ok(Some(admin_user_id))
    }
}

#[async_trait]
impl EmployeeRepository for SqliteRepository {
    async fn list_active(&self) -> Result<Vec<Employee>, RepoError> {
        let rows = sqlx::query("SELECT * FROM employee WHERE is_active = 1")
            .fetch_all(&self.pool)
            .await
            .map_err(to_repo_error)?;

        rows.into_iter().map(map_employee_row).collect()
    }

    async fn find(&self, id: Uuid) -> Result<Option<Employee>, RepoError> {
        let id_str = id.to_string();
        let row = sqlx::query("SELECT * FROM employee WHERE id = ?")
            .bind(id_str)
            .fetch_optional(&self.pool)
            .await
            .map_err(to_repo_error)?;

        row.map(map_employee_row).transpose()
    }

    async fn find_by_card(&self, card_id: &CardId) -> Result<Option<Employee>, RepoError> {
        let row = sqlx::query(
            r#"
            SELECT e.* FROM employee e
            JOIN card c ON e.id = c.employee_id
            WHERE c.card_identifier = ? AND c.is_active = 1 AND e.is_active = 1
            "#,
        )
        .bind(&card_id.0)
        .fetch_optional(&self.pool)
        .await
        .map_err(to_repo_error)?;

        row.map(map_employee_row).transpose()
    }

    async fn create(&self, input: NewEmployee) -> Result<Employee, RepoError> {
        let id = Uuid::now_v7();
        let now = Zoned::now();
        let id_str = id.to_string();
        let now_str = now.to_string();

        sqlx::query(
            r#"
            INSERT INTO employee (id, display_name, employment_type, affiliation, note, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(id_str)
        .bind(input.display_name.clone())
        .bind(input.employment_type.clone())
        .bind(input.affiliation.clone())
        .bind(input.note.clone())
        .bind(now_str.clone())
        .bind(now_str)
        .execute(&self.pool)
        .await
        .map_err(to_repo_error)?;

        Ok(Employee {
            id,
            display_name: input.display_name,
            employment_type: input.employment_type,
            affiliation: input.affiliation,
            is_active: true,
            note: input.note,
            created_at: now.clone(),
            updated_at: now,
        })
    }

    async fn update(&self, id: Uuid, patch: EmployeePatch) -> Result<Employee, RepoError> {
        let mut employee = EmployeeRepository::find(self, id)
            .await?
            .ok_or(RepoError::NotFound)?;

        if let Some(val) = patch.display_name {
            employee.display_name = val;
        }
        if let Some(val) = patch.employment_type {
            employee.employment_type = val;
        }
        if let Some(val) = patch.affiliation {
            employee.affiliation = val;
        }
        if let Some(val) = patch.is_active {
            employee.is_active = val;
        }
        if let Some(val) = patch.note {
            employee.note = val;
        }
        employee.updated_at = Zoned::now();

        let id_str = employee.id.to_string();
        let updated_at_str = employee.updated_at.to_string();

        sqlx::query(
            r#"
            UPDATE employee
            SET display_name = ?, employment_type = ?, affiliation = ?, is_active = ?, note = ?, updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(&employee.display_name)
        .bind(&employee.employment_type)
        .bind(&employee.affiliation)
        .bind(employee.is_active as i32)
        .bind(&employee.note)
        .bind(updated_at_str)
        .bind(id_str)
        .execute(&self.pool)
        .await
        .map_err(to_repo_error)?;

        Ok(employee)
    }

    async fn deactivate(&self, id: Uuid) -> Result<(), RepoError> {
        let id_str = id.to_string();
        sqlx::query("UPDATE employee SET is_active = 0 WHERE id = ?")
            .bind(id_str)
            .execute(&self.pool)
            .await
            .map_err(to_repo_error)?;

        Ok(())
    }
}

#[async_trait]
impl CardRepository for SqliteRepository {
    async fn find(&self, card_id: &CardId) -> Result<Option<Card>, RepoError> {
        let row = sqlx::query("SELECT * FROM card WHERE card_identifier = ?")
            .bind(&card_id.0)
            .fetch_optional(&self.pool)
            .await
            .map_err(to_repo_error)?;

        row.map(map_card_row).transpose()
    }

    async fn bind(&self, card_id: &CardId, employee_id: Uuid) -> Result<Card, RepoError> {
        let id = Uuid::now_v7();
        let now = Zoned::now();
        let id_str = id.to_string();
        let emp_id_str = employee_id.to_string();
        let now_str = now.to_string();

        sqlx::query(
            r#"
            INSERT INTO card (id, employee_id, card_identifier, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?)
            ON CONFLICT(card_identifier) DO UPDATE SET
                employee_id = excluded.employee_id,
                is_active = 1,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(id_str)
        .bind(emp_id_str)
        .bind(&card_id.0)
        .bind(now_str.clone())
        .bind(now_str)
        .execute(&self.pool)
        .await
        .map_err(to_repo_error)?;

        CardRepository::find(self, card_id)
            .await?
            .ok_or(RepoError::NotFound)
    }

    async fn unbind(&self, card_id: &CardId) -> Result<(), RepoError> {
        sqlx::query("UPDATE card SET is_active = 0 WHERE card_identifier = ?")
            .bind(&card_id.0)
            .execute(&self.pool)
            .await
            .map_err(to_repo_error)?;

        Ok(())
    }
}

#[async_trait]
impl PunchRepository for SqliteRepository {
    async fn insert(&self, event: NewPunchEvent) -> Result<PunchEvent, RepoError> {
        let now = Zoned::now();
        let id_str = event.id.to_string();
        let emp_id_str = event.employee_id.to_string();
        let card_id_str = event.card_id.map(|u| u.to_string());
        let event_type_str = serde_json::to_string(&event.event_type)
            .unwrap()
            .replace("\"", "");
        let occurred_at_str = event.occurred_at.to_string();
        let now_str = now.to_string();

        sqlx::query(
            r#"
            INSERT INTO punch_event (id, employee_id, card_id, event_type, occurred_at, server_recorded_at, source, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(id_str)
        .bind(emp_id_str)
        .bind(card_id_str)
        .bind(event_type_str)
        .bind(occurred_at_str)
        .bind(now_str.clone())
        .bind(event.source.clone())
        .bind(now_str.clone())
        .bind(now_str)
        .execute(&self.pool)
        .await
        .map_err(to_repo_error)?;

        Ok(PunchEvent {
            id: event.id,
            employee_id: event.employee_id,
            card_id: event.card_id,
            event_type: event.event_type,
            occurred_at: event.occurred_at,
            server_recorded_at: now.clone(),
            source: event.source,
            correction_reason: None,
            deleted_at: None,
            created_at: now.clone(),
            updated_at: now,
        })
    }

    async fn recent_for_employee(
        &self,
        employee_id: Uuid,
        limit: usize,
    ) -> Result<Vec<PunchEvent>, RepoError> {
        let emp_id_str = employee_id.to_string();
        let limit_i64 = limit as i64;
        let rows = sqlx::query(
            r#"
            SELECT * FROM punch_event
            WHERE employee_id = ? AND deleted_at IS NULL
            ORDER BY occurred_at DESC
            LIMIT ?
            "#,
        )
        .bind(emp_id_str)
        .bind(limit_i64)
        .fetch_all(&self.pool)
        .await
        .map_err(to_repo_error)?;

        rows.into_iter().map(map_punch_row).collect()
    }

    async fn list_in_range(
        &self,
        employee_id: Uuid,
        from: &Zoned,
        to: &Zoned,
    ) -> Result<Vec<PunchEvent>, RepoError> {
        let emp_id_str = employee_id.to_string();
        let from_str = from.to_string();
        let to_str = to.to_string();

        let rows = sqlx::query(
            r#"
            SELECT * FROM punch_event
            WHERE employee_id = ? AND occurred_at BETWEEN ? AND ? AND deleted_at IS NULL
            ORDER BY occurred_at ASC
            "#,
        )
        .bind(emp_id_str)
        .bind(from_str)
        .bind(to_str)
        .fetch_all(&self.pool)
        .await
        .map_err(to_repo_error)?;

        rows.into_iter().map(map_punch_row).collect()
    }

    async fn update(
        &self,
        id: Uuid,
        patch: PunchPatch,
        reason: String,
    ) -> Result<PunchEvent, RepoError> {
        let id_str = id.to_string();
        let row = sqlx::query("SELECT * FROM punch_event WHERE id = ?")
            .bind(&id_str)
            .fetch_optional(&self.pool)
            .await
            .map_err(to_repo_error)?
            .ok_or(RepoError::NotFound)?;

        let mut punch = map_punch_row(row)?;

        if let Some(val) = patch.event_type {
            punch.event_type = val;
        }
        if let Some(val) = patch.occurred_at {
            punch.occurred_at = val;
        }
        punch.correction_reason = Some(reason);
        punch.updated_at = Zoned::now();

        let event_type_str = serde_json::to_string(&punch.event_type)
            .unwrap()
            .replace("\"", "");
        let occurred_at_str = punch.occurred_at.to_string();
        let updated_at_str = punch.updated_at.to_string();

        sqlx::query(
            r#"
            UPDATE punch_event
            SET event_type = ?, occurred_at = ?, correction_reason = ?, updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(event_type_str)
        .bind(occurred_at_str)
        .bind(&punch.correction_reason)
        .bind(updated_at_str)
        .bind(id_str)
        .execute(&self.pool)
        .await
        .map_err(to_repo_error)?;

        Ok(punch)
    }

    async fn soft_delete(&self, id: Uuid, reason: String) -> Result<(), RepoError> {
        let id_str = id.to_string();
        let now_str = Zoned::now().to_string();
        sqlx::query("UPDATE punch_event SET deleted_at = ?, correction_reason = ? WHERE id = ?")
            .bind(now_str)
            .bind(reason)
            .bind(id_str)
            .execute(&self.pool)
            .await
            .map_err(to_repo_error)?;

        Ok(())
    }
}

#[async_trait]
impl ExternalAccountRepository for SqliteRepository {
    async fn find_by_external_id(
        &self,
        provider: &str,
        external_user_id: &str,
    ) -> Result<Option<ExternalAccount>, RepoError> {
        let row = sqlx::query(
            "SELECT * FROM external_account WHERE provider = ? AND external_user_id = ?",
        )
        .bind(provider)
        .bind(external_user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(to_repo_error)?;

        row.map(map_external_account_row).transpose()
    }

    async fn bind(
        &self,
        employee_id: Uuid,
        provider: &str,
        external_user_id: &str,
    ) -> Result<ExternalAccount, RepoError> {
        let id = Uuid::now_v7();
        let now = Zoned::now();
        let id_str = id.to_string();
        let emp_id_str = employee_id.to_string();
        let now_str = now.to_string();

        sqlx::query(
            r#"
            INSERT INTO external_account (id, employee_id, provider, external_user_id, is_verified, created_at, updated_at)
            VALUES (?, ?, ?, ?, 1, ?, ?)
            ON CONFLICT(provider, external_user_id) DO UPDATE SET
                employee_id = excluded.employee_id,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(id_str)
        .bind(emp_id_str)
        .bind(provider)
        .bind(external_user_id)
        .bind(now_str.clone())
        .bind(now_str)
        .execute(&self.pool)
        .await
        .map_err(to_repo_error)?;

        ExternalAccountRepository::find_by_external_id(self, provider, external_user_id)
            .await?
            .ok_or(RepoError::NotFound)
    }
}

#[async_trait]
impl ShiftRepository for SqliteRepository {
    async fn list_for_month(
        &self,
        employee_id: Uuid,
        year_month: YearMonth,
    ) -> Result<Vec<ShiftAssignment>, RepoError> {
        let emp_id_str = employee_id.to_string();
        let from_date = year_month.to_date(1);
        let to_date = year_month.to_date(year_month.days_in_month());
        let from_str = from_date.to_string();
        let to_str = to_date.to_string();

        let rows = sqlx::query(
            r#"
            SELECT * FROM shift_assignment
            WHERE employee_id = ? AND date BETWEEN ? AND ?
            ORDER BY date ASC
            "#,
        )
        .bind(emp_id_str)
        .bind(from_str)
        .bind(to_str)
        .fetch_all(&self.pool)
        .await
        .map_err(to_repo_error)?;

        rows.into_iter().map(map_shift_assignment_row).collect()
    }

    async fn list_types(&self) -> Result<Vec<ShiftType>, RepoError> {
        let rows = sqlx::query("SELECT * FROM shift_type WHERE is_active = 1")
            .fetch_all(&self.pool)
            .await
            .map_err(to_repo_error)?;

        rows.into_iter().map(map_shift_type_row).collect()
    }
}

#[async_trait]
impl AuditLogRepository for SqliteRepository {
    async fn append(&self, entry: NewAuditLog) -> Result<(), RepoError> {
        let id = Uuid::now_v7().to_string();
        let now = Zoned::now().to_string();

        sqlx::query(
            r#"
            INSERT INTO audit_log (id, actor_type, actor_id, action, target_type, target_id, before_json, after_json, metadata_json, created_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(id)
        .bind(entry.actor_type)
        .bind(entry.actor_id)
        .bind(entry.action)
        .bind(entry.target_type)
        .bind(entry.target_id)
        .bind(entry.before_json)
        .bind(entry.after_json)
        .bind(entry.metadata_json)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(to_repo_error)?;

        Ok(())
    }

    async fn list(&self, _filter: AuditLogFilter) -> Result<Vec<AuditLog>, RepoError> {
        let rows = sqlx::query("SELECT * FROM audit_log ORDER BY created_at DESC LIMIT 100")
            .fetch_all(&self.pool)
            .await
            .map_err(to_repo_error)?;

        rows.into_iter().map(map_audit_log_row).collect()
    }
}

#[async_trait]
impl AttendanceRequestRepository for SqliteRepository {
    async fn create(&self, input: NewAttendanceRequest) -> Result<AttendanceRequest, RepoError> {
        let id = Uuid::now_v7();
        let now = Zoned::now();
        let id_str = id.to_string();
        let emp_id_str = input.employee_id.to_string();
        let request_type_str = serde_json::to_string(&input.request_type)
            .unwrap()
            .replace("\"", "");
        let status_str = serde_json::to_string(&input.status)
            .unwrap()
            .replace("\"", "");
        let source_str = serde_json::to_string(&input.requested_via)
            .unwrap()
            .replace("\"", "");
        let requested_at_str = input.requested_at.to_string();
        let now_str = now.to_string();

        sqlx::query(
            r#"
            INSERT INTO attendance_request (id, employee_id, request_type, requested_payload_json, status, requested_via, requested_at, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(id_str)
        .bind(emp_id_str)
        .bind(request_type_str)
        .bind(input.requested_payload_json.clone())
        .bind(status_str)
        .bind(source_str)
        .bind(requested_at_str)
        .bind(now_str.clone())
        .bind(now_str)
        .execute(&self.pool)
        .await
        .map_err(to_repo_error)?;

        Ok(AttendanceRequest {
            id,
            employee_id: input.employee_id,
            request_type: input.request_type,
            requested_payload_json: input.requested_payload_json,
            status: input.status,
            requested_via: input.requested_via,
            requested_at: input.requested_at,
            reviewed_by_admin_user_id: None,
            reviewed_at: None,
            review_note: None,
            applied_event_id: None,
        })
    }

    async fn find(&self, id: Uuid) -> Result<Option<AttendanceRequest>, RepoError> {
        let id_str = id.to_string();
        let row = sqlx::query("SELECT * FROM attendance_request WHERE id = ?")
            .bind(id_str)
            .fetch_optional(&self.pool)
            .await
            .map_err(to_repo_error)?;

        row.map(map_attendance_request_row).transpose()
    }

    async fn update_status(
        &self,
        id: Uuid,
        status: pasori_core::domain::request::AttendanceRequestStatus,
        applied_event_id: Option<Uuid>,
    ) -> Result<AttendanceRequest, RepoError> {
        let existing = AttendanceRequestRepository::find(self, id)
            .await?
            .ok_or(RepoError::NotFound)?;
        let now = Zoned::now();
        let status_str = serde_json::to_string(&status).unwrap().replace("\"", "");

        sqlx::query(
            r#"
            UPDATE attendance_request
            SET status = ?, applied_event_id = ?, updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(status_str)
        .bind(applied_event_id.map(|value| value.to_string()))
        .bind(now.to_string())
        .bind(id.to_string())
        .execute(&self.pool)
        .await
        .map_err(to_repo_error)?;

        Ok(AttendanceRequest {
            status,
            applied_event_id,
            ..existing
        })
    }
}

// Helper mappers
fn map_employee_row(row: sqlx::sqlite::SqliteRow) -> Result<Employee, RepoError> {
    use sqlx::Row;
    Ok(Employee {
        id: Uuid::parse_str(row.get("id")).map_err(|e| RepoError::Db(e.to_string()))?,
        display_name: row.get("display_name"),
        employment_type: row.get("employment_type"),
        affiliation: row.get::<Option<String>, _>("affiliation"),
        is_active: row.get::<i32, _>("is_active") != 0,
        note: row.get::<Option<String>, _>("note"),
        created_at: parse_zoned(row.get("created_at"))?,
        updated_at: parse_zoned(row.get("updated_at"))?,
    })
}

fn map_card_row(row: sqlx::sqlite::SqliteRow) -> Result<Card, RepoError> {
    use sqlx::Row;
    Ok(Card {
        id: Uuid::parse_str(row.get("id")).map_err(|e| RepoError::Db(e.to_string()))?,
        employee_id: Uuid::parse_str(row.get("employee_id"))
            .map_err(|e| RepoError::Db(e.to_string()))?,
        card_identifier: CardId(row.get("card_identifier")),
        card_label: row.get("card_label"),
        is_active: row.get::<i32, _>("is_active") != 0,
        created_at: parse_zoned(row.get("created_at"))?,
        updated_at: parse_zoned(row.get("updated_at"))?,
    })
}

fn map_punch_row(row: sqlx::sqlite::SqliteRow) -> Result<PunchEvent, RepoError> {
    use sqlx::Row;
    let event_type_str: String = row.get("event_type");
    let event_type = serde_json::from_str(&format!("\"{}\"", event_type_str))
        .map_err(|e: serde_json::Error| RepoError::Db(e.to_string()))?;

    Ok(PunchEvent {
        id: Uuid::parse_str(row.get("id")).map_err(|e| RepoError::Db(e.to_string()))?,
        employee_id: Uuid::parse_str(row.get("employee_id"))
            .map_err(|e| RepoError::Db(e.to_string()))?,
        card_id: row
            .get::<Option<String>, _>("card_id")
            .map(|s| Uuid::parse_str(&s).map_err(|e| RepoError::Db(e.to_string())))
            .transpose()?,
        event_type,
        occurred_at: parse_zoned(row.get("occurred_at"))?,
        server_recorded_at: parse_zoned(row.get("server_recorded_at"))?,
        source: row.get("source"),
        correction_reason: row.get("correction_reason"),
        deleted_at: row
            .get::<Option<String>, _>("deleted_at")
            .map(|s| parse_zoned(&s))
            .transpose()?,
        created_at: parse_zoned(row.get("created_at"))?,
        updated_at: parse_zoned(row.get("updated_at"))?,
    })
}

fn map_external_account_row(row: sqlx::sqlite::SqliteRow) -> Result<ExternalAccount, RepoError> {
    use sqlx::Row;
    Ok(ExternalAccount {
        id: Uuid::parse_str(row.get("id")).map_err(|e| RepoError::Db(e.to_string()))?,
        employee_id: Uuid::parse_str(row.get("employee_id"))
            .map_err(|e| RepoError::Db(e.to_string()))?,
        provider: row.get("provider"),
        external_user_id: row.get("external_user_id"),
        external_domain_id: row.get("external_domain_id"),
        is_verified: row.get::<i32, _>("is_verified") != 0,
        created_at: parse_zoned(row.get("created_at"))?,
        updated_at: parse_zoned(row.get("updated_at"))?,
    })
}

fn map_shift_assignment_row(row: sqlx::sqlite::SqliteRow) -> Result<ShiftAssignment, RepoError> {
    use sqlx::Row;
    let status_str: String = row.get("status");
    let status = serde_json::from_str(&format!("\"{}\"", status_str))
        .map_err(|e| RepoError::Db(e.to_string()))?;

    Ok(ShiftAssignment {
        id: Uuid::parse_str(row.get("id")).map_err(|e| RepoError::Db(e.to_string()))?,
        employee_id: Uuid::parse_str(row.get("employee_id"))
            .map_err(|e| RepoError::Db(e.to_string()))?,
        date: row
            .get::<String, _>("date")
            .parse()
            .map_err(|e: jiff::Error| RepoError::Db(e.to_string()))?,
        shift_type_id: Uuid::parse_str(row.get("shift_type_id"))
            .map_err(|e| RepoError::Db(e.to_string()))?,
        planned_start_at: row
            .get::<Option<String>, _>("planned_start_at")
            .map(|s| parse_zoned(&s))
            .transpose()?,
        planned_end_at: row
            .get::<Option<String>, _>("planned_end_at")
            .map(|s| parse_zoned(&s))
            .transpose()?,
        note: row.get("note"),
        status,
        created_at: parse_zoned(row.get("created_at"))?,
        updated_at: parse_zoned(row.get("updated_at"))?,
    })
}

fn map_shift_type_row(row: sqlx::sqlite::SqliteRow) -> Result<ShiftType, RepoError> {
    use sqlx::Row;
    Ok(ShiftType {
        id: Uuid::parse_str(row.get("id")).map_err(|e| RepoError::Db(e.to_string()))?,
        code: row.get("code"),
        display_name: row.get("display_name"),
        planned_start_time: row.get("planned_start_time"),
        planned_end_time: row.get("planned_end_time"),
        default_break_minutes: row.get("default_break_minutes"),
        color: row.get("color"),
        is_active: row.get::<i32, _>("is_active") != 0,
        created_at: parse_zoned(row.get("created_at"))?,
        updated_at: parse_zoned(row.get("updated_at"))?,
    })
}

fn map_audit_log_row(row: sqlx::sqlite::SqliteRow) -> Result<AuditLog, RepoError> {
    use sqlx::Row;
    Ok(AuditLog {
        id: Uuid::parse_str(row.get("id")).map_err(|e| RepoError::Db(e.to_string()))?,
        actor_type: row.get("actor_type"),
        actor_id: row.get("actor_id"),
        action: row.get("action"),
        target_type: row.get("target_type"),
        target_id: row.get("target_id"),
        before_json: row.get("before_json"),
        after_json: row.get("after_json"),
        metadata_json: row.get("metadata_json"),
        created_at: parse_zoned(row.get("created_at"))?,
    })
}

fn map_terminal_row(row: sqlx::sqlite::SqliteRow) -> Result<TerminalRecord, RepoError> {
    use sqlx::Row;
    Ok(TerminalRecord {
        id: Uuid::parse_str(row.get("id")).map_err(|e| RepoError::Db(e.to_string()))?,
        name: row.get("name"),
        is_active: row.get::<i32, _>("is_active") != 0,
        last_seen_at: row
            .get::<Option<String>, _>("last_seen_at")
            .map(|s| parse_zoned(&s))
            .transpose()?,
        created_at: parse_zoned(row.get("created_at"))?,
        updated_at: parse_zoned(row.get("updated_at"))?,
    })
}

fn map_attendance_request_row(
    row: sqlx::sqlite::SqliteRow,
) -> Result<AttendanceRequest, RepoError> {
    use sqlx::Row;
    let request_type_str: String = row.get("request_type");
    let request_type = serde_json::from_str(&format!("\"{}\"", request_type_str))
        .map_err(|e| RepoError::Db(e.to_string()))?;

    let status_str: String = row.get("status");
    let status = serde_json::from_str(&format!("\"{}\"", status_str))
        .map_err(|e| RepoError::Db(e.to_string()))?;

    let requested_via_str: String = row.get("requested_via");
    let requested_via = serde_json::from_str(&format!("\"{}\"", requested_via_str))
        .map_err(|e| RepoError::Db(e.to_string()))?;

    Ok(AttendanceRequest {
        id: Uuid::parse_str(row.get("id")).map_err(|e| RepoError::Db(e.to_string()))?,
        employee_id: Uuid::parse_str(row.get("employee_id"))
            .map_err(|e| RepoError::Db(e.to_string()))?,
        request_type,
        requested_payload_json: row.get("requested_payload_json"),
        status,
        requested_via,
        requested_at: parse_zoned(row.get("requested_at"))?,
        reviewed_by_admin_user_id: row
            .get::<Option<String>, _>("reviewed_by_admin_user_id")
            .map(|s| Uuid::parse_str(&s).map_err(|e| RepoError::Db(e.to_string())))
            .transpose()?,
        reviewed_at: row
            .get::<Option<String>, _>("reviewed_at")
            .map(|s| parse_zoned(&s))
            .transpose()?,
        review_note: row.get("review_note"),
        applied_event_id: row
            .get::<Option<String>, _>("applied_event_id")
            .map(|s| Uuid::parse_str(&s).map_err(|e| RepoError::Db(e.to_string())))
            .transpose()?,
    })
}

fn parse_zoned(s: &str) -> Result<Zoned, RepoError> {
    s.parse::<Zoned>().map_err(|e| RepoError::Db(e.to_string()))
}

fn to_repo_error(e: sqlx::Error) -> RepoError {
    match e {
        sqlx::Error::RowNotFound => RepoError::NotFound,
        sqlx::Error::Database(err) if err.message().contains("UNIQUE constraint failed") => {
            RepoError::Conflict(err.message().to_string())
        }
        _ => RepoError::Db(e.to_string()),
    }
}

fn session_expiry_from(now: &Zoned) -> Result<Zoned, RepoError> {
    now.clone()
        .checked_add(jiff::SignedDuration::from_hours(24))
        .map_err(|e| RepoError::Db(e.to_string()))
}

fn admin_lockout_until(now: &Zoned) -> Result<Zoned, RepoError> {
    now.clone()
        .checked_add(jiff::SignedDuration::from_mins(15))
        .map_err(|e| RepoError::Db(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use argon2::password_hash::{PasswordHasher, SaltString};
    use pasori_core::port::policy::PunchEventType;
    use sha2::{Digest, Sha384};
    use sqlx::sqlite::SqlitePoolOptions;
    use std::str::FromStr;

    async fn setup_db() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .expect("failed to connect to memory db");

        sqlx::migrate!("../../migrations")
            .run(&pool)
            .await
            .expect("failed to run migrations");

        pool
    }

    #[tokio::test]
    // 旧 seed migration が適用済みの DB にも新しい migration を継続適用できる。
    async fn keeps_migrating_when_seed_migration_was_already_applied_with_legacy_checksum() {
        let database_url = format!("sqlite:file:{}?mode=memory&cache=shared", Uuid::now_v7());
        let pool = SqlitePoolOptions::new()
            .connect(&database_url)
            .await
            .expect("failed to connect to memory db");

        sqlx::query(
            r#"
            CREATE TABLE _sqlx_migrations (
                version BIGINT PRIMARY KEY,
                description TEXT NOT NULL,
                installed_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                success BOOLEAN NOT NULL,
                checksum BLOB NOT NULL,
                execution_time BIGINT NOT NULL
            )
            "#,
        )
        .execute(&pool)
        .await
        .expect("failed to create migration table");

        sqlx::query(include_str!(
            "../../../../migrations/20260416000000_initial_schema.sql"
        ))
        .execute(&pool)
        .await
        .expect("failed to apply initial schema");
        sqlx::query(include_str!(
            "../../../../migrations/20260417000001_seed_test_data.sql"
        ))
        .execute(&pool)
        .await
        .expect("failed to apply seed data");
        sqlx::query(include_str!(
            "../../../../migrations/20260420000100_admin_lockout_and_session_activity.sql"
        ))
        .execute(&pool)
        .await
        .expect("failed to apply admin migration");

        let legacy_seed_sql = r#"-- Seed test data for development
-- カード ID '01010112A91B9843' を持つ従業員を登録

INSERT INTO employee (
    id, display_name, employment_type, affiliation, is_active, created_at, updated_at
) VALUES (
    '0195085e-9900-7f21-88f5-66778899aabb', -- UUID v7
    'テスト 太郎',
    'regular',
    '開発部',
    1,
    '2026-04-17T00:00:00+09:00',
    '2026-04-17T00:00:00+09:00'
);

INSERT INTO card (
    id, employee_id, card_identifier, card_label, is_active, created_at, updated_at
) VALUES (
    '0195085e-9901-7acc-99aa-bbccddeeff00',
    '0195085e-9900-7f21-88f5-66778899aabb',
    '01010112A91B9843', -- スキャンされた IDm
    'テスト用カード',
    1,
    '2026-04-17T00:00:00+09:00',
    '2026-04-17T00:00:00+09:00'
);
"#;

        let initial_checksum = Sha384::digest(
            include_str!("../../../../migrations/20260416000000_initial_schema.sql").as_bytes(),
        );
        let legacy_seed_checksum = Sha384::digest(legacy_seed_sql.as_bytes());
        let admin_checksum = Sha384::digest(
            include_str!(
                "../../../../migrations/20260420000100_admin_lockout_and_session_activity.sql"
            )
            .as_bytes(),
        );

        for (version, description, checksum) in [
            (
                20260416000000_i64,
                "initial schema",
                initial_checksum.as_slice(),
            ),
            (
                20260417000001_i64,
                "seed test data",
                legacy_seed_checksum.as_slice(),
            ),
            (
                20260420000100_i64,
                "admin lockout and session activity",
                admin_checksum.as_slice(),
            ),
        ] {
            sqlx::query(
                "INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time) VALUES (?, ?, 1, ?, 0)",
            )
            .bind(version)
            .bind(description)
            .bind(checksum)
            .execute(&pool)
            .await
            .expect("failed to insert applied migration");
        }

        sqlx::migrate!("../../migrations")
            .run(&pool)
            .await
            .expect("failed to continue migrations");

        let (created_at, updated_at): (String, String) =
            sqlx::query_as("SELECT created_at, updated_at FROM employee WHERE id = ?")
                .bind("0195085e-9900-7f21-88f5-66778899aabb")
                .fetch_one(&pool)
                .await
                .expect("failed to fetch seeded employee");

        let created_at = Zoned::from_str(&created_at).expect("created_at should parse as Zoned");
        let updated_at = Zoned::from_str(&updated_at).expect("updated_at should parse as Zoned");

        assert_eq!(created_at.time_zone().iana_name(), Some("Asia/Tokyo"));
        assert_eq!(updated_at.time_zone().iana_name(), Some("Asia/Tokyo"));
    }

    #[tokio::test]
    async fn test_employee_workflow() {
        let pool = setup_db().await;
        let repo = SqliteRepository::new(pool);

        // Create
        let new_emp = NewEmployee {
            display_name: "Test User".to_string(),
            employment_type: "regular".to_string(),
            affiliation: Some("Engineering".to_string()),
            note: None,
        };
        let emp = EmployeeRepository::create(&repo, new_emp)
            .await
            .expect("create failed");
        assert_eq!(emp.display_name, "Test User");

        // Find
        let found = EmployeeRepository::find(&repo, emp.id)
            .await
            .expect("find failed");
        assert_eq!(found.unwrap().display_name, "Test User");

        // List
        let active = repo.list_active().await.expect("list failed");
        assert!(active.iter().any(|employee| employee.id == emp.id));

        // Update
        let patch = EmployeePatch {
            display_name: Some("Updated Name".to_string()),
            ..Default::default()
        };
        let updated = EmployeeRepository::update(&repo, emp.id, patch)
            .await
            .expect("update failed");
        assert_eq!(updated.display_name, "Updated Name");

        // Deactivate
        repo.deactivate(emp.id).await.expect("deactivate failed");
        let active_after = repo.list_active().await.expect("list failed");
        assert!(!active_after.iter().any(|employee| employee.id == emp.id));
    }

    #[tokio::test]
    async fn test_card_workflow() {
        let pool = setup_db().await;
        let repo = SqliteRepository::new(pool.clone());

        let emp = EmployeeRepository::create(
            &repo,
            NewEmployee {
                display_name: "User".to_string(),
                employment_type: "regular".to_string(),
                affiliation: None,
                note: None,
            },
        )
        .await
        .unwrap();

        let card_id = CardId("12345".to_string());

        // Bind
        let card = CardRepository::bind(&repo, &card_id, emp.id)
            .await
            .expect("bind failed");
        assert_eq!(card.card_identifier, card_id);

        // Find by card
        let found_emp = repo
            .find_by_card(&card_id)
            .await
            .expect("find_by_card failed");
        assert_eq!(found_emp.unwrap().id, emp.id);

        // Unbind
        repo.unbind(&card_id).await.expect("unbind failed");
        let found_card = CardRepository::find(&repo, &card_id)
            .await
            .expect("find card failed");
        assert!(!found_card.unwrap().is_active);
    }

    #[tokio::test]
    async fn test_punch_workflow() {
        let pool = setup_db().await;
        let repo = SqliteRepository::new(pool.clone());

        let emp = EmployeeRepository::create(
            &repo,
            NewEmployee {
                display_name: "User".to_string(),
                employment_type: "regular".to_string(),
                affiliation: None,
                note: None,
            },
        )
        .await
        .unwrap();

        let now = Zoned::now();
        let punch_id = Uuid::now_v7();

        // Insert
        let punch = repo
            .insert(NewPunchEvent {
                id: punch_id,
                employee_id: emp.id,
                card_id: None,
                event_type: PunchEventType::ClockIn,
                occurred_at: now.clone(),
                source: "nfc".to_string(),
            })
            .await
            .expect("insert failed");
        assert_eq!(punch.id, punch_id);

        // Recent
        let recent = repo
            .recent_for_employee(emp.id, 10)
            .await
            .expect("recent failed");
        assert_eq!(recent.len(), 1);

        // List in range
        let from = now
            .checked_sub(jiff::SignedDuration::from_hours(1))
            .unwrap();
        let to = now
            .checked_add(jiff::SignedDuration::from_hours(1))
            .unwrap();
        let range = repo
            .list_in_range(emp.id, &from, &to)
            .await
            .expect("list in range failed");
        assert_eq!(range.len(), 1);

        // Update
        let updated = PunchRepository::update(
            &repo,
            punch_id,
            PunchPatch {
                event_type: Some(PunchEventType::ClockOut),
                ..Default::default()
            },
            "correction".to_string(),
        )
        .await
        .expect("update failed");
        assert_eq!(updated.event_type, PunchEventType::ClockOut);

        // Soft delete
        repo.soft_delete(punch_id, "test".to_string())
            .await
            .expect("delete failed");
        let recent_after = repo
            .recent_for_employee(emp.id, 10)
            .await
            .expect("recent failed");
        assert_eq!(recent_after.len(), 0);
    }

    #[tokio::test]
    // 正しい資格情報では管理者を返し、失敗回数をリセットする。
    async fn verifies_admin_credentials() {
        let pool = setup_db().await;
        let repo = SqliteRepository::new(pool.clone());
        let admin_id = Uuid::now_v7();
        let now = Zoned::now();
        let password_hash = hash_password("correct-password");

        sqlx::query(
            "INSERT INTO admin_user (id, username, password_hash, display_name, is_active, created_at, updated_at)
             VALUES (?, ?, ?, ?, 1, ?, ?)",
        )
        .bind(admin_id.to_string())
        .bind("admin")
        .bind(password_hash)
        .bind("管理者")
        .bind(now.to_string())
        .bind(now.to_string())
        .execute(&pool)
        .await
        .expect("insert admin");

        let admin = repo
            .verify_admin_credentials("admin", "correct-password")
            .await
            .expect("verify");
        assert_eq!(
            admin,
            AdminAuthenticationResult::Authenticated(AdminUserRecord {
                id: admin_id,
                display_name: "管理者".to_string(),
            })
        );
    }

    #[tokio::test]
    // 5 回連続失敗した管理者は 15 分ロックされる。
    async fn locks_admin_after_five_failed_logins() {
        let pool = setup_db().await;
        let repo = SqliteRepository::new(pool.clone());
        let admin_id = Uuid::now_v7();
        let now = Zoned::now();
        let password_hash = hash_password("correct-password");

        sqlx::query(
            "INSERT INTO admin_user (id, username, password_hash, display_name, is_active, created_at, updated_at)
             VALUES (?, ?, ?, ?, 1, ?, ?)",
        )
        .bind(admin_id.to_string())
        .bind("admin")
        .bind(password_hash)
        .bind("管理者")
        .bind(now.to_string())
        .bind(now.to_string())
        .execute(&pool)
        .await
        .expect("insert admin");

        for _ in 0..4 {
            let result = repo
                .verify_admin_credentials("admin", "wrong-password")
                .await
                .expect("verify");
            assert_eq!(result, AdminAuthenticationResult::InvalidCredentials);
        }

        let result = repo
            .verify_admin_credentials("admin", "wrong-password")
            .await
            .expect("verify");
        assert!(matches!(result, AdminAuthenticationResult::Locked { .. }));
    }

    #[tokio::test]
    // session 利用時は期限だけでなく最終活動時刻も更新する。
    async fn extends_admin_session_on_authentication() {
        let pool = setup_db().await;
        let repo = SqliteRepository::new(pool.clone());
        let admin_id = Uuid::now_v7();
        let session_id = Uuid::now_v7();
        let now = Zoned::now();
        let expires_at = now
            .checked_add(jiff::SignedDuration::from_hours(1))
            .expect("future expiry");
        let password_hash = hash_password("secret");

        sqlx::query(
            "INSERT INTO admin_user (id, username, password_hash, display_name, is_active, created_at, updated_at)
             VALUES (?, ?, ?, ?, 1, ?, ?)",
        )
        .bind(admin_id.to_string())
        .bind("admin")
        .bind(password_hash)
        .bind("管理者")
        .bind(now.to_string())
        .bind(now.to_string())
        .execute(&pool)
        .await
        .expect("insert admin");

        sqlx::query(
            "INSERT INTO admin_session (id, admin_user_id, expires_at, created_at, last_active_at)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(session_id.to_string())
        .bind(admin_id.to_string())
        .bind(expires_at.to_string())
        .bind(now.to_string())
        .bind(now.to_string())
        .execute(&pool)
        .await
        .expect("insert session");

        let authenticated = repo
            .authenticate_admin_session(&session_id.to_string())
            .await
            .expect("authenticate admin");
        assert_eq!(authenticated, Some(AuthenticatedAdmin { id: admin_id }));

        let row = sqlx::query("SELECT expires_at, last_active_at FROM admin_session WHERE id = ?")
            .bind(session_id.to_string())
            .fetch_one(&pool)
            .await
            .expect("stored session");
        let updated_expiry =
            parse_zoned(row.get::<String, _>("expires_at").as_str()).expect("expiry should parse");
        let updated_last_active = parse_zoned(row.get::<String, _>("last_active_at").as_str())
            .expect("last_active_at should parse");
        assert!(updated_expiry > expires_at);
        assert!(updated_last_active >= now);
    }

    #[tokio::test]
    // logout では admin_session を削除できる。
    async fn deletes_admin_session() {
        let pool = setup_db().await;
        let repo = SqliteRepository::new(pool.clone());
        let admin_id = Uuid::now_v7();
        let session_id = Uuid::now_v7();
        let now = Zoned::now();
        let password_hash = hash_password("secret");

        sqlx::query(
            "INSERT INTO admin_user (id, username, password_hash, display_name, is_active, created_at, updated_at)
             VALUES (?, ?, ?, ?, 1, ?, ?)",
        )
        .bind(admin_id.to_string())
        .bind("admin")
        .bind(password_hash)
        .bind("管理者")
        .bind(now.to_string())
        .bind(now.to_string())
        .execute(&pool)
        .await
        .expect("insert admin");

        sqlx::query(
            "INSERT INTO admin_session (id, admin_user_id, expires_at, created_at, last_active_at)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(session_id.to_string())
        .bind(admin_id.to_string())
        .bind(
            session_expiry_from(&now)
                .expect("session expiry")
                .to_string(),
        )
        .bind(now.to_string())
        .bind(now.to_string())
        .execute(&pool)
        .await
        .expect("insert session");

        let deleted_admin_id = repo
            .delete_admin_session(&session_id.to_string())
            .await
            .expect("delete session");
        assert_eq!(deleted_admin_id, Some(admin_id));

        let remaining = sqlx::query("SELECT COUNT(*) AS count FROM admin_session WHERE id = ?")
            .bind(session_id.to_string())
            .fetch_one(&pool)
            .await
            .expect("remaining session count")
            .get::<i64, _>("count");
        assert_eq!(remaining, 0);
    }

    fn hash_password(password: &str) -> String {
        let salt =
            SaltString::from_b64("dGVzdF9hZG1pbl9zYWx0").expect("static salt should be valid");
        Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .expect("hash password")
            .to_string()
    }
}

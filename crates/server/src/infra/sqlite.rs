use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordVerifier},
};
use async_trait::async_trait;
use jiff::Zoned;
use pasori_core::domain::admin::AdminUser;
use pasori_core::domain::audit::{AuditLog, AuditLogFilter, NewAuditLog};
use pasori_core::domain::card::Card;
use pasori_core::domain::employee::{Employee, EmployeePatch, ExternalAccount, NewEmployee};
use pasori_core::domain::punch::{NewPunchEvent, PunchEvent, PunchPatch};
use pasori_core::domain::request::{
    AttendanceRequest, AttendanceRequestStatus, NewAttendanceRequest,
};
use pasori_core::domain::shift::{ShiftAssignment, ShiftType};
use pasori_core::domain::time::YearMonth;
use pasori_core::port::policy::PunchEventType;
use pasori_core::port::reader::CardId;
use pasori_core::port::repo::{
    AttendanceRequestRepository, AuditLogRepository, CardRepository, EmployeeRepository,
    ExternalAccountRepository, PunchRepository, RepoError, ShiftRepository,
};
use rand::RngCore;
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
pub enum AdminAuthenticationResult {
    Authenticated(AdminUser),
    InvalidCredentials,
    Locked { locked_until: Zoned },
}

#[derive(Debug, Clone)]
pub enum AttendanceApprovalOperation {
    Correction {
        from: Zoned,
        to: Zoned,
        target_event_type: PunchEventType,
        corrected_at: Zoned,
        reason: String,
    },
    MissingPunch {
        event: NewPunchEvent,
    },
}

mod repos;

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

    pub async fn bind_new_card_from_terminal(
        &self,
        card_id: &CardId,
        employee_id: Uuid,
        terminal_id: Uuid,
    ) -> Result<Card, RepoError> {
        let mut tx = self.pool.begin().await.map_err(to_repo_error)?;

        let now = Zoned::now();
        let employee_id_str = employee_id.to_string();
        let now_str = now.to_string();

        let updated = sqlx::query(
            r#"
            UPDATE card
            SET employee_id = ?, is_active = 1, updated_at = ?
            WHERE card_identifier = ? AND is_active = 0
            "#,
        )
        .bind(&employee_id_str)
        .bind(&now_str)
        .bind(&card_id.0)
        .execute(&mut *tx)
        .await
        .map_err(to_repo_error)?;

        if updated.rows_affected() == 0 {
            sqlx::query(
                r#"
                INSERT INTO card (id, employee_id, card_identifier, created_at, updated_at)
                VALUES (?, ?, ?, ?, ?)
                "#,
            )
            .bind(Uuid::now_v7().to_string())
            .bind(&employee_id_str)
            .bind(&card_id.0)
            .bind(&now_str)
            .bind(&now_str)
            .execute(&mut *tx)
            .await
            .map_err(to_repo_error)?;
        }

        let card = sqlx::query("SELECT * FROM card WHERE card_identifier = ?")
            .bind(&card_id.0)
            .fetch_one(&mut *tx)
            .await
            .map_err(to_repo_error)
            .and_then(map_card_row)?;

        sqlx::query(
            r#"
            INSERT INTO audit_log (id, actor_type, actor_id, action, target_type, target_id, before_json, after_json, metadata_json, created_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(Uuid::now_v7().to_string())
        .bind("terminal")
        .bind(terminal_id.to_string())
        .bind("card.bind")
        .bind("card")
        .bind(card.id.to_string())
        .bind(Option::<String>::None)
        .bind(
            serde_json::json!({
                "id": card.id,
                "employee_id": employee_id,
                "is_active": card.is_active,
            })
            .to_string(),
        )
        .bind(
            serde_json::json!({
                "employee_id": employee_id,
                "source": "terminal_unregistered_card_flow",
            })
            .to_string(),
        )
        .bind(now_str)
        .execute(&mut *tx)
        .await
        .map_err(to_repo_error)?;

        tx.commit().await.map_err(to_repo_error)?;

        Ok(card)
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

    pub async fn approve_attendance_request_atomically(
        &self,
        id: Uuid,
        reviewed_by_admin_user_id: Uuid,
        review_note: Option<String>,
        operation: AttendanceApprovalOperation,
    ) -> Result<AttendanceRequest, RepoError> {
        let mut tx = self.pool.begin().await.map_err(to_repo_error)?;
        let existing = fetch_attendance_request_in_tx(&mut tx, id)
            .await?
            .ok_or(RepoError::NotFound)?;
        existing
            .status
            .transition_to(AttendanceRequestStatus::Approved)
            .map_err(|e| RepoError::Conflict(e.to_string()))?;

        let reviewed_at = Zoned::now();
        let approved_status = status_to_db_string(AttendanceRequestStatus::Approved)?;
        sqlx::query(
            r#"
            UPDATE attendance_request
            SET status = ?, reviewed_by_admin_user_id = ?, reviewed_at = ?, review_note = ?, updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(approved_status)
        .bind(reviewed_by_admin_user_id.to_string())
        .bind(reviewed_at.to_string())
        .bind(review_note.clone())
        .bind(reviewed_at.to_string())
        .bind(id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(to_repo_error)?;

        let reviewed = AttendanceRequest {
            status: AttendanceRequestStatus::Approved,
            reviewed_by_admin_user_id: Some(reviewed_by_admin_user_id),
            reviewed_at: Some(reviewed_at.clone()),
            review_note: review_note.clone(),
            ..existing.clone()
        };

        let applied_event_id = match operation {
            AttendanceApprovalOperation::Correction {
                from,
                to,
                target_event_type,
                corrected_at,
                reason,
            } => {
                let before_punch = fetch_target_punch_in_tx(
                    &mut tx,
                    existing.employee_id,
                    &from,
                    &to,
                    target_event_type,
                )
                .await?
                .ok_or_else(|| RepoError::Conflict("target punch not found".to_string()))?;
                let updated_punch = update_punch_in_tx(
                    &mut tx,
                    &before_punch,
                    target_event_type,
                    corrected_at,
                    reason,
                )
                .await?;
                insert_audit_log_in_tx(
                    &mut tx,
                    NewAuditLog {
                        actor_type: "admin".to_string(),
                        actor_id: Some(reviewed_by_admin_user_id.to_string()),
                        action: "punch.update".to_string(),
                        target_type: "punch_event".to_string(),
                        target_id: Some(updated_punch.id.to_string()),
                        before_json: Some(to_json_string(&before_punch)?),
                        after_json: Some(to_json_string(&updated_punch)?),
                        metadata_json: Some(
                            serde_json::json!({
                                "reason": "lineworks request approved",
                                "attendance_request_id": id,
                            })
                            .to_string(),
                        ),
                    },
                )
                .await?;
                updated_punch.id
            }
            AttendanceApprovalOperation::MissingPunch { event } => {
                let punch = insert_punch_in_tx(&mut tx, event).await?;
                insert_audit_log_in_tx(
                    &mut tx,
                    NewAuditLog {
                        actor_type: "admin".to_string(),
                        actor_id: Some(reviewed_by_admin_user_id.to_string()),
                        action: "punch.create_manual".to_string(),
                        target_type: "punch_event".to_string(),
                        target_id: Some(punch.id.to_string()),
                        before_json: None,
                        after_json: Some(to_json_string(&punch)?),
                        metadata_json: Some(
                            serde_json::json!({
                                "request_type": format!("{:?}", existing.request_type),
                                "attendance_request_id": id,
                            })
                            .to_string(),
                        ),
                    },
                )
                .await?;
                punch.id
            }
        };

        reviewed
            .status
            .transition_to(AttendanceRequestStatus::Applied)
            .map_err(|e| RepoError::Conflict(e.to_string()))?;
        let applied_status = status_to_db_string(AttendanceRequestStatus::Applied)?;
        sqlx::query(
            r#"
            UPDATE attendance_request
            SET status = ?, applied_event_id = ?, updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(applied_status)
        .bind(applied_event_id.to_string())
        .bind(reviewed_at.to_string())
        .bind(id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(to_repo_error)?;

        let applied = AttendanceRequest {
            status: AttendanceRequestStatus::Applied,
            applied_event_id: Some(applied_event_id),
            ..reviewed.clone()
        };

        insert_audit_log_in_tx(
            &mut tx,
            NewAuditLog {
                actor_type: "admin".to_string(),
                actor_id: Some(reviewed_by_admin_user_id.to_string()),
                action: "request.approved".to_string(),
                target_type: "attendance_request".to_string(),
                target_id: Some(id.to_string()),
                before_json: Some(to_json_string(&existing)?),
                after_json: Some(to_json_string(&reviewed)?),
                metadata_json: Some(
                    serde_json::json!({
                        "review_note": review_note,
                        "applied_event_id": applied_event_id,
                    })
                    .to_string(),
                ),
            },
        )
        .await?;

        tx.commit().await.map_err(to_repo_error)?;
        Ok(applied)
    }

    pub async fn reject_attendance_request_atomically(
        &self,
        id: Uuid,
        reviewed_by_admin_user_id: Uuid,
        review_note: Option<String>,
    ) -> Result<AttendanceRequest, RepoError> {
        let mut tx = self.pool.begin().await.map_err(to_repo_error)?;
        let existing = fetch_attendance_request_in_tx(&mut tx, id)
            .await?
            .ok_or(RepoError::NotFound)?;
        existing
            .status
            .transition_to(AttendanceRequestStatus::Rejected)
            .map_err(|e| RepoError::Conflict(e.to_string()))?;

        let reviewed_at = Zoned::now();
        let rejected_status = status_to_db_string(AttendanceRequestStatus::Rejected)?;
        sqlx::query(
            r#"
            UPDATE attendance_request
            SET status = ?, reviewed_by_admin_user_id = ?, reviewed_at = ?, review_note = ?, updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(rejected_status)
        .bind(reviewed_by_admin_user_id.to_string())
        .bind(reviewed_at.to_string())
        .bind(review_note.clone())
        .bind(reviewed_at.to_string())
        .bind(id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(to_repo_error)?;

        let rejected = AttendanceRequest {
            status: AttendanceRequestStatus::Rejected,
            reviewed_by_admin_user_id: Some(reviewed_by_admin_user_id),
            reviewed_at: Some(reviewed_at),
            review_note: review_note.clone(),
            ..existing.clone()
        };

        insert_audit_log_in_tx(
            &mut tx,
            NewAuditLog {
                actor_type: "admin".to_string(),
                actor_id: Some(reviewed_by_admin_user_id.to_string()),
                action: "request.rejected".to_string(),
                target_type: "attendance_request".to_string(),
                target_id: Some(id.to_string()),
                before_json: Some(to_json_string(&existing)?),
                after_json: Some(to_json_string(&rejected)?),
                metadata_json: Some(
                    serde_json::json!({
                        "review_note": review_note,
                    })
                    .to_string(),
                ),
            },
        )
        .await?;

        tx.commit().await.map_err(to_repo_error)?;
        Ok(rejected)
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
            SELECT id, username, display_name, password_hash, is_active, failed_login_attempts, locked_until, created_at, updated_at
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
        let admin_username = row
            .try_get::<String, _>("username")
            .map_err(to_repo_error)?;
        let display_name = row
            .try_get::<String, _>("display_name")
            .map_err(to_repo_error)?;
        let is_active = row.try_get::<i32, _>("is_active").map_err(to_repo_error)? != 0;
        let created_at = parse_zoned(
            &row.try_get::<String, _>("created_at")
                .map_err(to_repo_error)?,
        )?;
        let updated_at = parse_zoned(
            &row.try_get::<String, _>("updated_at")
                .map_err(to_repo_error)?,
        )?;
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

        Ok(AdminAuthenticationResult::Authenticated(AdminUser {
            id: admin_id,
            username: admin_username,
            display_name,
            is_active,
            created_at,
            updated_at,
        }))
    }

    pub async fn create_admin_session(
        &self,
        admin_user_id: Uuid,
    ) -> Result<(String, Zoned), RepoError> {
        let session_id = generate_admin_session_token();
        let now = Zoned::now();
        let expires_at = session_expiry_from(&now)?;

        sqlx::query(
            r#"
            INSERT INTO admin_session (id, admin_user_id, issued_at, expires_at, last_active_at)
            VALUES (?, ?, ?, ?, ?)
            "#,
        )
        .bind(&session_id)
        .bind(admin_user_id.to_string())
        .bind(now.to_string())
        .bind(expires_at.to_string())
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

async fn fetch_attendance_request_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    id: Uuid,
) -> Result<Option<AttendanceRequest>, RepoError> {
    let row = sqlx::query("SELECT * FROM attendance_request WHERE id = ?")
        .bind(id.to_string())
        .fetch_optional(&mut **tx)
        .await
        .map_err(to_repo_error)?;

    row.map(map_attendance_request_row).transpose()
}

async fn fetch_target_punch_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    employee_id: Uuid,
    from: &Zoned,
    to: &Zoned,
    target_event_type: PunchEventType,
) -> Result<Option<PunchEvent>, RepoError> {
    let rows = sqlx::query(
        r#"
        SELECT * FROM punch_event
        WHERE employee_id = ? AND occurred_at BETWEEN ? AND ? AND deleted_at IS NULL
        ORDER BY occurred_at ASC
        "#,
    )
    .bind(employee_id.to_string())
    .bind(from.to_string())
    .bind(to.to_string())
    .fetch_all(&mut **tx)
    .await
    .map_err(to_repo_error)?;

    let punches: Result<Vec<_>, _> = rows.into_iter().map(map_punch_row).collect();
    Ok(punches?
        .into_iter()
        .find(|punch| punch.event_type == target_event_type))
}

async fn update_punch_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    before: &PunchEvent,
    event_type: PunchEventType,
    occurred_at: Zoned,
    reason: String,
) -> Result<PunchEvent, RepoError> {
    let mut punch = before.clone();
    punch.event_type = event_type;
    punch.occurred_at = occurred_at;
    punch.correction_reason = Some(reason);
    punch.updated_at = Zoned::now();

    sqlx::query(
        r#"
        UPDATE punch_event
        SET event_type = ?, occurred_at = ?, correction_reason = ?, updated_at = ?
        WHERE id = ?
        "#,
    )
    .bind(event_type_to_db_string(punch.event_type)?)
    .bind(punch.occurred_at.to_string())
    .bind(&punch.correction_reason)
    .bind(punch.updated_at.to_string())
    .bind(punch.id.to_string())
    .execute(&mut **tx)
    .await
    .map_err(to_repo_error)?;

    Ok(punch)
}

async fn insert_punch_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    event: NewPunchEvent,
) -> Result<PunchEvent, RepoError> {
    let now = Zoned::now();
    let card_id = event.card_id.map(|id| id.to_string());
    let now_str = now.to_string();

    sqlx::query(
        r#"
        INSERT INTO punch_event (id, employee_id, card_id, event_type, occurred_at, server_recorded_at, source, created_at, updated_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(event.id.to_string())
    .bind(event.employee_id.to_string())
    .bind(card_id)
    .bind(event_type_to_db_string(event.event_type)?)
    .bind(event.occurred_at.to_string())
    .bind(&now_str)
    .bind(event.source.clone())
    .bind(&now_str)
    .bind(&now_str)
    .execute(&mut **tx)
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

async fn insert_audit_log_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    entry: NewAuditLog,
) -> Result<(), RepoError> {
    sqlx::query(
        r#"
        INSERT INTO audit_log (id, actor_type, actor_id, action, target_type, target_id, before_json, after_json, metadata_json, created_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(Uuid::now_v7().to_string())
    .bind(entry.actor_type)
    .bind(entry.actor_id)
    .bind(entry.action)
    .bind(entry.target_type)
    .bind(entry.target_id)
    .bind(entry.before_json)
    .bind(entry.after_json)
    .bind(entry.metadata_json)
    .bind(Zoned::now().to_string())
    .execute(&mut **tx)
    .await
    .map_err(to_repo_error)?;

    Ok(())
}

fn status_to_db_string(status: AttendanceRequestStatus) -> Result<String, RepoError> {
    serde_json::to_string(&status)
        .map(|value| value.replace('"', ""))
        .map_err(|e| RepoError::Db(e.to_string()))
}

fn event_type_to_db_string(event_type: PunchEventType) -> Result<String, RepoError> {
    serde_json::to_string(&event_type)
        .map(|value| value.replace('"', ""))
        .map_err(|e| RepoError::Db(e.to_string()))
}

fn to_json_string<T: serde::Serialize>(value: &T) -> Result<String, RepoError> {
    serde_json::to_string(value).map_err(|e| RepoError::Db(e.to_string()))
}

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

fn generate_admin_session_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    encode_hex_lower(&bytes)
}

fn encode_hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn admin_lockout_until(now: &Zoned) -> Result<Zoned, RepoError> {
    now.clone()
        .checked_add(jiff::SignedDuration::from_mins(15))
        .map_err(|e| RepoError::Db(e.to_string()))
}

#[cfg(test)]
mod tests;

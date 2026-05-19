use super::*;

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

    async fn find_by_employee(&self, employee_id: Uuid) -> Result<Option<Card>, RepoError> {
        let row = sqlx::query("SELECT * FROM card WHERE employee_id = ? AND is_active = 1")
            .bind(employee_id.to_string())
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

    async fn find_by_employee_id(
        &self,
        provider: &str,
        employee_id: Uuid,
    ) -> Result<Option<ExternalAccount>, RepoError> {
        let emp_id_str = employee_id.to_string();
        let row = sqlx::query(
            "SELECT * FROM external_account WHERE provider = ? AND employee_id = ? AND is_verified = 1",
        )
        .bind(provider)
        .bind(emp_id_str)
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

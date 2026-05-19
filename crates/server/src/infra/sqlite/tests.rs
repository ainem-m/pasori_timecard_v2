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
        "../../../../../migrations/20260416000000_initial_schema.sql"
    ))
    .execute(&pool)
    .await
    .expect("failed to apply initial schema");
    sqlx::query(include_str!(
        "../../../../../migrations/20260417000001_seed_test_data.sql"
    ))
    .execute(&pool)
    .await
    .expect("failed to apply seed data");
    sqlx::query(include_str!(
        "../../../../../migrations/20260420000100_admin_lockout_and_session_activity.sql"
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
        include_str!("../../../../../migrations/20260416000000_initial_schema.sql").as_bytes(),
    );
    let legacy_seed_checksum = Sha384::digest(legacy_seed_sql.as_bytes());
    let admin_checksum = Sha384::digest(
        include_str!(
            "../../../../../migrations/20260420000100_admin_lockout_and_session_activity.sql"
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
    let AdminAuthenticationResult::Authenticated(admin_user) = admin else {
        panic!("expected authenticated result");
    };
    assert_eq!(admin_user.id, admin_id);
    assert_eq!(admin_user.display_name, "管理者");
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
// UUID ではない random token の session でも認証でき、期限だけでなく最終活動時刻も更新する。
async fn extends_admin_session_on_authentication() {
    let pool = setup_db().await;
    let repo = SqliteRepository::new(pool.clone());
    let admin_id = Uuid::now_v7();
    let session_id = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
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
        "INSERT INTO admin_session (id, admin_user_id, issued_at, expires_at, last_active_at)
             VALUES (?, ?, ?, ?, ?)",
    )
    .bind(session_id)
    .bind(admin_id.to_string())
    .bind(now.to_string())
    .bind(expires_at.to_string())
    .bind(now.to_string())
    .execute(&pool)
    .await
    .expect("insert session");

    let authenticated = repo
        .authenticate_admin_session(session_id)
        .await
        .expect("authenticate admin");
    assert_eq!(authenticated, Some(AuthenticatedAdmin { id: admin_id }));

    let row = sqlx::query("SELECT expires_at, last_active_at FROM admin_session WHERE id = ?")
        .bind(session_id)
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
// logout では random token の admin_session を削除できる。
async fn deletes_admin_session() {
    let pool = setup_db().await;
    let repo = SqliteRepository::new(pool.clone());
    let admin_id = Uuid::now_v7();
    let session_id = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";
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
        "INSERT INTO admin_session (id, admin_user_id, issued_at, expires_at, last_active_at)
             VALUES (?, ?, ?, ?, ?)",
    )
    .bind(session_id)
    .bind(admin_id.to_string())
    .bind(now.to_string())
    .bind(
        session_expiry_from(&now)
            .expect("session expiry")
            .to_string(),
    )
    .bind(now.to_string())
    .execute(&pool)
    .await
    .expect("insert session");

    let deleted_admin_id = repo
        .delete_admin_session(session_id)
        .await
        .expect("delete session");
    assert_eq!(deleted_admin_id, Some(admin_id));

    let remaining = sqlx::query("SELECT COUNT(*) AS count FROM admin_session WHERE id = ?")
        .bind(session_id)
        .fetch_one(&pool)
        .await
        .expect("remaining session count")
        .get::<i64, _>("count");
    assert_eq!(remaining, 0);
}

fn hash_password(password: &str) -> String {
    let salt = SaltString::from_b64("dGVzdF9hZG1pbl9zYWx0").expect("static salt should be valid");
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .expect("hash password")
        .to_string()
}

#[tokio::test]
// audit_log の UPDATE はトリガーで禁止される。
async fn prevents_audit_log_update() {
    let pool = setup_db().await;
    let repo = SqliteRepository::new(pool.clone());

    repo.append(NewAuditLog {
        actor_type: "system".to_string(),
        actor_id: None,
        action: "test.action".to_string(),
        target_type: "test".to_string(),
        target_id: None,
        before_json: None,
        after_json: None,
        metadata_json: None,
    })
    .await
    .expect("append audit log");

    let result =
        sqlx::query("UPDATE audit_log SET action = 'tampered' WHERE action = 'test.action'")
            .execute(&pool)
            .await;

    assert!(
        result.is_err(),
        "UPDATE on audit_log should be prohibited by trigger"
    );
}

#[tokio::test]
// audit_log の DELETE はトリガーで禁止される。
async fn prevents_audit_log_delete() {
    let pool = setup_db().await;
    let repo = SqliteRepository::new(pool.clone());

    repo.append(NewAuditLog {
        actor_type: "system".to_string(),
        actor_id: None,
        action: "test.action".to_string(),
        target_type: "test".to_string(),
        target_id: None,
        before_json: None,
        after_json: None,
        metadata_json: None,
    })
    .await
    .expect("append audit log");

    let result = sqlx::query("DELETE FROM audit_log WHERE action = 'test.action'")
        .execute(&pool)
        .await;

    assert!(
        result.is_err(),
        "DELETE on audit_log should be prohibited by trigger"
    );
}

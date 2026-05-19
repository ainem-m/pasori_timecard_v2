use super::router;
use crate::infra::sqlite::SqliteRepository;
use argon2::{
    Argon2,
    password_hash::{PasswordHasher, SaltString},
};
use axum::{body::Body, http::Request};
use sqlx::Row;
use sqlx::sqlite::SqlitePoolOptions;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

pub(super) async fn test_app() -> axum::Router {
    let (app, _pool) = test_app_with_pool().await;
    app
}

pub(super) async fn test_app_with_pool() -> (axum::Router, sqlx::SqlitePool) {
    let database_url = format!(
        "sqlite:file:{}?mode=memory&cache=shared",
        uuid::Uuid::now_v7()
    );
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .expect("sqlite pool");

    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("migrate");

    let now = "2026-04-20T00:00:00+09:00[Asia/Tokyo]";
    let admin_hash = hash_password("correct horse battery staple");
    sqlx::query(
            "INSERT INTO admin_user (id, username, password_hash, display_name, is_active, created_at, updated_at)
             VALUES (?, ?, ?, ?, 1, ?, ?)",
        )
        .bind(Uuid::now_v7().to_string())
        .bind("admin")
        .bind(admin_hash)
        .bind("管理者")
        .bind(now)
        .bind(now)
        .execute(&pool)
        .await
        .expect("insert admin user");

    let app = router(Arc::new(SqliteRepository::new(pool.clone())));
    (app, pool)
}

pub(super) async fn login_and_extract_cookie(app: axum::Router) -> String {
    let login_response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/login")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "username": "admin",
                        "password": "correct horse battery staple",
                    })
                    .to_string(),
                ))
                .expect("request"),
        )
        .await
        .expect("response");

    login_response
        .headers()
        .get(axum::http::header::SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .expect("set-cookie header")
        .split(';')
        .next()
        .expect("cookie pair")
        .to_string()
}

pub(super) async fn insert_employee(pool: &sqlx::SqlitePool, employee_id: Uuid) {
    sqlx::query(
            "INSERT INTO employee (id, display_name, employment_type, affiliation, is_active, note, created_at, updated_at)
             VALUES (?, ?, ?, NULL, 1, NULL, ?, ?)",
        )
        .bind(employee_id.to_string())
        .bind("山田太郎")
        .bind("regular")
        .bind("2026-04-20T00:00:00+09:00[Asia/Tokyo]")
        .bind("2026-04-20T00:00:00+09:00[Asia/Tokyo]")
        .execute(pool)
        .await
        .expect("insert employee");
}

pub(super) async fn insert_card(pool: &sqlx::SqlitePool, card_id: Uuid, employee_id: Uuid) {
    insert_card_with_identifier(pool, card_id, employee_id, "02020212A91B9843").await;
}

pub(super) async fn insert_card_with_identifier(
    pool: &sqlx::SqlitePool,
    card_id: Uuid,
    employee_id: Uuid,
    card_identifier: &str,
) {
    sqlx::query(
            "INSERT INTO card (id, employee_id, card_identifier, card_label, is_active, created_at, updated_at)
             VALUES (?, ?, ?, NULL, 1, ?, ?)",
        )
        .bind(card_id.to_string())
        .bind(employee_id.to_string())
        .bind(card_identifier)
        .bind("2026-04-20T00:00:00+09:00[Asia/Tokyo]")
        .bind("2026-04-20T00:00:00+09:00[Asia/Tokyo]")
        .execute(pool)
        .await
        .expect("insert card");
}

pub(super) async fn insert_terminal(
    pool: &sqlx::SqlitePool,
    terminal_id: Uuid,
    name: &str,
    token: &str,
) {
    let now = "2026-04-20T00:00:00+09:00[Asia/Tokyo]";
    sqlx::query(
        "INSERT INTO terminal (id, name, api_token_hash, is_active, created_at, updated_at)
             VALUES (?, ?, ?, 1, ?, ?)",
    )
    .bind(terminal_id.to_string())
    .bind(name)
    .bind(hash_token(token))
    .bind(now)
    .bind(now)
    .execute(pool)
    .await
    .expect("insert terminal");
}

fn hash_password(password: &str) -> String {
    let salt = SaltString::from_b64("dGVzdF9hZG1pbl9zYWx0").expect("static salt should be valid");
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .expect("hash password")
        .to_string()
}

fn hash_token(token: &str) -> String {
    let salt =
        SaltString::from_b64("dGVzdF90ZXJtaW5hbF9zYWx0").expect("static salt should be valid");
    Argon2::default()
        .hash_password(token.as_bytes(), &salt)
        .expect("hash token")
        .to_string()
}

pub(super) async fn insert_punch(
    pool: &sqlx::SqlitePool,
    employee_id: Uuid,
    card_id: Uuid,
    event_type: &str,
    occurred_at: &str,
) {
    let id = Uuid::now_v7().to_string();
    sqlx::query(
            "INSERT INTO punch_event (id, employee_id, card_id, event_type, occurred_at, server_recorded_at, source, correction_reason, deleted_at, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, 'nfc', NULL, NULL, ?, ?)",
        )
        .bind(id)
        .bind(employee_id.to_string())
        .bind(card_id.to_string())
        .bind(event_type)
        .bind(occurred_at)
        .bind(occurred_at)
        .bind(occurred_at)
        .bind(occurred_at)
        .execute(pool)
        .await
        .expect("insert punch event");

    let row = sqlx::query("SELECT COUNT(*) AS count FROM punch_event WHERE employee_id = ?")
        .bind(employee_id.to_string())
        .fetch_one(pool)
        .await
        .expect("count punch event");
    assert!(row.get::<i64, _>("count") >= 1);
}

pub(super) async fn insert_attendance_request(
    pool: &sqlx::SqlitePool,
    request_id: Uuid,
    employee_id: Uuid,
    request_type: &str,
    requested_payload_json: &str,
    status: &str,
) {
    sqlx::query(
            "INSERT INTO attendance_request (id, employee_id, request_type, requested_payload_json, status, requested_via, requested_at, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, 'lineworks', '2026-04-16T10:00:00+09:00[Asia/Tokyo]', '2026-04-16T10:00:00+09:00[Asia/Tokyo]', '2026-04-16T10:00:00+09:00[Asia/Tokyo]')",
        )
        .bind(request_id.to_string())
        .bind(employee_id.to_string())
        .bind(request_type)
        .bind(requested_payload_json)
        .bind(status)
        .execute(pool)
        .await
        .expect("insert attendance request");
}

use super::test_support::{
    insert_attendance_request, insert_card, insert_card_with_identifier, insert_employee,
    insert_punch, insert_terminal, login_and_extract_cookie, test_app, test_app_with_pool,
};
use crate::infra::sqlite::SqliteRepository;
use axum::{body::Body, http::Request, http::StatusCode};
use serde_json::Value;
use sqlx::Row;
use tower::ServiceExt;
use uuid::Uuid;

#[tokio::test]
// Admin API は session cookie なしでは利用できない。
async fn rejects_admin_request_without_session_cookie() {
    let app = test_app().await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/admin/employees")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
// 正しい資格情報でログインすると 256bit random hex の session cookie を返す。
async fn logs_in_and_sets_admin_session_cookie() {
    let app = test_app().await;
    let body = serde_json::json!({
        "username": "admin",
        "password": "correct horse battery staple",
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/login")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    let cookie = response
        .headers()
        .get(axum::http::header::SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .expect("set-cookie header");
    assert!(cookie.contains("admin_session="));
    assert!(cookie.contains("HttpOnly"));
    assert!(cookie.contains("SameSite=Strict"));
    let token = cookie
        .split(';')
        .next()
        .and_then(|pair| pair.split_once('='))
        .map(|(_, value)| value)
        .expect("session cookie value");
    assert_eq!(token.len(), 64);
    assert!(token.chars().all(|ch| ch.is_ascii_hexdigit()));
    assert!(Uuid::parse_str(token).is_err());
}

#[tokio::test]
// 連続失敗でロックされた管理者は 423 を返す。
async fn rejects_locked_admin_login() {
    let app = test_app().await;
    let body = serde_json::json!({
        "username": "admin",
        "password": "wrong-password",
    });

    for _ in 0..4 {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/admin/login")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.to_string()))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    let locked = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/login")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(locked.status(), StatusCode::LOCKED);
}

#[tokio::test]
// logout は session を破棄して cookie を失効させる。
async fn logs_out_and_clears_admin_session_cookie() {
    let app = test_app().await;
    let login_response = app
        .clone()
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
    let cookie = login_response
        .headers()
        .get(axum::http::header::SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .expect("set-cookie header")
        .split(';')
        .next()
        .expect("cookie pair")
        .to_string();

    let logout_response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/logout")
                .header(axum::http::header::COOKIE, cookie)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(logout_response.status(), StatusCode::NO_CONTENT);
    let cleared_cookie = logout_response
        .headers()
        .get(axum::http::header::SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .expect("cleared set-cookie header");
    assert!(cleared_cookie.contains("Max-Age=0"));
}

#[tokio::test]
// 月次勤怠 API は従業員と年月を受けて締め期間内の日次勤怠を返す。
async fn returns_monthly_timesheet_for_employee_and_year_month() {
    let (app, pool) = test_app_with_pool().await;
    let employee_id = Uuid::now_v7();
    let card_id = Uuid::now_v7();

    sqlx::query(
            "INSERT INTO employee (id, display_name, employment_type, affiliation, is_active, note, created_at, updated_at)
             VALUES (?, ?, ?, NULL, 1, NULL, ?, ?)",
        )
        .bind(employee_id.to_string())
        .bind("山田太郎")
        .bind("regular")
        .bind("2026-04-20T00:00:00+09:00[Asia/Tokyo]")
        .bind("2026-04-20T00:00:00+09:00[Asia/Tokyo]")
        .execute(&pool)
        .await
        .expect("insert employee");

    sqlx::query(
            "INSERT INTO card (id, employee_id, card_identifier, card_label, is_active, created_at, updated_at)
             VALUES (?, ?, ?, NULL, 1, ?, ?)",
        )
        .bind(card_id.to_string())
        .bind(employee_id.to_string())
        .bind("02020212A91B9843")
        .bind("2026-04-20T00:00:00+09:00[Asia/Tokyo]")
        .bind("2026-04-20T00:00:00+09:00[Asia/Tokyo]")
        .execute(&pool)
        .await
        .expect("insert card");

    insert_punch(
        &pool,
        employee_id,
        card_id,
        "clock_in",
        "2026-03-16T09:00:00+09:00[Asia/Tokyo]",
    )
    .await;
    insert_punch(
        &pool,
        employee_id,
        card_id,
        "clock_out",
        "2026-03-16T18:00:00+09:00[Asia/Tokyo]",
    )
    .await;
    insert_punch(
        &pool,
        employee_id,
        card_id,
        "clock_in",
        "2026-04-15T09:30:00+09:00[Asia/Tokyo]",
    )
    .await;
    insert_punch(
        &pool,
        employee_id,
        card_id,
        "clock_out",
        "2026-04-15T18:00:00+09:00[Asia/Tokyo]",
    )
    .await;
    insert_punch(
        &pool,
        employee_id,
        card_id,
        "clock_in",
        "2026-04-16T09:00:00+09:00[Asia/Tokyo]",
    )
    .await;

    let login_response = app
        .clone()
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
    let cookie = login_response
        .headers()
        .get(axum::http::header::SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .expect("set-cookie header")
        .split(';')
        .next()
        .expect("cookie pair")
        .to_string();

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/admin/attendance/monthly?employee_id={employee_id}&year=2026&month=4"
                ))
                .header(axum::http::header::COOKIE, cookie)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    let json: Value = serde_json::from_slice(&body).expect("json body");

    assert_eq!(json["employee_id"], employee_id.to_string());
    assert_eq!(json["year_month"]["year"], 2026);
    assert_eq!(json["year_month"]["month"], 4);
    assert_eq!(json["period_start"], "2026-03-16");
    assert_eq!(json["period_end"], "2026-04-15");
    assert_eq!(json["days"].as_array().expect("days array").len(), 2);
    assert_eq!(json["total_work_minutes"], 1050);
}

#[tokio::test]
// 管理者は requested の修正申請を承認すると打刻を更新し、申請を applied に進める。
async fn approves_requested_correction_and_applies_punch_update() {
    let (app, pool) = test_app_with_pool().await;
    let employee_id = Uuid::now_v7();
    let card_id = Uuid::now_v7();
    let request_id = Uuid::now_v7();

    insert_employee(&pool, employee_id).await;
    insert_card(&pool, card_id, employee_id).await;
    insert_punch(
        &pool,
        employee_id,
        card_id,
        "clock_in",
        "2026-04-15T09:00:00+09:00[Asia/Tokyo]",
    )
    .await;
    insert_attendance_request(
        &pool,
        request_id,
        employee_id,
        "correction",
        r#"{"date":"2026-04-15","target":"clock_in","time":"08:32"}"#,
        "requested",
    )
    .await;

    let cookie = login_and_extract_cookie(app.clone()).await;
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/admin/attendance_requests/{request_id}/approve"))
                .header(axum::http::header::COOKIE, cookie)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "review_note": "承認して反映",
                    })
                    .to_string(),
                ))
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);

    let request_row = sqlx::query(
            "SELECT status, reviewed_by_admin_user_id, reviewed_at, review_note, applied_event_id FROM attendance_request WHERE id = ?",
        )
        .bind(request_id.to_string())
        .fetch_one(&pool)
        .await
        .expect("attendance request row");
    assert_eq!(request_row.get::<String, _>("status"), "applied");
    assert!(
        request_row
            .get::<Option<String>, _>("reviewed_by_admin_user_id")
            .is_some()
    );
    assert!(
        request_row
            .get::<Option<String>, _>("reviewed_at")
            .is_some()
    );
    assert_eq!(
        request_row.get::<Option<String>, _>("review_note"),
        Some("承認して反映".to_string())
    );
    assert!(
        request_row
            .get::<Option<String>, _>("applied_event_id")
            .is_some()
    );

    let punch_row = sqlx::query(
            "SELECT occurred_at, correction_reason FROM punch_event WHERE employee_id = ? AND event_type = 'clock_in'",
        )
        .bind(employee_id.to_string())
        .fetch_one(&pool)
        .await
        .expect("punch row");
    assert_eq!(
        punch_row.get::<String, _>("occurred_at"),
        "2026-04-15T08:32:00+09:00[Asia/Tokyo]"
    );
    assert_eq!(
        punch_row.get::<Option<String>, _>("correction_reason"),
        Some("admin approved correction".to_string())
    );

    let audit_rows =
        sqlx::query("SELECT action FROM audit_log WHERE target_id = ? ORDER BY created_at ASC")
            .bind(request_id.to_string())
            .fetch_all(&pool)
            .await
            .expect("audit rows");
    let actions: Vec<String> = audit_rows
        .into_iter()
        .map(|row| row.get::<String, _>("action"))
        .collect();
    assert!(actions.contains(&"request.approved".to_string()));
}

#[tokio::test]
// 管理者は requested の修正申請を却下でき、audit_log に却下を残す。
async fn rejects_requested_correction_and_records_audit() {
    let (app, pool) = test_app_with_pool().await;
    let employee_id = Uuid::now_v7();
    let request_id = Uuid::now_v7();

    insert_employee(&pool, employee_id).await;
    insert_attendance_request(
        &pool,
        request_id,
        employee_id,
        "correction",
        r#"{"date":"2026-04-15","target":"clock_out","time":"18:05"}"#,
        "requested",
    )
    .await;

    let cookie = login_and_extract_cookie(app.clone()).await;
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/admin/attendance_requests/{request_id}/reject"))
                .header(axum::http::header::COOKIE, cookie)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "review_note": "証跡不足のため差し戻し",
                    })
                    .to_string(),
                ))
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);

    let request_row = sqlx::query(
        "SELECT status, review_note, applied_event_id FROM attendance_request WHERE id = ?",
    )
    .bind(request_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("attendance request row");
    assert_eq!(request_row.get::<String, _>("status"), "rejected");
    assert_eq!(
        request_row.get::<Option<String>, _>("review_note"),
        Some("証跡不足のため差し戻し".to_string())
    );
    assert!(
        request_row
            .get::<Option<String>, _>("applied_event_id")
            .is_none()
    );

    let audit_row = sqlx::query(
            "SELECT action, metadata_json FROM audit_log WHERE target_id = ? AND action = 'request.rejected'",
        )
        .bind(request_id.to_string())
        .fetch_one(&pool)
        .await
        .expect("request.rejected audit row");
    let metadata: Value =
        serde_json::from_str(&audit_row.get::<String, _>("metadata_json")).expect("metadata json");
    assert_eq!(metadata["review_note"], "証跡不足のため差し戻し");
}

#[tokio::test]
// 管理者は status 指定で修正申請一覧を絞り込める。
async fn filters_attendance_requests_by_status() {
    let (app, pool) = test_app_with_pool().await;
    let employee_id = Uuid::now_v7();

    insert_employee(&pool, employee_id).await;
    insert_attendance_request(
        &pool,
        Uuid::now_v7(),
        employee_id,
        "correction",
        r#"{"date":"2026-04-15","target":"clock_in","time":"08:32"}"#,
        "requested",
    )
    .await;
    insert_attendance_request(
        &pool,
        Uuid::now_v7(),
        employee_id,
        "correction",
        r#"{"date":"2026-04-14","target":"clock_out","time":"18:05"}"#,
        "rejected",
    )
    .await;

    let cookie = login_and_extract_cookie(app.clone()).await;
    let response = app
        .oneshot(
            Request::builder()
                .uri("/admin/attendance_requests?status=requested")
                .header(axum::http::header::COOKIE, cookie)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    let json: Value = serde_json::from_slice(&body).expect("json body");
    assert_eq!(json.as_array().expect("array").len(), 1);
    assert_eq!(json[0]["status"], "requested");
}

#[tokio::test]
// 管理者は登録済み terminal 一覧を取得できる。
async fn lists_registered_terminals() {
    let (app, pool) = test_app_with_pool().await;
    let terminal_id = Uuid::now_v7();

    insert_terminal(&pool, terminal_id, "受付端末", "terminal-secret").await;

    let cookie = login_and_extract_cookie(app.clone()).await;
    let response = app
        .oneshot(
            Request::builder()
                .uri("/admin/terminals")
                .header(axum::http::header::COOKIE, cookie)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    let json: Value = serde_json::from_slice(&body).expect("json body");
    assert_eq!(json.as_array().expect("terminals array").len(), 1);
    assert_eq!(json[0]["id"], terminal_id.to_string());
    assert_eq!(json[0]["name"], "受付端末");
    assert_eq!(json[0]["is_active"], true);
}

#[tokio::test]
// 管理者は terminal を登録すると平文 token を一度だけ受け取り、監査ログが残る。
async fn registers_terminal_and_returns_plaintext_token() {
    let (app, pool) = test_app_with_pool().await;
    let cookie = login_and_extract_cookie(app.clone()).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/terminals")
                .header(axum::http::header::COOKIE, cookie)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "name": "受付端末",
                    })
                    .to_string(),
                ))
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::CREATED);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    let json: Value = serde_json::from_slice(&body).expect("json body");
    assert_eq!(json["terminal"]["name"], "受付端末");
    assert_eq!(json["terminal"]["is_active"], true);
    assert!(
        json["api_token"]
            .as_str()
            .is_some_and(|token| !token.is_empty())
    );

    let terminal_id = json["terminal"]["id"].as_str().expect("terminal id");
    let terminal_row =
        sqlx::query("SELECT name, api_token_hash, is_active FROM terminal WHERE id = ?")
            .bind(terminal_id)
            .fetch_one(&pool)
            .await
            .expect("terminal row");
    assert_eq!(terminal_row.get::<String, _>("name"), "受付端末");
    assert_ne!(
        terminal_row.get::<String, _>("api_token_hash"),
        json["api_token"].as_str().expect("api token")
    );
    assert_eq!(terminal_row.get::<i64, _>("is_active"), 1);

    let audit_row = sqlx::query(
        "SELECT action, target_type, target_id FROM audit_log WHERE action = 'terminal.registered'",
    )
    .fetch_one(&pool)
    .await
    .expect("terminal.registered audit row");
    assert_eq!(audit_row.get::<String, _>("target_type"), "terminal");
    assert_eq!(audit_row.get::<String, _>("target_id"), terminal_id);
}

#[tokio::test]
// 管理者が terminal token を再発行すると旧 token は使えなくなり、新 token だけが有効になる。
async fn rotates_terminal_token_and_invalidates_previous_token() {
    let (app, pool) = test_app_with_pool().await;
    let terminal_id = Uuid::now_v7();
    insert_terminal(&pool, terminal_id, "受付端末", "terminal-secret").await;
    let cookie = login_and_extract_cookie(app.clone()).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/admin/terminals/{terminal_id}/rotate_token"))
                .header(axum::http::header::COOKIE, cookie)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    let json: Value = serde_json::from_slice(&body).expect("json body");
    let new_token = json["api_token"].as_str().expect("new api token");
    assert!(!new_token.is_empty());

    let repo = SqliteRepository::new(pool.clone());
    let old_authenticated = repo
        .authenticate_terminal_token("terminal-secret")
        .await
        .expect("authenticate old token");
    assert!(old_authenticated.is_none());

    let new_authenticated = repo
        .authenticate_terminal_token(new_token)
        .await
        .expect("authenticate new token");
    assert_eq!(
        new_authenticated.expect("new token should authenticate").id,
        terminal_id
    );

    let audit_row = sqlx::query(
        "SELECT action, target_id FROM audit_log WHERE action = 'terminal.token_rotated'",
    )
    .fetch_one(&pool)
    .await
    .expect("terminal.token_rotated audit row");
    assert_eq!(
        audit_row.get::<String, _>("target_id"),
        terminal_id.to_string()
    );
}

#[tokio::test]
// 管理者が terminal を無効化すると terminal token 認証に使えなくなる。
async fn deactivates_terminal_and_revokes_token_authentication() {
    let (app, pool) = test_app_with_pool().await;
    let terminal_id = Uuid::now_v7();
    insert_terminal(&pool, terminal_id, "受付端末", "terminal-secret").await;
    let cookie = login_and_extract_cookie(app.clone()).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/admin/terminals/{terminal_id}"))
                .header(axum::http::header::COOKIE, cookie)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let row = sqlx::query("SELECT is_active FROM terminal WHERE id = ?")
        .bind(terminal_id.to_string())
        .fetch_one(&pool)
        .await
        .expect("terminal row");
    assert_eq!(row.get::<i64, _>("is_active"), 0);

    let repo = SqliteRepository::new(pool.clone());
    let authenticated = repo
        .authenticate_terminal_token("terminal-secret")
        .await
        .expect("authenticate token");
    assert!(authenticated.is_none());
}

#[tokio::test]
// 管理者はカードを従業員に紐付けると 201 と作成された card を受け取り、監査ログに card.bind が残る。
async fn binds_card_to_employee_and_records_audit() {
    let (app, pool) = test_app_with_pool().await;
    let employee_id = Uuid::now_v7();
    insert_employee(&pool, employee_id).await;
    let cookie = login_and_extract_cookie(app.clone()).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/cards/bind")
                .header(axum::http::header::COOKIE, cookie)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "card_identifier": "03030312A91B9843",
                        "employee_id": employee_id,
                    })
                    .to_string(),
                ))
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::CREATED);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    let json: Value = serde_json::from_slice(&body).expect("json body");
    assert_eq!(json["employee_id"], employee_id.to_string());
    assert_eq!(json["card_identifier"], "03030312A91B9843");
    assert_eq!(json["is_active"], true);

    let card_id = json["id"].as_str().expect("card id");
    let audit_row = sqlx::query(
        "SELECT action, target_type, target_id FROM audit_log WHERE action = 'card.bind'",
    )
    .fetch_one(&pool)
    .await
    .expect("card.bind audit row");
    assert_eq!(audit_row.get::<String, _>("target_type"), "card");
    assert_eq!(audit_row.get::<String, _>("target_id"), card_id);
}

#[tokio::test]
// 管理者はカード紐付けを解除すると 204 を受け取り、card を inactive にして監査ログに card.unbind を残す。
async fn unbinds_card_and_records_audit() {
    let (app, pool) = test_app_with_pool().await;
    let employee_id = Uuid::now_v7();
    let card_id = Uuid::now_v7();
    insert_employee(&pool, employee_id).await;
    insert_card_with_identifier(&pool, card_id, employee_id, "04040412A91B9843").await;
    let cookie = login_and_extract_cookie(app.clone()).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/cards/unbind")
                .header(axum::http::header::COOKIE, cookie)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "card_identifier": "04040412A91B9843",
                    })
                    .to_string(),
                ))
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let card_row = sqlx::query("SELECT is_active FROM card WHERE id = ?")
        .bind(card_id.to_string())
        .fetch_one(&pool)
        .await
        .expect("card row");
    assert_eq!(card_row.get::<i64, _>("is_active"), 0);

    let audit_row = sqlx::query(
        "SELECT action, target_type, target_id FROM audit_log WHERE action = 'card.unbind'",
    )
    .fetch_one(&pool)
    .await
    .expect("card.unbind audit row");
    assert_eq!(audit_row.get::<String, _>("target_type"), "card");
    assert_eq!(audit_row.get::<String, _>("target_id"), card_id.to_string());
}

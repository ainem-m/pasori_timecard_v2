use jiff::Zoned;
use jiff::civil::date;
use proptest::prelude::*;

use super::{CorrectionTarget, LineworksCommand, parse_lineworks_command};
use crate::domain::request::AttendanceRequestStatus;

#[test]
// 勤怠照会コマンドは当日勤怠の照会として解釈する。
fn parses_today_attendance() {
    assert_eq!(
        parse_lineworks_command("今日の勤怠"),
        LineworksCommand::TodayAttendance
    );
}

#[test]
// 月次シフト照会コマンドは月次シフトの照会として解釈する。
fn parses_month_shift() {
    assert_eq!(
        parse_lineworks_command("今月のシフト"),
        LineworksCommand::MonthShift
    );
}

#[test]
// 出勤忘れコマンドは時刻を含めて解釈する。
fn parses_missing_in_request() {
    assert_eq!(
        parse_lineworks_command("出勤忘れ 08:30"),
        LineworksCommand::MissingIn {
            time: super::ClockTime::new(8, 30).expect("valid time"),
        }
    );
}

#[test]
// 修正コマンドは日付・対象・時刻をすべて解釈する。
fn parses_correction_request() {
    assert_eq!(
        parse_lineworks_command("修正 2026-04-16 出勤 08:32"),
        LineworksCommand::Correction {
            date: date(2026, 4, 16),
            target: CorrectionTarget::ClockIn,
            time: super::ClockTime::new(8, 32).expect("valid time"),
        }
    );
}

#[test]
// 不明コマンドはヘルプ誘導に落とす。
fn falls_back_to_help_for_unknown_command() {
    assert_eq!(
        parse_lineworks_command("こんにちは"),
        LineworksCommand::Help
    );
}

#[test]
// 照会コマンドは自動承認扱いになる。
fn auto_approves_query_commands() {
    let requested_at = tokyo_datetime(2026, 4, 16, 10, 0);

    let status = super::decide_lineworks_request_status(
        &LineworksCommand::TodayAttendance,
        &requested_at,
        false,
        120,
        None,
    );

    assert_eq!(status, Some(AttendanceRequestStatus::AutoApproved));
}

#[test]
// ヘルプ誘導は申請状態を返さない。
fn returns_none_for_help() {
    let requested_at = tokyo_datetime(2026, 4, 16, 10, 0);

    let status = super::decide_lineworks_request_status(
        &LineworksCommand::Help,
        &requested_at,
        false,
        120,
        None,
    );

    assert_eq!(status, None);
}

proptest! {
    #[test]
    // 任意の入力でもパーサは panic せず、未知入力はヘルプ扱いに落ちる。
    fn never_panics_for_arbitrary_input(input in ".*") {
        let parsed = parse_lineworks_command(&input);
        let is_supported = matches!(
            parsed,
            LineworksCommand::TodayAttendance
                | LineworksCommand::MonthAttendance
                | LineworksCommand::TodayShift
                | LineworksCommand::MonthShift
                | LineworksCommand::MissingIn { .. }
                | LineworksCommand::MissingOut { .. }
                | LineworksCommand::Correction { .. }
                | LineworksCommand::Help
        );

        prop_assert!(is_supported);
    }
}

#[tokio::test]
// 紐付けられていないユーザーからのコマンドには、登録を促す返信をする。
async fn responds_with_unbound_message_when_user_not_found() {
    let external_repo = Arc::new(MockExternalRepo { account: None });
    let request_repo = Arc::new(MockRequestRepo::default());
    let punch_repo = Arc::new(MockPunchRepo {
        punches: vec![sample_punch(
            crate::port::policy::PunchEventType::ClockIn,
            2026,
            4,
            16,
            9,
            0,
        )],
        ..Default::default()
    });
    let shift_repo = Arc::new(MockShiftRepo);
    let notifier = Arc::new(MockNotifier::default());

    let use_case = super::LineworksUseCase::new(
        external_repo,
        request_repo,
        punch_repo,
        shift_repo,
        Arc::new(MockCardRepo { card: None }),
        notifier.clone(),
    );

    let requested_at = tokyo_datetime(2026, 4, 16, 10, 0);
    use_case
        .process_event(
            "unknown-user",
            super::LineworksCommand::TodayAttendance,
            &requested_at,
        )
        .await
        .expect("should process");

    let events = notifier.events.lock().await;
    assert_eq!(events.len(), 1);
    if let crate::port::notify::NotifyEvent::LineworksResponse { user_id, text } = &events[0] {
        assert_eq!(user_id, "unknown-user");
        assert!(text.contains("紐付いていません"));
    } else {
        panic!("unexpected notify event");
    }
}

#[tokio::test]
// 今日の勤怠コマンドには、当日の打刻一覧を返信する。
async fn responds_with_today_punches() {
    let employee_id = uuid::Uuid::now_v7();
    let external_repo = Arc::new(MockExternalRepo {
        account: Some(crate::domain::employee::ExternalAccount {
            id: uuid::Uuid::now_v7(),
            employee_id,
            provider: "lineworks".to_string(),
            external_user_id: "user-1".to_string(),
            external_domain_id: None,
            is_verified: true,
            created_at: tokyo_datetime(2026, 4, 16, 0, 0),
            updated_at: tokyo_datetime(2026, 4, 16, 0, 0),
        }),
    });
    let request_repo = Arc::new(MockRequestRepo::default());
    let punch_repo = Arc::new(MockPunchRepo {
        punches: vec![sample_punch(
            crate::port::policy::PunchEventType::ClockIn,
            2026,
            4,
            16,
            9,
            0,
        )],
        ..Default::default()
    });
    let shift_repo = Arc::new(MockShiftRepo);
    let notifier = Arc::new(MockNotifier::default());

    let use_case = super::LineworksUseCase::new(
        external_repo,
        request_repo,
        punch_repo,
        shift_repo,
        Arc::new(MockCardRepo { card: None }),
        notifier.clone(),
    );

    let requested_at = tokyo_datetime(2026, 4, 16, 10, 0);
    use_case
        .process_event(
            "user-1",
            super::LineworksCommand::TodayAttendance,
            &requested_at,
        )
        .await
        .expect("should process");

    let events = notifier.events.lock().await;
    assert_eq!(events.len(), 1);
    if let crate::port::notify::NotifyEvent::LineworksResponse { user_id, text } = &events[0] {
        assert_eq!(user_id, "user-1");
        assert!(text.contains("【本日の打刻】"));
        assert!(text.contains("出勤: 09:00"));
    } else {
        panic!("unexpected notify event");
    }
}

#[tokio::test]
// 今月の勤怠コマンドには、締め期間の合計勤務時間を返信する。
async fn responds_with_monthly_attendance_summary() {
    let employee_id = uuid::Uuid::now_v7();
    let external_repo = Arc::new(MockExternalRepo {
        account: Some(crate::domain::employee::ExternalAccount {
            id: uuid::Uuid::now_v7(),
            employee_id,
            provider: "lineworks".to_string(),
            external_user_id: "user-1".to_string(),
            external_domain_id: None,
            is_verified: true,
            created_at: tokyo_datetime(2026, 4, 16, 0, 0),
            updated_at: tokyo_datetime(2026, 4, 16, 0, 0),
        }),
    });
    let request_repo = Arc::new(MockRequestRepo::default());
    let punch_repo = Arc::new(MockPunchRepo {
        punches: vec![
            sample_punch(
                crate::port::policy::PunchEventType::ClockIn,
                2026,
                3,
                16,
                9,
                0,
            ),
            sample_punch(
                crate::port::policy::PunchEventType::ClockOut,
                2026,
                3,
                16,
                18,
                0,
            ),
            sample_punch(
                crate::port::policy::PunchEventType::ClockIn,
                2026,
                4,
                15,
                9,
                30,
            ),
            sample_punch(
                crate::port::policy::PunchEventType::ClockOut,
                2026,
                4,
                15,
                18,
                0,
            ),
            sample_punch(
                crate::port::policy::PunchEventType::ClockIn,
                2026,
                4,
                16,
                9,
                0,
            ),
        ],
        ..Default::default()
    });
    let shift_repo = Arc::new(MockShiftRepo);
    let notifier = Arc::new(MockNotifier::default());

    let use_case = super::LineworksUseCase::new(
        external_repo,
        request_repo,
        punch_repo,
        shift_repo,
        Arc::new(MockCardRepo { card: None }),
        notifier.clone(),
    );

    let requested_at = tokyo_datetime(2026, 4, 16, 10, 0);
    use_case
        .process_event(
            "user-1",
            super::LineworksCommand::MonthAttendance,
            &requested_at,
        )
        .await
        .expect("should process");

    let events = notifier.events.lock().await;
    assert_eq!(events.len(), 1);
    if let crate::port::notify::NotifyEvent::LineworksResponse { user_id, text } = &events[0] {
        assert_eq!(user_id, "user-1");
        assert!(text.contains("【今月の勤怠】"));
        assert!(text.contains("期間: 2026-03-16 - 2026-04-15"));
        assert!(text.contains("勤務時間合計: 17時間30分"));
    } else {
        panic!("unexpected notify event");
    }
}

#[tokio::test]
// 当日軽微修正は自動承認として申請を保存し、その旨を返信する。
async fn auto_approves_same_day_correction_request() {
    let employee_id = uuid::Uuid::now_v7();
    let external_repo = Arc::new(MockExternalRepo {
        account: Some(crate::domain::employee::ExternalAccount {
            id: uuid::Uuid::now_v7(),
            employee_id,
            provider: "lineworks".to_string(),
            external_user_id: "user-1".to_string(),
            external_domain_id: None,
            is_verified: true,
            created_at: tokyo_datetime(2026, 4, 16, 0, 0),
            updated_at: tokyo_datetime(2026, 4, 16, 0, 0),
        }),
    });
    let request_repo = Arc::new(MockRequestRepo::default());
    let punch_repo = Arc::new(MockPunchRepo {
        punches: vec![sample_punch(
            crate::port::policy::PunchEventType::ClockIn,
            2026,
            4,
            16,
            9,
            0,
        )],
        ..Default::default()
    });
    let shift_repo = Arc::new(MockShiftRepo);
    let notifier = Arc::new(MockNotifier::default());

    let use_case = super::LineworksUseCase::new(
        external_repo,
        request_repo.clone(),
        punch_repo.clone(),
        shift_repo,
        Arc::new(MockCardRepo { card: None }),
        notifier.clone(),
    );

    let requested_at = tokyo_datetime(2026, 4, 16, 10, 0);
    use_case
        .process_event(
            "user-1",
            super::LineworksCommand::Correction {
                date: date(2026, 4, 16),
                target: super::CorrectionTarget::ClockIn,
                time: super::ClockTime::new(8, 32).expect("valid time"),
            },
            &requested_at,
        )
        .await
        .expect("should process");

    let requests = request_repo.requests.lock().await;
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].request_type,
        crate::domain::request::AttendanceRequestType::Correction
    );
    assert_eq!(
        requests[0].created_status,
        crate::domain::request::AttendanceRequestStatus::AutoApproved
    );
    assert!(
        requests[0]
            .requested_payload_json
            .contains("\"date\":\"2026-04-16\"")
    );
    assert!(
        requests[0]
            .requested_payload_json
            .contains("\"target\":\"clock_in\"")
    );

    let transitions = request_repo.transitions.lock().await;
    assert_eq!(transitions.len(), 1);
    assert_eq!(
        transitions[0].status,
        crate::domain::request::AttendanceRequestStatus::Applied
    );
    assert!(transitions[0].applied_event_id.is_some());

    let updates = punch_repo.updates.lock().await;
    assert_eq!(updates.len(), 1);
    assert_eq!(
        updates[0]
            .patch
            .occurred_at
            .as_ref()
            .map(|z| z.strftime("%H:%M").to_string()),
        Some("08:32".to_string())
    );

    let events = notifier.events.lock().await;
    assert_eq!(events.len(), 1);
    if let crate::port::notify::NotifyEvent::LineworksResponse { text, .. } = &events[0] {
        assert!(text.contains("修正申請を自動承認し、反映しました。"));
    } else {
        panic!("unexpected notify event");
    }
}

#[tokio::test]
// 過去日修正は requested 状態で保存し、管理者承認待ちにする。
async fn queues_past_day_correction_for_admin_review() {
    let employee_id = uuid::Uuid::now_v7();
    let external_repo = Arc::new(MockExternalRepo {
        account: Some(crate::domain::employee::ExternalAccount {
            id: uuid::Uuid::now_v7(),
            employee_id,
            provider: "lineworks".to_string(),
            external_user_id: "user-1".to_string(),
            external_domain_id: None,
            is_verified: true,
            created_at: tokyo_datetime(2026, 4, 16, 0, 0),
            updated_at: tokyo_datetime(2026, 4, 16, 0, 0),
        }),
    });
    let request_repo = Arc::new(MockRequestRepo::default());
    let punch_repo = Arc::new(MockPunchRepo {
        punches: vec![sample_punch(
            crate::port::policy::PunchEventType::ClockOut,
            2026,
            4,
            15,
            18,
            0,
        )],
        ..Default::default()
    });
    let shift_repo = Arc::new(MockShiftRepo);
    let notifier = Arc::new(MockNotifier::default());

    let use_case = super::LineworksUseCase::new(
        external_repo,
        request_repo.clone(),
        punch_repo.clone(),
        shift_repo,
        Arc::new(MockCardRepo { card: None }),
        notifier.clone(),
    );

    let requested_at = tokyo_datetime(2026, 4, 16, 10, 0);
    use_case
        .process_event(
            "user-1",
            super::LineworksCommand::Correction {
                date: date(2026, 4, 15),
                target: super::CorrectionTarget::ClockOut,
                time: super::ClockTime::new(18, 5).expect("valid time"),
            },
            &requested_at,
        )
        .await
        .expect("should process");

    let requests = request_repo.requests.lock().await;
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].created_status,
        crate::domain::request::AttendanceRequestStatus::Requested
    );
    assert!(
        requests[0]
            .requested_payload_json
            .contains("\"target\":\"clock_out\"")
    );

    let transitions = request_repo.transitions.lock().await;
    assert_eq!(transitions.len(), 0);

    let events = notifier.events.lock().await;
    assert_eq!(events.len(), 1);
    if let crate::port::notify::NotifyEvent::LineworksResponse { text, .. } = &events[0] {
        assert!(text.contains("修正申請を受け付けました。管理者の承認をお待ちください。"));
    } else {
        panic!("unexpected notify event");
    }
}

#[tokio::test]
// 当日 出勤忘れ 08:30 は punch_event を作成して applied になる。
async fn creates_punch_event_for_auto_approved_missing_in() {
    let employee_id = uuid::Uuid::now_v7();
    let card_id = uuid::Uuid::now_v7();
    let external_repo = Arc::new(MockExternalRepo {
        account: Some(crate::domain::employee::ExternalAccount {
            id: uuid::Uuid::now_v7(),
            employee_id,
            provider: "lineworks".to_string(),
            external_user_id: "user-1".to_string(),
            external_domain_id: None,
            is_verified: true,
            created_at: tokyo_datetime(2026, 4, 16, 0, 0),
            updated_at: tokyo_datetime(2026, 4, 16, 0, 0),
        }),
    });
    let request_repo = Arc::new(MockRequestRepo::default());
    let punch_repo = Arc::new(MockPunchRepo::default());
    let shift_repo = Arc::new(MockShiftRepo);
    let card_repo = Arc::new(MockCardRepo {
        card: Some(crate::domain::card::Card {
            id: card_id,
            employee_id,
            card_identifier: crate::port::reader::CardId("01ABCDEF".to_string()),
            card_label: None,
            is_active: true,
            created_at: tokyo_datetime(2026, 4, 1, 0, 0),
            updated_at: tokyo_datetime(2026, 4, 1, 0, 0),
        }),
    });
    let notifier = Arc::new(MockNotifier::default());

    let use_case = super::LineworksUseCase::new(
        external_repo,
        request_repo.clone(),
        punch_repo.clone(),
        shift_repo,
        card_repo,
        notifier.clone(),
    );

    let requested_at = tokyo_datetime(2026, 4, 16, 8, 30);
    use_case
        .process_event(
            "user-1",
            super::LineworksCommand::MissingIn {
                time: super::ClockTime::new(8, 30).expect("valid time"),
            },
            &requested_at,
        )
        .await
        .expect("should process");

    let requests = request_repo.requests.lock().await;
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].request_type,
        crate::domain::request::AttendanceRequestType::MissingIn
    );
    assert_eq!(
        requests[0].created_status,
        crate::domain::request::AttendanceRequestStatus::AutoApproved
    );

    let transitions = request_repo.transitions.lock().await;
    assert_eq!(transitions.len(), 1);
    assert_eq!(
        transitions[0].status,
        crate::domain::request::AttendanceRequestStatus::Applied
    );
    assert!(transitions[0].applied_event_id.is_some());

    let inserts = punch_repo.inserts.lock().await;
    assert_eq!(inserts.len(), 1);
    assert_eq!(
        inserts[0].event_type,
        crate::port::policy::PunchEventType::ClockIn
    );
    assert_eq!(inserts[0].source, "lineworks");
    assert_eq!(inserts[0].employee_id, employee_id);

    let events = notifier.events.lock().await;
    assert_eq!(events.len(), 1);
    if let crate::port::notify::NotifyEvent::LineworksResponse { text, .. } = &events[0] {
        assert!(text.contains("自動承認し、反映しました"));
    } else {
        panic!("unexpected notify event");
    }
}

#[tokio::test]
// 当日 退勤忘れ 18:05 は punch_event を作成して applied になる。
async fn creates_punch_event_for_auto_approved_missing_out() {
    let employee_id = uuid::Uuid::now_v7();
    let card_id = uuid::Uuid::now_v7();
    let external_repo = Arc::new(MockExternalRepo {
        account: Some(crate::domain::employee::ExternalAccount {
            id: uuid::Uuid::now_v7(),
            employee_id,
            provider: "lineworks".to_string(),
            external_user_id: "user-1".to_string(),
            external_domain_id: None,
            is_verified: true,
            created_at: tokyo_datetime(2026, 4, 16, 0, 0),
            updated_at: tokyo_datetime(2026, 4, 16, 0, 0),
        }),
    });
    let request_repo = Arc::new(MockRequestRepo::default());
    let punch_repo = Arc::new(MockPunchRepo::default());
    let shift_repo = Arc::new(MockShiftRepo);
    let card_repo = Arc::new(MockCardRepo {
        card: Some(crate::domain::card::Card {
            id: card_id,
            employee_id,
            card_identifier: crate::port::reader::CardId("01ABCDEF".to_string()),
            card_label: None,
            is_active: true,
            created_at: tokyo_datetime(2026, 4, 1, 0, 0),
            updated_at: tokyo_datetime(2026, 4, 1, 0, 0),
        }),
    });
    let notifier = Arc::new(MockNotifier::default());

    let use_case = super::LineworksUseCase::new(
        external_repo,
        request_repo.clone(),
        punch_repo.clone(),
        shift_repo,
        card_repo,
        notifier.clone(),
    );

    let requested_at = tokyo_datetime(2026, 4, 16, 18, 5);
    use_case
        .process_event(
            "user-1",
            super::LineworksCommand::MissingOut {
                time: super::ClockTime::new(18, 5).expect("valid time"),
            },
            &requested_at,
        )
        .await
        .expect("should process");

    let inserts = punch_repo.inserts.lock().await;
    assert_eq!(inserts.len(), 1);
    assert_eq!(
        inserts[0].event_type,
        crate::port::policy::PunchEventType::ClockOut
    );
    assert_eq!(inserts[0].source, "lineworks");
}

use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Default)]
struct MockShiftRepo;

#[async_trait::async_trait]
impl crate::port::repo::ShiftRepository for MockShiftRepo {
    async fn list_for_month(
        &self,
        _: uuid::Uuid,
        _: crate::domain::time::YearMonth,
    ) -> Result<Vec<crate::domain::shift::ShiftAssignment>, crate::port::repo::RepoError> {
        Ok(vec![])
    }
    async fn list_types(
        &self,
    ) -> Result<Vec<crate::domain::shift::ShiftType>, crate::port::repo::RepoError> {
        Ok(vec![])
    }
}

struct MockCardRepo {
    card: Option<crate::domain::card::Card>,
}

#[async_trait::async_trait]
impl crate::port::repo::CardRepository for MockCardRepo {
    async fn find(
        &self,
        _: &crate::port::reader::CardId,
    ) -> Result<Option<crate::domain::card::Card>, crate::port::repo::RepoError> {
        Ok(self.card.clone())
    }
    async fn find_by_employee(
        &self,
        _: uuid::Uuid,
    ) -> Result<Option<crate::domain::card::Card>, crate::port::repo::RepoError> {
        Ok(self.card.clone())
    }
    async fn bind(
        &self,
        _: &crate::port::reader::CardId,
        _: uuid::Uuid,
    ) -> Result<crate::domain::card::Card, crate::port::repo::RepoError> {
        unimplemented!()
    }
    async fn unbind(
        &self,
        _: &crate::port::reader::CardId,
    ) -> Result<(), crate::port::repo::RepoError> {
        unimplemented!()
    }
}
#[derive(Default)]
struct MockNotifier {
    events: Mutex<Vec<crate::port::notify::NotifyEvent>>,
}
#[async_trait::async_trait]
impl crate::port::notify::Notifier for MockNotifier {
    async fn notify(
        &self,
        event: crate::port::notify::NotifyEvent,
    ) -> Result<(), crate::port::notify::NotifyError> {
        self.events.lock().await.push(event);
        Ok(())
    }
}

struct MockExternalRepo {
    account: Option<crate::domain::employee::ExternalAccount>,
}
#[async_trait::async_trait]
impl crate::port::repo::ExternalAccountRepository for MockExternalRepo {
    async fn find_by_external_id(
        &self,
        _: &str,
        _: &str,
    ) -> Result<Option<crate::domain::employee::ExternalAccount>, crate::port::repo::RepoError>
    {
        Ok(self.account.clone())
    }
    async fn find_by_employee_id(
        &self,
        _: &str,
        _: uuid::Uuid,
    ) -> Result<Option<crate::domain::employee::ExternalAccount>, crate::port::repo::RepoError>
    {
        Ok(self.account.clone())
    }
    async fn bind(
        &self,
        _: uuid::Uuid,
        _: &str,
        _: &str,
    ) -> Result<crate::domain::employee::ExternalAccount, crate::port::repo::RepoError> {
        unimplemented!()
    }
}

#[derive(Default)]
struct MockRequestRepo {
    requests: Mutex<Vec<RecordedRequest>>,
    transitions: Mutex<Vec<RecordedTransition>>,
}

struct RecordedRequest {
    request_type: crate::domain::request::AttendanceRequestType,
    requested_payload_json: String,
    created_status: crate::domain::request::AttendanceRequestStatus,
}

struct RecordedTransition {
    status: crate::domain::request::AttendanceRequestStatus,
    applied_event_id: Option<uuid::Uuid>,
}

#[async_trait::async_trait]
impl crate::port::repo::AttendanceRequestRepository for MockRequestRepo {
    async fn create(
        &self,
        input: crate::domain::request::NewAttendanceRequest,
    ) -> Result<crate::domain::request::AttendanceRequest, crate::port::repo::RepoError> {
        self.requests.lock().await.push(RecordedRequest {
            request_type: input.request_type,
            requested_payload_json: input.requested_payload_json.clone(),
            created_status: input.status,
        });

        Ok(crate::domain::request::AttendanceRequest {
            id: uuid::Uuid::now_v7(),
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
    async fn find(
        &self,
        _: uuid::Uuid,
    ) -> Result<Option<crate::domain::request::AttendanceRequest>, crate::port::repo::RepoError>
    {
        unimplemented!()
    }

    async fn update_status(
        &self,
        id: uuid::Uuid,
        status: crate::domain::request::AttendanceRequestStatus,
        applied_event_id: Option<uuid::Uuid>,
    ) -> Result<crate::domain::request::AttendanceRequest, crate::port::repo::RepoError> {
        self.transitions.lock().await.push(RecordedTransition {
            status,
            applied_event_id,
        });

        Ok(crate::domain::request::AttendanceRequest {
            id,
            employee_id: uuid::Uuid::now_v7(),
            request_type: crate::domain::request::AttendanceRequestType::Correction,
            requested_payload_json: "{}".to_string(),
            status,
            requested_via: crate::domain::request::AttendanceRequestSource::LineWorks,
            requested_at: jiff::Timestamp::now()
                .to_zoned(jiff::tz::TimeZone::get("Asia/Tokyo").unwrap()),
            reviewed_by_admin_user_id: None,
            reviewed_at: None,
            review_note: None,
            applied_event_id,
        })
    }
}

#[derive(Default)]
struct MockPunchRepo {
    punches: Vec<crate::domain::punch::PunchEvent>,
    inserts: Mutex<Vec<crate::domain::punch::NewPunchEvent>>,
    updates: Mutex<Vec<RecordedPunchUpdate>>,
}

struct RecordedPunchUpdate {
    patch: crate::domain::punch::PunchPatch,
}
#[async_trait::async_trait]
impl crate::port::repo::PunchRepository for MockPunchRepo {
    async fn insert(
        &self,
        event: crate::domain::punch::NewPunchEvent,
    ) -> Result<crate::domain::punch::PunchEvent, crate::port::repo::RepoError> {
        let now = jiff::Zoned::now();
        let punch = crate::domain::punch::PunchEvent {
            id: event.id,
            employee_id: event.employee_id,
            card_id: event.card_id,
            event_type: event.event_type,
            occurred_at: event.occurred_at.clone(),
            server_recorded_at: now.clone(),
            source: event.source.clone(),
            correction_reason: None,
            deleted_at: None,
            created_at: now.clone(),
            updated_at: now,
        };
        self.inserts.lock().await.push(event);
        Ok(punch)
    }
    async fn recent_for_employee(
        &self,
        _: uuid::Uuid,
        _: usize,
    ) -> Result<Vec<crate::domain::punch::PunchEvent>, crate::port::repo::RepoError> {
        unimplemented!()
    }
    async fn list_in_range(
        &self,
        _: uuid::Uuid,
        from: &Zoned,
        to: &Zoned,
    ) -> Result<Vec<crate::domain::punch::PunchEvent>, crate::port::repo::RepoError> {
        Ok(self
            .punches
            .iter()
            .filter(|punch| punch.occurred_at >= *from && punch.occurred_at <= *to)
            .cloned()
            .collect())
    }
    async fn update(
        &self,
        id: uuid::Uuid,
        patch: crate::domain::punch::PunchPatch,
        _: String,
    ) -> Result<crate::domain::punch::PunchEvent, crate::port::repo::RepoError> {
        self.updates.lock().await.push(RecordedPunchUpdate {
            patch: patch.clone(),
        });

        let original = self
            .punches
            .iter()
            .find(|punch| punch.id == id)
            .cloned()
            .ok_or(crate::port::repo::RepoError::NotFound)?;

        Ok(crate::domain::punch::PunchEvent {
            event_type: patch.event_type.unwrap_or(original.event_type),
            occurred_at: patch.occurred_at.unwrap_or(original.occurred_at.clone()),
            correction_reason: Some("lineworks correction".to_string()),
            updated_at: tokyo_datetime(2026, 4, 16, 10, 0),
            ..original
        })
    }
    async fn soft_delete(
        &self,
        _: uuid::Uuid,
        _: String,
    ) -> Result<(), crate::port::repo::RepoError> {
        unimplemented!()
    }
}

fn tokyo_datetime(year: i16, month: i8, day: i8, hour: i8, minute: i8) -> Zoned {
    date(year, month, day)
        .at(hour, minute, 0, 0)
        .in_tz("Asia/Tokyo")
        .expect("Asia/Tokyo datetime should be valid")
}

fn sample_punch(
    event_type: crate::port::policy::PunchEventType,
    year: i16,
    month: i8,
    day: i8,
    hour: i8,
    minute: i8,
) -> crate::domain::punch::PunchEvent {
    crate::domain::punch::PunchEvent {
        id: uuid::Uuid::now_v7(),
        employee_id: uuid::Uuid::now_v7(),
        card_id: None,
        event_type,
        occurred_at: tokyo_datetime(year, month, day, hour, minute),
        server_recorded_at: tokyo_datetime(year, month, day, hour, minute),
        source: "nfc".to_string(),
        correction_reason: None,
        deleted_at: None,
        created_at: tokyo_datetime(year, month, day, hour, minute),
        updated_at: tokyo_datetime(year, month, day, hour, minute),
    }
}

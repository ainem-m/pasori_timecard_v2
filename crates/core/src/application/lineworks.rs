use jiff::Zoned;
use jiff::civil::Date;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineworksCommand {
    TodayAttendance,
    MonthAttendance,
    TodayShift,
    MonthShift,
    MissingIn {
        time: ClockTime,
    },
    MissingOut {
        time: ClockTime,
    },
    Correction {
        date: Date,
        target: CorrectionTarget,
        time: ClockTime,
    },
    Help,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CorrectionTarget {
    ClockIn,
    ClockOut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockTime {
    hour: u8,
    minute: u8,
}

impl ClockTime {
    pub fn new(hour: u8, minute: u8) -> Option<Self> {
        if hour < 24 && minute < 60 {
            Some(Self { hour, minute })
        } else {
            None
        }
    }

    pub fn hour(self) -> u8 {
        self.hour
    }

    pub fn minute(self) -> u8 {
        self.minute
    }
}

pub fn parse_lineworks_command(input: &str) -> LineworksCommand {
    let tokens: Vec<&str> = input.split_whitespace().collect();

    match tokens.as_slice() {
        ["今日の勤怠"] => LineworksCommand::TodayAttendance,
        ["今月の勤怠"] => LineworksCommand::MonthAttendance,
        ["今日のシフト"] => LineworksCommand::TodayShift,
        ["今月のシフト"] => LineworksCommand::MonthShift,
        ["ヘルプ"] => LineworksCommand::Help,
        ["出勤忘れ", time] => parse_clock_time(time)
            .map(|time| LineworksCommand::MissingIn { time })
            .unwrap_or(LineworksCommand::Help),
        ["退勤忘れ", time] => parse_clock_time(time)
            .map(|time| LineworksCommand::MissingOut { time })
            .unwrap_or(LineworksCommand::Help),
        ["修正", date, target, time] => match (
            parse_date(date),
            parse_target(target),
            parse_clock_time(time),
        ) {
            (Some(date), Some(target), Some(time)) => {
                LineworksCommand::Correction { date, target, time }
            }
            _ => LineworksCommand::Help,
        },
        _ => LineworksCommand::Help,
    }
}

use crate::domain::request::{
    AttendanceRequestSource, AttendanceRequestStatus, AttendanceRequestType, NewAttendanceRequest,
};
use crate::domain::time::YearMonth;
use crate::port::notify::{Notifier, NotifyEvent};
use crate::port::repo::{
    AttendanceRequestRepository, ExternalAccountRepository, PunchRepository, ShiftRepository,
};
use std::sync::Arc;

pub struct LineworksUseCase {
    external_repo: Arc<dyn ExternalAccountRepository>,
    request_repo: Arc<dyn AttendanceRequestRepository>,
    punch_repo: Arc<dyn PunchRepository>,
    shift_repo: Arc<dyn ShiftRepository>,
    notifier: Arc<dyn Notifier>,
}

impl LineworksUseCase {
    pub fn new(
        external_repo: Arc<dyn ExternalAccountRepository>,
        request_repo: Arc<dyn AttendanceRequestRepository>,
        punch_repo: Arc<dyn PunchRepository>,
        shift_repo: Arc<dyn ShiftRepository>,
        notifier: Arc<dyn Notifier>,
    ) -> Self {
        Self {
            external_repo,
            request_repo,
            punch_repo,
            shift_repo,
            notifier,
        }
    }

    pub async fn process_event(
        &self,
        external_user_id: &str,
        command: LineworksCommand,
        requested_at: &Zoned,
    ) -> anyhow::Result<()> {
        let external_account = self
            .external_repo
            .find_by_external_id("lineworks", external_user_id)
            .await?;

        let Some(account) = external_account else {
            // 未紐付けユーザーへの返信
            self.notifier
                .notify(NotifyEvent::LineworksResponse {
                    user_id: external_user_id.to_string(),
                    text: "従業員アカウントと紐付いていません。管理者に連絡してください。"
                        .to_string(),
                })
                .await?;
            return Ok(());
        };

        match command {
            LineworksCommand::Help => {
                self.notifier
                    .notify(NotifyEvent::LineworksResponse {
                        user_id: external_user_id.to_string(),
                        text: "【ヘルプ】\n今日の勤怠: 今日の打刻を表示\n出勤忘れ HH:mm: 出勤打刻を申請\n退勤忘れ HH:mm: 退勤打刻を申請"
                            .to_string(),
                    })
                    .await?;
            }
            LineworksCommand::TodayAttendance => {
                let punches: Vec<crate::domain::punch::PunchEvent> = self
                    .punch_repo
                    .list_in_range(account.employee_id, requested_at, requested_at)
                    .await?;

                let message = if punches.is_empty() {
                    "本日の打刻はありません。".to_string()
                } else {
                    let mut lines = vec!["【本日の打刻】".to_string()];
                    for p in punches {
                        lines.push(format!(
                            "{}: {}",
                            p.event_type_label(),
                            p.occurred_at.strftime("%H:%M")
                        ));
                    }
                    lines.join("\n")
                };

                self.notifier
                    .notify(NotifyEvent::LineworksResponse {
                        user_id: external_user_id.to_string(),
                        text: message,
                    })
                    .await?;
            }
            LineworksCommand::MissingIn { time } | LineworksCommand::MissingOut { time } => {
                let request_type = if matches!(command, LineworksCommand::MissingIn { .. }) {
                    AttendanceRequestType::MissingIn
                } else {
                    AttendanceRequestType::MissingOut
                };

                let status = decide_lineworks_request_status(
                    &command,
                    requested_at,
                    false, // TODO: check period lock
                    120,   // TODO: settings
                    None,
                )
                .unwrap_or(AttendanceRequestStatus::Requested);

                let payload = serde_json::json!({
                    "command": format!("{:?}", command),
                    "time": format!("{:02}:{:02}", time.hour(), time.minute()),
                });

                self.request_repo
                    .create(NewAttendanceRequest {
                        employee_id: account.employee_id,
                        request_type,
                        requested_payload_json: payload.to_string(),
                        requested_via: AttendanceRequestSource::LineWorks,
                        requested_at: requested_at.clone(),
                    })
                    .await?;

                let response_text = if status == AttendanceRequestStatus::AutoApproved {
                    "申請を自動承認し、反映しました。".to_string()
                } else {
                    "申請を受け付けました。管理者の承認をお待ちください。".to_string()
                };

                self.notifier
                    .notify(NotifyEvent::LineworksResponse {
                        user_id: external_user_id.to_string(),
                        text: response_text,
                    })
                    .await?;
            }
            LineworksCommand::TodayShift => {
                let today = requested_at.date();
                let year_month = YearMonth::new(today.year(), today.month())
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                let assignments = self
                    .shift_repo
                    .list_for_month(account.employee_id, year_month)
                    .await?;

                let today_shift = assignments.iter().find(|a| a.date == today);

                let message = match today_shift {
                    None => "本日のシフトはありません。".to_string(),
                    Some(shift) => {
                        let start = shift
                            .planned_start_at
                            .as_ref()
                            .map(|z| z.strftime("%H:%M").to_string())
                            .unwrap_or_else(|| "未設定".to_string());
                        let end = shift
                            .planned_end_at
                            .as_ref()
                            .map(|z| z.strftime("%H:%M").to_string())
                            .unwrap_or_else(|| "未設定".to_string());
                        format!("【本日のシフト】\n{start} - {end}")
                    }
                };

                self.notifier
                    .notify(NotifyEvent::LineworksResponse {
                        user_id: external_user_id.to_string(),
                        text: message,
                    })
                    .await?;
            }
            LineworksCommand::MonthShift => {
                let today = requested_at.date();
                let year_month = YearMonth::new(today.year(), today.month())
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                let assignments = self
                    .shift_repo
                    .list_for_month(account.employee_id, year_month)
                    .await?;

                let message = if assignments.is_empty() {
                    "今月のシフトはまだ登録されていません。".to_string()
                } else {
                    let mut lines = vec![format!("【{}月のシフト】", today.month())];
                    for a in assignments {
                        let start = a
                            .planned_start_at
                            .as_ref()
                            .map(|z| z.strftime("%H:%M").to_string())
                            .unwrap_or_else(|| "未設定".to_string());
                        let end = a
                            .planned_end_at
                            .as_ref()
                            .map(|z| z.strftime("%H:%M").to_string())
                            .unwrap_or_else(|| "未設定".to_string());
                        lines.push(format!("{}: {start} - {end}", a.date.strftime("%m/%d")));
                    }
                    lines.join("\n")
                };

                self.notifier
                    .notify(NotifyEvent::LineworksResponse {
                        user_id: external_user_id.to_string(),
                        text: message,
                    })
                    .await?;
            }
            LineworksCommand::MonthAttendance | LineworksCommand::Correction { .. } => {
                // TODO: 月次勤怠集計・修正申請の承認フローを実装
                self.notifier
                    .notify(NotifyEvent::LineworksResponse {
                        user_id: external_user_id.to_string(),
                        text: "申し訳ありません。そのコマンドは現在準備中です。".to_string(),
                    })
                    .await?;
            }
        }

        Ok(())
    }
}

pub fn decide_lineworks_request_status(
    command: &LineworksCommand,
    requested_at: &Zoned,
    is_period_locked: bool,
    minor_correction_threshold_minutes: i64,
    correction_delta_minutes: Option<i64>,
) -> Option<AttendanceRequestStatus> {
    match command {
        LineworksCommand::TodayAttendance
        | LineworksCommand::MonthAttendance
        | LineworksCommand::TodayShift
        | LineworksCommand::MonthShift => Some(AttendanceRequestStatus::AutoApproved),
        LineworksCommand::MissingIn { .. } => {
            Some(super::attendance::decide_attendance_request_status(
                AttendanceRequestType::MissingIn,
                requested_at,
                requested_at.date(),
                is_period_locked,
                minor_correction_threshold_minutes,
                None,
            ))
        }
        LineworksCommand::MissingOut { .. } => {
            Some(super::attendance::decide_attendance_request_status(
                AttendanceRequestType::MissingOut,
                requested_at,
                requested_at.date(),
                is_period_locked,
                minor_correction_threshold_minutes,
                None,
            ))
        }
        LineworksCommand::Correction { date, .. } => {
            Some(super::attendance::decide_attendance_request_status(
                AttendanceRequestType::Correction,
                requested_at,
                *date,
                is_period_locked,
                minor_correction_threshold_minutes,
                correction_delta_minutes,
            ))
        }
        LineworksCommand::Help => None,
    }
}

fn parse_clock_time(input: &str) -> Option<ClockTime> {
    let (hour, minute) = input.split_once(':')?;
    let hour = hour.parse::<u8>().ok()?;
    let minute = minute.parse::<u8>().ok()?;
    ClockTime::new(hour, minute)
}

fn parse_date(input: &str) -> Option<Date> {
    let (year, rest) = input.split_once('-')?;
    let (month, day) = rest.split_once('-')?;
    let year = year.parse::<i16>().ok()?;
    let month = month.parse::<i8>().ok()?;
    let day = day.parse::<i8>().ok()?;

    Date::new(year, month, day).ok()
}

fn parse_target(input: &str) -> Option<CorrectionTarget> {
    match input {
        "出勤" => Some(CorrectionTarget::ClockIn),
        "退勤" => Some(CorrectionTarget::ClockOut),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
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
        let request_repo = Arc::new(MockRequestRepo);
        let punch_repo = Arc::new(MockPunchRepo);
        let shift_repo = Arc::new(MockShiftRepo);
        let notifier = Arc::new(MockNotifier::default());

        let use_case = super::LineworksUseCase::new(
            external_repo,
            request_repo,
            punch_repo,
            shift_repo,
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
        let request_repo = Arc::new(MockRequestRepo);
        let punch_repo = Arc::new(MockPunchRepo);
        let shift_repo = Arc::new(MockShiftRepo);
        let notifier = Arc::new(MockNotifier::default());

        let use_case = super::LineworksUseCase::new(
            external_repo,
            request_repo,
            punch_repo,
            shift_repo,
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

    use std::sync::Arc;
    use tokio::sync::Mutex;

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
        async fn bind(
            &self,
            _: uuid::Uuid,
            _: &str,
            _: &str,
        ) -> Result<crate::domain::employee::ExternalAccount, crate::port::repo::RepoError>
        {
            unimplemented!()
        }
    }

    struct MockRequestRepo;
    #[async_trait::async_trait]
    impl crate::port::repo::AttendanceRequestRepository for MockRequestRepo {
        async fn create(
            &self,
            _: crate::domain::request::NewAttendanceRequest,
        ) -> Result<crate::domain::request::AttendanceRequest, crate::port::repo::RepoError>
        {
            Ok(crate::domain::request::AttendanceRequest {
                id: uuid::Uuid::now_v7(),
                employee_id: uuid::Uuid::now_v7(),
                request_type: crate::domain::request::AttendanceRequestType::MissingIn,
                requested_payload_json: "{}".to_string(),
                status: crate::domain::request::AttendanceRequestStatus::Requested,
                requested_via: crate::domain::request::AttendanceRequestSource::LineWorks,
                requested_at: jiff::Timestamp::now()
                    .to_zoned(jiff::tz::TimeZone::get("Asia/Tokyo").unwrap()),
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
    }

    struct MockPunchRepo;
    #[async_trait::async_trait]
    impl crate::port::repo::PunchRepository for MockPunchRepo {
        async fn insert(
            &self,
            _: crate::domain::punch::NewPunchEvent,
        ) -> Result<crate::domain::punch::PunchEvent, crate::port::repo::RepoError> {
            unimplemented!()
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
            _: &Zoned,
            _: &Zoned,
        ) -> Result<Vec<crate::domain::punch::PunchEvent>, crate::port::repo::RepoError> {
            Ok(vec![crate::domain::punch::PunchEvent {
                id: uuid::Uuid::now_v7(),
                employee_id: uuid::Uuid::now_v7(),
                card_id: None,
                event_type: crate::port::policy::PunchEventType::ClockIn,
                occurred_at: tokyo_datetime(2026, 4, 16, 9, 0),
                server_recorded_at: tokyo_datetime(2026, 4, 16, 9, 1),
                source: "nfc".to_string(),
                correction_reason: None,
                deleted_at: None,
                created_at: tokyo_datetime(2026, 4, 16, 9, 1),
                updated_at: tokyo_datetime(2026, 4, 16, 9, 1),
            }])
        }
        async fn update(
            &self,
            _: uuid::Uuid,
            _: crate::domain::punch::PunchPatch,
            _: String,
        ) -> Result<crate::domain::punch::PunchEvent, crate::port::repo::RepoError> {
            unimplemented!()
        }
        async fn soft_delete(
            &self,
            _: uuid::Uuid,
            _: String,
        ) -> Result<(), crate::port::repo::RepoError> {
            unimplemented!()
        }
    }

    struct MockShiftRepo;
    #[async_trait::async_trait]
    impl crate::port::repo::ShiftRepository for MockShiftRepo {
        async fn list_for_month(
            &self,
            _: uuid::Uuid,
            _: crate::domain::time::YearMonth,
        ) -> Result<Vec<crate::domain::shift::ShiftAssignment>, crate::port::repo::RepoError>
        {
            unimplemented!()
        }
        async fn list_types(
            &self,
        ) -> Result<Vec<crate::domain::shift::ShiftType>, crate::port::repo::RepoError> {
            unimplemented!()
        }
    }

    fn tokyo_datetime(year: i16, month: i8, day: i8, hour: i8, minute: i8) -> Zoned {
        date(year, month, day)
            .at(hour, minute, 0, 0)
            .in_tz("Asia/Tokyo")
            .expect("Asia/Tokyo datetime should be valid")
    }
}

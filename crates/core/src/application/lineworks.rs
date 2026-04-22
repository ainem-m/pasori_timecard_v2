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
use crate::domain::time::{CutoffDay, CutoffRule, YearMonth};
use crate::port::notify::{Notifier, NotifyEvent};
use crate::port::policy::NoRounding;
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
                let today = requested_at.date();
                let from = day_start_in_tokyo(today).map_err(|e| anyhow::anyhow!("{e}"))?;
                let to = day_end_in_tokyo(today).map_err(|e| anyhow::anyhow!("{e}"))?;
                let punches: Vec<crate::domain::punch::PunchEvent> = self
                    .punch_repo
                    .list_in_range(account.employee_id, &from, &to)
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
            LineworksCommand::MonthAttendance => {
                let today = requested_at.date();
                let year_month = YearMonth::new(today.year(), today.month())
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                let cutoff_rule =
                    CutoffRule::DayOfMonth(CutoffDay::new(15).map_err(|e| anyhow::anyhow!("{e}"))?);
                let period = year_month
                    .attendance_period(cutoff_rule)
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                let from =
                    day_start_in_tokyo(period.period_start).map_err(|e| anyhow::anyhow!("{e}"))?;
                let to = day_end_in_tokyo(period.period_end).map_err(|e| anyhow::anyhow!("{e}"))?;
                let punches = self
                    .punch_repo
                    .list_in_range(account.employee_id, &from, &to)
                    .await?;

                let timesheet = build_lineworks_monthly_timesheet(
                    account.employee_id,
                    year_month,
                    cutoff_rule,
                    punches,
                )?;

                let message = format!(
                    "【今月の勤怠】\n期間: {} - {}\n勤務時間合計: {}",
                    timesheet.period_start,
                    timesheet.period_end,
                    format_work_minutes(timesheet.total_work_minutes),
                );

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
                        status,
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
            LineworksCommand::Correction { date, target, time } => {
                let from = day_start_in_tokyo(date).map_err(|e| anyhow::anyhow!("{e}"))?;
                let to = day_end_in_tokyo(date).map_err(|e| anyhow::anyhow!("{e}"))?;
                let punches = self
                    .punch_repo
                    .list_in_range(account.employee_id, &from, &to)
                    .await?;
                let correction_delta_minutes =
                    calculate_correction_delta_minutes(&punches, target, time, &from);
                let status = decide_lineworks_request_status(
                    &command,
                    requested_at,
                    false, // TODO: check period lock
                    120,   // TODO: settings
                    correction_delta_minutes,
                )
                .unwrap_or(AttendanceRequestStatus::Requested);

                let payload = serde_json::json!({
                    "command": format!("{:?}", command),
                    "date": date.to_string(),
                    "target": match target {
                        CorrectionTarget::ClockIn => "clock_in",
                        CorrectionTarget::ClockOut => "clock_out",
                    },
                    "time": format!("{:02}:{:02}", time.hour(), time.minute()),
                });

                let request = self
                    .request_repo
                    .create(NewAttendanceRequest {
                        employee_id: account.employee_id,
                        request_type: AttendanceRequestType::Correction,
                        requested_payload_json: payload.to_string(),
                        status,
                        requested_via: AttendanceRequestSource::LineWorks,
                        requested_at: requested_at.clone(),
                    })
                    .await?;

                if status == AttendanceRequestStatus::AutoApproved {
                    let applied_punch_id = apply_correction_to_punch_repo(
                        self.punch_repo.as_ref(),
                        &punches,
                        target,
                        time,
                    )
                    .await?;
                    self.request_repo
                        .update_status(
                            request.id,
                            AttendanceRequestStatus::Applied,
                            Some(applied_punch_id),
                        )
                        .await?;
                }

                let response_text = if status == AttendanceRequestStatus::AutoApproved {
                    "修正申請を自動承認し、反映しました。".to_string()
                } else {
                    "修正申請を受け付けました。管理者の承認をお待ちください。".to_string()
                };

                self.notifier
                    .notify(NotifyEvent::LineworksResponse {
                        user_id: external_user_id.to_string(),
                        text: response_text,
                    })
                    .await?;
            }
        }

        Ok(())
    }
}

fn build_lineworks_monthly_timesheet(
    employee_id: uuid::Uuid,
    year_month: YearMonth,
    cutoff_rule: CutoffRule,
    punches: Vec<crate::domain::punch::PunchEvent>,
) -> anyhow::Result<crate::domain::time::MonthlyTimesheet> {
    let mut grouped: std::collections::BTreeMap<Date, Vec<crate::domain::punch::PunchEvent>> =
        std::collections::BTreeMap::new();

    for punch in punches {
        grouped
            .entry(punch.occurred_at.date())
            .or_default()
            .push(punch);
    }

    let days = grouped
        .into_iter()
        .map(|(date, events)| {
            super::attendance::build_attendance_day(
                date,
                events,
                crate::domain::punch::AttendanceDayStatus::Confirmed,
                &NoRounding,
            )
        })
        .collect();

    super::attendance::build_monthly_timesheet(employee_id, year_month, cutoff_rule, days)
        .map_err(|e| anyhow::anyhow!("{e}"))
}

fn day_start_in_tokyo(date: Date) -> Result<Zoned, jiff::Error> {
    format!("{date}T00:00:00+09:00[Asia/Tokyo]").parse()
}

fn day_end_in_tokyo(date: Date) -> Result<Zoned, jiff::Error> {
    format!("{date}T23:59:59+09:00[Asia/Tokyo]").parse()
}

fn format_work_minutes(total_minutes: i64) -> String {
    let hours = total_minutes / 60;
    let minutes = total_minutes % 60;
    format!("{hours}時間{minutes}分")
}

async fn apply_correction_to_punch_repo(
    punch_repo: &dyn PunchRepository,
    punches: &[crate::domain::punch::PunchEvent],
    target: CorrectionTarget,
    time: ClockTime,
) -> anyhow::Result<uuid::Uuid> {
    let target_event_type = match target {
        CorrectionTarget::ClockIn => crate::port::policy::PunchEventType::ClockIn,
        CorrectionTarget::ClockOut => crate::port::policy::PunchEventType::ClockOut,
    };

    let existing = punches
        .iter()
        .find(|punch| punch.event_type == target_event_type)
        .ok_or_else(|| anyhow::anyhow!("target punch not found for correction"))?;
    let corrected_at = build_corrected_time(&existing.occurred_at, time)?;
    let updated = punch_repo
        .update(
            existing.id,
            crate::domain::punch::PunchPatch {
                event_type: Some(target_event_type),
                occurred_at: Some(corrected_at),
            },
            "lineworks correction".to_string(),
        )
        .await?;

    Ok(updated.id)
}

fn build_corrected_time(base: &Zoned, time: ClockTime) -> anyhow::Result<Zoned> {
    base.date()
        .at(
            i8::try_from(time.hour()).map_err(|e| anyhow::anyhow!("{e}"))?,
            i8::try_from(time.minute()).map_err(|e| anyhow::anyhow!("{e}"))?,
            0,
            0,
        )
        .in_tz("Asia/Tokyo")
        .map_err(|e| anyhow::anyhow!("{e}"))
}

fn calculate_correction_delta_minutes(
    punches: &[crate::domain::punch::PunchEvent],
    target: CorrectionTarget,
    time: ClockTime,
    fallback_day_start: &Zoned,
) -> Option<i64> {
    let target_event_type = match target {
        CorrectionTarget::ClockIn => crate::port::policy::PunchEventType::ClockIn,
        CorrectionTarget::ClockOut => crate::port::policy::PunchEventType::ClockOut,
    };

    let requested_minutes = i64::from(time.hour()) * 60 + i64::from(time.minute());
    let reference = punches
        .iter()
        .find(|punch| punch.event_type == target_event_type)
        .map(|punch| punch.occurred_at.clone())
        .unwrap_or_else(|| fallback_day_start.clone());
    let reference_minutes = i64::from(reference.hour()) * 60 + i64::from(reference.minute());

    Some((requested_minutes - reference_minutes).abs())
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
        ) -> Result<crate::domain::request::AttendanceRequest, crate::port::repo::RepoError>
        {
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
        ) -> Result<crate::domain::request::AttendanceRequest, crate::port::repo::RepoError>
        {
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
        updates: Mutex<Vec<RecordedPunchUpdate>>,
    }

    struct RecordedPunchUpdate {
        patch: crate::domain::punch::PunchPatch,
    }
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
}

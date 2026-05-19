use jiff::Zoned;
use jiff::civil::Date;
use uuid::Uuid;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessEventOutcome {
    NoAction,
    HelpSent,
    TodayAttendanceSent,
    MonthAttendanceSent,
    TodayShiftSent,
    MonthShiftSent,
    UnregisteredUser,
    RequestCreated {
        request_id: Uuid,
    },
    AutoApproved {
        request_id: Uuid,
        punch_id: Uuid,
        request_type: AttendanceRequestType,
    },
}

impl ProcessEventOutcome {
    pub fn is_auto_approved(&self) -> bool {
        matches!(self, Self::AutoApproved { .. })
    }
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

use crate::domain::punch::NewPunchEvent;
use crate::domain::request::{
    AttendanceRequestSource, AttendanceRequestStatus, AttendanceRequestType, NewAttendanceRequest,
};
use crate::domain::time::{CutoffDay, CutoffRule, YearMonth};
use crate::port::notify::{Notifier, NotifyEvent};
use crate::port::policy::{NoRounding, PunchEventType};
use crate::port::repo::{
    AttendanceRequestRepository, CardRepository, ExternalAccountRepository, PunchRepository,
    ShiftRepository,
};
use std::sync::Arc;

const MVP_PERIOD_LOCKED: bool = false;
const MVP_MINOR_CORRECTION_THRESHOLD_MINUTES: i64 = 120;

pub struct LineworksUseCase {
    external_repo: Arc<dyn ExternalAccountRepository>,
    request_repo: Arc<dyn AttendanceRequestRepository>,
    punch_repo: Arc<dyn PunchRepository>,
    shift_repo: Arc<dyn ShiftRepository>,
    card_repo: Arc<dyn CardRepository>,
    notifier: Arc<dyn Notifier>,
}

impl LineworksUseCase {
    pub fn new(
        external_repo: Arc<dyn ExternalAccountRepository>,
        request_repo: Arc<dyn AttendanceRequestRepository>,
        punch_repo: Arc<dyn PunchRepository>,
        shift_repo: Arc<dyn ShiftRepository>,
        card_repo: Arc<dyn CardRepository>,
        notifier: Arc<dyn Notifier>,
    ) -> Self {
        Self {
            external_repo,
            request_repo,
            punch_repo,
            shift_repo,
            card_repo,
            notifier,
        }
    }

    pub async fn process_event(
        &self,
        external_user_id: &str,
        command: LineworksCommand,
        requested_at: &Zoned,
    ) -> anyhow::Result<ProcessEventOutcome> {
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
            return Ok(ProcessEventOutcome::UnregisteredUser);
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
                Ok(ProcessEventOutcome::HelpSent)
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
                Ok(ProcessEventOutcome::TodayAttendanceSent)
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
                Ok(ProcessEventOutcome::MonthAttendanceSent)
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
                    MVP_PERIOD_LOCKED,
                    MVP_MINOR_CORRECTION_THRESHOLD_MINUTES,
                    None,
                )
                .unwrap_or(AttendanceRequestStatus::Requested);

                let payload = serde_json::json!({
                    "command": format!("{:?}", command),
                    "time": format!("{:02}:{:02}", time.hour(), time.minute()),
                });

                let request = self
                    .request_repo
                    .create(NewAttendanceRequest {
                        employee_id: account.employee_id,
                        request_type,
                        requested_payload_json: payload.to_string(),
                        status,
                        requested_via: AttendanceRequestSource::LineWorks,
                        requested_at: requested_at.clone(),
                    })
                    .await?;

                let mut punch_id: Option<Uuid> = None;

                if status == AttendanceRequestStatus::AutoApproved {
                    let event_type = match request_type {
                        AttendanceRequestType::MissingIn => PunchEventType::ClockIn,
                        AttendanceRequestType::MissingOut => PunchEventType::ClockOut,
                        _ => unreachable!(),
                    };
                    let occurred_at = build_punch_time(requested_at.date(), time)?;
                    let card_id = self
                        .card_repo
                        .find_by_employee(account.employee_id)
                        .await
                        .ok()
                        .flatten()
                        .map(|c| c.id);

                    let punch = self
                        .punch_repo
                        .insert(NewPunchEvent {
                            id: Uuid::now_v7(),
                            employee_id: account.employee_id,
                            card_id,
                            event_type,
                            occurred_at: occurred_at.clone(),
                            source: "lineworks".to_string(),
                        })
                        .await?;

                    punch_id = Some(punch.id);

                    self.request_repo
                        .update_status(request.id, AttendanceRequestStatus::Applied, Some(punch.id))
                        .await?;
                }

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

                if status == AttendanceRequestStatus::AutoApproved {
                    Ok(ProcessEventOutcome::AutoApproved {
                        request_id: request.id,
                        punch_id: punch_id.ok_or_else(|| {
                            anyhow::anyhow!("punch_id missing after auto-approval")
                        })?,
                        request_type,
                    })
                } else {
                    Ok(ProcessEventOutcome::RequestCreated {
                        request_id: request.id,
                    })
                }
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
                Ok(ProcessEventOutcome::TodayShiftSent)
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
                Ok(ProcessEventOutcome::MonthShiftSent)
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
                    MVP_PERIOD_LOCKED,
                    MVP_MINOR_CORRECTION_THRESHOLD_MINUTES,
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

                Ok(ProcessEventOutcome::RequestCreated {
                    request_id: request.id,
                })
            }
        }
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

fn build_punch_time(date: Date, time: ClockTime) -> anyhow::Result<Zoned> {
    date.at(
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
mod tests;

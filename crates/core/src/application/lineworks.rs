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

pub fn decide_lineworks_request_status(
    command: &LineworksCommand,
    requested_at: &Zoned,
    is_period_locked: bool,
    minor_correction_threshold_minutes: i64,
    correction_delta_minutes: Option<i64>,
) -> Option<crate::domain::request::AttendanceRequestStatus> {
    use crate::domain::request::{AttendanceRequestStatus, AttendanceRequestType};

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

    fn tokyo_datetime(year: i16, month: i8, day: i8, hour: i8, minute: i8) -> Zoned {
        date(year, month, day)
            .at(hour, minute, 0, 0)
            .in_tz("Asia/Tokyo")
            .expect("Asia/Tokyo datetime should be valid")
    }
}

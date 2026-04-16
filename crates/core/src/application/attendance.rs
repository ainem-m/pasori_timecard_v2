use crate::domain::punch::{AttendanceDay, PunchEvent};
use crate::domain::request::{AttendanceRequestStatus, AttendanceRequestType};
use crate::domain::time::{CutoffRule, MonthlyTimesheet, TimeDomainError, YearMonth};
use crate::port::policy::{PunchEventType, RoundingPolicy};
use jiff::{Zoned, civil::Date};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShiftPlanKind {
    Work,
    Off,
}

#[derive(Debug, Clone)]
pub struct ShiftPlan {
    pub kind: ShiftPlanKind,
    pub planned_start_at: Option<Zoned>,
    pub planned_end_at: Option<Zoned>,
}

pub fn build_attendance_day(
    date: jiff::civil::Date,
    mut events: Vec<PunchEvent>,
    status: crate::domain::punch::AttendanceDayStatus,
    rounding_policy: &dyn RoundingPolicy,
) -> AttendanceDay {
    events.sort_by_key(|event| event.occurred_at.timestamp().as_second());
    AttendanceDay::from_events(date, events, status, rounding_policy)
}

pub fn build_monthly_timesheet(
    employee_id: Uuid,
    year_month: YearMonth,
    cutoff_rule: CutoffRule,
    mut days: Vec<AttendanceDay>,
) -> Result<MonthlyTimesheet, TimeDomainError> {
    days.sort_by_key(|day| day.date);
    MonthlyTimesheet::from_days(employee_id, year_month, cutoff_rule, days)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShiftMismatch {
    MissingPunch,
    PunchOnScheduledOffDay,
    StartTimeMismatch { delta_minutes: i64 },
    EndTimeMismatch { delta_minutes: i64 },
    ClockOutMissing,
}

pub fn compare_with_shift(
    plan: Option<&ShiftPlan>,
    attendance_day: Option<&AttendanceDay>,
    threshold_minutes: i64,
) -> Vec<ShiftMismatch> {
    match plan {
        None => match attendance_day {
            None => vec![],
            Some(_day) => vec![ShiftMismatch::PunchOnScheduledOffDay],
        },
        Some(plan) => match plan.kind {
            ShiftPlanKind::Work => match attendance_day {
                None => vec![ShiftMismatch::MissingPunch],
                Some(day) => compare_work_plan(plan, day, threshold_minutes),
            },
            ShiftPlanKind::Off => match attendance_day {
                None => vec![],
                Some(_day) => vec![ShiftMismatch::PunchOnScheduledOffDay],
            },
        },
    }
}

pub fn decide_attendance_request_status(
    request_type: AttendanceRequestType,
    requested_at: &Zoned,
    target_date: Date,
    is_period_locked: bool,
    minor_correction_threshold_minutes: i64,
    correction_delta_minutes: Option<i64>,
) -> AttendanceRequestStatus {
    match request_type {
        AttendanceRequestType::QueryAttendance | AttendanceRequestType::QueryShift => {
            AttendanceRequestStatus::AutoApproved
        }
        AttendanceRequestType::Correction
        | AttendanceRequestType::MissingIn
        | AttendanceRequestType::MissingOut => {
            if is_period_locked {
                return AttendanceRequestStatus::Rejected;
            }

            let is_same_day = requested_at.date() == target_date;
            let is_minor_correction = correction_delta_minutes
                .map(|delta| delta.abs() <= minor_correction_threshold_minutes)
                .unwrap_or(false);

            if is_same_day
                && (matches!(
                    request_type,
                    AttendanceRequestType::MissingIn | AttendanceRequestType::MissingOut
                ) || is_minor_correction)
            {
                AttendanceRequestStatus::AutoApproved
            } else {
                AttendanceRequestStatus::Approved
            }
        }
    }
}

fn compare_work_plan(
    plan: &ShiftPlan,
    day: &AttendanceDay,
    threshold_minutes: i64,
) -> Vec<ShiftMismatch> {
    let mut mismatches = Vec::new();
    let actual_start = first_clock_in(&day.events);
    let actual_end = last_clock_out(&day.events);

    if actual_start.is_none() && actual_end.is_none() {
        mismatches.push(ShiftMismatch::MissingPunch);
        return mismatches;
    }

    if actual_start.is_none() {
        mismatches.push(ShiftMismatch::MissingPunch);
        return mismatches;
    }

    if actual_end.is_none() {
        mismatches.push(ShiftMismatch::ClockOutMissing);
    }

    if let (Some(planned_start), Some(actual_start)) =
        (plan.planned_start_at.as_ref(), actual_start)
    {
        let delta_minutes = absolute_minutes_between(planned_start, &actual_start);
        if delta_minutes > threshold_minutes {
            mismatches.push(ShiftMismatch::StartTimeMismatch { delta_minutes });
        }
    }

    if let (Some(planned_end), Some(actual_end)) = (plan.planned_end_at.as_ref(), actual_end) {
        let delta_minutes = absolute_minutes_between(planned_end, &actual_end);
        if delta_minutes > threshold_minutes {
            mismatches.push(ShiftMismatch::EndTimeMismatch { delta_minutes });
        }
    }

    mismatches
}

fn first_clock_in(events: &[PunchEvent]) -> Option<Zoned> {
    events.iter().find_map(|event| match event.event_type {
        PunchEventType::ClockIn => Some(event.occurred_at.clone()),
        _ => None,
    })
}

fn last_clock_out(events: &[PunchEvent]) -> Option<Zoned> {
    events
        .iter()
        .rev()
        .find_map(|event| match event.event_type {
            PunchEventType::ClockOut => Some(event.occurred_at.clone()),
            _ => None,
        })
}

fn absolute_minutes_between(a: &Zoned, b: &Zoned) -> i64 {
    let delta_seconds = b.timestamp().as_second() - a.timestamp().as_second();
    delta_seconds.abs() / 60
}

#[cfg(test)]
mod tests {
    use super::{
        ShiftMismatch, ShiftPlan, ShiftPlanKind, build_attendance_day, build_monthly_timesheet,
        compare_with_shift, decide_attendance_request_status,
    };
    use crate::domain::punch::{AttendanceDay, AttendanceDayStatus, PunchEvent};
    use crate::domain::request::{AttendanceRequestStatus, AttendanceRequestType};
    use crate::domain::time::{CutoffDay, CutoffRule, TimeDomainError, YearMonth};
    use crate::port::policy::{NoRounding, PunchEventType};
    use jiff::{Zoned, civil::date};
    use proptest::prelude::*;
    use uuid::Uuid;

    #[test]
    // 予定勤務日に打刻がない場合は打刻漏れ疑いになる。
    fn marks_missing_punch_when_work_day_has_no_attendance() {
        let plan = work_plan(2026, 4, 16, 9, 0, 18, 0);

        let mismatches = compare_with_shift(Some(&plan), None, 30);

        assert_eq!(mismatches, vec![ShiftMismatch::MissingPunch]);
    }

    #[test]
    // 休み予定日に打刻がある場合はシフト外打刻として扱う。
    fn marks_punch_on_off_day() {
        let plan = ShiftPlan {
            kind: ShiftPlanKind::Off,
            planned_start_at: None,
            planned_end_at: None,
        };
        let day = attendance_day(vec![clock_in(2026, 4, 16, 9, 0)]);

        let mismatches = compare_with_shift(Some(&plan), Some(&day), 30);

        assert_eq!(mismatches, vec![ShiftMismatch::PunchOnScheduledOffDay]);
    }

    #[test]
    // 予定開始時刻と実績開始時刻の差が閾値以内なら不整合にしない。
    fn ignores_start_time_difference_within_threshold() {
        let plan = work_plan(2026, 4, 16, 9, 0, 18, 0);
        let day = attendance_day(vec![
            clock_in(2026, 4, 16, 9, 20),
            clock_out(2026, 4, 16, 18, 0),
        ]);

        let mismatches = compare_with_shift(Some(&plan), Some(&day), 30);

        assert!(mismatches.is_empty());
    }

    #[test]
    // 予定開始時刻と実績開始時刻の差が閾値を超える場合は不整合にする。
    fn marks_start_time_difference_over_threshold() {
        let plan = work_plan(2026, 4, 16, 9, 0, 18, 0);
        let day = attendance_day(vec![
            clock_in(2026, 4, 16, 10, 31),
            clock_out(2026, 4, 16, 18, 0),
        ]);

        let mismatches = compare_with_shift(Some(&plan), Some(&day), 30);

        assert_eq!(
            mismatches,
            vec![ShiftMismatch::StartTimeMismatch { delta_minutes: 91 }]
        );
    }

    #[test]
    // 退勤打刻がなければ退勤漏れとして扱う。
    fn marks_missing_clock_out() {
        let plan = work_plan(2026, 4, 16, 9, 0, 18, 0);
        let day = attendance_day(vec![clock_in(2026, 4, 16, 9, 0)]);

        let mismatches = compare_with_shift(Some(&plan), Some(&day), 30);

        assert_eq!(mismatches, vec![ShiftMismatch::ClockOutMissing]);
    }

    #[test]
    // 勤怠照会は常に自動応答扱いにする。
    fn auto_approves_query_requests() {
        let requested_at = tokyo_datetime(2026, 4, 16, 10, 0);

        let status = decide_attendance_request_status(
            AttendanceRequestType::QueryAttendance,
            &requested_at,
            date(2026, 4, 16),
            false,
            120,
            None,
        );

        assert_eq!(status, AttendanceRequestStatus::AutoApproved);
    }

    #[test]
    // 当日中の出勤忘れ申請は自動承認する。
    fn auto_approves_same_day_missing_in_request() {
        let requested_at = tokyo_datetime(2026, 4, 16, 10, 0);

        let status = decide_attendance_request_status(
            AttendanceRequestType::MissingIn,
            &requested_at,
            date(2026, 4, 16),
            false,
            120,
            None,
        );

        assert_eq!(status, AttendanceRequestStatus::AutoApproved);
    }

    #[test]
    // 当日中の軽微修正は閾値内なら自動承認する。
    fn auto_approves_same_day_minor_correction() {
        let requested_at = tokyo_datetime(2026, 4, 16, 10, 0);

        let status = decide_attendance_request_status(
            AttendanceRequestType::Correction,
            &requested_at,
            date(2026, 4, 16),
            false,
            120,
            Some(90),
        );

        assert_eq!(status, AttendanceRequestStatus::AutoApproved);
    }

    #[test]
    // 当日中でも軽微修正の閾値を超える場合は管理者承認に回す。
    fn sends_large_same_day_correction_to_manual_review() {
        let requested_at = tokyo_datetime(2026, 4, 16, 10, 0);

        let status = decide_attendance_request_status(
            AttendanceRequestType::Correction,
            &requested_at,
            date(2026, 4, 16),
            false,
            120,
            Some(181),
        );

        assert_eq!(status, AttendanceRequestStatus::Approved);
    }

    #[test]
    // 締め済み期間の修正は自動却下する。
    fn rejects_locked_period_requests() {
        let requested_at = tokyo_datetime(2026, 4, 16, 10, 0);

        let status = decide_attendance_request_status(
            AttendanceRequestType::Correction,
            &requested_at,
            date(2026, 4, 15),
            true,
            120,
            Some(15),
        );

        assert_eq!(status, AttendanceRequestStatus::Rejected);
    }

    #[test]
    // 日次勤怠は時系列順に並び替えてから組み立てる。
    fn builds_attendance_day_after_sorting_events() {
        let day = build_attendance_day(
            date(2026, 4, 16),
            vec![clock_out(2026, 4, 16, 18, 0), clock_in(2026, 4, 16, 9, 0)],
            AttendanceDayStatus::Confirmed,
            &NoRounding,
        );

        assert_eq!(day.events[0].event_type, PunchEventType::ClockIn);
        assert_eq!(day.events[1].event_type, PunchEventType::ClockOut);
        assert_eq!(day.work_minutes, 540);
    }

    #[test]
    // 日次勤怠の組み立ては domain の不整合判定をそのまま使う。
    fn builds_inconsistent_attendance_day_for_missing_clock_out() {
        let day = build_attendance_day(
            date(2026, 4, 16),
            vec![clock_in(2026, 4, 16, 9, 0)],
            AttendanceDayStatus::Unconfirmed,
            &NoRounding,
        );

        assert!(day.has_inconsistency);
    }

    #[test]
    // 月次勤怠表は日付順に並び替えてから組み立てる。
    fn builds_monthly_timesheet_after_sorting_days() {
        let employee_id = Uuid::now_v7();
        let year_month = YearMonth::new(2026, 4).expect("valid year_month");
        let cutoff_rule = CutoffRule::DayOfMonth(CutoffDay::new(15).expect("valid cutoff day"));
        let days = vec![
            attendance_day_with_minutes(date(2026, 4, 2), 240),
            attendance_day_with_minutes(date(2026, 3, 16), 480),
        ];

        let timesheet = build_monthly_timesheet(employee_id, year_month, cutoff_rule, days)
            .expect("monthly timesheet should be built");

        assert_eq!(timesheet.days[0].date, date(2026, 3, 16));
        assert_eq!(timesheet.days[1].date, date(2026, 4, 2));
        assert_eq!(timesheet.total_work_minutes, 720);
    }

    #[test]
    // 月次勤怠表の組み立て時も締め期間外の日付は拒否する。
    fn rejects_days_outside_period_when_building_monthly_timesheet() {
        let employee_id = Uuid::now_v7();
        let year_month = YearMonth::new(2026, 4).expect("valid year_month");
        let cutoff_rule = CutoffRule::DayOfMonth(CutoffDay::new(15).expect("valid cutoff day"));
        let days = vec![attendance_day_with_minutes(date(2026, 4, 16), 480)];

        let error = build_monthly_timesheet(employee_id, year_month, cutoff_rule, days)
            .expect_err("day outside period should be rejected");

        assert!(matches!(error, TimeDomainError::DayOutOfRange { .. }));
    }

    proptest! {
        #[test]
        // 休み予定日に打刻がある限り、シフト外打刻として検出される。
        fn marks_any_attendance_on_off_day_as_mismatch(
            hour in 0i8..=23i8,
            minute in 0i8..=59i8,
        ) {
            let plan = ShiftPlan {
                kind: ShiftPlanKind::Off,
                planned_start_at: None,
                planned_end_at: None,
            };
            let day = attendance_day(vec![clock_in(2026, 4, 16, hour, minute)]);

            let mismatches = compare_with_shift(Some(&plan), Some(&day), 30);

            prop_assert_eq!(mismatches, vec![ShiftMismatch::PunchOnScheduledOffDay]);
        }

        #[test]
        // 予定勤務日の実績開始時刻が閾値内なら開始時刻不整合は出ない。
        fn does_not_mark_start_mismatch_within_threshold(
            offset_minutes in 0i64..=30i64,
        ) {
            let plan = work_plan(2026, 4, 16, 9, 0, 18, 0);
            let actual_start = add_minutes(2026, 4, 16, 9, 0, offset_minutes);
            let day = attendance_day(vec![
                actual_start,
                clock_out(2026, 4, 16, 18, 0),
            ]);

            let mismatches = compare_with_shift(Some(&plan), Some(&day), 30);

            let has_start_mismatch = mismatches
                .iter()
                .any(|m| matches!(m, ShiftMismatch::StartTimeMismatch { .. }));

            prop_assert!(!has_start_mismatch);
        }

        #[test]
        // 当日中の軽微修正は閾値以内であれば自動承認になる。
        fn auto_approves_same_day_corrections_within_threshold(
            delta_minutes in 0i64..=120i64,
        ) {
            let requested_at = tokyo_datetime(2026, 4, 16, 10, 0);
            let status = decide_attendance_request_status(
                AttendanceRequestType::Correction,
                &requested_at,
                date(2026, 4, 16),
                false,
                120,
                Some(delta_minutes),
            );

            prop_assert_eq!(status, AttendanceRequestStatus::AutoApproved);
        }
    }

    fn work_plan(
        year: i16,
        month: i8,
        day: i8,
        start_hour: i8,
        start_minute: i8,
        end_hour: i8,
        end_minute: i8,
    ) -> ShiftPlan {
        ShiftPlan {
            kind: ShiftPlanKind::Work,
            planned_start_at: Some(tokyo_datetime(year, month, day, start_hour, start_minute)),
            planned_end_at: Some(tokyo_datetime(year, month, day, end_hour, end_minute)),
        }
    }

    fn attendance_day(events: Vec<PunchEvent>) -> AttendanceDay {
        AttendanceDay {
            date: date(2026, 4, 16),
            events,
            work_minutes: 0,
            has_inconsistency: false,
            status: AttendanceDayStatus::Confirmed,
        }
    }

    fn attendance_day_with_minutes(date: jiff::civil::Date, work_minutes: i64) -> AttendanceDay {
        AttendanceDay {
            date,
            events: vec![],
            work_minutes,
            has_inconsistency: false,
            status: AttendanceDayStatus::Confirmed,
        }
    }

    fn clock_in(year: i16, month: i8, day: i8, hour: i8, minute: i8) -> PunchEvent {
        PunchEvent {
            event_type: PunchEventType::ClockIn,
            occurred_at: tokyo_datetime(year, month, day, hour, minute),
        }
    }

    fn clock_out(year: i16, month: i8, day: i8, hour: i8, minute: i8) -> PunchEvent {
        PunchEvent {
            event_type: PunchEventType::ClockOut,
            occurred_at: tokyo_datetime(year, month, day, hour, minute),
        }
    }

    fn add_minutes(
        year: i16,
        month: i8,
        day: i8,
        hour: i8,
        minute: i8,
        offset_minutes: i64,
    ) -> PunchEvent {
        let total_minutes = i64::from(hour) * 60 + i64::from(minute) + offset_minutes;
        let new_hour = i8::try_from(total_minutes / 60).expect("valid hour");
        let new_minute = i8::try_from(total_minutes % 60).expect("valid minute");

        clock_in(year, month, day, new_hour, new_minute)
    }

    fn tokyo_datetime(year: i16, month: i8, day: i8, hour: i8, minute: i8) -> Zoned {
        date(year, month, day)
            .at(hour, minute, 0, 0)
            .in_tz("Asia/Tokyo")
            .expect("Asia/Tokyo datetime should be valid")
    }
}

use crate::port::policy::{NoRounding, PunchEventType, RoundingPolicy};
use jiff::Zoned;
use jiff::civil::{Date, Weekday};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const MINIMUM_REASONABLE_LOOP_MINUTES: i64 = 180;
const MAXIMUM_CONTINUOUS_WORK_MINUTES: i64 = 24 * 60;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PunchEvent {
    pub id: Uuid,
    pub employee_id: Uuid,
    pub card_id: Option<Uuid>,
    pub event_type: PunchEventType,
    pub occurred_at: Zoned,
    pub server_recorded_at: Zoned,
    pub source: String, // 'nfc' / 'manual' / 'import' / 'local_cached'
    pub correction_reason: Option<String>,
    pub deleted_at: Option<Zoned>,
    pub created_at: Zoned,
    pub updated_at: Zoned,
}

impl PunchEvent {
    pub fn event_type_label(&self) -> &'static str {
        match self.event_type {
            PunchEventType::ClockIn => "出勤",
            PunchEventType::ClockOut => "退勤",
            PunchEventType::BreakStart => "休憩開始",
            PunchEventType::BreakEnd => "休憩終了",
            PunchEventType::TemporaryOut => "一時外出",
            PunchEventType::TemporaryReturn => "戻り",
            PunchEventType::ManualCorrection => "修正",
        }
    }
}

#[derive(Debug, Clone)]
pub struct NewPunchEvent {
    pub id: Uuid, // Terminal generated
    pub employee_id: Uuid,
    pub card_id: Option<Uuid>,
    pub event_type: PunchEventType,
    pub occurred_at: Zoned,
    pub source: String,
}

#[derive(Debug, Clone, Default)]
pub struct PunchPatch {
    pub event_type: Option<PunchEventType>,
    pub occurred_at: Option<Zoned>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AttendanceDayStatus {
    Unconfirmed,
    Confirmed,
    Locked,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PolicyProfile {
    #[serde(rename = "legacy_regular_2026")]
    LegacyRegular2026,
    #[serde(rename = "legacy_part_time_2026")]
    LegacyPartTime2026,
    #[serde(rename = "legacy_doctor_2026")]
    LegacyDoctor2026,
}

impl PolicyProfile {
    pub fn from_employment_type(employment_type: &str) -> Self {
        match employment_type {
            "part_time" => Self::LegacyPartTime2026,
            "doctor" => Self::LegacyDoctor2026,
            _ => Self::LegacyRegular2026,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DerivedAttendance {
    pub counted_work_minutes: i64,
    pub fixed_time_extra_minutes: i64,
    pub within_8h_work_minutes: i64,
    pub over_8h_work_minutes: i64,
    pub paid_leave_days: f64,
    pub work_days: i64,
    pub reference_work_minutes: i64,
    pub attendance_notes: Vec<String>,
}

impl Default for DerivedAttendance {
    fn default() -> Self {
        Self {
            counted_work_minutes: 0,
            fixed_time_extra_minutes: 0,
            within_8h_work_minutes: 0,
            over_8h_work_minutes: 0,
            paid_leave_days: 0.0,
            work_days: 0,
            reference_work_minutes: 0,
            attendance_notes: Vec::new(),
        }
    }
}

impl DerivedAttendance {
    pub fn for_day(profile: PolicyProfile, day: &AttendanceDay) -> Self {
        match profile {
            PolicyProfile::LegacyRegular2026 => regular_derived(day),
            PolicyProfile::LegacyPartTime2026 => part_time_derived(day),
            PolicyProfile::LegacyDoctor2026 => doctor_derived(day),
        }
    }

    pub fn sum_days(days: &[AttendanceDay]) -> Self {
        let mut total = Self::default();

        for day in days {
            total.counted_work_minutes += day.derived.counted_work_minutes;
            total.fixed_time_extra_minutes += day.derived.fixed_time_extra_minutes;
            total.within_8h_work_minutes += day.derived.within_8h_work_minutes;
            total.over_8h_work_minutes += day.derived.over_8h_work_minutes;
            total.paid_leave_days += day.derived.paid_leave_days;
            total.work_days += day.derived.work_days;
            total.reference_work_minutes += day.derived.reference_work_minutes;
            total
                .attendance_notes
                .extend(day.derived.attendance_notes.iter().cloned());
        }

        total
    }
}

#[derive(Debug, Clone)]
pub struct AttendanceDay {
    pub date: Date,
    pub events: Vec<PunchEvent>,
    pub work_minutes: i64,
    pub derived: DerivedAttendance,
    pub has_inconsistency: bool,
    pub status: AttendanceDayStatus,
}

impl AttendanceDay {
    pub fn from_events(
        date: Date,
        mut events: Vec<PunchEvent>,
        status: AttendanceDayStatus,
        _rounding_policy: &dyn RoundingPolicy,
    ) -> Self {
        events.sort_by_key(|event| event.occurred_at.timestamp().as_second());

        let mut work_minutes = 0;
        let mut has_inconsistency = false;
        let mut open_clock_in: Option<Zoned> = None;
        let mut pair_count = 0;

        for event in &events {
            match event.event_type {
                PunchEventType::ClockIn => {
                    if open_clock_in.is_some() {
                        has_inconsistency = true;
                    }

                    open_clock_in = Some(event.occurred_at.clone());
                }
                PunchEventType::ClockOut => {
                    if let Some(clock_in) = open_clock_in.take() {
                        let duration_minutes =
                            calculate_duration_minutes(&clock_in, &event.occurred_at);
                        pair_count += 1;

                        if duration_minutes < 0 {
                            has_inconsistency = true;
                            continue;
                        }

                        if duration_minutes > MAXIMUM_CONTINUOUS_WORK_MINUTES {
                            has_inconsistency = true;
                        }

                        if pair_count > 1 && duration_minutes < MINIMUM_REASONABLE_LOOP_MINUTES {
                            has_inconsistency = true;
                        }

                        work_minutes += duration_minutes;
                    } else {
                        has_inconsistency = true;
                    }
                }
                _ => {}
            }
        }

        if open_clock_in.is_some() {
            has_inconsistency = true;
        }

        Self {
            date,
            events,
            work_minutes,
            derived: DerivedAttendance::default(),
            has_inconsistency,
            status,
        }
    }

    pub fn from_events_with_policy_profile(
        date: Date,
        events: Vec<PunchEvent>,
        status: AttendanceDayStatus,
        policy_profile: PolicyProfile,
    ) -> Self {
        let mut day = Self::from_events(date, events, status, &NoRounding);
        day.derived = DerivedAttendance::for_day(policy_profile, &day);
        day
    }
}

fn calculate_duration_minutes(start: &Zoned, end: &Zoned) -> i64 {
    (end.timestamp().as_second() - start.timestamp().as_second()) / 60
}

fn regular_derived(day: &AttendanceDay) -> DerivedAttendance {
    let mut derived = DerivedAttendance {
        counted_work_minutes: day.work_minutes,
        work_days: i64::from(day.work_minutes > 0),
        reference_work_minutes: day.work_minutes,
        ..DerivedAttendance::default()
    };

    if matches!(day.date.weekday(), Weekday::Saturday | Weekday::Sunday) {
        return derived;
    }

    let Some(threshold) = tokyo_datetime_for_date(day.date, 19, 0) else {
        return derived;
    };
    derived.fixed_time_extra_minutes = valid_work_pairs(&day.events)
        .into_iter()
        .map(|(clock_in, clock_out)| minutes_after_threshold(&clock_in, &clock_out, &threshold))
        .sum();

    derived
}

fn part_time_derived(day: &AttendanceDay) -> DerivedAttendance {
    let counted_work_minutes: i64 = valid_work_pairs(&day.events)
        .into_iter()
        .map(|(clock_in, clock_out)| {
            let rounded_clock_in = round_clock_in_to_next_30_minutes(&clock_in);
            calculate_duration_minutes(&rounded_clock_in, &clock_out).max(0)
        })
        .sum();

    DerivedAttendance {
        counted_work_minutes,
        within_8h_work_minutes: counted_work_minutes.min(8 * 60),
        over_8h_work_minutes: (counted_work_minutes - 8 * 60).max(0),
        work_days: i64::from(counted_work_minutes > 0),
        reference_work_minutes: day.work_minutes,
        ..DerivedAttendance::default()
    }
}

fn doctor_derived(day: &AttendanceDay) -> DerivedAttendance {
    DerivedAttendance {
        work_days: i64::from(day.work_minutes > 0),
        reference_work_minutes: day.work_minutes,
        ..DerivedAttendance::default()
    }
}

fn valid_work_pairs(events: &[PunchEvent]) -> Vec<(Zoned, Zoned)> {
    let mut pairs = Vec::new();
    let mut open_clock_in: Option<Zoned> = None;

    for event in events {
        match event.event_type {
            PunchEventType::ClockIn => {
                open_clock_in = Some(event.occurred_at.clone());
            }
            PunchEventType::ClockOut => {
                if let Some(clock_in) = open_clock_in.take()
                    && calculate_duration_minutes(&clock_in, &event.occurred_at) >= 0
                {
                    pairs.push((clock_in, event.occurred_at.clone()));
                }
            }
            _ => {}
        }
    }

    pairs
}

fn minutes_after_threshold(clock_in: &Zoned, clock_out: &Zoned, threshold: &Zoned) -> i64 {
    let start_second = clock_in
        .timestamp()
        .as_second()
        .max(threshold.timestamp().as_second());
    let end_second = clock_out.timestamp().as_second();

    if end_second <= start_second {
        return 0;
    }

    (end_second - start_second) / 60
}

fn round_clock_in_to_next_30_minutes(at: &Zoned) -> Zoned {
    let datetime = at.datetime();
    let total_minutes = i64::from(datetime.hour()) * 60 + i64::from(datetime.minute());
    let rounded_total_minutes = ((total_minutes + 29) / 30) * 30;
    let mut rounded_date = at.date();
    let mut minute_of_day = rounded_total_minutes;

    if minute_of_day >= 24 * 60 {
        let Ok(next_date) = rounded_date.tomorrow() else {
            return at.clone();
        };
        rounded_date = next_date;
        minute_of_day -= 24 * 60;
    }

    let Ok(hour) = i8::try_from(minute_of_day / 60) else {
        return at.clone();
    };
    let Ok(minute) = i8::try_from(minute_of_day % 60) else {
        return at.clone();
    };

    tokyo_datetime_for_date(rounded_date, hour, minute).unwrap_or_else(|| at.clone())
}

fn tokyo_datetime_for_date(date: Date, hour: i8, minute: i8) -> Option<Zoned> {
    date.at(hour, minute, 0, 0).in_tz("Asia/Tokyo").ok()
}

#[cfg(test)]
mod tests {
    use super::{AttendanceDay, AttendanceDayStatus, PolicyProfile, PunchEvent};
    use crate::port::policy::{NoRounding, PunchEventType, RoundingPolicy};
    use jiff::{Zoned, civil::date};
    use proptest::prelude::*;
    use uuid::Uuid;

    #[test]
    // 出勤と退勤が 1 組なら勤務分数を集計し、不整合なしとして扱う。
    fn calculates_work_minutes_for_single_clock_in_out_pair() {
        let day = AttendanceDay::from_events(
            date(2026, 4, 16),
            vec![
                punch_event(PunchEventType::ClockIn, 2026, 4, 16, 9, 0),
                punch_event(PunchEventType::ClockOut, 2026, 4, 16, 18, 0),
            ],
            AttendanceDayStatus::Confirmed,
            &NoRounding,
        );

        assert_eq!(day.work_minutes, 540);
        assert!(!day.has_inconsistency);
    }

    #[test]
    // 打刻は時系列順に並べ替えてから集計する。
    fn sorts_events_before_aggregation() {
        let day = AttendanceDay::from_events(
            date(2026, 4, 16),
            vec![
                punch_event(PunchEventType::ClockOut, 2026, 4, 16, 18, 0),
                punch_event(PunchEventType::ClockIn, 2026, 4, 16, 9, 0),
            ],
            AttendanceDayStatus::Confirmed,
            &NoRounding,
        );

        assert_eq!(day.work_minutes, 540);
        assert_eq!(day.events[0].event_type, PunchEventType::ClockIn);
        assert_eq!(day.events[1].event_type, PunchEventType::ClockOut);
    }

    #[test]
    // 退勤だけで始まる日は出勤漏れ疑いとして不整合にする。
    fn marks_day_inconsistent_when_clock_out_has_no_open_clock_in() {
        let day = AttendanceDay::from_events(
            date(2026, 4, 16),
            vec![punch_event(PunchEventType::ClockOut, 2026, 4, 16, 18, 0)],
            AttendanceDayStatus::Unconfirmed,
            &NoRounding,
        );

        assert_eq!(day.work_minutes, 0);
        assert!(day.has_inconsistency);
    }

    #[test]
    // 出勤だけで終わる日は退勤漏れ疑いとして不整合にする。
    fn marks_day_inconsistent_when_clock_in_has_no_matching_clock_out() {
        let day = AttendanceDay::from_events(
            date(2026, 4, 16),
            vec![punch_event(PunchEventType::ClockIn, 2026, 4, 16, 9, 0)],
            AttendanceDayStatus::Unconfirmed,
            &NoRounding,
        );

        assert_eq!(day.work_minutes, 0);
        assert!(day.has_inconsistency);
    }

    #[test]
    // 同日に複数ペアがあり、そのうち短すぎる入退勤ループがある日は不整合にする。
    fn marks_day_inconsistent_for_short_loop_when_multiple_pairs_exist() {
        let day = AttendanceDay::from_events(
            date(2026, 4, 16),
            vec![
                punch_event(PunchEventType::ClockIn, 2026, 4, 16, 9, 0),
                punch_event(PunchEventType::ClockOut, 2026, 4, 16, 12, 0),
                punch_event(PunchEventType::ClockIn, 2026, 4, 16, 13, 0),
                punch_event(PunchEventType::ClockOut, 2026, 4, 16, 14, 0),
            ],
            AttendanceDayStatus::Confirmed,
            &NoRounding,
        );

        assert_eq!(day.work_minutes, 240);
        assert!(day.has_inconsistency);
    }

    #[test]
    // 休憩系の将来拡張打刻は勤務分数集計の対象外にする。
    fn ignores_non_clock_in_out_events_for_work_minutes() {
        let day = AttendanceDay::from_events(
            date(2026, 4, 16),
            vec![
                punch_event(PunchEventType::ClockIn, 2026, 4, 16, 9, 0),
                punch_event(PunchEventType::BreakStart, 2026, 4, 16, 12, 0),
                punch_event(PunchEventType::BreakEnd, 2026, 4, 16, 13, 0),
                punch_event(PunchEventType::ClockOut, 2026, 4, 16, 18, 0),
            ],
            AttendanceDayStatus::Confirmed,
            &NoRounding,
        );

        assert_eq!(day.work_minutes, 540);
        assert!(!day.has_inconsistency);
    }

    #[test]
    // raw 勤務分数は policy 丸めで変更されない。
    fn keeps_raw_work_minutes_when_rounding_policy_would_change_clock_in() {
        let day = AttendanceDay::from_events(
            date(2026, 4, 16),
            vec![
                punch_event(PunchEventType::ClockIn, 2026, 4, 16, 9, 1),
                punch_event(PunchEventType::ClockOut, 2026, 4, 16, 18, 0),
            ],
            AttendanceDayStatus::Confirmed,
            &CeilClockInToHalfHour,
        );

        assert_eq!(day.work_minutes, 539);
        assert_eq!(day.events[0].occurred_at, tokyo_datetime(2026, 4, 16, 9, 1));
    }

    #[test]
    // 雇用区分は MVP preset の既定 policy profile に対応する。
    fn maps_employment_type_to_default_policy_profile() {
        assert_eq!(
            PolicyProfile::from_employment_type("regular"),
            PolicyProfile::LegacyRegular2026
        );
        assert_eq!(
            PolicyProfile::from_employment_type("part_time"),
            PolicyProfile::LegacyPartTime2026
        );
        assert_eq!(
            PolicyProfile::from_employment_type("doctor"),
            PolicyProfile::LegacyDoctor2026
        );
        assert_eq!(
            PolicyProfile::from_employment_type("unknown"),
            PolicyProfile::LegacyRegular2026
        );
    }

    #[test]
    // 正社員 preset は平日 19:00 以降の勤務だけを fixed_time_extra に集計する。
    fn derives_regular_fixed_time_extra_after_19_on_weekdays() {
        let day = AttendanceDay::from_events_with_policy_profile(
            date(2026, 4, 20),
            vec![
                punch_event(PunchEventType::ClockIn, 2026, 4, 20, 18, 30),
                punch_event(PunchEventType::ClockOut, 2026, 4, 20, 20, 0),
            ],
            AttendanceDayStatus::Confirmed,
            PolicyProfile::LegacyRegular2026,
        );

        assert_eq!(day.work_minutes, 90);
        assert_eq!(day.derived.fixed_time_extra_minutes, 60);
    }

    #[test]
    // パート preset は出勤だけを 30 分切り上げ、8 時間以内と 8 時間超に分ける。
    fn derives_part_time_rounded_counted_minutes_and_over_8h() {
        let day = AttendanceDay::from_events_with_policy_profile(
            date(2026, 4, 16),
            vec![
                punch_event(PunchEventType::ClockIn, 2026, 4, 16, 8, 46),
                punch_event(PunchEventType::ClockOut, 2026, 4, 16, 18, 0),
            ],
            AttendanceDayStatus::Confirmed,
            PolicyProfile::LegacyPartTime2026,
        );

        assert_eq!(day.work_minutes, 554);
        assert_eq!(day.derived.counted_work_minutes, 540);
        assert_eq!(day.derived.within_8h_work_minutes, 480);
        assert_eq!(day.derived.over_8h_work_minutes, 60);
    }

    #[test]
    // ドクター preset は出勤日数を主集計し、勤務時間は参考値に留める。
    fn derives_doctor_work_days_and_reference_minutes() {
        let day = AttendanceDay::from_events_with_policy_profile(
            date(2026, 4, 16),
            vec![
                punch_event(PunchEventType::ClockIn, 2026, 4, 16, 9, 0),
                punch_event(PunchEventType::ClockOut, 2026, 4, 16, 18, 0),
            ],
            AttendanceDayStatus::Confirmed,
            PolicyProfile::LegacyDoctor2026,
        );

        assert_eq!(day.derived.work_days, 1);
        assert_eq!(day.derived.reference_work_minutes, 540);
        assert_eq!(day.derived.fixed_time_extra_minutes, 0);
    }

    proptest! {
        #[test]
        // 先頭が退勤の列は常に不整合になる。
        fn marks_sequences_starting_with_clock_out_as_inconsistent(
            hour in 0i8..=23i8,
            minute in 0i8..=59i8,
        ) {
            let day = AttendanceDay::from_events(
                date(2026, 4, 16),
                vec![punch_event(PunchEventType::ClockOut, 2026, 4, 16, hour, minute)],
                AttendanceDayStatus::Unconfirmed,
                &NoRounding,
            );

            prop_assert!(day.has_inconsistency);
        }

        #[test]
        // 出勤から始まる奇数個の交互列は最後の退勤漏れで不整合になる。
        fn marks_odd_alternating_sequences_as_inconsistent(
            pair_count in 1usize..=4usize,
        ) {
            let mut events = Vec::new();
            let mut total_minutes = 9_i64 * 60;
            let required_minutes = i64::try_from(pair_count).expect("pair_count should fit in i64") * 240 + 60;
            prop_assume!(total_minutes + required_minutes <= 23_i64 * 60 + 59);

            for _ in 0..pair_count {
                events.push(punch_event_from_total_minutes(
                    PunchEventType::ClockIn,
                    2026,
                    4,
                    16,
                    total_minutes,
                ));
                total_minutes += 180;
                events.push(punch_event_from_total_minutes(
                    PunchEventType::ClockOut,
                    2026,
                    4,
                    16,
                    total_minutes,
                ));
                total_minutes += 60;
            }
            events.push(punch_event_from_total_minutes(
                PunchEventType::ClockIn,
                2026,
                4,
                16,
                total_minutes,
            ));

            let day = AttendanceDay::from_events(
                date(2026, 4, 16),
                events,
                AttendanceDayStatus::Unconfirmed,
                &NoRounding,
            );

            prop_assert!(day.has_inconsistency);
        }

        #[test]
        // 正常な単一ペアの勤務分数は常に非負になる。
        fn keeps_work_minutes_non_negative_for_valid_single_pair(
            start_hour in 0i8..=20i8,
            start_minute in 0i8..=59i8,
            duration_minutes in 1i64..=180i64,
        ) {
            let start_total_minutes = i64::from(start_hour) * 60 + i64::from(start_minute);
            prop_assume!(start_total_minutes + duration_minutes <= (23 * 60 + 59) as i64);

            let end_total_minutes = start_total_minutes + duration_minutes;
            let end_hour = i8::try_from(end_total_minutes / 60).expect("valid end hour");
            let end_minute = i8::try_from(end_total_minutes % 60).expect("valid end minute");

            let day = AttendanceDay::from_events(
                date(2026, 4, 16),
                vec![
                    punch_event(PunchEventType::ClockIn, 2026, 4, 16, start_hour, start_minute),
                    punch_event(PunchEventType::ClockOut, 2026, 4, 16, end_hour, end_minute),
                ],
                AttendanceDayStatus::Confirmed,
                &NoRounding,
            );

            prop_assert!(day.work_minutes >= 0);
            prop_assert!(!day.has_inconsistency);
        }
    }

    fn punch_event(
        event_type: PunchEventType,
        year: i16,
        month: i8,
        day: i8,
        hour: i8,
        minute: i8,
    ) -> PunchEvent {
        PunchEvent {
            id: Uuid::now_v7(),
            employee_id: Uuid::now_v7(),
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

    fn punch_event_from_total_minutes(
        event_type: PunchEventType,
        year: i16,
        month: i8,
        day: i8,
        total_minutes: i64,
    ) -> PunchEvent {
        let hour = i8::try_from(total_minutes / 60).expect("valid hour");
        let minute = i8::try_from(total_minutes % 60).expect("valid minute");

        punch_event(event_type, year, month, day, hour, minute)
    }

    fn tokyo_datetime(year: i16, month: i8, day: i8, hour: i8, minute: i8) -> Zoned {
        date(year, month, day)
            .at(hour, minute, 0, 0)
            .in_tz("Asia/Tokyo")
            .expect("Asia/Tokyo datetime should be valid")
    }

    struct CeilClockInToHalfHour;

    impl RoundingPolicy for CeilClockInToHalfHour {
        fn round(&self, event_type: PunchEventType, at: &Zoned) -> Zoned {
            if event_type != PunchEventType::ClockIn {
                return at.clone();
            }

            let datetime = at.datetime();
            let total_minutes = i64::from(datetime.hour()) * 60 + i64::from(datetime.minute());
            let rounded_total_minutes = ((total_minutes + 29) / 30) * 30;
            let rounded_hour = i8::try_from(rounded_total_minutes / 60).expect("valid hour");
            let rounded_minute = i8::try_from(rounded_total_minutes % 60).expect("valid minute");

            tokyo_datetime(
                at.date().year(),
                at.date().month(),
                at.date().day(),
                rounded_hour,
                rounded_minute,
            )
        }
    }
}

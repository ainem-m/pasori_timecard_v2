use crate::port::policy::{PunchEventType, RoundingPolicy};
use jiff::{Zoned, civil::Date};

const MINIMUM_REASONABLE_LOOP_MINUTES: i64 = 180;
const MAXIMUM_CONTINUOUS_WORK_MINUTES: i64 = 24 * 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PunchEvent {
    pub event_type: PunchEventType,
    pub occurred_at: Zoned,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttendanceDayStatus {
    Unconfirmed,
    Confirmed,
    Locked,
}

#[derive(Debug, Clone)]
pub struct AttendanceDay {
    pub date: Date,
    pub events: Vec<PunchEvent>,
    pub work_minutes: i64,
    pub has_inconsistency: bool,
    pub status: AttendanceDayStatus,
}

impl AttendanceDay {
    pub fn from_events(
        date: Date,
        mut events: Vec<PunchEvent>,
        status: AttendanceDayStatus,
        rounding_policy: &dyn RoundingPolicy,
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

                    open_clock_in =
                        Some(rounding_policy.round(event.event_type, &event.occurred_at));
                }
                PunchEventType::ClockOut => {
                    let rounded_clock_out =
                        rounding_policy.round(event.event_type, &event.occurred_at);

                    if let Some(clock_in) = open_clock_in.take() {
                        let duration_minutes =
                            calculate_duration_minutes(&clock_in, &rounded_clock_out);
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
            has_inconsistency,
            status,
        }
    }
}

fn calculate_duration_minutes(start: &Zoned, end: &Zoned) -> i64 {
    (end.timestamp().as_second() - start.timestamp().as_second()) / 60
}

#[cfg(test)]
mod tests {
    use super::{AttendanceDay, AttendanceDayStatus, PunchEvent};
    use crate::port::policy::{NoRounding, PunchEventType};
    use jiff::{Zoned, civil::date};
    use proptest::prelude::*;

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
            event_type,
            occurred_at: tokyo_datetime(year, month, day, hour, minute),
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
}

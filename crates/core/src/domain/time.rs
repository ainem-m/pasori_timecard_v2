use crate::domain::punch::AttendanceDay;
use jiff::civil::Date;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct YearMonth {
    year: i16,
    month: i8,
}

impl YearMonth {
    pub fn new(year: i16, month: i8) -> Result<Self, TimeDomainError> {
        Date::new(year, month, 1).map_err(|_| TimeDomainError::InvalidYearMonth { year, month })?;

        Ok(Self { year, month })
    }

    pub fn year(self) -> i16 {
        self.year
    }

    pub fn month(self) -> i8 {
        self.month
    }

    pub fn attendance_period(
        self,
        cutoff_rule: CutoffRule,
    ) -> Result<AttendancePeriod, TimeDomainError> {
        let period_end = self.resolve_cutoff_date(cutoff_rule)?;
        let previous_month = self.previous_month()?;
        let previous_cutoff = previous_month.resolve_cutoff_date(cutoff_rule)?;
        let period_start = previous_cutoff.tomorrow().map_err(|source| {
            TimeDomainError::DateCalculationFailed {
                context: "failed to build period start".to_string(),
                details: source.to_string(),
            }
        })?;

        Ok(AttendancePeriod {
            year_month: self,
            cutoff_rule,
            period_start,
            period_end,
        })
    }

    fn previous_month(self) -> Result<Self, TimeDomainError> {
        if self.month == 1 {
            return Self::new(self.year - 1, 12);
        }

        Self::new(self.year, self.month - 1)
    }

    fn resolve_cutoff_date(self, cutoff_rule: CutoffRule) -> Result<Date, TimeDomainError> {
        match cutoff_rule {
            CutoffRule::DayOfMonth(day) => {
                Date::new(self.year, self.month, day.value()).map_err(|source| {
                    TimeDomainError::DateCalculationFailed {
                        context: "failed to build cutoff date".to_string(),
                        details: source.to_string(),
                    }
                })
            }
            CutoffRule::EndOfMonth => Ok(Date::new(self.year, self.month, 1)
                .map_err(|source| TimeDomainError::DateCalculationFailed {
                    context: "failed to build month start".to_string(),
                    details: source.to_string(),
                })?
                .last_of_month()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CutoffDay(i8);

impl CutoffDay {
    pub fn new(value: i8) -> Result<Self, TimeDomainError> {
        if !(1..=28).contains(&value) {
            return Err(TimeDomainError::InvalidCutoffDay { value });
        }

        Ok(Self(value))
    }

    pub fn value(self) -> i8 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CutoffRule {
    DayOfMonth(CutoffDay),
    EndOfMonth,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttendancePeriod {
    pub year_month: YearMonth,
    pub cutoff_rule: CutoffRule,
    pub period_start: Date,
    pub period_end: Date,
}

#[derive(Debug, Clone)]
pub struct MonthlyTimesheet {
    pub employee_id: Uuid,
    pub year_month: YearMonth,
    pub days: Vec<AttendanceDay>,
    pub total_work_minutes: i64,
    pub cutoff_rule: CutoffRule,
    pub period_start: Date,
    pub period_end: Date,
}

impl MonthlyTimesheet {
    pub fn from_days(
        employee_id: Uuid,
        year_month: YearMonth,
        cutoff_rule: CutoffRule,
        days: Vec<AttendanceDay>,
    ) -> Result<Self, TimeDomainError> {
        let period = year_month.attendance_period(cutoff_rule)?;

        if days
            .iter()
            .any(|day| day.date < period.period_start || day.date > period.period_end)
        {
            return Err(TimeDomainError::DayOutOfRange {
                year_month,
                cutoff_rule,
                period_start: period.period_start,
                period_end: period.period_end,
            });
        }

        let total_work_minutes = days.iter().map(|day| day.work_minutes).sum();

        Ok(Self {
            employee_id,
            year_month,
            days,
            total_work_minutes,
            cutoff_rule,
            period_start: period.period_start,
            period_end: period.period_end,
        })
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TimeDomainError {
    #[error("invalid year-month: {year:04}-{month:02}")]
    InvalidYearMonth { year: i16, month: i8 },
    #[error("invalid cutoff day: {value}")]
    InvalidCutoffDay { value: i8 },
    #[error(
        "day is outside attendance period: year_month={year_month:?}, cutoff_rule={cutoff_rule:?}, period_start={period_start}, period_end={period_end}"
    )]
    DayOutOfRange {
        year_month: YearMonth,
        cutoff_rule: CutoffRule,
        period_start: Date,
        period_end: Date,
    },
    #[error("{context}: {details}")]
    DateCalculationFailed { context: String, details: String },
}

#[cfg(test)]
mod tests {
    use super::{
        AttendanceDay, CutoffDay, CutoffRule, MonthlyTimesheet, TimeDomainError, YearMonth,
    };
    use crate::domain::punch::AttendanceDayStatus;
    use jiff::civil::{Date, date};
    use proptest::prelude::*;
    use uuid::Uuid;

    #[test]
    // 締め日が 15 日なら前月 16 日から当月 15 日までを期間として返す。
    fn returns_attendance_period_for_mid_month_cutoff() {
        let year_month = YearMonth::new(2026, 4).expect("valid year_month");
        let cutoff_rule = CutoffRule::DayOfMonth(CutoffDay::new(15).expect("valid cutoff day"));

        let period = year_month
            .attendance_period(cutoff_rule)
            .expect("attendance period should be calculated");

        assert_eq!(period.period_start, date(2026, 3, 16));
        assert_eq!(period.period_end, date(2026, 4, 15));
    }

    #[test]
    // 1 月の締め日は前年 12 月をまたいで期間開始日を計算する。
    fn returns_attendance_period_across_year_boundary() {
        let year_month = YearMonth::new(2026, 1).expect("valid year_month");
        let cutoff_rule = CutoffRule::DayOfMonth(CutoffDay::new(15).expect("valid cutoff day"));

        let period = year_month
            .attendance_period(cutoff_rule)
            .expect("attendance period should be calculated");

        assert_eq!(period.period_start, date(2025, 12, 16));
        assert_eq!(period.period_end, date(2026, 1, 15));
    }

    #[test]
    // 月末締めは当月初日から当月末日までを期間として返す。
    fn returns_attendance_period_for_end_of_month_cutoff() {
        let year_month = YearMonth::new(2026, 4).expect("valid year_month");

        let period = year_month
            .attendance_period(CutoffRule::EndOfMonth)
            .expect("attendance period should be calculated");

        assert_eq!(period.period_start, date(2026, 4, 1));
        assert_eq!(period.period_end, date(2026, 4, 30));
    }

    #[test]
    // うるう年の月末締めは 2 月 29 日までを期間として返す。
    fn returns_attendance_period_for_end_of_month_cutoff_in_leap_year() {
        let year_month = YearMonth::new(2024, 2).expect("valid year_month");

        let period = year_month
            .attendance_period(CutoffRule::EndOfMonth)
            .expect("attendance period should be calculated");

        assert_eq!(period.period_start, date(2024, 2, 1));
        assert_eq!(period.period_end, date(2024, 2, 29));
    }

    #[test]
    // 固定日締めは 28 日を超える値を受け付けない。
    fn rejects_cutoff_day_above_28() {
        let error = CutoffDay::new(29).expect_err("cutoff day should be rejected");

        assert_eq!(error, TimeDomainError::InvalidCutoffDay { value: 29 });
    }

    #[test]
    // 月は 1 から 12 の範囲外を受け付けない。
    fn rejects_invalid_month_for_year_month() {
        let error = YearMonth::new(2026, 13).expect_err("month should be rejected");

        assert_eq!(
            error,
            TimeDomainError::InvalidYearMonth {
                year: 2026,
                month: 13,
            }
        );
    }

    #[test]
    // 月次勤怠表は日次勤怠の勤務分数を合算して返す。
    fn sums_daily_work_minutes_into_monthly_timesheet() {
        let employee_id = Uuid::now_v7();
        let year_month = YearMonth::new(2026, 4).expect("valid year_month");
        let cutoff_rule = CutoffRule::DayOfMonth(CutoffDay::new(15).expect("valid cutoff day"));
        let days = vec![
            attendance_day(date(2026, 3, 16), 480),
            attendance_day(date(2026, 4, 2), 240),
        ];

        let timesheet = MonthlyTimesheet::from_days(employee_id, year_month, cutoff_rule, days)
            .expect("monthly timesheet should be calculated");

        assert_eq!(timesheet.total_work_minutes, 720);
        assert_eq!(timesheet.period_start, date(2026, 3, 16));
        assert_eq!(timesheet.period_end, date(2026, 4, 15));
    }

    #[test]
    // 集計対象外の日付が混ざる月次勤怠表は作成できない。
    fn rejects_day_outside_attendance_period() {
        let employee_id = Uuid::now_v7();
        let year_month = YearMonth::new(2026, 4).expect("valid year_month");
        let cutoff_rule = CutoffRule::DayOfMonth(CutoffDay::new(15).expect("valid cutoff day"));
        let days = vec![attendance_day(date(2026, 4, 16), 480)];

        let error = MonthlyTimesheet::from_days(employee_id, year_month, cutoff_rule, days)
            .expect_err("day outside period should be rejected");

        assert!(matches!(error, TimeDomainError::DayOutOfRange { .. }));
    }

    proptest! {
        #[test]
        // 任意の有効な年月と締め日に対し、期間終了日は必ず当月の締め日になる。
        fn period_end_always_matches_requested_year_month(
            year in 1900i16..=2100i16,
            month in 1i8..=12i8,
            cutoff in 1i8..=28i8,
        ) {
            let year_month = YearMonth::new(year, month).expect("valid year_month");
            let cutoff_rule = CutoffRule::DayOfMonth(CutoffDay::new(cutoff).expect("valid cutoff day"));

            let period = year_month
                .attendance_period(cutoff_rule)
                .expect("attendance period should be calculated");

            prop_assert_eq!(period.period_end, date(year, month, cutoff));
        }

        #[test]
        // 任意の有効な年月と締め日に対し、期間開始日は前月締め日の翌日になる。
        fn period_start_is_day_after_previous_month_cutoff(
            year in 1901i16..=2100i16,
            month in 1i8..=12i8,
            cutoff in 1i8..=28i8,
        ) {
            let year_month = YearMonth::new(year, month).expect("valid year_month");
            let cutoff_rule = CutoffRule::DayOfMonth(CutoffDay::new(cutoff).expect("valid cutoff day"));

            let period = year_month
                .attendance_period(cutoff_rule)
                .expect("attendance period should be calculated");

            let expected_previous_month = if month == 1 {
                date(year - 1, 12, cutoff)
            } else {
                date(year, month - 1, cutoff)
            };

            prop_assert_eq!(
                period.period_start,
                expected_previous_month.tomorrow().expect("tomorrow should exist"),
            );
        }

        #[test]
        // 任意の有効な年月と締め日に対し、期間日数は 28 日以上 31 日以下に収まる。
        fn period_length_stays_within_one_month_plus_or_minus_one_day(
            year in 1901i16..=2100i16,
            month in 1i8..=12i8,
            cutoff in 1i8..=28i8,
        ) {
            let year_month = YearMonth::new(year, month).expect("valid year_month");
            let cutoff_rule = CutoffRule::DayOfMonth(CutoffDay::new(cutoff).expect("valid cutoff day"));

            let period = year_month
                .attendance_period(cutoff_rule)
                .expect("attendance period should be calculated");
            let days = inclusive_day_count(period.period_start, period.period_end);

            prop_assert!((28..=31).contains(&days));
        }

        #[test]
        // 任意の有効な年月に対し、月末締めの期間終了日は必ず当月末日になる。
        fn period_end_matches_last_day_of_month_for_end_of_month_rule(
            year in 1900i16..=2100i16,
            month in 1i8..=12i8,
        ) {
            let year_month = YearMonth::new(year, month).expect("valid year_month");

            let period = year_month
                .attendance_period(CutoffRule::EndOfMonth)
                .expect("attendance period should be calculated");

            prop_assert_eq!(period.period_end, date(year, month, 1).last_of_month());
        }

        #[test]
        // 任意の有効な年月に対し、月末締めの期間開始日は必ず当月初日になる。
        fn period_start_matches_first_day_of_month_for_end_of_month_rule(
            year in 1900i16..=2100i16,
            month in 1i8..=12i8,
        ) {
            let year_month = YearMonth::new(year, month).expect("valid year_month");

            let period = year_month
                .attendance_period(CutoffRule::EndOfMonth)
                .expect("attendance period should be calculated");

            prop_assert_eq!(period.period_start, date(year, month, 1));
        }

        #[test]
        // 月次勤怠表の合計勤務分数は日次勤怠の合計になる。
        fn monthly_timesheet_total_matches_sum_of_days(
            year in 1900i16..=2100i16,
            month in 1i8..=12i8,
            day in 1i8..=28i8,
            work1 in 0i64..=600i64,
            work2 in 0i64..=600i64,
        ) {
            let year_month = YearMonth::new(year, month).expect("valid year_month");
            let cutoff_rule = CutoffRule::DayOfMonth(CutoffDay::new(day).expect("valid cutoff day"));
            let period = year_month.attendance_period(cutoff_rule).expect("period should exist");
            let employee_id = Uuid::now_v7();
            let days = vec![
                attendance_day(period.period_start, work1),
                attendance_day(period.period_start.tomorrow().expect("next day should exist"), work2),
            ];

            let timesheet = MonthlyTimesheet::from_days(employee_id, year_month, cutoff_rule, days)
                .expect("monthly timesheet should be calculated");

            prop_assert_eq!(timesheet.total_work_minutes, work1 + work2);
        }
    }

    fn inclusive_day_count(start: Date, end: Date) -> i32 {
        let mut days = 1;
        let mut current = start;

        while current != end {
            current = current
                .tomorrow()
                .expect("tomorrow should exist while counting days");
            days += 1;
        }

        days
    }

    fn attendance_day(date: Date, work_minutes: i64) -> AttendanceDay {
        AttendanceDay {
            date,
            events: vec![],
            work_minutes,
            has_inconsistency: false,
            status: AttendanceDayStatus::Confirmed,
        }
    }
}

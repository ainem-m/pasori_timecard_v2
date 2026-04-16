use jiff::Zoned;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PunchEventType {
    ClockIn,
    ClockOut,
    BreakStart,
    BreakEnd,
    TemporaryOut,
    TemporaryReturn,
    ManualCorrection,
}

#[derive(Debug, Clone)]
pub struct PunchEventRef {
    pub event_type: PunchEventType,
    pub occurred_at: Zoned,
}

pub trait PunchPolicy: Send + Sync {
    /// 直近の打刻履歴 (降順) から、次の打刻種別を推定する
    fn decide(&self, recent: &[PunchEventRef], now: &Zoned) -> PunchEventType;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultPunchPolicy;

impl PunchPolicy for DefaultPunchPolicy {
    fn decide(&self, recent: &[PunchEventRef], now: &Zoned) -> PunchEventType {
        let today = now.date();

        match recent.first() {
            None => PunchEventType::ClockIn,
            Some(last) if last.occurred_at.date() < today => PunchEventType::ClockIn,
            Some(last) if last.event_type == PunchEventType::ClockIn => PunchEventType::ClockOut,
            Some(_) => PunchEventType::ClockIn,
        }
    }
}

pub trait RoundingPolicy: Send + Sync {
    /// 集計時に適用する時刻丸め。MVP の既定は NoRounding (素通し)。
    fn round(&self, event_type: PunchEventType, at: &Zoned) -> Zoned;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NoRounding;

impl RoundingPolicy for NoRounding {
    fn round(&self, _event_type: PunchEventType, at: &Zoned) -> Zoned {
        at.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DefaultPunchPolicy, NoRounding, PunchEventRef, PunchEventType, PunchPolicy, RoundingPolicy,
    };
    use jiff::{Zoned, civil::date};
    use proptest::prelude::*;

    #[test]
    // 直近打刻がなければ出勤と推定する。
    fn decides_clock_in_when_no_recent_events_exist() {
        let policy = DefaultPunchPolicy;
        let now = tokyo_datetime(2026, 4, 16, 9, 0);

        let decided = policy.decide(&[], &now);

        assert_eq!(decided, PunchEventType::ClockIn);
    }

    #[test]
    // 最終打刻が前日なら次は出勤と推定する。
    fn decides_clock_in_when_last_event_was_on_previous_day() {
        let policy = DefaultPunchPolicy;
        let now = tokyo_datetime(2026, 4, 16, 9, 0);
        let recent = vec![PunchEventRef {
            event_type: PunchEventType::ClockOut,
            occurred_at: tokyo_datetime(2026, 4, 15, 18, 0),
        }];

        let decided = policy.decide(&recent, &now);

        assert_eq!(decided, PunchEventType::ClockIn);
    }

    #[test]
    // 同日の最終打刻が出勤なら次は退勤と推定する。
    fn decides_clock_out_when_last_same_day_event_was_clock_in() {
        let policy = DefaultPunchPolicy;
        let now = tokyo_datetime(2026, 4, 16, 18, 0);
        let recent = vec![PunchEventRef {
            event_type: PunchEventType::ClockIn,
            occurred_at: tokyo_datetime(2026, 4, 16, 9, 0),
        }];

        let decided = policy.decide(&recent, &now);

        assert_eq!(decided, PunchEventType::ClockOut);
    }

    #[test]
    // 同日の最終打刻が退勤なら次は出勤と推定する。
    fn decides_clock_in_when_last_same_day_event_was_clock_out() {
        let policy = DefaultPunchPolicy;
        let now = tokyo_datetime(2026, 4, 16, 19, 0);
        let recent = vec![PunchEventRef {
            event_type: PunchEventType::ClockOut,
            occurred_at: tokyo_datetime(2026, 4, 16, 18, 0),
        }];

        let decided = policy.decide(&recent, &now);

        assert_eq!(decided, PunchEventType::ClockIn);
    }

    #[test]
    // 丸めなしポリシーは打刻時刻を変更しない。
    fn keeps_original_time_for_no_rounding_policy() {
        let policy = NoRounding;
        let at = tokyo_datetime(2026, 4, 16, 9, 30);

        let rounded = policy.round(PunchEventType::ClockIn, &at);

        assert_eq!(rounded, at);
    }

    proptest! {
        #[test]
        // 推定結果は常に出勤または退勤になる。
        fn decides_only_clock_in_or_clock_out(
            event_type in any::<ArbitraryPunchEventType>(),
            now_day in 1u8..=28,
            last_day in 1u8..=28,
            hour in 0u8..=23,
            minute in 0u8..=59,
        ) {
            let policy = DefaultPunchPolicy;
            let now = tokyo_datetime(2026, 4, i8::try_from(now_day).expect("valid day"), 12, 0);
            let recent = vec![PunchEventRef {
                event_type: event_type.into(),
                occurred_at: tokyo_datetime(
                    2026,
                    4,
                    i8::try_from(last_day).expect("valid day"),
                    i8::try_from(hour).expect("valid hour"),
                    i8::try_from(minute).expect("valid minute"),
                ),
            }];

            let decided = policy.decide(&recent, &now);

            prop_assert!(matches!(decided, PunchEventType::ClockIn | PunchEventType::ClockOut));
        }

        #[test]
        // 直近打刻が空なら常に出勤と推定する。
        fn always_decides_clock_in_when_recent_events_are_empty(
            month in 1u8..=12,
            day in 1u8..=28,
            hour in 0u8..=23,
            minute in 0u8..=59,
        ) {
            let policy = DefaultPunchPolicy;
            let now = tokyo_datetime(
                2026,
                i8::try_from(month).expect("valid month"),
                i8::try_from(day).expect("valid day"),
                i8::try_from(hour).expect("valid hour"),
                i8::try_from(minute).expect("valid minute"),
            );

            let decided = policy.decide(&[], &now);

            prop_assert_eq!(decided, PunchEventType::ClockIn);
        }

        #[test]
        // 最終打刻が前日なら常に出勤と推定する。
        fn always_decides_clock_in_when_last_event_is_previous_day(
            month in 1u8..=12,
            day in 2u8..=28,
            event_type in any::<ArbitraryPunchEventType>(),
        ) {
            let policy = DefaultPunchPolicy;
            let now = tokyo_datetime(
                2026,
                i8::try_from(month).expect("valid month"),
                i8::try_from(day).expect("valid day"),
                9,
                0,
            );
            let recent = vec![PunchEventRef {
                event_type: event_type.into(),
                occurred_at: tokyo_datetime(
                    2026,
                    i8::try_from(month).expect("valid month"),
                    i8::try_from(day - 1).expect("valid prior day"),
                    18,
                    0,
                ),
            }];

            let decided = policy.decide(&recent, &now);

            prop_assert_eq!(decided, PunchEventType::ClockIn);
        }

        #[test]
        // 同日かつ最終打刻が出勤なら常に退勤と推定する。
        fn always_decides_clock_out_when_last_same_day_event_is_clock_in(
            month in 1u8..=12,
            day in 1u8..=28,
            hour in 0u8..=23,
            minute in 0u8..=59,
        ) {
            let policy = DefaultPunchPolicy;
            let now = tokyo_datetime(
                2026,
                i8::try_from(month).expect("valid month"),
                i8::try_from(day).expect("valid day"),
                23,
                59,
            );
            let recent = vec![PunchEventRef {
                event_type: PunchEventType::ClockIn,
                occurred_at: tokyo_datetime(
                    2026,
                    i8::try_from(month).expect("valid month"),
                    i8::try_from(day).expect("valid day"),
                    i8::try_from(hour).expect("valid hour"),
                    i8::try_from(minute).expect("valid minute"),
                ),
            }];

            let decided = policy.decide(&recent, &now);

            prop_assert_eq!(decided, PunchEventType::ClockOut);
        }

        #[test]
        // 同日かつ最終打刻が退勤なら常に出勤と推定する。
        fn always_decides_clock_in_when_last_same_day_event_is_not_clock_in(
            month in 1u8..=12,
            day in 1u8..=28,
            event_type in prop_oneof![
                Just(PunchEventType::ClockOut),
                Just(PunchEventType::BreakStart),
                Just(PunchEventType::BreakEnd),
                Just(PunchEventType::TemporaryOut),
                Just(PunchEventType::TemporaryReturn),
                Just(PunchEventType::ManualCorrection),
            ],
        ) {
            let policy = DefaultPunchPolicy;
            let now = tokyo_datetime(
                2026,
                i8::try_from(month).expect("valid month"),
                i8::try_from(day).expect("valid day"),
                12,
                0,
            );
            let recent = vec![PunchEventRef {
                event_type,
                occurred_at: tokyo_datetime(
                    2026,
                    i8::try_from(month).expect("valid month"),
                    i8::try_from(day).expect("valid day"),
                    9,
                    0,
                ),
            }];

            let decided = policy.decide(&recent, &now);

            prop_assert_eq!(decided, PunchEventType::ClockIn);
        }
    }

    #[derive(Debug, Clone, Copy)]
    enum ArbitraryPunchEventType {
        ClockIn,
        ClockOut,
        BreakStart,
        BreakEnd,
        TemporaryOut,
        TemporaryReturn,
        ManualCorrection,
    }

    impl Arbitrary for ArbitraryPunchEventType {
        type Parameters = ();
        type Strategy = BoxedStrategy<Self>;

        fn arbitrary_with(_args: Self::Parameters) -> Self::Strategy {
            prop_oneof![
                Just(Self::ClockIn),
                Just(Self::ClockOut),
                Just(Self::BreakStart),
                Just(Self::BreakEnd),
                Just(Self::TemporaryOut),
                Just(Self::TemporaryReturn),
                Just(Self::ManualCorrection),
            ]
            .boxed()
        }
    }

    impl From<ArbitraryPunchEventType> for PunchEventType {
        fn from(value: ArbitraryPunchEventType) -> Self {
            match value {
                ArbitraryPunchEventType::ClockIn => PunchEventType::ClockIn,
                ArbitraryPunchEventType::ClockOut => PunchEventType::ClockOut,
                ArbitraryPunchEventType::BreakStart => PunchEventType::BreakStart,
                ArbitraryPunchEventType::BreakEnd => PunchEventType::BreakEnd,
                ArbitraryPunchEventType::TemporaryOut => PunchEventType::TemporaryOut,
                ArbitraryPunchEventType::TemporaryReturn => PunchEventType::TemporaryReturn,
                ArbitraryPunchEventType::ManualCorrection => PunchEventType::ManualCorrection,
            }
        }
    }

    fn tokyo_datetime(year: i16, month: i8, day: i8, hour: i8, minute: i8) -> Zoned {
        date(year, month, day)
            .at(hour, minute, 0, 0)
            .in_tz("Asia/Tokyo")
            .expect("Asia/Tokyo datetime should be valid")
    }
}

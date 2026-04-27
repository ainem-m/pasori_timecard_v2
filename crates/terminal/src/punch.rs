use crate::api_client::SubmitPunchRequest;
use pasori_core::port::policy::PunchEventType;
use uuid::Uuid;

// 打刻時刻を1分単位に丸める。秒以下を切り捨てる。
// AGENTS.md の「打刻時刻の保存粒度は1分単位」に基づく。
// RoundingPolicy は集計時の丸めを定義するものであり、ここでは保存粒度としての1分切り捨てを行う。
fn truncate_to_minute(zoned: &jiff::Zoned) -> jiff::Zoned {
    let dt = zoned.datetime();
    let truncated = jiff::civil::DateTime::new(
        dt.year(),
        dt.month(),
        dt.day(),
        dt.hour(),
        dt.minute(),
        0,
        0,
    )
    .expect("truncation to minute should always produce a valid datetime");

    truncated
        .in_tz("Asia/Tokyo")
        .expect("Asia/Tokyo should always be valid")
}

/// カードスキャンから打刻要求を生成する。
/// punch_id は UUID v7、occurred_at は Asia/Tokyo aware な1分切り捨て時刻、
/// source は "nfc" (オンライン) を使用する。
pub fn create_punch_request(card_id: String, event_type: PunchEventType) -> SubmitPunchRequest {
    let punch_id = Uuid::now_v7();
    let now = jiff::Zoned::now();
    let occurred_at = truncate_to_minute(&now);

    SubmitPunchRequest {
        punch_id,
        card_id,
        event_type,
        occurred_at,
        source: "nfc".to_string(),
    }
}

/// オフライン再送用の打刻要求を生成する。
/// source は "local_cached" を使用する。
pub fn create_offline_punch_request(
    punch_id: Uuid,
    card_id: String,
    event_type: PunchEventType,
    occurred_at: jiff::Zoned,
) -> SubmitPunchRequest {
    SubmitPunchRequest {
        punch_id,
        card_id,
        event_type,
        occurred_at,
        source: "local_cached".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    // 生成される punch_id は UUID v7 である。
    fn creates_punch_id_as_uuid_v7() {
        let req = create_punch_request("0123456789ABCDEF".to_string(), PunchEventType::ClockIn);

        let uuid_version = req.punch_id.get_version();
        assert_eq!(
            uuid_version,
            Some(uuid::Version::SortRand),
            "punch_id should be UUID v7, got version {:?}",
            uuid_version
        );
    }

    #[test]
    // occurred_at は Asia/Tokyo タイムゾーンを持つ。
    fn creates_occurred_at_in_asia_tokyo_timezone() {
        let req = create_punch_request("0123456789ABCDEF".to_string(), PunchEventType::ClockOut);

        assert_eq!(
            req.occurred_at.time_zone().iana_name(),
            Some("Asia/Tokyo"),
            "occurred_at should be in Asia/Tokyo timezone"
        );
    }

    #[test]
    // オンライン打刻の source は "nfc" である。
    fn creates_source_as_nfc_for_online_punch() {
        let req = create_punch_request("0123456789ABCDEF".to_string(), PunchEventType::ClockIn);

        assert_eq!(req.source, "nfc");
    }

    #[test]
    // 丸め後の時刻は秒・ナノ秒がゼロである。
    fn truncates_seconds_and_nanoseconds_to_zero() {
        let dt = jiff::civil::DateTime::new(2026, 4, 25, 14, 37, 45, 123_456_789)
            .expect("valid datetime");
        let zoned = dt.in_tz("Asia/Tokyo").expect("Asia/Tokyo");

        let truncated = truncate_to_minute(&zoned);

        assert_eq!(truncated.second(), 0);
        assert_eq!(truncated.subsec_nanosecond(), 0);
        assert_eq!(truncated.hour(), 14);
        assert_eq!(truncated.minute(), 37);
    }

    #[test]
    // occurred_at は秒・ナノ秒がゼロに切り捨てられている。
    fn creates_occurred_at_with_seconds_truncated() {
        let req = create_punch_request("0123456789ABCDEF".to_string(), PunchEventType::ClockIn);

        assert_eq!(req.occurred_at.second(), 0, "seconds should be truncated");
        assert_eq!(
            req.occurred_at.subsec_nanosecond(),
            0,
            "nanoseconds should be truncated"
        );
    }

    #[test]
    // オフライン再送要求の source は "local_cached" である。
    fn creates_source_as_local_cached_for_offline_punch() {
        let occurred_at = jiff::civil::datetime(2026, 4, 25, 9, 0, 0, 0)
            .in_tz("Asia/Tokyo")
            .expect("Asia/Tokyo");

        let req = create_offline_punch_request(
            Uuid::now_v7(),
            "0123456789ABCDEF".to_string(),
            PunchEventType::ClockIn,
            occurred_at,
        );

        assert_eq!(req.source, "local_cached");
    }
}

use anyhow::Result;
use uuid::Uuid;

use crate::offline::{CachedCard, OfflineRepository};

pub async fn cache_bound_card(
    offline_repo: &OfflineRepository,
    card_id: &str,
    employee_id: Uuid,
    employee_name: &str,
) -> Result<()> {
    offline_repo
        .cache_card(&CachedCard {
            card_id: card_id.to_string(),
            employee_id,
            employee_name: employee_name.to_string(),
            suggested_type: "ClockIn".to_string(),
            recent_events_json: "[]".to_string(),
            cached_at: jiff::Zoned::now(),
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    // 紐付け成功後は local card cache に従業員名つきで保存される。
    async fn caches_bound_card_after_successful_binding() {
        let repo = OfflineRepository::new("sqlite::memory:")
            .await
            .expect("repo");
        let employee_id = Uuid::now_v7();

        cache_bound_card(&repo, "0123456789ABCDEF", employee_id, "山田太郎")
            .await
            .expect("cache");

        let cached = repo
            .find_cached_card("0123456789ABCDEF")
            .await
            .expect("find")
            .expect("cached card");
        assert_eq!(cached.employee_id, employee_id);
        assert_eq!(cached.employee_name, "山田太郎");
        assert_eq!(cached.suggested_type, "ClockIn");
        assert_eq!(cached.recent_events_json, "[]");
    }
}

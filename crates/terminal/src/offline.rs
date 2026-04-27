use anyhow::Result;
use jiff::Zoned;
use sqlx::{Pool, Sqlite, sqlite::SqliteConnectOptions};
use std::str::FromStr;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct LocalPunch {
    pub id: Uuid,
    pub card_id: String,
    pub event_type: String,
    pub occurred_at: Zoned,
    #[allow(dead_code)]
    pub source: String,
    pub _synced_at: Option<Zoned>,
}

#[derive(Debug, Clone)]
pub struct CachedCard {
    pub card_id: String,
    pub employee_id: Uuid,
    pub employee_name: String,
    pub suggested_type: String,
    pub recent_events_json: String,
    pub cached_at: Zoned,
}

pub struct OfflineRepository {
    pool: Pool<Sqlite>,
}

impl OfflineRepository {
    pub async fn new(db_path: &str) -> Result<Self> {
        let options = SqliteConnectOptions::from_str(db_path)?
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal);

        let pool = Pool::<Sqlite>::connect_with(options).await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS local_punches (
                id TEXT PRIMARY KEY,
                card_id TEXT NOT NULL,
                event_type TEXT NOT NULL,
                occurred_at TEXT NOT NULL,
                source TEXT NOT NULL,
                synced_at TEXT
            )",
        )
        .execute(&pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS local_card_cache (
                card_id TEXT PRIMARY KEY,
                employee_id TEXT NOT NULL,
                employee_name TEXT NOT NULL,
                suggested_type TEXT NOT NULL,
                recent_events_json TEXT NOT NULL DEFAULT '[]',
                cached_at TEXT NOT NULL
            )",
        )
        .execute(&pool)
        .await?;

        Ok(Self { pool })
    }

    pub async fn save_punch(
        &self,
        id: Uuid,
        card_id: &str,
        event_type: &str,
        occurred_at: &Zoned,
        source: &str,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO local_punches (id, card_id, event_type, occurred_at, source) VALUES (?, ?, ?, ?, ?)"
        )
        .bind(id.to_string())
        .bind(card_id)
        .bind(event_type)
        .bind(occurred_at.to_string())
        .bind(source)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn mark_as_synced(&self, id: Uuid, synced_at: &Zoned) -> Result<()> {
        sqlx::query("UPDATE local_punches SET synced_at = ? WHERE id = ?")
            .bind(synced_at.to_string())
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    pub async fn get_unsynced_punches(&self) -> Result<Vec<LocalPunch>> {
        let rows = sqlx::query_as::<_, (String, String, String, String, String, Option<String>)>(
            "SELECT id, card_id, event_type, occurred_at, source, synced_at FROM local_punches WHERE synced_at IS NULL ORDER BY occurred_at ASC"
        )
        .fetch_all(&self.pool)
        .await?;

        let mut punches = Vec::new();
        for row in rows {
            punches.push(LocalPunch {
                id: Uuid::parse_str(&row.0)?,
                card_id: row.1,
                event_type: row.2,
                occurred_at: Zoned::from_str(&row.3)?,
                source: row.4,
                _synced_at: row.5.map(|s| Zoned::from_str(&s)).transpose()?,
            });
        }

        Ok(punches)
    }

    pub async fn cache_card(&self, card: &CachedCard) -> Result<()> {
        sqlx::query(
            "INSERT OR REPLACE INTO local_card_cache (card_id, employee_id, employee_name, suggested_type, recent_events_json, cached_at)
             VALUES (?, ?, ?, ?, ?, ?)"
        )
        .bind(&card.card_id)
        .bind(card.employee_id.to_string())
        .bind(&card.employee_name)
        .bind(&card.suggested_type)
        .bind(&card.recent_events_json)
        .bind(card.cached_at.to_string())
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn find_cached_card(&self, card_id: &str) -> Result<Option<CachedCard>> {
        let row = sqlx::query_as::<_, (String, String, String, String, String, String)>(
            "SELECT card_id, employee_id, employee_name, suggested_type, recent_events_json, cached_at FROM local_card_cache WHERE card_id = ?"
        )
        .bind(card_id)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(r) => Ok(Some(CachedCard {
                card_id: r.0,
                employee_id: Uuid::parse_str(&r.1)?,
                employee_name: r.2,
                suggested_type: r.3,
                recent_events_json: r.4,
                cached_at: Zoned::from_str(&r.5)?,
            })),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    // オフライン時に local cache から登録済みカードを解決できる。
    async fn finds_cached_card_by_card_id() {
        let repo = OfflineRepository::new("sqlite::memory:")
            .await
            .expect("repo");

        let cached = CachedCard {
            card_id: "0123456789ABCDEF".to_string(),
            employee_id: Uuid::now_v7(),
            employee_name: "テスト太郎".to_string(),
            suggested_type: "ClockIn".to_string(),
            recent_events_json: "[]".to_string(),
            cached_at: jiff::Zoned::now(),
        };
        repo.cache_card(&cached).await.expect("cache_card");

        let found = repo
            .find_cached_card("0123456789ABCDEF")
            .await
            .expect("find");
        assert!(found.is_some());
        let found = found.expect("cached card");
        assert_eq!(found.card_id, "0123456789ABCDEF");
        assert_eq!(found.employee_name, "テスト太郎");
        assert_eq!(found.suggested_type, "ClockIn");
    }

    #[tokio::test]
    // local cache にないカードは None を返す。
    async fn returns_none_for_unknown_card() {
        let repo = OfflineRepository::new("sqlite::memory:")
            .await
            .expect("repo");

        let found = repo.find_cached_card("UNKNOWN").await.expect("find");
        assert!(found.is_none());
    }

    #[tokio::test]
    // 同一カードの再キャッシュは上書きされる。
    async fn overwrites_existing_cache_on_same_card() {
        let repo = OfflineRepository::new("sqlite::memory:")
            .await
            .expect("repo");

        let cached_v1 = CachedCard {
            card_id: "0123456789ABCDEF".to_string(),
            employee_id: Uuid::now_v7(),
            employee_name: "テスト太郎".to_string(),
            suggested_type: "ClockIn".to_string(),
            recent_events_json: "[]".to_string(),
            cached_at: jiff::Zoned::now(),
        };
        repo.cache_card(&cached_v1).await.expect("cache_card v1");

        let cached_v2 = CachedCard {
            card_id: "0123456789ABCDEF".to_string(),
            employee_id: cached_v1.employee_id,
            employee_name: "テスト太郎".to_string(),
            suggested_type: "ClockOut".to_string(),
            recent_events_json: "[]".to_string(),
            cached_at: jiff::Zoned::now(),
        };
        repo.cache_card(&cached_v2).await.expect("cache_card v2");

        let found = repo
            .find_cached_card("0123456789ABCDEF")
            .await
            .expect("find")
            .expect("card");
        assert_eq!(found.suggested_type, "ClockOut");
    }

    #[tokio::test]
    // pending punch は古い順に返される。
    async fn returns_unsynced_punches_in_chronological_order() {
        let repo = OfflineRepository::new("sqlite::memory:")
            .await
            .expect("repo");

        let now = jiff::Zoned::now();
        let earlier = now
            .checked_sub(jiff::Span::new().minutes(5))
            .expect("5 min ago");
        let later = now
            .checked_sub(jiff::Span::new().minutes(1))
            .expect("1 min ago");

        repo.save_punch(Uuid::now_v7(), "CARD1", "ClockIn", &later, "local_cached")
            .await
            .expect("save punch 1");
        repo.save_punch(
            Uuid::now_v7(),
            "CARD2",
            "ClockOut",
            &earlier,
            "local_cached",
        )
        .await
        .expect("save punch 2");

        let punches = repo.get_unsynced_punches().await.expect("get unsynced");
        assert_eq!(punches.len(), 2);
        assert_eq!(punches[0].event_type, "ClockOut");
        assert_eq!(punches[1].event_type, "ClockIn");
    }

    #[tokio::test]
    // mark_as_synced 後は get_unsynced_punches に含まれない。
    async fn synced_punches_are_excluded_from_unsynced() {
        let repo = OfflineRepository::new("sqlite::memory:")
            .await
            .expect("repo");

        let id = Uuid::now_v7();
        repo.save_punch(id, "CARD1", "ClockIn", &jiff::Zoned::now(), "local_cached")
            .await
            .expect("save");

        repo.mark_as_synced(id, &jiff::Zoned::now())
            .await
            .expect("mark synced");

        let punches = repo.get_unsynced_punches().await.expect("get unsynced");
        assert!(punches.is_empty());
    }
}

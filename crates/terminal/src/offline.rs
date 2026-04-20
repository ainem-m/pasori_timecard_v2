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
    pub source: String,
    pub _synced_at: Option<Zoned>,
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

        // Ensure table exists
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
            "SELECT id, card_id, event_type, occurred_at, source, synced_at FROM local_punches WHERE synced_at IS NULL"
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
}

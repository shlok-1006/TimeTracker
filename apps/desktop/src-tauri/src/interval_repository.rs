//! Repository for local interval segments + the sync queue.
//!
//! Segments are append-only and immutable (Rule 2), tagged with a `kind`
//! (active | idle | meeting | break). Worked time = active + meeting. Sync
//! state lives in a separate `interval_sync` table (Rule 4). Timestamps are
//! RFC3339 UTC text (Rule 3).

use anyhow::Context;
use chrono::{DateTime, Utc};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Interval {
    pub id: Uuid,
    pub user_id: Uuid,
    pub start_utc: DateTime<Utc>,
    pub end_utc: DateTime<Utc>,
    /// active | idle | meeting | break
    pub kind: String,
}

impl Interval {
    /// Worked duration in seconds (active + meeting count; idle/break = 0).
    pub fn worked_seconds(&self) -> i64 {
        if self.kind == "active" || self.kind == "meeting" {
            (self.end_utc - self.start_utc).num_seconds().max(0)
        } else {
            0
        }
    }
}

fn parse_ts(s: &str) -> anyhow::Result<DateTime<Utc>> {
    Ok(DateTime::parse_from_rfc3339(s)
        .with_context(|| format!("invalid timestamp in db: {s}"))?
        .with_timezone(&Utc))
}

fn row_to_interval(row: &sqlx::sqlite::SqliteRow) -> anyhow::Result<Interval> {
    let id: String = row.try_get("id")?;
    let user_id: String = row.try_get("user_id")?;
    let start_utc: String = row.try_get("start_utc")?;
    let end_utc: String = row.try_get("end_utc")?;
    let kind: String = row.try_get("kind")?;
    Ok(Interval {
        id: Uuid::parse_str(&id).context("invalid interval id")?,
        user_id: Uuid::parse_str(&user_id).context("invalid user id")?,
        start_utc: parse_ts(&start_utc)?,
        end_utc: parse_ts(&end_utc)?,
        kind,
    })
}

/// Persist a finished segment. Insert-only (Rule 2).
pub async fn insert(pool: &SqlitePool, interval: &Interval) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO intervals (id, user_id, start_utc, end_utc, idle, kind) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(interval.id.to_string())
    .bind(interval.user_id.to_string())
    .bind(interval.start_utc.to_rfc3339())
    .bind(interval.end_utc.to_rfc3339())
    .bind((interval.kind == "idle") as i64)
    .bind(&interval.kind)
    .execute(pool)
    .await
    .context("failed to insert interval")?;
    Ok(())
}

/// Segments not yet acknowledged by the server (the sync queue).
pub async fn pending_sync(pool: &SqlitePool) -> anyhow::Result<Vec<Interval>> {
    let rows = sqlx::query(
        r#"
        SELECT i.id, i.user_id, i.start_utc, i.end_utc, i.kind
        FROM intervals i
        LEFT JOIN interval_sync s ON s.interval_id = i.id
        WHERE s.interval_id IS NULL
        ORDER BY i.start_utc
        "#,
    )
    .fetch_all(pool)
    .await
    .context("failed to load pending intervals")?;
    rows.iter().map(row_to_interval).collect()
}

/// Mark intervals as synced (idempotent).
pub async fn mark_synced(pool: &SqlitePool, ids: &[Uuid]) -> anyhow::Result<()> {
    let now = Utc::now().to_rfc3339();
    let mut tx = pool.begin().await?;
    for id in ids {
        sqlx::query("INSERT OR IGNORE INTO interval_sync (interval_id, synced_at) VALUES (?, ?)")
            .bind(id.to_string())
            .bind(&now)
            .execute(&mut *tx)
            .await
            .context("failed to mark interval synced")?;
    }
    tx.commit().await?;
    Ok(())
}

/// Load all intervals for a user.
pub async fn for_user(pool: &SqlitePool, user_id: Uuid) -> anyhow::Result<Vec<Interval>> {
    let rows = sqlx::query(
        "SELECT id, user_id, start_utc, end_utc, kind FROM intervals WHERE user_id = ? ORDER BY start_utc",
    )
    .bind(user_id.to_string())
    .fetch_all(pool)
    .await
    .context("failed to load intervals")?;
    rows.iter().map(row_to_interval).collect()
}

/// Total worked seconds for a user — computed from intervals (Rule 2).
pub async fn total_worked_seconds(pool: &SqlitePool, user_id: Uuid) -> anyhow::Result<i64> {
    Ok(sum_worked(&for_user(pool, user_id).await?))
}

/// Pure helper: sum worked seconds across intervals (active + meeting).
pub fn sum_worked(intervals: &[Interval]) -> i64 {
    intervals.iter().map(Interval::worked_seconds).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    fn interval(user: Uuid, start: &str, end: &str, kind: &str) -> Interval {
        Interval {
            id: Uuid::new_v4(),
            user_id: user,
            start_utc: DateTime::parse_from_rfc3339(start).unwrap().with_timezone(&Utc),
            end_utc: DateTime::parse_from_rfc3339(end).unwrap().with_timezone(&Utc),
            kind: kind.to_string(),
        }
    }

    #[test]
    fn worked_counts_active_and_meeting_only() {
        let u = Uuid::new_v4();
        let items = vec![
            interval(u, "2026-01-01T00:00:00Z", "2026-01-01T01:00:00Z", "active"), // 3600
            interval(u, "2026-01-01T01:00:00Z", "2026-01-01T01:30:00Z", "meeting"), // 1800
            interval(u, "2026-01-01T02:00:00Z", "2026-01-01T02:15:00Z", "idle"),   // 0
            interval(u, "2026-01-01T03:00:00Z", "2026-01-01T03:10:00Z", "break"),  // 0
        ];
        assert_eq!(sum_worked(&items), 5400);
    }

    #[tokio::test]
    async fn insert_and_round_trip_with_kind() {
        let pool = db::connect_in_memory().await.unwrap();
        db::migrate(&pool).await.unwrap();
        let u = Uuid::new_v4();
        insert(&pool, &interval(u, "2026-01-01T00:00:00Z", "2026-01-01T00:30:00Z", "meeting"))
            .await
            .unwrap();
        let loaded = for_user(&pool, u).await.unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].kind, "meeting");
        assert_eq!(total_worked_seconds(&pool, u).await.unwrap(), 1800);
    }

    #[tokio::test]
    async fn sync_queue_flow() {
        let pool = db::connect_in_memory().await.unwrap();
        db::migrate(&pool).await.unwrap();
        let u = Uuid::new_v4();
        let i = interval(u, "2026-01-01T00:00:00Z", "2026-01-01T00:10:00Z", "active");
        insert(&pool, &i).await.unwrap();
        assert_eq!(pending_sync(&pool).await.unwrap().len(), 1);
        mark_synced(&pool, &[i.id]).await.unwrap();
        assert!(pending_sync(&pool).await.unwrap().is_empty());
        mark_synced(&pool, &[i.id]).await.unwrap(); // idempotent
    }
}

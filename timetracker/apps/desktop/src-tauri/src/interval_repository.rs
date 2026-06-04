//! Repository for local intervals + the sync queue.
//!
//! Intervals are append-only and immutable (Rule 2). Sync state is tracked in a
//! separate `interval_sync` table so the `intervals` table is never mutated
//! (Rule 4). Timestamps are stored as RFC3339 UTC text (Rule 3); we control the
//! exact format on both write and read for determinism.

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
    pub idle: bool,
}

impl Interval {
    /// Worked duration in seconds (0 for idle intervals).
    pub fn worked_seconds(&self) -> i64 {
        if self.idle {
            0
        } else {
            (self.end_utc - self.start_utc).num_seconds().max(0)
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
    let idle: i64 = row.try_get("idle")?;
    Ok(Interval {
        id: Uuid::parse_str(&id).context("invalid interval id")?,
        user_id: Uuid::parse_str(&user_id).context("invalid user id")?,
        start_utc: parse_ts(&start_utc)?,
        end_utc: parse_ts(&end_utc)?,
        idle: idle != 0,
    })
}

/// Persist a finished interval. Insert-only (Rule 2).
pub async fn insert(pool: &SqlitePool, interval: &Interval) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO intervals (id, user_id, start_utc, end_utc, idle) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(interval.id.to_string())
    .bind(interval.user_id.to_string())
    .bind(interval.start_utc.to_rfc3339())
    .bind(interval.end_utc.to_rfc3339())
    .bind(interval.idle as i64)
    .execute(pool)
    .await
    .context("failed to insert interval")?;
    Ok(())
}

/// Intervals not yet acknowledged by the server (the sync queue).
pub async fn pending_sync(pool: &SqlitePool) -> anyhow::Result<Vec<Interval>> {
    let rows = sqlx::query(
        r#"
        SELECT i.id, i.user_id, i.start_utc, i.end_utc, i.idle
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

/// Mark intervals as synced (insert into the sync-state table, idempotent).
pub async fn mark_synced(pool: &SqlitePool, ids: &[Uuid]) -> anyhow::Result<()> {
    let now = Utc::now().to_rfc3339();
    let mut tx = pool.begin().await?;
    for id in ids {
        sqlx::query(
            "INSERT OR IGNORE INTO interval_sync (interval_id, synced_at) VALUES (?, ?)",
        )
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
        "SELECT id, user_id, start_utc, end_utc, idle FROM intervals WHERE user_id = ? ORDER BY start_utc",
    )
    .bind(user_id.to_string())
    .fetch_all(pool)
    .await
    .context("failed to load intervals")?;

    rows.iter().map(row_to_interval).collect()
}

/// Total worked seconds for a user — computed from intervals, never stored
/// (Rule 2).
pub async fn total_worked_seconds(pool: &SqlitePool, user_id: Uuid) -> anyhow::Result<i64> {
    Ok(sum_worked(&for_user(pool, user_id).await?))
}

/// Pure helper: sum worked seconds across intervals (excludes idle).
pub fn sum_worked(intervals: &[Interval]) -> i64 {
    intervals.iter().map(Interval::worked_seconds).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    fn interval(user: Uuid, start: &str, end: &str, idle: bool) -> Interval {
        Interval {
            id: Uuid::new_v4(),
            user_id: user,
            start_utc: DateTime::parse_from_rfc3339(start).unwrap().with_timezone(&Utc),
            end_utc: DateTime::parse_from_rfc3339(end).unwrap().with_timezone(&Utc),
            idle,
        }
    }

    #[test]
    fn sum_worked_excludes_idle() {
        let u = Uuid::new_v4();
        let items = vec![
            interval(u, "2026-01-01T00:00:00Z", "2026-01-01T01:00:00Z", false), // 3600
            interval(u, "2026-01-01T01:00:00Z", "2026-01-01T01:30:00Z", true),  // idle -> 0
            interval(u, "2026-01-01T02:00:00Z", "2026-01-01T02:15:00Z", false), // 900
        ];
        assert_eq!(sum_worked(&items), 4500);
    }

    #[tokio::test]
    async fn insert_and_totals_round_trip() {
        let pool = db::connect_in_memory().await.unwrap();
        db::migrate(&pool).await.unwrap();
        let u = Uuid::new_v4();

        insert(&pool, &interval(u, "2026-01-01T00:00:00Z", "2026-01-01T01:00:00Z", false))
            .await
            .unwrap();
        insert(&pool, &interval(u, "2026-01-01T02:00:00Z", "2026-01-01T02:30:00Z", false))
            .await
            .unwrap();

        assert_eq!(total_worked_seconds(&pool, u).await.unwrap(), 5400);
    }

    #[tokio::test]
    async fn interval_survives_restart() {
        // Use a real file DB, close the pool (simulating app exit), reopen, and
        // confirm the interval is still there (acceptance: survives app restart).
        let path = std::env::temp_dir().join(format!("tt-test-{}.db", Uuid::new_v4()));
        let u = Uuid::new_v4();
        let i = interval(u, "2026-01-01T00:00:00Z", "2026-01-01T00:45:00Z", false);

        {
            let pool = db::connect(&path).await.unwrap();
            db::migrate(&pool).await.unwrap();
            insert(&pool, &i).await.unwrap();
            pool.close().await;
        }

        // "Restart": brand-new pool against the same file.
        let pool2 = db::connect(&path).await.unwrap();
        let loaded = for_user(&pool2, u).await.unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0], i);
        assert_eq!(total_worked_seconds(&pool2, u).await.unwrap(), 2700);
        pool2.close().await;

        std::fs::remove_file(&path).ok();
    }

    #[tokio::test]
    async fn sync_queue_flow() {
        let pool = db::connect_in_memory().await.unwrap();
        db::migrate(&pool).await.unwrap();
        let u = Uuid::new_v4();

        let i = interval(u, "2026-01-01T00:00:00Z", "2026-01-01T00:10:00Z", false);
        insert(&pool, &i).await.unwrap();

        // Initially pending.
        let pending = pending_sync(&pool).await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, i.id);

        // After marking, nothing pending.
        mark_synced(&pool, &[i.id]).await.unwrap();
        assert!(pending_sync(&pool).await.unwrap().is_empty());

        // mark_synced is idempotent.
        mark_synced(&pool, &[i.id]).await.unwrap();
    }
}

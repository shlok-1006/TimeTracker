//! Intervals repository (Rule 7: SQLx, compile-time checked queries).
//!
//! Intervals are immutable and arrive from the desktop sync worker. Inserts are
//! idempotent on the client-generated `id`, so re-syncing the same interval is a
//! no-op (Rule 4 — at-least-once delivery is safe).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;

/// Wire representation of an interval (shared by the API and the sync worker).
/// `user_id` is intentionally absent — the server derives it from the JWT so a
/// client can never write intervals for another user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntervalDto {
    pub id: Uuid,
    pub start_utc: DateTime<Utc>,
    pub end_utc: DateTime<Utc>,
    #[serde(default)]
    pub idle: bool,
}

/// Insert a batch of intervals for `user_id` in a single transaction.
/// Returns the number of newly-inserted rows (duplicates are ignored).
pub async fn insert_batch(
    pool: &PgPool,
    user_id: Uuid,
    items: &[IntervalDto],
) -> Result<u64, AppError> {
    let mut tx = pool.begin().await?;
    let mut inserted = 0u64;

    for item in items {
        let res = sqlx::query!(
            r#"
            INSERT INTO intervals (id, user_id, start_utc, end_utc, idle)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (id) DO NOTHING
            "#,
            item.id,
            user_id,
            item.start_utc,
            item.end_utc,
            item.idle
        )
        .execute(&mut *tx)
        .await?;
        inserted += res.rows_affected();
    }

    tx.commit().await?;
    Ok(inserted)
}

/// Dashboard hours summary (computed from intervals; Rule 2).
#[derive(Debug, Serialize)]
pub struct HoursSummary {
    pub total_seconds: i64,
    pub today_seconds: i64,
    pub week_seconds: i64,
    pub active_seconds: i64,
    pub idle_seconds: i64,
}

pub async fn hours_summary(pool: &PgPool, user_id: Uuid) -> Result<HoursSummary, AppError> {
    let r = sqlx::query!(
        r#"
        SELECT
          CAST(COALESCE(SUM(EXTRACT(EPOCH FROM (end_utc-start_utc))) FILTER (WHERE NOT idle),0) AS BIGINT) AS "total!",
          CAST(COALESCE(SUM(EXTRACT(EPOCH FROM (end_utc-start_utc))) FILTER (WHERE NOT idle AND start_utc >= date_trunc('day', now())),0) AS BIGINT) AS "today!",
          CAST(COALESCE(SUM(EXTRACT(EPOCH FROM (end_utc-start_utc))) FILTER (WHERE NOT idle AND start_utc >= date_trunc('week', now())),0) AS BIGINT) AS "week!",
          CAST(COALESCE(SUM(EXTRACT(EPOCH FROM (end_utc-start_utc))) FILTER (WHERE idle),0) AS BIGINT) AS "idle!"
        FROM intervals WHERE user_id = $1
        "#,
        user_id
    )
    .fetch_one(pool)
    .await?;

    Ok(HoursSummary {
        total_seconds: r.total,
        today_seconds: r.today,
        week_seconds: r.week,
        active_seconds: r.total, // worked == active (non-idle)
        idle_seconds: r.idle,
    })
}

/// Total worked seconds for a user — computed from intervals, never stored
/// (Rule 2). Idle intervals are excluded.
pub async fn total_worked_seconds(pool: &PgPool, user_id: Uuid) -> Result<i64, AppError> {
    let row = sqlx::query!(
        r#"
        SELECT CAST(COALESCE(SUM(EXTRACT(EPOCH FROM (end_utc - start_utc))), 0) AS BIGINT)
                   AS "total!"
        FROM intervals
        WHERE user_id = $1 AND idle = false
        "#,
        user_id
    )
    .fetch_one(pool)
    .await?;

    Ok(row.total)
}

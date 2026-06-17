//! Intervals repository (Rule 7: SQLx, compile-time checked queries).
//!
//! Intervals are immutable, status-tagged segments (`kind`: active | idle |
//! meeting | break) synced from the desktop. Worked time = active + meeting;
//! idle and break are excluded from totals (Rule 2 — totals are derived).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;

/// Wire representation of an interval segment. `user_id` is derived from the JWT.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntervalDto {
    pub id: Uuid,
    pub start_utc: DateTime<Utc>,
    pub end_utc: DateTime<Utc>,
    /// active | idle | meeting | break
    pub kind: String,
    /// Team the work was logged under (Feature 4). Optional so older desktop
    /// builds that don't send it still sync.
    #[serde(default)]
    pub team_id: Option<Uuid>,
}

/// Insert a batch of intervals for `user_id` in a single transaction.
/// Idempotent (`ON CONFLICT (id) DO NOTHING`). Returns rows inserted.
pub async fn insert_batch(
    pool: &PgPool,
    user_id: Uuid,
    items: &[IntervalDto],
) -> Result<u64, AppError> {
    let mut tx = pool.begin().await?;
    let mut inserted = 0u64;

    for item in items {
        let idle = item.kind == "idle";
        let res = sqlx::query!(
            r#"
            INSERT INTO intervals (id, user_id, start_utc, end_utc, idle, kind, team_id)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            ON CONFLICT (id) DO NOTHING
            "#,
            item.id,
            user_id,
            item.start_utc,
            item.end_utc,
            idle,
            item.kind,
            item.team_id
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
    pub meeting_seconds: i64,
    pub break_seconds: i64,
}

pub async fn hours_summary(pool: &PgPool, user_id: Uuid) -> Result<HoursSummary, AppError> {
    let r = sqlx::query!(
        r#"
        SELECT
          CAST(COALESCE(SUM(EXTRACT(EPOCH FROM (end_utc-start_utc))) FILTER (WHERE kind IN ('active','meeting')),0) AS BIGINT) AS "total!",
          CAST(COALESCE(SUM(EXTRACT(EPOCH FROM (end_utc-start_utc))) FILTER (WHERE kind IN ('active','meeting') AND start_utc >= date_trunc('day', now())),0) AS BIGINT) AS "today!",
          CAST(COALESCE(SUM(EXTRACT(EPOCH FROM (end_utc-start_utc))) FILTER (WHERE kind IN ('active','meeting') AND start_utc >= date_trunc('week', now())),0) AS BIGINT) AS "week!",
          CAST(COALESCE(SUM(EXTRACT(EPOCH FROM (end_utc-start_utc))) FILTER (WHERE kind='active'),0) AS BIGINT) AS "active!",
          CAST(COALESCE(SUM(EXTRACT(EPOCH FROM (end_utc-start_utc))) FILTER (WHERE kind='idle'),0) AS BIGINT) AS "idle!",
          CAST(COALESCE(SUM(EXTRACT(EPOCH FROM (end_utc-start_utc))) FILTER (WHERE kind='meeting'),0) AS BIGINT) AS "meeting!",
          CAST(COALESCE(SUM(EXTRACT(EPOCH FROM (end_utc-start_utc))) FILTER (WHERE kind='break'),0) AS BIGINT) AS "brk!"
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
        active_seconds: r.active,
        idle_seconds: r.idle,
        meeting_seconds: r.meeting,
        break_seconds: r.brk,
    })
}

/// A timeline segment for the activity bar.
#[derive(Debug)]
pub struct Segment {
    pub start_utc: DateTime<Utc>,
    pub end_utc: DateTime<Utc>,
    pub kind: String,
}

/// Intervals overlapping the `[from, to)` window (for the day-activity timeline).
pub async fn day_segments(
    pool: &PgPool,
    user_id: Uuid,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<Vec<Segment>, AppError> {
    let rows = sqlx::query!(
        r#"
        SELECT start_utc, end_utc, kind
        FROM intervals
        WHERE user_id = $1 AND end_utc > $2 AND start_utc < $3
        ORDER BY start_utc
        "#,
        user_id,
        from,
        to
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| Segment {
            start_utc: r.start_utc,
            end_utc: r.end_utc,
            kind: r.kind,
        })
        .collect())
}

/// Total worked seconds (active + meeting) for a user.
pub async fn total_worked_seconds(pool: &PgPool, user_id: Uuid) -> Result<i64, AppError> {
    let row = sqlx::query!(
        r#"
        SELECT CAST(COALESCE(SUM(EXTRACT(EPOCH FROM (end_utc - start_utc))), 0) AS BIGINT) AS "total!"
        FROM intervals
        WHERE user_id = $1 AND kind IN ('active','meeting')
        "#,
        user_id
    )
    .fetch_one(pool)
    .await?;
    Ok(row.total)
}

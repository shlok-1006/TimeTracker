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

/// Insert a batch of intervals for `user_id` in a SINGLE bulk statement.
/// Idempotent (`ON CONFLICT (id) DO NOTHING`). Returns rows inserted.
///
/// Uses `UNNEST` so the whole batch is one round-trip instead of one INSERT per
/// row in a transaction — critical when a client drains a large offline backlog
/// (the old per-row loop was slow enough to time out, so those users' time
/// never synced). A stale/deleted `team_id` is coerced to NULL per row so a
/// removed team can't abort the batch.
pub async fn insert_batch(
    pool: &PgPool,
    user_id: Uuid,
    items: &[IntervalDto],
) -> Result<u64, AppError> {
    if items.is_empty() {
        return Ok(0);
    }
    let ids: Vec<Uuid> = items.iter().map(|i| i.id).collect();
    let starts: Vec<DateTime<Utc>> = items.iter().map(|i| i.start_utc).collect();
    let ends: Vec<DateTime<Utc>> = items.iter().map(|i| i.end_utc).collect();
    let idles: Vec<bool> = items.iter().map(|i| i.kind == "idle").collect();
    let kinds: Vec<String> = items.iter().map(|i| i.kind.clone()).collect();
    let team_ids: Vec<Option<Uuid>> = items.iter().map(|i| i.team_id).collect();

    let res = sqlx::query!(
        r#"
        INSERT INTO intervals (id, user_id, start_utc, end_utc, idle, kind, team_id)
        SELECT t.id, $2, t.start_utc, t.end_utc, t.idle, t.kind,
               (SELECT id FROM teams WHERE id = t.team_id)
        FROM UNNEST($1::uuid[], $3::timestamptz[], $4::timestamptz[],
                    $5::bool[], $6::text[], $7::uuid[])
             AS t(id, start_utc, end_utc, idle, kind, team_id)
        ON CONFLICT (id) DO NOTHING
        "#,
        &ids,
        user_id,
        &starts,
        &ends,
        &idles,
        &kinds,
        &team_ids as &[Option<Uuid>],
    )
    .execute(pool)
    .await?;

    Ok(res.rows_affected())
}

/// Dashboard hours summary (computed from intervals; Rule 2).
///
/// A "day's work" = active + idle + meeting (only Break is excluded — it's a
/// deliberate pause). Both TODAY and THIS WEEK are period-scoped totals of
/// that, each broken out into active / idle / meeting so idle and meeting are
/// visible on their own. All windows use the 4 AM local business-day boundary.
#[derive(Debug, Serialize)]
pub struct HoursSummary {
    pub today_seconds: i64,
    pub today_active_seconds: i64,
    pub today_idle_seconds: i64,
    pub today_meeting_seconds: i64,
    pub week_seconds: i64,
    pub week_active_seconds: i64,
    pub week_idle_seconds: i64,
    pub week_meeting_seconds: i64,
    /// All-time worked (active+idle+meeting) — used only for the desktop's
    /// "server total (reconciled)" line.
    pub total_seconds: i64,
}

pub async fn hours_summary(pool: &PgPool, user_id: Uuid) -> Result<HoursSummary, AppError> {
    // "Today"/"this week" use a 04:00 LOCAL business-day boundary in the user's
    // own timezone (reported by the desktop; falls back to UTC), so late-night
    // work counts toward the day it began and this figure matches the desktop's
    // local one. The 4 AM shift: subtract 4h, truncate to day/week, add 4h back,
    // then interpret that wall time in the user's zone (DST-correct). Bound as
    // $2::text so `AT TIME ZONE` uses the text-zone (not interval) overload.
    let tz = sqlx::query!(
        r#"SELECT COALESCE(timezone, 'UTC') AS "z!" FROM users WHERE id = $1"#,
        user_id
    )
    .fetch_optional(pool)
    .await?
    .map(|r| r.z)
    .unwrap_or_else(|| "UTC".to_string());

    let r = sqlx::query!(
        r#"
        WITH b AS (
          SELECT
            ((date_trunc('day',  (now() AT TIME ZONE $2::text) - interval '4 hours') + interval '4 hours') AT TIME ZONE $2::text) AS day_start,
            ((date_trunc('week', (now() AT TIME ZONE $2::text) - interval '4 hours') + interval '4 hours') AT TIME ZONE $2::text) AS week_start
        )
        SELECT
          -- Today (active+idle+meeting) + breakdown
          CAST(COALESCE(SUM(EXTRACT(EPOCH FROM (end_utc-start_utc))) FILTER (WHERE kind IN ('active','idle','meeting') AND start_utc >= b.day_start),0) AS BIGINT) AS "today!",
          CAST(COALESCE(SUM(EXTRACT(EPOCH FROM (end_utc-start_utc))) FILTER (WHERE kind='active'  AND start_utc >= b.day_start),0) AS BIGINT) AS "today_active!",
          CAST(COALESCE(SUM(EXTRACT(EPOCH FROM (end_utc-start_utc))) FILTER (WHERE kind='idle'    AND start_utc >= b.day_start),0) AS BIGINT) AS "today_idle!",
          CAST(COALESCE(SUM(EXTRACT(EPOCH FROM (end_utc-start_utc))) FILTER (WHERE kind='meeting' AND start_utc >= b.day_start),0) AS BIGINT) AS "today_meeting!",
          -- This week (active+idle+meeting) + breakdown
          CAST(COALESCE(SUM(EXTRACT(EPOCH FROM (end_utc-start_utc))) FILTER (WHERE kind IN ('active','idle','meeting') AND start_utc >= b.week_start),0) AS BIGINT) AS "week!",
          CAST(COALESCE(SUM(EXTRACT(EPOCH FROM (end_utc-start_utc))) FILTER (WHERE kind='active'  AND start_utc >= b.week_start),0) AS BIGINT) AS "week_active!",
          CAST(COALESCE(SUM(EXTRACT(EPOCH FROM (end_utc-start_utc))) FILTER (WHERE kind='idle'    AND start_utc >= b.week_start),0) AS BIGINT) AS "week_idle!",
          CAST(COALESCE(SUM(EXTRACT(EPOCH FROM (end_utc-start_utc))) FILTER (WHERE kind='meeting' AND start_utc >= b.week_start),0) AS BIGINT) AS "week_meeting!",
          -- All-time worked (reconcile line)
          CAST(COALESCE(SUM(EXTRACT(EPOCH FROM (end_utc-start_utc))) FILTER (WHERE kind IN ('active','idle','meeting')),0) AS BIGINT) AS "total!"
        FROM intervals, b WHERE user_id = $1
        GROUP BY b.day_start, b.week_start
        "#,
        user_id,
        tz
    )
    .fetch_optional(pool)
    .await?;

    // No intervals yet → all zeros (the GROUP BY yields no row).
    Ok(match r {
        Some(r) => HoursSummary {
            today_seconds: r.today,
            today_active_seconds: r.today_active,
            today_idle_seconds: r.today_idle,
            today_meeting_seconds: r.today_meeting,
            week_seconds: r.week,
            week_active_seconds: r.week_active,
            week_idle_seconds: r.week_idle,
            week_meeting_seconds: r.week_meeting,
            total_seconds: r.total,
        },
        None => HoursSummary {
            today_seconds: 0,
            today_active_seconds: 0,
            today_idle_seconds: 0,
            today_meeting_seconds: 0,
            week_seconds: 0,
            week_active_seconds: 0,
            week_idle_seconds: 0,
            week_meeting_seconds: 0,
            total_seconds: 0,
        },
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

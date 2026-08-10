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
    /// This week's total INCLUDING any manual grace time (so the dashboard total
    /// reflects grants). `week_grace_seconds` says how much of it is grace.
    pub week_seconds: i64,
    pub week_active_seconds: i64,
    pub week_idle_seconds: i64,
    pub week_meeting_seconds: i64,
    /// Manually-granted "grace" time in the current week (0 = none). When > 0 the
    /// UI tags the week total as including grace.
    pub week_grace_seconds: i64,
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
        -- Each window is unioned, not summed: `interval_seconds` (migration 0044)
        -- counts a wall-clock second once even when two devices recorded it. Without
        -- that, a second install signed in as the same person doubles these figures.
        --
        -- The windows are now half-open ranges rather than "everything from the
        -- boundary onward", so a session spanning the 4 AM boundary contributes its
        -- part to each side instead of landing wholly in the day it began. That makes
        -- the day and week windows disjoint, which is what lets them be unioned
        -- independently without one swallowing the other.
        WITH b AS (
          SELECT
            ((date_trunc('day',  (now() AT TIME ZONE $2::text) - interval '4 hours') + interval '4 hours') AT TIME ZONE $2::text) AS day_start,
            ((date_trunc('week', (now() AT TIME ZONE $2::text) - interval '4 hours') + interval '4 hours') AT TIME ZONE $2::text) AS week_start
        ),
        w AS (
          SELECT day_start, day_start  + interval '1 day'  AS day_end,
                 week_start, week_start + interval '7 days' AS week_end
          FROM b
        ),
        d AS (SELECT * FROM w, LATERAL interval_seconds($1, w.day_start,  w.day_end,  NULL)),
        k AS (SELECT * FROM w, LATERAL interval_seconds($1, w.week_start, w.week_end, NULL)),
        t AS (SELECT * FROM interval_seconds($1, '-infinity'::timestamptz, 'infinity'::timestamptz, NULL))
        SELECT
          CAST(d.active + d.idle + d.meeting AS BIGINT) AS "today!",
          CAST(d.active  AS BIGINT) AS "today_active!",
          CAST(d.idle    AS BIGINT) AS "today_idle!",
          CAST(d.meeting AS BIGINT) AS "today_meeting!",
          CAST(k.active + k.idle + k.meeting AS BIGINT) AS "week!",
          CAST(k.active  AS BIGINT) AS "week_active!",
          CAST(k.idle    AS BIGINT) AS "week_idle!",
          CAST(k.meeting AS BIGINT) AS "week_meeting!",
          CAST(t.active + t.idle + t.meeting AS BIGINT) AS "total!"
        FROM d, k, t
        "#,
        user_id,
        tz
    )
    // Always a row now: the aggregate used to GROUP BY and vanish for a person with
    // no intervals, whereas `interval_seconds` answers zero.
    .fetch_one(pool)
    .await?;

    // Manual "grace" time granted for the CURRENT business week — added to the
    // week total (Rule 2: intervals stay pure; grace is a separate additive
    // layer). Same Monday/4 AM/user-tz boundary as the week window above, so the
    // two line up. Computed separately so grace shows even with zero intervals.
    let grace: i64 = sqlx::query_scalar!(
        r#"SELECT COALESCE(SUM(seconds), 0)::bigint AS "grace!"
           FROM time_grants
           WHERE user_id = $1
             AND week_start = date_trunc('week', ((now() AT TIME ZONE $2::text) - interval '4 hours'))::date"#,
        user_id,
        tz
    )
    .fetch_one(pool)
    .await?;

    // Someone with no intervals reads as zeros — and grace still applies on top,
    // which is why it is fetched separately rather than joined into the query above.
    Ok(HoursSummary {
        today_seconds: r.today,
        today_active_seconds: r.today_active,
        today_idle_seconds: r.today_idle,
        today_meeting_seconds: r.today_meeting,
        week_seconds: r.week + grace,
        week_active_seconds: r.week_active,
        week_idle_seconds: r.week_idle,
        week_meeting_seconds: r.week_meeting,
        week_grace_seconds: grace,
        total_seconds: r.total + grace,
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

//! Screenshots repository — metadata only (Rule 5; Rule 7 compile-time queries).
//!
//! Feature 2: each screenshot records `captured_status` — the employee's
//! presence status at capture time — so sampling/analysis can exclude
//! non-working (e.g. meeting) shots. A working shot the analyzer recognises as
//! a call is re-tagged `meeting` afterwards (`mark_meeting_detected`).

use chrono::{DateTime, NaiveDate, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;

/// UTC `[start, end)` bounds of a calendar day.
/// A `day` means the ORG-LOCAL calendar day (IST) — see crate::org_time.
fn day_bounds(day: NaiveDate) -> (DateTime<Utc>, DateTime<Utc>) {
    crate::org_time::day_bounds_utc(day)
}

/// Statuses a screenshot can be captured under (must match the DB CHECK).
pub const CAPTURED_STATUSES: [&str; 5] = ["working", "meeting", "break", "idle", "not_working"];

/// Validate a client-supplied captured status.
pub fn is_valid_captured_status(s: &str) -> bool {
    CAPTURED_STATUSES.contains(&s)
}

/// Insert screenshot metadata, returning the row id. Idempotent on the storage
/// key: the desktop retries a queued screenshot until it hears back, so a retry
/// after a lost response returns the existing row instead of failing on the
/// UNIQUE key forever. (The key is validated to sit in the caller's namespace,
/// so an existing row with that key is always the caller's own.)
pub async fn insert(
    pool: &PgPool,
    user_id: Uuid,
    storage_key: &str,
    taken_at: DateTime<Utc>,
    interval_id: Option<Uuid>,
    captured_status: &str,
) -> Result<Uuid, AppError> {
    let row = sqlx::query!(
        r#"
        INSERT INTO screenshots (user_id, storage_key, taken_at, interval_id, captured_status)
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (storage_key) DO UPDATE SET storage_key = EXCLUDED.storage_key
        RETURNING id
        "#,
        user_id,
        storage_key,
        taken_at,
        interval_id,
        captured_status
    )
    .fetch_one(pool)
    .await?;
    Ok(row.id)
}

/// A screenshot's stored metadata.
pub struct ScreenshotRow {
    pub id: Uuid,
    pub storage_key: String,
    pub taken_at: DateTime<Utc>,
    pub interval_id: Option<Uuid>,
    pub captured_status: String,
}

/// List a user's most recent screenshots (metadata only).
pub async fn list_for_user(
    pool: &PgPool,
    user_id: Uuid,
    limit: i64,
) -> Result<Vec<ScreenshotRow>, AppError> {
    let rows = sqlx::query!(
        r#"
        SELECT id, storage_key, taken_at, interval_id, captured_status
        FROM screenshots
        WHERE user_id = $1
        ORDER BY taken_at DESC
        LIMIT $2
        "#,
        user_id,
        limit
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| ScreenshotRow {
            id: r.id,
            storage_key: r.storage_key,
            taken_at: r.taken_at,
            interval_id: r.interval_id,
            captured_status: r.captured_status,
        })
        .collect())
}

/// A screenshot within a day window, joined to its analysis verdict (if any).
pub struct DayScreenshot {
    pub id: Uuid,
    pub taken_at: DateTime<Utc>,
    pub storage_key: String,
    pub captured_status: String,
    /// The AI verdict for this screenshot, or `None` if it was never analysed
    /// (e.g. a meeting shot, or one not picked by the sampler).
    pub verdict: Option<String>,
}

/// List a user's screenshots captured on `day` (UTC), each with its analysis
/// verdict via a LEFT JOIN (NULL when not analysed), oldest first.
pub async fn list_for_day(
    pool: &PgPool,
    user_id: Uuid,
    day: NaiveDate,
) -> Result<Vec<DayScreenshot>, AppError> {
    let (from, to) = day_bounds(day);
    let rows = sqlx::query!(
        r#"
        SELECT sc.id, sc.taken_at, sc.storage_key, sc.captured_status,
               ar.verdict AS "verdict?"
        FROM screenshots sc
        LEFT JOIN analysis_results ar ON ar.screenshot_id = sc.id
        WHERE sc.user_id = $1 AND sc.taken_at >= $2 AND sc.taken_at < $3
        ORDER BY sc.taken_at
        "#,
        user_id,
        from,
        to
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| DayScreenshot {
            id: r.id,
            taken_at: r.taken_at,
            storage_key: r.storage_key,
            captured_status: r.captured_status,
            verdict: r.verdict,
        })
        .collect())
}

/// How many screenshots a user has in `[from, to)`, total and working-only.
/// Drives the admin range-analysis preview (count + cost estimate before running).
pub struct RangeCounts {
    pub total: i64,
    pub working: i64,
}

pub async fn count_in_range(
    pool: &PgPool,
    user_id: Uuid,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<RangeCounts, AppError> {
    let row = sqlx::query!(
        r#"
        SELECT COUNT(*) AS "total!",
               COUNT(*) FILTER (WHERE captured_status = 'working') AS "working!"
        FROM screenshots
        WHERE user_id = $1 AND taken_at >= $2 AND taken_at < $3
        "#,
        user_id,
        from,
        to
    )
    .fetch_one(pool)
    .await?;
    Ok(RangeCounts {
        total: row.total,
        working: row.working,
    })
}

/// Every *working* screenshot a user captured in `[from, to)`, oldest first.
/// Unlike the sampler (4–5 per day) this is exhaustive — the admin range
/// analysis verifies each one. Served by idx_screenshots_taken (user_id, taken_at).
pub async fn list_working_in_range(
    pool: &PgPool,
    user_id: Uuid,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<Vec<ScreenshotRow>, AppError> {
    let rows = sqlx::query!(
        r#"
        SELECT id, storage_key, taken_at, interval_id, captured_status
        FROM screenshots
        WHERE user_id = $1 AND captured_status = 'working'
              AND taken_at >= $2 AND taken_at < $3
        ORDER BY taken_at
        "#,
        user_id,
        from,
        to
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| ScreenshotRow {
            id: r.id,
            storage_key: r.storage_key,
            taken_at: r.taken_at,
            interval_id: r.interval_id,
            captured_status: r.captured_status,
        })
        .collect())
}

/// Distinct users who captured *working* screenshots on `day` (UTC). Used by the
/// nightly scheduler to know whose reports to build.
pub async fn working_user_ids_on_day(pool: &PgPool, day: NaiveDate) -> Result<Vec<Uuid>, AppError> {
    let (from, to) = day_bounds(day);
    let rows = sqlx::query!(
        r#"SELECT DISTINCT user_id FROM screenshots
           WHERE captured_status = 'working' AND taken_at >= $1 AND taken_at < $2"#,
        from,
        to
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|r| r.user_id).collect())
}

/// Users with working screenshots on `day` whose analysis job for that day has
/// no sampled shots — the nightly review ran before the screenshots arrived
/// (a laptop on a VPN that blocked uploads), or never covered them. Sampling
/// never resamples a day that HAS samples, so these are exactly the days a
/// fresh review can still fill in.
pub async fn users_with_unsampled_working_shots(
    pool: &PgPool,
    day: NaiveDate,
) -> Result<Vec<Uuid>, AppError> {
    let (from, to) = day_bounds(day);
    let rows = sqlx::query!(
        r#"SELECT DISTINCT s.user_id FROM screenshots s
           WHERE s.captured_status = 'working' AND s.taken_at >= $1 AND s.taken_at < $2
             AND NOT EXISTS (
                 SELECT 1 FROM analysis_jobs j
                 JOIN analysis_job_samples x ON x.job_id = j.id
                 WHERE j.user_id = s.user_id AND j.day = $3)"#,
        from,
        to,
        day
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|r| r.user_id).collect())
}

/// Re-tag a working screenshot the analyzer recognised as a call (Google Meet,
/// Slack huddle) as `meeting`, stamping when that was decided. From then on it is
/// treated like a meeting shot everywhere: never sampled or scored, and shown with
/// the meeting badge. A verdict stored for it earlier is removed so it can't count
/// towards a report. Returns whether the row changed (false if it was already
/// tagged, or gone).
pub async fn mark_meeting_detected(pool: &PgPool, screenshot_id: Uuid) -> Result<bool, AppError> {
    let mut tx = pool.begin().await?;
    let changed = sqlx::query!(
        r#"UPDATE screenshots
           SET captured_status = 'meeting', meeting_detected_at = now()
           WHERE id = $1 AND captured_status = 'working'"#,
        screenshot_id
    )
    .execute(&mut *tx)
    .await?
    .rows_affected()
        > 0;
    sqlx::query!(
        "DELETE FROM analysis_results WHERE screenshot_id = $1",
        screenshot_id
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(changed)
}

/// Count a user's screenshots (used by tests / future reporting).
pub async fn count_for_user(pool: &PgPool, user_id: Uuid) -> Result<i64, AppError> {
    let row = sqlx::query!(
        r#"SELECT COUNT(*) AS "count!" FROM screenshots WHERE user_id = $1"#,
        user_id
    )
    .fetch_one(pool)
    .await?;
    Ok(row.count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_captured_statuses() {
        for s in CAPTURED_STATUSES {
            assert!(is_valid_captured_status(s), "{s} should be valid");
        }
        assert!(!is_valid_captured_status("offline"));
        assert!(!is_valid_captured_status("Working")); // case-sensitive
        assert!(!is_valid_captured_status(""));
    }
}

//! Activity repository (Rule 7): per-app foreground seconds and 10-minute
//! input-activity blocks, synced up from the desktop as monotonic counters.
//!
//! Upserts use GREATEST(old, new) so the desktop's at-least-once sync can
//! resend absolute values freely — a retry can never double-count.

use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;

/// UTC `[start, end)` bounds of a calendar day.
fn day_bounds(day: NaiveDate) -> (DateTime<Utc>, DateTime<Utc>) {
    let start = Utc.from_utc_datetime(&day.and_hms_opt(0, 0, 0).expect("valid midnight"));
    (start, start + Duration::days(1))
}

/// Record (or raise) the foreground seconds for one (user, day, app).
pub async fn upsert_app_usage(
    pool: &PgPool,
    user_id: Uuid,
    day: NaiveDate,
    app_name: &str,
    seconds: i32,
) -> Result<(), AppError> {
    sqlx::query!(
        r#"
        INSERT INTO app_usage (user_id, day, app_name, seconds)
        VALUES ($1, $2, $3, $4)
        ON CONFLICT (user_id, day, app_name)
        DO UPDATE SET seconds = GREATEST(app_usage.seconds, EXCLUDED.seconds),
                      updated_at = now()
        "#,
        user_id,
        day,
        app_name,
        seconds
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Record (or raise) one 10-minute activity block.
pub async fn upsert_block(
    pool: &PgPool,
    user_id: Uuid,
    block_start: DateTime<Utc>,
    active_seconds: i32,
    total_seconds: i32,
) -> Result<(), AppError> {
    sqlx::query!(
        r#"
        INSERT INTO activity_blocks (user_id, block_start, active_seconds, total_seconds)
        VALUES ($1, $2, $3, $4)
        ON CONFLICT (user_id, block_start)
        DO UPDATE SET active_seconds = GREATEST(activity_blocks.active_seconds, EXCLUDED.active_seconds),
                      total_seconds  = GREATEST(activity_blocks.total_seconds,  EXCLUDED.total_seconds),
                      updated_at = now()
        "#,
        user_id,
        block_start,
        active_seconds,
        total_seconds
    )
    .execute(pool)
    .await?;
    Ok(())
}

#[derive(Serialize)]
pub struct AppUsageRow {
    pub app_name: String,
    pub seconds: i64,
}

/// A user's per-app foreground time for one UTC day, biggest first.
pub async fn apps_for_day(
    pool: &PgPool,
    user_id: Uuid,
    day: NaiveDate,
) -> Result<Vec<AppUsageRow>, AppError> {
    let rows = sqlx::query!(
        r#"
        SELECT app_name, CAST(seconds AS BIGINT) AS "seconds!"
        FROM app_usage
        WHERE user_id = $1 AND day = $2
        ORDER BY seconds DESC
        "#,
        user_id,
        day
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| AppUsageRow {
            app_name: r.app_name,
            seconds: r.seconds,
        })
        .collect())
}

#[derive(Serialize)]
pub struct ActivityBlockRow {
    pub block_start: DateTime<Utc>,
    pub active_seconds: i32,
    pub total_seconds: i32,
}

/// A user's 10-minute activity blocks within one UTC day, oldest first.
pub async fn blocks_for_day(
    pool: &PgPool,
    user_id: Uuid,
    day: NaiveDate,
) -> Result<Vec<ActivityBlockRow>, AppError> {
    let (from, to) = day_bounds(day);
    let rows = sqlx::query!(
        r#"
        SELECT block_start, active_seconds, total_seconds
        FROM activity_blocks
        WHERE user_id = $1 AND block_start >= $2 AND block_start < $3
        ORDER BY block_start
        "#,
        user_id,
        from,
        to
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| ActivityBlockRow {
            block_start: r.block_start,
            active_seconds: r.active_seconds,
            total_seconds: r.total_seconds,
        })
        .collect())
}

/// Overall activity percentage across a set of blocks (None when no data).
pub fn activity_pct(blocks: &[ActivityBlockRow]) -> Option<f64> {
    let total: i64 = blocks.iter().map(|b| b.total_seconds as i64).sum();
    if total == 0 {
        return None;
    }
    let active: i64 = blocks.iter().map(|b| b.active_seconds as i64).sum();
    Some((active as f64 / total as f64) * 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(active: i32, total: i32) -> ActivityBlockRow {
        ActivityBlockRow {
            block_start: Utc::now(),
            active_seconds: active,
            total_seconds: total,
        }
    }

    #[test]
    fn pct_is_weighted_across_blocks() {
        // 600/600 and 0/600 => 50% overall, regardless of block count.
        let blocks = vec![block(600, 600), block(0, 600)];
        assert_eq!(activity_pct(&blocks), Some(50.0));
    }

    #[test]
    fn pct_empty_and_zero_total_is_none() {
        assert_eq!(activity_pct(&[]), None);
        assert_eq!(activity_pct(&[block(0, 0)]), None);
    }
}

//! Activity tracker (Activity feature): while the user is **Working**, sample
//! the foreground application every 10s and accumulate two privacy-bounded
//! aggregates in local SQLite (Rule 1 — local first):
//!
//!   * `app_usage`        — seconds per (UTC day, app NAME). Names only:
//!                          window titles are never read, stored, or synced.
//!   * `activity_blocks`  — per 10-minute UTC block, seconds with real
//!                          keyboard/mouse input vs seconds tracked. Input is
//!                          observed only as "was there input since the last
//!                          sample?" — never what was typed.
//!
//! Rows are mutable counters, so sync uses a dirty flag (not the synced-once
//! queue intervals use): accumulation sets `dirty = 1`; every ~60s the worker
//! POSTs absolute values to `/activity` (server upserts are monotonic, so
//! at-least-once delivery never double-counts) and clears `dirty` only when
//! `updated_at` is unchanged — a concurrent accumulation keeps the row dirty.

use std::sync::atomic::Ordering;
use std::time::Duration;

use chrono::{DateTime, SecondsFormat, Utc};
use serde::Serialize;
use serde_json::json;
use sqlx::SqlitePool;

use crate::presence::derive_status;
use crate::timer::DesktopState;

/// Seconds between foreground samples (each sample credits this much time).
const SAMPLE_SECS: i64 = 10;
/// Sync every N samples (6 × 10s = every minute).
const SYNC_EVERY_TICKS: u32 = 6;
/// Activity block width in seconds (10 minutes).
const BLOCK_SECS: i64 = 600;

/// Truncate a timestamp down to its 10-minute block boundary (UTC).
pub fn block_start(now: DateTime<Utc>) -> DateTime<Utc> {
    let secs = now.timestamp();
    DateTime::from_timestamp(secs - secs.rem_euclid(BLOCK_SECS), 0)
        .expect("block boundary is a valid timestamp")
}

fn rfc3339(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// The foreground application's NAME (never the window title). `None` when
/// detection fails (locked screen, Wayland without support, etc.).
fn foreground_app_name() -> Option<String> {
    let win = active_win_pos_rs::get_active_window().ok()?;
    let name = win.app_name.trim().to_string();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

// ---- Local accumulation ----

/// Credit one sample: `SAMPLE_SECS` to (today, app) and to the current block.
pub async fn accumulate(
    pool: &SqlitePool,
    now: DateTime<Utc>,
    app_name: &str,
    input_active: bool,
) -> anyhow::Result<()> {
    let day = now.date_naive().to_string();
    let updated = rfc3339(now);
    sqlx::query(
        "INSERT INTO app_usage (day, app_name, seconds, dirty, updated_at)
         VALUES (?, ?, ?, 1, ?)
         ON CONFLICT (day, app_name)
         DO UPDATE SET seconds = seconds + excluded.seconds, dirty = 1,
                       updated_at = excluded.updated_at",
    )
    .bind(&day)
    .bind(app_name)
    .bind(SAMPLE_SECS)
    .bind(&updated)
    .execute(pool)
    .await?;

    let active = if input_active { SAMPLE_SECS } else { 0 };
    sqlx::query(
        "INSERT INTO activity_blocks (block_start, active_seconds, total_seconds, dirty, updated_at)
         VALUES (?, ?, ?, 1, ?)
         ON CONFLICT (block_start)
         DO UPDATE SET active_seconds = active_seconds + excluded.active_seconds,
                       total_seconds  = total_seconds  + excluded.total_seconds,
                       dirty = 1, updated_at = excluded.updated_at",
    )
    .bind(rfc3339(block_start(now)))
    .bind(active)
    .bind(SAMPLE_SECS)
    .bind(&updated)
    .execute(pool)
    .await?;
    Ok(())
}

// ---- Sync (dirty rows → POST /activity) ----

#[derive(sqlx::FromRow)]
struct DirtyApp {
    day: String,
    app_name: String,
    seconds: i64,
    updated_at: String,
}

#[derive(sqlx::FromRow)]
struct DirtyBlock {
    block_start: String,
    active_seconds: i64,
    total_seconds: i64,
    updated_at: String,
}

/// Push all dirty rows as absolute values; clear `dirty` only where the row
/// wasn't touched while the request was in flight (updated_at guard).
pub async fn sync_once(pool: &SqlitePool) -> anyhow::Result<()> {
    if crate::auth::stored_access().is_none() {
        return Ok(());
    }
    let apps: Vec<DirtyApp> =
        sqlx::query_as("SELECT day, app_name, seconds, updated_at FROM app_usage WHERE dirty = 1")
            .fetch_all(pool)
            .await?;
    let blocks: Vec<DirtyBlock> = sqlx::query_as(
        "SELECT block_start, active_seconds, total_seconds, updated_at
         FROM activity_blocks WHERE dirty = 1",
    )
    .fetch_all(pool)
    .await?;
    if apps.is_empty() && blocks.is_empty() {
        return Ok(());
    }

    let payload = json!({
        "apps": apps.iter().map(|a| json!({
            "day": a.day, "app_name": a.app_name, "seconds": a.seconds,
        })).collect::<Vec<_>>(),
        "blocks": blocks.iter().map(|b| json!({
            "block_start": b.block_start,
            "active_seconds": b.active_seconds,
            "total_seconds": b.total_seconds,
        })).collect::<Vec<_>>(),
    });
    crate::http::post_json("/activity", payload)
        .await
        .map_err(|e| anyhow::anyhow!("activity sync failed: {e}"))?;

    for a in &apps {
        sqlx::query(
            "UPDATE app_usage SET dirty = 0
             WHERE day = ? AND app_name = ? AND updated_at = ?",
        )
        .bind(&a.day)
        .bind(&a.app_name)
        .bind(&a.updated_at)
        .execute(pool)
        .await?;
    }
    for b in &blocks {
        sqlx::query(
            "UPDATE activity_blocks SET dirty = 0
             WHERE block_start = ? AND updated_at = ?",
        )
        .bind(&b.block_start)
        .bind(&b.updated_at)
        .execute(pool)
        .await?;
    }
    Ok(())
}

// ---- The worker ----

/// Background sampler: every 10s, if the current status is `working`, credit
/// the foreground app + input activity; every minute, sync dirty rows.
pub async fn run(state: DesktopState) {
    let mut tick: u32 = 0;
    loop {
        tokio::time::sleep(Duration::from_secs(SAMPLE_SECS as u64)).await;
        tick = tick.wrapping_add(1);

        let status = {
            let on_break = state.on_break.load(Ordering::Relaxed);
            let in_meeting = state.in_meeting.load(Ordering::Relaxed);
            let tracking = state.tracker.lock().await.is_some();
            derive_status(on_break, in_meeting, tracking, state.idle.is_idle())
        };

        if status == "working" {
            // Input in the last sample window? (Duration since last OS input.)
            let input_active = state.idle.idle_for() < Duration::from_secs(SAMPLE_SECS as u64);
            // Foreground lookup is a blocking OS call.
            let app = tokio::task::spawn_blocking(foreground_app_name)
                .await
                .ok()
                .flatten();
            if let Some(app) = app {
                if let Err(e) = accumulate(&state.pool, Utc::now(), &app, input_active).await {
                    tracing::warn!("activity accumulate failed: {e}");
                }
            }
        }

        if tick % SYNC_EVERY_TICKS == 0 {
            if let Err(e) = sync_once(&state.pool).await {
                tracing::debug!("{e}"); // offline is normal; retry next minute
            }
        }
    }
}

// ---- Frontend command ----

#[derive(Serialize)]
pub struct AppRow {
    pub app_name: String,
    pub seconds: i64,
}

#[derive(Serialize)]
pub struct ActivitySummary {
    /// Overall input-activity percentage for today (None = no data yet).
    pub activity_pct: Option<f64>,
    /// Today's foreground time per app, biggest first.
    pub apps: Vec<AppRow>,
}

/// Today's activity from LOCAL data (works offline; the employee sees exactly
/// what is collected about them).
#[tauri::command]
pub async fn activity_today(
    state: tauri::State<'_, DesktopState>,
) -> Result<ActivitySummary, String> {
    summary_for_day(&state.pool, Utc::now())
        .await
        .map_err(|e| e.to_string())
}

pub async fn summary_for_day(
    pool: &SqlitePool,
    now: DateTime<Utc>,
) -> anyhow::Result<ActivitySummary> {
    let day = now.date_naive().to_string();
    let apps: Vec<(String, i64)> = sqlx::query_as(
        "SELECT app_name, seconds FROM app_usage WHERE day = ? ORDER BY seconds DESC",
    )
    .bind(&day)
    .fetch_all(pool)
    .await?;

    let day_prefix = format!("{day}T%");
    let row: Option<(i64, i64)> = sqlx::query_as(
        "SELECT CAST(COALESCE(SUM(active_seconds), 0) AS INTEGER),
                CAST(COALESCE(SUM(total_seconds), 0) AS INTEGER)
         FROM activity_blocks WHERE block_start LIKE ?",
    )
    .bind(&day_prefix)
    .fetch_optional(pool)
    .await?;

    let activity_pct = match row {
        Some((active, total)) if total > 0 => Some((active as f64 / total as f64) * 100.0),
        _ => None,
    };
    Ok(ActivitySummary {
        activity_pct,
        apps: apps
            .into_iter()
            .map(|(app_name, seconds)| AppRow { app_name, seconds })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    fn t(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    #[test]
    fn block_start_truncates_to_ten_minutes() {
        assert_eq!(
            block_start(t("2020-06-01T09:07:59Z")),
            t("2020-06-01T09:00:00Z")
        );
        assert_eq!(
            block_start(t("2020-06-01T09:10:00Z")),
            t("2020-06-01T09:10:00Z")
        );
        assert_eq!(
            block_start(t("2020-06-01T23:59:59Z")),
            t("2020-06-01T23:50:00Z")
        );
    }

    #[tokio::test]
    async fn accumulate_adds_up_and_summary_reports() {
        let pool = db::connect_in_memory().await.unwrap();
        db::migrate(&pool).await.unwrap();

        let now = t("2020-06-01T09:00:05Z");
        // 3 samples in Chrome (2 active, 1 inactive), 1 sample in Slack (active).
        accumulate(&pool, now, "Chrome", true).await.unwrap();
        accumulate(&pool, now, "Chrome", true).await.unwrap();
        accumulate(&pool, now, "Chrome", false).await.unwrap();
        accumulate(&pool, now, "Slack", true).await.unwrap();

        let s = summary_for_day(&pool, now).await.unwrap();
        assert_eq!(s.apps.len(), 2);
        assert_eq!(s.apps[0].app_name, "Chrome");
        assert_eq!(s.apps[0].seconds, 3 * SAMPLE_SECS);
        assert_eq!(s.apps[1].seconds, SAMPLE_SECS);
        // 30 active of 40 total => 75%.
        assert_eq!(s.activity_pct, Some(75.0));
    }

    #[tokio::test]
    async fn dirty_rows_survive_touch_during_sync() {
        let pool = db::connect_in_memory().await.unwrap();
        db::migrate(&pool).await.unwrap();
        let now = t("2020-06-01T09:00:05Z");
        accumulate(&pool, now, "Chrome", true).await.unwrap();

        // Simulate the guard: clearing with a STALE updated_at must not clear.
        let cleared = sqlx::query(
            "UPDATE app_usage SET dirty = 0 WHERE day = ? AND app_name = ? AND updated_at = ?",
        )
        .bind("2020-06-01")
        .bind("Chrome")
        .bind("1999-01-01T00:00:00Z")
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(cleared.rows_affected(), 0);

        let dirty: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM app_usage WHERE dirty = 1")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(dirty.0, 1);
    }
}

//! Interval engine (Rule 2: immutable intervals; monotonic clock for duration).
//!
//! `start_tracking` anchors a wall-clock start (UTC) and a monotonic `Instant`.
//! `stop_tracking` measures elapsed time from the monotonic clock — immune to
//! wall-clock adjustments (NTP, DST, manual changes) — and derives `end_utc`
//! from `start_utc + elapsed`. The finished interval is written to SQLite
//! immediately (local-first); syncing happens later in the background.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::SqlitePool;
use tauri::State;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::idle::IdleHandle;
use crate::interval_repository::{self, Interval};

/// An in-progress tracking session (held in memory only).
#[derive(Debug, Clone)]
pub struct ActiveSession {
    pub user_id: Uuid,
    pub start_utc: DateTime<Utc>,
    pub start_instant: Instant,
}

/// Shared desktop state: local DB pool, active session, break flag, idle handle.
#[derive(Clone)]
pub struct DesktopState {
    pub pool: SqlitePool,
    pub tracker: Arc<Mutex<Option<ActiveSession>>>,
    pub on_break: Arc<AtomicBool>,
    pub in_meeting: Arc<AtomicBool>,
    pub idle: IdleHandle,
}

impl DesktopState {
    pub fn new(pool: SqlitePool, idle: IdleHandle) -> Self {
        Self {
            pool,
            tracker: Arc::new(Mutex::new(None)),
            on_break: Arc::new(AtomicBool::new(false)),
            in_meeting: Arc::new(AtomicBool::new(false)),
            idle,
        }
    }
}

/// Close out a session into an immutable interval using the monotonic clock.
pub fn finalize(session: &ActiveSession) -> Interval {
    let elapsed = chrono::Duration::from_std(session.start_instant.elapsed())
        .unwrap_or_else(|_| chrono::Duration::zero());
    Interval {
        id: Uuid::new_v4(),
        user_id: session.user_id,
        start_utc: session.start_utc,
        end_utc: session.start_utc + elapsed,
        idle: false,
    }
}

#[derive(Serialize)]
pub struct IntervalView {
    pub id: String,
    pub start_utc: String,
    pub end_utc: String,
    pub idle: bool,
    pub worked_seconds: i64,
}

impl From<&Interval> for IntervalView {
    fn from(i: &Interval) -> Self {
        Self {
            id: i.id.to_string(),
            start_utc: i.start_utc.to_rfc3339(),
            end_utc: i.end_utc.to_rfc3339(),
            idle: i.idle,
            worked_seconds: i.worked_seconds(),
        }
    }
}

#[tauri::command]
pub async fn start_tracking(
    state: State<'_, DesktopState>,
    user_id: String,
) -> Result<(), String> {
    let user_id = Uuid::parse_str(&user_id).map_err(|_| "invalid user id".to_string())?;
    let mut guard = state.tracker.lock().await;
    if guard.is_some() {
        return Err("tracking already in progress".to_string());
    }
    *guard = Some(ActiveSession {
        user_id,
        start_utc: Utc::now(),
        start_instant: Instant::now(),
    });
    Ok(())
}

#[tauri::command]
pub async fn stop_tracking(state: State<'_, DesktopState>) -> Result<IntervalView, String> {
    let session = {
        let mut guard = state.tracker.lock().await;
        guard.take()
    }
    .ok_or_else(|| "not currently tracking".to_string())?;

    let interval = finalize(&session);
    // Local-first: persist before anything else (Rule 1).
    interval_repository::insert(&state.pool, &interval)
        .await
        .map_err(|e| e.to_string())?;

    Ok(IntervalView::from(&interval))
}

#[tauri::command]
pub async fn is_tracking(state: State<'_, DesktopState>) -> Result<bool, String> {
    Ok(state.tracker.lock().await.is_some())
}

#[tauri::command]
pub async fn get_total_seconds(
    state: State<'_, DesktopState>,
    user_id: String,
) -> Result<i64, String> {
    let user_id = Uuid::parse_str(&user_id).map_err(|_| "invalid user id".to_string())?;
    interval_repository::total_worked_seconds(&state.pool, user_id)
        .await
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finalize_produces_ordered_interval_from_monotonic_clock() {
        let session = ActiveSession {
            user_id: Uuid::new_v4(),
            start_utc: Utc::now(),
            start_instant: Instant::now(),
        };
        std::thread::sleep(std::time::Duration::from_millis(20));
        let interval = finalize(&session);

        assert_eq!(interval.start_utc, session.start_utc);
        assert!(interval.end_utc > session.start_utc);
        assert!(!interval.idle);
        // end is derived from elapsed monotonic time (~20ms).
        assert!((interval.end_utc - interval.start_utc).num_milliseconds() >= 20);
    }
}

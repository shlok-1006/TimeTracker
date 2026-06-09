//! Interval engine (Rule 2: immutable segments; monotonic clock for durations).
//!
//! A background **recorder** writes status-tagged segments (active | idle |
//! meeting | break) while a session is active. `start_tracking` opens a session;
//! `stop_tracking` closes it. Break/meeting are statuses *within* a session, so
//! they're captured for the timeline; only active+meeting count as worked time.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use sqlx::SqlitePool;
use tauri::State;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::idle::IdleHandle;
use crate::interval_repository::{self, Interval};

/// How often the recorder samples status, and how long a single segment grows
/// before rolling over (so worked totals accrue without one giant row).
const RECORDER_TICK: Duration = Duration::from_secs(10);
const SEGMENT_ROLL: Duration = Duration::from_secs(60);

/// An in-progress tracking session (held in memory only).
#[derive(Debug, Clone)]
pub struct ActiveSession {
    pub user_id: Uuid,
    pub start_utc: DateTime<Utc>,
    pub start_instant: Instant,
}

/// The currently-open recorder segment.
#[derive(Debug, Clone)]
struct RecSegment {
    user_id: Uuid,
    kind: String,
    start_utc: DateTime<Utc>,
    start_instant: Instant,
}

#[derive(Clone)]
pub struct DesktopState {
    pub pool: SqlitePool,
    pub tracker: Arc<Mutex<Option<ActiveSession>>>,
    pub on_break: Arc<AtomicBool>,
    pub in_meeting: Arc<AtomicBool>,
    pub idle: IdleHandle,
    segment: Arc<Mutex<Option<RecSegment>>>,
}

impl DesktopState {
    pub fn new(pool: SqlitePool, idle: IdleHandle) -> Self {
        Self {
            pool,
            tracker: Arc::new(Mutex::new(None)),
            on_break: Arc::new(AtomicBool::new(false)),
            in_meeting: Arc::new(AtomicBool::new(false)),
            idle,
            segment: Arc::new(Mutex::new(None)),
        }
    }
}

/// The interval kind being recorded right now (None = untracked).
pub fn current_kind(
    on_break: bool,
    in_meeting: bool,
    is_tracking: bool,
    is_idle: bool,
) -> Option<&'static str> {
    if !is_tracking {
        None
    } else if on_break {
        Some("break")
    } else if in_meeting {
        Some("meeting")
    } else if is_idle {
        Some("idle")
    } else {
        Some("active")
    }
}

#[tauri::command]
pub async fn start_tracking(
    state: State<'_, DesktopState>,
    user_id: String,
) -> Result<(), String> {
    let user_id = Uuid::parse_str(&user_id).map_err(|_| "invalid user id".to_string())?;
    state.on_break.store(false, Ordering::Relaxed);
    state.in_meeting.store(false, Ordering::Relaxed);
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
pub async fn stop_tracking(state: State<'_, DesktopState>) -> Result<(), String> {
    {
        let mut guard = state.tracker.lock().await;
        if guard.is_none() {
            return Err("not currently tracking".to_string());
        }
        *guard = None;
    }
    state.on_break.store(false, Ordering::Relaxed);
    state.in_meeting.store(false, Ordering::Relaxed);
    // Finalize the open segment immediately so totals update promptly.
    record_tick(&state).await;
    Ok(())
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

/// Background recorder loop: samples status and writes segments.
pub async fn run_recorder(state: DesktopState) {
    loop {
        tokio::time::sleep(RECORDER_TICK).await;
        record_tick(&state).await;
    }
}

/// One recorder step: roll the open segment on status change or after
/// `SEGMENT_ROLL`, recording the elapsed time (monotonic) as an interval.
async fn record_tick(state: &DesktopState) {
    let (tracking, user_id) = {
        let g = state.tracker.lock().await;
        (g.is_some(), g.as_ref().map(|s| s.user_id))
    };
    let kind = current_kind(
        state.on_break.load(Ordering::Relaxed),
        state.in_meeting.load(Ordering::Relaxed),
        tracking,
        state.idle.is_idle(),
    );

    let mut seg = state.segment.lock().await;
    let needs_roll = match seg.as_ref() {
        None => kind.is_some(),
        Some(s) => Some(s.kind.as_str()) != kind || s.start_instant.elapsed() >= SEGMENT_ROLL,
    };
    if !needs_roll {
        return;
    }

    // Finalize the open segment.
    if let Some(s) = seg.take() {
        let elapsed = ChronoDuration::from_std(s.start_instant.elapsed())
            .unwrap_or_else(|_| ChronoDuration::zero());
        if elapsed.num_seconds() > 0 {
            let interval = Interval {
                id: Uuid::new_v4(),
                user_id: s.user_id,
                start_utc: s.start_utc,
                end_utc: s.start_utc + elapsed,
                kind: s.kind,
            };
            if let Err(e) = interval_repository::insert(&state.pool, &interval).await {
                tracing::warn!("failed to record segment: {e}");
            }
        }
    }

    // Open a new segment if we're recording.
    if let (Some(k), Some(uid)) = (kind, user_id) {
        *seg = Some(RecSegment {
            user_id: uid,
            kind: k.to_string(),
            start_utc: Utc::now(),
            start_instant: Instant::now(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_kind_precedence() {
        assert_eq!(current_kind(false, false, false, false), None); // not tracking
        assert_eq!(current_kind(false, false, true, false), Some("active"));
        assert_eq!(current_kind(false, false, true, true), Some("idle"));
        assert_eq!(current_kind(false, true, true, true), Some("meeting")); // meeting > idle
        assert_eq!(current_kind(true, true, true, false), Some("break")); // break > meeting
        assert_eq!(current_kind(true, false, false, false), None); // break needs a session
    }
}

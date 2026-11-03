//! Interval engine (Rule 2: immutable segments; monotonic clock for durations).
//!
//! A background **recorder** writes status-tagged segments (active | idle |
//! break) while a session is active. `start_tracking` opens a session;
//! `stop_tracking` closes it. Break is a status *within* a session, so it's
//! captured for the timeline; only active counts as worked time. (Older
//! versions also recorded `meeting` segments; those are still read and synced.)

use std::path::PathBuf;
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
use crate::outbox::ScreenshotOutbox;
use crate::sync_worker::SharedSyncHealth;

/// How often the recorder samples status, and how long a single segment grows
/// before rolling over (so worked totals accrue without one giant row).
const RECORDER_TICK: Duration = Duration::from_secs(10);
const SEGMENT_ROLL: Duration = Duration::from_secs(60);

/// Written to the data dir just before an update restarts the app, if the timer
/// was running, so the new version picks the session back up by itself.
const RESUME_FILE: &str = "resume-after-update.json";
/// A marker older than this did not come from the restart we just did (e.g. an
/// install that crashed, then the app was opened days later) — never resume it.
const RESUME_WITHIN: ChronoDuration = ChronoDuration::minutes(15);

/// Who was tracking, under which team, when the update restart began.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct ResumeMarker {
    user_id: Uuid,
    team_id: Option<Uuid>,
    written_at: DateTime<Utc>,
}

/// An in-progress tracking session (held in memory only).
#[derive(Debug, Clone)]
pub struct ActiveSession {
    pub user_id: Uuid,
    /// Team the work is being logged under (Feature 4); `None` if unassigned.
    pub team_id: Option<Uuid>,
    pub start_utc: DateTime<Utc>,
    pub start_instant: Instant,
}

/// The currently-open recorder segment.
#[derive(Debug, Clone)]
struct RecSegment {
    user_id: Uuid,
    team_id: Option<Uuid>,
    kind: String,
    start_utc: DateTime<Utc>,
    start_instant: Instant,
}

#[derive(Clone)]
pub struct DesktopState {
    pub pool: SqlitePool,
    pub tracker: Arc<Mutex<Option<ActiveSession>>>,
    pub on_break: Arc<AtomicBool>,
    /// Suppress break reminders for the CURRENT break (reset when a new break
    /// starts). Set by the "Don't remind me" action.
    pub break_reminders_muted: Arc<AtomicBool>,
    pub idle: IdleHandle,
    /// Screenshots captured but not yet uploaded (local first).
    pub outbox: ScreenshotOutbox,
    /// Whether the server answered lately — drives the "offline" banner.
    pub sync_health: SharedSyncHealth,
    /// The app's data directory (database, logs, outbox, resume marker).
    pub data_dir: PathBuf,
    segment: Arc<Mutex<Option<RecSegment>>>,
}

impl DesktopState {
    pub fn new(
        pool: SqlitePool,
        idle: IdleHandle,
        outbox: ScreenshotOutbox,
        data_dir: PathBuf,
    ) -> Self {
        Self {
            pool,
            tracker: Arc::new(Mutex::new(None)),
            on_break: Arc::new(AtomicBool::new(false)),
            break_reminders_muted: Arc::new(AtomicBool::new(false)),
            idle,
            outbox,
            sync_health: SharedSyncHealth::default(),
            data_dir,
            segment: Arc::new(Mutex::new(None)),
        }
    }
}

/// The interval kind being recorded right now (None = untracked).
pub fn current_kind(on_break: bool, is_tracking: bool, is_idle: bool) -> Option<&'static str> {
    if !is_tracking {
        None
    } else if on_break {
        Some("break")
    } else if is_idle {
        Some("idle")
    } else {
        Some("active")
    }
}

#[tauri::command]
pub async fn start_tracking(
    app: tauri::AppHandle,
    state: State<'_, DesktopState>,
    user_id: String,
    team_id: Option<String>,
) -> Result<(), String> {
    let user_id = Uuid::parse_str(&user_id).map_err(|_| "invalid user id".to_string())?;
    // Team is optional (an employee with no teams tracks untagged); when present
    // it must be a valid UUID. Intervals of this session are stamped with it.
    let team_id = match team_id.as_deref() {
        None | Some("") => None,
        Some(t) => Some(Uuid::parse_str(t).map_err(|_| "invalid team id".to_string())?),
    };
    state.on_break.store(false, Ordering::Relaxed);
    let mut guard = state.tracker.lock().await;
    if guard.is_some() {
        return Err("tracking already in progress".to_string());
    }
    *guard = Some(ActiveSession {
        user_id,
        team_id,
        start_utc: Utc::now(),
        start_instant: Instant::now(),
    });
    // The "you haven't started the timer" reminder has just been answered.
    crate::reminder::close(&app, "tracking started");
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
    // Finalize the open segment immediately so totals update promptly.
    record_tick(&state).await;
    Ok(())
}

/// Before an update restarts the app: if the timer is running, note whose
/// session it is, then stop it and write out the open segment, so no tracked
/// time is held only in memory when the process goes away.
pub async fn suspend_for_update(state: &DesktopState) -> anyhow::Result<()> {
    let path = state.data_dir.join(RESUME_FILE);
    let session = state.tracker.lock().await.take();
    let Some(session) = session else {
        let _ = std::fs::remove_file(&path);
        return Ok(());
    };
    let marker = ResumeMarker {
        user_id: session.user_id,
        team_id: session.team_id,
        written_at: Utc::now(),
    };
    let written = serde_json::to_vec(&marker)
        .map_err(anyhow::Error::from)
        .and_then(|bytes| std::fs::write(&path, bytes).map_err(anyhow::Error::from));
    state.on_break.store(false, Ordering::Relaxed);
    record_tick(state).await;
    written.map_err(|e| e.context("save the running session for after the update"))
}

/// Should a marker written at `written_at` for `user` be resumed now, with
/// `signed_in` the account currently signed in?
fn should_resume(marker: &ResumeMarker, now: DateTime<Utc>, signed_in: Option<Uuid>) -> bool {
    let age = now - marker.written_at;
    signed_in == Some(marker.user_id) && age >= ChronoDuration::zero() && age <= RESUME_WITHIN
}

/// At startup (and after an install that failed): pick up the session an
/// update restart interrupted, for the same signed-in account. The marker is
/// removed either way, so it can only ever resume once.
pub async fn resume_after_update(state: &DesktopState) {
    let path = state.data_dir.join(RESUME_FILE);
    let Ok(bytes) = std::fs::read(&path) else {
        return;
    };
    let _ = std::fs::remove_file(&path);
    let marker: ResumeMarker = match serde_json::from_slice(&bytes) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!("ignoring an unreadable resume-after-update marker: {e}");
            return;
        }
    };
    if !should_resume(&marker, Utc::now(), crate::auth::signed_in_user_id()) {
        tracing::info!("not resuming tracking: the marker is stale or for another account");
        return;
    }
    let mut guard = state.tracker.lock().await;
    if guard.is_none() {
        *guard = Some(ActiveSession {
            user_id: marker.user_id,
            team_id: marker.team_id,
            start_utc: Utc::now(),
            start_instant: Instant::now(),
        });
        tracing::info!("resumed tracking after the update");
    }
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
    let (tracking, user_id, team_id) = {
        let g = state.tracker.lock().await;
        (
            g.is_some(),
            g.as_ref().map(|s| s.user_id),
            g.as_ref().and_then(|s| s.team_id),
        )
    };
    let kind = current_kind(
        state.on_break.load(Ordering::Relaxed),
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
                team_id: s.team_id,
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
            team_id,
            kind: k.to_string(),
            start_utc: Utc::now(),
            start_instant: Instant::now(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marker(minutes_ago: i64) -> (ResumeMarker, DateTime<Utc>) {
        let now = Utc::now();
        (
            ResumeMarker {
                user_id: Uuid::from_u128(7),
                team_id: None,
                written_at: now - ChronoDuration::minutes(minutes_ago),
            },
            now,
        )
    }

    #[test]
    fn resumes_right_after_the_update_for_the_same_account() {
        let (m, now) = marker(1);
        assert!(should_resume(&m, now, Some(Uuid::from_u128(7))));
    }

    #[test]
    fn never_resumes_a_stale_marker_or_someone_else() {
        let (m, now) = marker(60);
        assert!(!should_resume(&m, now, Some(Uuid::from_u128(7))), "too old");
        let (m, now) = marker(1);
        assert!(
            !should_resume(&m, now, Some(Uuid::from_u128(8))),
            "other account"
        );
        assert!(!should_resume(&m, now, None), "signed out");
        let (m, now) = marker(-5);
        assert!(
            !should_resume(&m, now, Some(Uuid::from_u128(7))),
            "from the future"
        );
    }

    #[test]
    fn current_kind_precedence() {
        assert_eq!(current_kind(false, false, false), None); // not tracking
        assert_eq!(current_kind(false, true, false), Some("active"));
        assert_eq!(current_kind(false, true, true), Some("idle"));
        assert_eq!(current_kind(true, true, true), Some("break")); // break > idle
        assert_eq!(current_kind(true, false, false), None); // break needs a session
    }
}

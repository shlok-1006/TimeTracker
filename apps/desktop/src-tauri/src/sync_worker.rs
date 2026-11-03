//! Background sync worker (Rule 4: SQLite -> sync queue -> API -> PostgreSQL).
//!
//! Periodically pushes the signed-in user's pending intervals to
//! `POST /intervals` and drains the screenshot `outbox`, via the authenticated
//! `http` helper (which refreshes the token on expiry). Never blocks user
//! actions; at-least-once delivery (server inserts are idempotent).
//!
//! It also keeps [`SyncHealth`] — whether the last contact with the server
//! worked — so the app can say "you're offline, N hours are waiting" instead of
//! leaving someone on a VPN that blocks us to find out from HR.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::Serialize;
use tauri::State;
use uuid::Uuid;

use crate::auth;
use crate::http;
use crate::interval_repository::{self, Interval};
use crate::timer::DesktopState;

const SYNC_INTERVAL: Duration = Duration::from_secs(15);
/// Max intervals per POST. A large offline backlog is drained in bounded chunks
/// so request bodies stay small and each insert is fast — one giant request
/// could time out or exceed the server's body limit and then never get acked.
const SYNC_CHUNK: usize = 500;

/// Outcome of the most recent attempts to reach the server (any worker).
#[derive(Debug, Default)]
pub struct SyncHealth {
    last_ok: Option<DateTime<Utc>>,
    last_err: Option<(DateTime<Utc>, String)>,
    /// Whether the most recent attempt failed. Kept as its own flag rather than
    /// compared from the timestamps: two attempts in the same clock tick (the
    /// interval sync and the screenshot drain run back to back) would tie.
    latest_failed: bool,
}

pub type SharedSyncHealth = Arc<Mutex<SyncHealth>>;

impl SyncHealth {
    pub fn record_ok(&mut self) {
        self.last_ok = Some(Utc::now());
        self.latest_failed = false;
    }

    pub fn record_err(&mut self, err: impl Into<String>) {
        self.last_err = Some((Utc::now(), err.into()));
        self.latest_failed = true;
    }

    /// Did the latest attempt fail?
    pub fn offline(&self) -> bool {
        self.latest_failed
    }
}

/// Record a server round-trip's outcome. Never fails: health is diagnostics.
pub fn note<T, E: std::fmt::Display>(health: &SharedSyncHealth, result: &Result<T, E>) {
    if let Ok(mut h) = health.lock() {
        match result {
            Ok(_) => h.record_ok(),
            Err(e) => h.record_err(e.to_string()),
        }
    }
}

/// What the app shows about data not yet on the server.
#[derive(Debug, Serialize)]
pub struct SyncStatus {
    pub pending_intervals: usize,
    /// Tracked (non-break) seconds still only on this computer.
    pub pending_tracked_seconds: i64,
    pub oldest_pending_utc: Option<String>,
    pub pending_screenshots: i64,
    /// The latest attempt to reach the server failed.
    pub offline: bool,
    pub last_synced_utc: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Serialize)]
struct IntervalPayload {
    id: Uuid,
    start_utc: String,
    end_utc: String,
    kind: String,
    team_id: Option<Uuid>,
}

impl From<&Interval> for IntervalPayload {
    fn from(i: &Interval) -> Self {
        Self {
            id: i.id,
            start_utc: i.start_utc.to_rfc3339(),
            end_utc: i.end_utc.to_rfc3339(),
            kind: i.kind.clone(),
            team_id: i.team_id,
        }
    }
}

pub async fn run(state: DesktopState) {
    loop {
        tokio::time::sleep(SYNC_INTERVAL).await;
        if let Err(e) = sync_all(&state).await {
            tracing::warn!("sync failed (will retry): {e}");
        }
    }
}

/// One full pass: intervals, then queued screenshots. Both run even if the
/// first fails (screenshot storage may be reachable when the API isn't, or the
/// other way round); the first error is returned.
async fn sync_all(state: &DesktopState) -> anyhow::Result<()> {
    let intervals = sync_intervals(state).await;
    let shots = state.outbox.drain(&state.pool).await;
    match &shots {
        Ok(r) if r.uploaded > 0 => note::<_, String>(&state.sync_health, &Ok(())),
        Ok(_) => {}
        Err(e) => note::<(), _>(&state.sync_health, &Err(e)),
    }
    intervals.and(shots.map(|_| ()))
}

async fn sync_intervals(state: &DesktopState) -> anyhow::Result<()> {
    let Some(user_id) = auth::signed_in_user_id() else {
        return Ok(()); // not logged in
    };
    let pending = interval_repository::pending_sync(&state.pool, user_id).await?;
    if pending.is_empty() {
        return Ok(());
    }

    // Drain in bounded chunks. Each chunk is acked (marked synced) on its own,
    // so a large backlog makes steady progress and a failure only retries the
    // un-acked remainder rather than the whole pile.
    for chunk in pending.chunks(SYNC_CHUNK) {
        let payload: Vec<IntervalPayload> = chunk.iter().map(IntervalPayload::from).collect();
        let sent = http::post_json("/intervals", serde_json::to_value(&payload)?).await;
        note(&state.sync_health, &sent);
        sent.map_err(|e| anyhow::anyhow!(e))?;

        let ids: Vec<Uuid> = chunk.iter().map(|i| i.id).collect();
        interval_repository::mark_synced(&state.pool, &ids).await?;
        tracing::info!("synced {} interval(s)", ids.len());
    }
    Ok(())
}

/// Tracked (non-break) seconds across `intervals`.
fn tracked_seconds(intervals: &[Interval]) -> i64 {
    intervals
        .iter()
        .filter(|i| i.kind != "break")
        .map(|i| (i.end_utc - i.start_utc).num_seconds().max(0))
        .sum()
}

async fn status(state: &DesktopState) -> anyhow::Result<SyncStatus> {
    let (offline, last_synced_utc, last_error) = match state.sync_health.lock() {
        Ok(h) => (
            h.offline(),
            h.last_ok.map(|t| t.to_rfc3339()),
            h.last_err.as_ref().map(|(_, e)| e.clone()),
        ),
        Err(_) => (false, None, None),
    };
    let Some(user_id) = auth::signed_in_user_id() else {
        return Ok(SyncStatus {
            pending_intervals: 0,
            pending_tracked_seconds: 0,
            oldest_pending_utc: None,
            pending_screenshots: 0,
            offline,
            last_synced_utc,
            last_error,
        });
    };
    let pending = interval_repository::pending_sync(&state.pool, user_id).await?;
    Ok(SyncStatus {
        pending_intervals: pending.len(),
        pending_tracked_seconds: tracked_seconds(&pending),
        oldest_pending_utc: pending.first().map(|i| i.start_utc.to_rfc3339()),
        pending_screenshots: state.outbox.pending_count(&state.pool, user_id).await?,
        offline,
        last_synced_utc,
        last_error,
    })
}

/// What's still waiting to reach the server, and whether it can be reached.
#[tauri::command]
pub async fn sync_status(state: State<'_, DesktopState>) -> Result<SyncStatus, String> {
    status(&state).await.map_err(|e| e.to_string())
}

/// "Sync now": run a full pass immediately (e.g. right after the VPN drops)
/// and report what is left. A failed pass is not an error here — it shows up
/// as `offline` with the reason in `last_error`.
#[tauri::command]
pub async fn sync_now(state: State<'_, DesktopState>) -> Result<SyncStatus, String> {
    if let Err(e) = sync_all(&state).await {
        tracing::warn!("manual sync failed: {e}");
    }
    status(&state).await.map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interval(start: &str, end: &str, kind: &str) -> Interval {
        Interval {
            id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            start_utc: DateTime::parse_from_rfc3339(start)
                .unwrap()
                .with_timezone(&Utc),
            end_utc: DateTime::parse_from_rfc3339(end)
                .unwrap()
                .with_timezone(&Utc),
            kind: kind.to_string(),
            team_id: None,
        }
    }

    #[test]
    fn health_is_offline_only_when_the_latest_attempt_failed() {
        let mut h = SyncHealth::default();
        assert!(!h.offline(), "no attempts yet is not offline");
        h.record_err("request failed");
        assert!(h.offline());
        h.record_ok();
        assert!(!h.offline());
        h.record_err("server returned 502 Bad Gateway");
        assert!(h.offline());
    }

    #[test]
    fn pending_time_counts_everything_but_breaks() {
        let items = vec![
            interval("2026-01-01T09:00:00Z", "2026-01-01T10:00:00Z", "active"),
            interval("2026-01-01T10:00:00Z", "2026-01-01T10:30:00Z", "meeting"),
            interval("2026-01-01T10:30:00Z", "2026-01-01T10:40:00Z", "idle"),
            interval("2026-01-01T10:40:00Z", "2026-01-01T11:40:00Z", "break"),
        ];
        assert_eq!(tracked_seconds(&items), 3600 + 1800 + 600);
    }
}

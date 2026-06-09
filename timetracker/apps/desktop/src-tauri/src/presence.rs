//! Presence: status derivation, break toggle, and the heartbeat worker (STEP 3).
//!
//! State transitions:
//!   working <-> idle   (automatic, via idle detection)
//!   working <-> break  (manual, via `set_break`)
//! Break takes precedence over idle while active.

use std::sync::atomic::Ordering;
use std::time::Duration;

use tauri::State;

use crate::auth;
use crate::http;
use crate::timer::DesktopState;

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(45);

/// Pure status derivation.
///
/// Precedence: break > not_working (timer stopped) > meeting > idle > working.
/// - `break`       — manual; timer stopped.
/// - `not_working` — logged in but the timer is not running.
/// - `meeting`     — manual; timer running, idle suppressed (meetings are work).
/// - `idle`        — timer running but no input past the idle threshold.
/// - `working`     — timer running with recent input.
/// (`not_logged_in` is derived server-side when heartbeats stop.)
pub fn derive_status(
    on_break: bool,
    in_meeting: bool,
    is_tracking: bool,
    is_idle: bool,
) -> &'static str {
    if !is_tracking {
        "not_working"
    } else if on_break {
        "break"
    } else if in_meeting {
        "meeting"
    } else if is_idle {
        "idle"
    } else {
        "working"
    }
}

#[tauri::command]
pub fn set_break(state: State<'_, DesktopState>, on: bool) -> Result<(), String> {
    state.on_break.store(on, Ordering::Relaxed);
    Ok(())
}

#[tauri::command]
pub fn is_on_break(state: State<'_, DesktopState>) -> Result<bool, String> {
    Ok(state.on_break.load(Ordering::Relaxed))
}

#[tauri::command]
pub fn set_meeting(state: State<'_, DesktopState>, on: bool) -> Result<(), String> {
    state.in_meeting.store(on, Ordering::Relaxed);
    Ok(())
}

#[tauri::command]
pub fn is_in_meeting(state: State<'_, DesktopState>) -> Result<bool, String> {
    Ok(state.in_meeting.load(Ordering::Relaxed))
}

/// Current derived status string (for the UI badge).
#[tauri::command]
pub async fn current_status(state: State<'_, DesktopState>) -> Result<String, String> {
    let on_break = state.on_break.load(Ordering::Relaxed);
    let in_meeting = state.in_meeting.load(Ordering::Relaxed);
    let tracking = state.tracker.lock().await.is_some();
    Ok(derive_status(on_break, in_meeting, tracking, state.idle.is_idle()).to_string())
}

/// Background heartbeat: POST /presence immediately, then every 45s while
/// logged in (beating right away keeps the dashboard fresh after launch).
pub async fn run(state: DesktopState) {
    loop {
        if let Err(e) = send_heartbeat(&state).await {
            tracing::warn!("presence heartbeat failed (will retry): {e}");
        }
        tokio::time::sleep(HEARTBEAT_INTERVAL).await;
    }
}

/// Push a single heartbeat now (used by the worker and after login/toggles so
/// the server reflects the new status immediately).
#[tauri::command]
pub async fn heartbeat_now(state: State<'_, DesktopState>) -> Result<(), String> {
    send_heartbeat(&state).await.map_err(|e| e.to_string())
}

async fn send_heartbeat(state: &DesktopState) -> anyhow::Result<()> {
    // Only beat while logged in; otherwise the server derives `not_logged_in`
    // after the grace period.
    if auth::stored_access().is_none() {
        return Ok(());
    }

    let on_break = state.on_break.load(Ordering::Relaxed);
    let in_meeting = state.in_meeting.load(Ordering::Relaxed);
    let tracking = state.tracker.lock().await.is_some();
    let status = derive_status(on_break, in_meeting, tracking, state.idle.is_idle());

    http::post_json("/presence", serde_json::json!({ "status": status }))
        .await
        .map_err(|e| anyhow::anyhow!(e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_transitions() {
        // (on_break, in_meeting, is_tracking, is_idle)
        assert_eq!(derive_status(false, false, true, false), "working"); // tracking + active
        assert_eq!(derive_status(false, false, true, true), "idle"); // tracking + no input
        assert_eq!(derive_status(false, false, false, false), "not_working"); // timer off
        assert_eq!(derive_status(true, false, false, false), "not_working"); // not tracking wins
        assert_eq!(derive_status(false, true, true, true), "meeting"); // meeting > idle
        assert_eq!(derive_status(true, true, true, false), "break"); // break > meeting
    }
}

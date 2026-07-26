//! Presence: status derivation, break toggle, and the heartbeat worker (STEP 3).
//!
//! State transitions:
//!   working <-> idle   (automatic, via idle detection)
//!   working <-> break  (manual, via `set_break`)
//! Break takes precedence over idle while active.

use std::sync::atomic::Ordering;
use std::time::Duration;

use tauri::{AppHandle, State};
use tauri_plugin_notification::NotificationExt;

use crate::auth;
use crate::http;
use crate::timer::DesktopState;

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(45);
/// How often to nudge someone who's still on a break (until they resume or mute).
const BREAK_REMINDER_INTERVAL: Duration = Duration::from_secs(180); // 3 minutes
/// How often to nudge a signed-in user whose machine is in use but who hasn't
/// started the timer.
const NOT_TRACKING_REMINDER_INTERVAL: Duration = Duration::from_secs(300); // 5 minutes

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
    // Starting a fresh break re-enables reminders — a new break should nudge
    // again even if the previous one was muted with "Don't remind me".
    if on {
        state.break_reminders_muted.store(false, Ordering::Relaxed);
    }
    Ok(())
}

/// Silence break reminders for the CURRENT break (the "Don't remind me" action).
/// The next break re-enables them.
#[tauri::command]
pub fn mute_break_reminders(state: State<'_, DesktopState>) -> Result<(), String> {
    state.break_reminders_muted.store(true, Ordering::Relaxed);
    Ok(())
}

/// Whether reminders are currently muted (so the UI can hide the button once
/// the user has opted out for this break).
#[tauri::command]
pub fn break_reminders_muted(state: State<'_, DesktopState>) -> Result<bool, String> {
    Ok(state.break_reminders_muted.load(Ordering::Relaxed))
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

/// Background reminder: while a break is actually in progress (session running
/// + on break) and the user hasn't muted it, send a native notification every
/// `BREAK_REMINDER_INTERVAL` nudging them to resume. "Don't remind me" mutes the
/// current break via `mute_break_reminders`.
pub async fn run_break_reminders(app: AppHandle, state: DesktopState) {
    loop {
        tokio::time::sleep(BREAK_REMINDER_INTERVAL).await;
        let on_break = state.on_break.load(Ordering::Relaxed);
        let muted = state.break_reminders_muted.load(Ordering::Relaxed);
        let tracking = state.tracker.lock().await.is_some();
        if on_break && tracking && !muted {
            if let Err(e) = app
                .notification()
                .builder()
                .title("Still on a break?")
                .body("TimeTracker is paused. Resume when you're back — or pick \"Don't remind me\" in the app to silence this break.")
                .show()
            {
                tracing::warn!("break reminder notification failed: {e}");
            }
        }
    }
}

/// Background reminder: while the user is signed in and actively using the
/// machine (not idle) but the timer is NOT running, nudge them to start
/// tracking. Skipped when signed out (nothing to start), when the machine is
/// idle (they're away), or when the timer is already running.
pub async fn run_not_tracking_reminders(app: AppHandle, state: DesktopState) {
    loop {
        tokio::time::sleep(NOT_TRACKING_REMINDER_INTERVAL).await;
        // Only nudge a signed-in user — a logged-out user has nothing to start.
        if auth::stored_refresh().is_none() {
            continue;
        }
        let tracking = state.tracker.lock().await.is_some();
        // "System is on but the app isn't": machine in active use with the timer
        // stopped.
        if !tracking && !state.idle.is_idle() {
            if let Err(e) = app
                .notification()
                .builder()
                .title("You haven't started the timer")
                .body("Your computer is active but TimeTracker isn't recording. Open the app and click Start tracking.")
                .show()
            {
                tracing::warn!("not-tracking reminder notification failed: {e}");
            }
        }
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

    // Report the machine's IANA timezone so the server can bucket the hours
    // display at this employee's local 4 AM boundary (matches the desktop's own
    // local figure). Falls back to UTC if detection fails.
    let timezone = iana_time_zone::get_timezone().unwrap_or_else(|_| "UTC".to_string());
    http::post_json(
        "/presence",
        serde_json::json!({ "status": status, "timezone": timezone }),
    )
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

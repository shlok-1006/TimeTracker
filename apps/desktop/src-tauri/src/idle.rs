//! Idle detection (STEP 3) via the OS-native idle timer (`user-idle`).
//!
//! Reports how long since the last system-wide input event. On macOS this uses
//! CoreGraphics (`CGEventSourceSecondsSinceLastEventType`) and needs **no**
//! special permission; on Windows it uses `GetLastInputInfo`. This replaces the
//! previous global mouse/keyboard polling, which on macOS required Input
//! Monitoring / Accessibility permission and — when that was missing — never
//! observed activity, leaving the app permanently stuck in "idle".
//!
//! A call in progress (see `call_detect`) holds idle off: [`IdleHandle::idle_for`]
//! counts from the later of the last input and the last moment a call was seen.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use user_idle::UserIdle;

/// Default idle threshold if `TIMETRACKER_IDLE_THRESHOLD_SECS` is unset.
/// 5 minutes of no mouse/keyboard input => idle.
const DEFAULT_THRESHOLD_SECS: u64 = 300;

#[derive(Clone)]
pub struct IdleHandle {
    threshold: Duration,
    /// When a Meet call or Slack huddle was last seen in progress.
    call_seen: Arc<Mutex<Option<Instant>>>,
}

/// Idle time once calls are taken into account: a call is as good as input,
/// so idle runs from whichever was more recent.
fn idle_after_call(since_input: Duration, since_call: Option<Duration>) -> Duration {
    since_call.map_or(since_input, |c| since_input.min(c))
}

impl IdleHandle {
    pub fn new(threshold: Duration) -> Self {
        Self {
            threshold,
            call_seen: Arc::new(Mutex::new(None)),
        }
    }

    /// A call is in progress right now (called by `call_detect` on each look).
    pub fn note_call(&self) {
        if let Ok(mut seen) = self.call_seen.lock() {
            *seen = Some(Instant::now());
        }
    }

    /// How long the person has been idle: since the last input, or since a call
    /// was last seen if that is more recent.
    pub fn idle_for(&self) -> Duration {
        let since_call = self
            .call_seen
            .lock()
            .ok()
            .and_then(|seen| seen.map(|t| t.elapsed()));
        idle_after_call(self.since_input(), since_call)
    }

    /// Build from the `TIMETRACKER_IDLE_THRESHOLD_SECS` env var (configurable).
    pub fn from_env() -> Self {
        let secs = std::env::var("TIMETRACKER_IDLE_THRESHOLD_SECS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .filter(|s| *s > 0)
            .unwrap_or(DEFAULT_THRESHOLD_SECS);
        Self::new(Duration::from_secs(secs))
    }

    /// Time since the last system-wide input event, calls aside. On a transient
    /// failure or unsupported platform we report zero, so the user is treated as
    /// **active** rather than being falsely marked idle.
    pub fn since_input(&self) -> Duration {
        // Headless Linux guard: with no display server, the X11 screensaver
        // query inside `user-idle` can SIGSEGV in native code (a crashed C call
        // never becomes an Err). No display => nothing to observe => active.
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            if std::env::var_os("DISPLAY").is_none()
                && std::env::var_os("WAYLAND_DISPLAY").is_none()
            {
                return Duration::ZERO;
            }
        }
        match UserIdle::get_time() {
            Ok(t) => Duration::from_secs(t.as_seconds()),
            Err(e) => {
                tracing::debug!("idle query failed, assuming active: {e}");
                Duration::ZERO
            }
        }
    }

    /// No input for this long counts as idle.
    pub fn threshold(&self) -> Duration {
        self.threshold
    }

    pub fn is_idle(&self) -> bool {
        self.idle_for() >= self.threshold
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_input_is_not_idle_under_large_threshold() {
        // We are running this test now, so the last input is recent (or the
        // query fails on a headless CI host and reports zero) — either way the
        // session is well under a one-day threshold.
        let handle = IdleHandle::new(Duration::from_secs(86_400));
        assert!(!handle.is_idle());
    }

    #[test]
    fn a_call_holds_idle_off_and_idle_restarts_when_it_ends() {
        let min = |m: u64| Duration::from_secs(m * 60);
        // No call: plain time since input.
        assert_eq!(idle_after_call(min(40), None), min(40));
        // 40 minutes without touching anything, call still going (seen 10s ago).
        assert_eq!(
            idle_after_call(min(40), Some(Duration::from_secs(10))),
            Duration::from_secs(10)
        );
        // The call ended 3 minutes ago: idle is 3 minutes, not 40.
        assert_eq!(idle_after_call(min(40), Some(min(3))), min(3));
        // Input after the call: input is the more recent.
        assert_eq!(idle_after_call(min(1), Some(min(30))), min(1));
    }

    #[test]
    fn noting_a_call_ends_idle() {
        let handle = IdleHandle::new(Duration::from_secs(1));
        handle.note_call();
        assert!(handle.idle_for() < Duration::from_secs(1));
        assert!(!handle.is_idle());
    }

    #[test]
    fn zero_threshold_reports_idle() {
        // A zero threshold means any elapsed time counts as idle.
        let handle = IdleHandle::new(Duration::ZERO);
        assert!(handle.is_idle());
    }
}

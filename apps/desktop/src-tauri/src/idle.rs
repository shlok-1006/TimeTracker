//! Idle detection (STEP 3) via the OS-native idle timer (`user-idle`).
//!
//! Reports how long since the last system-wide input event. On macOS this uses
//! CoreGraphics (`CGEventSourceSecondsSinceLastEventType`) and needs **no**
//! special permission; on Windows it uses `GetLastInputInfo`. This replaces the
//! previous global mouse/keyboard polling, which on macOS required Input
//! Monitoring / Accessibility permission and — when that was missing — never
//! observed activity, leaving the app permanently stuck in "idle".

use std::time::Duration;

use user_idle::UserIdle;

/// Default idle threshold if `TIMETRACKER_IDLE_THRESHOLD_SECS` is unset.
/// 3 minutes of no mouse/keyboard input => idle.
const DEFAULT_THRESHOLD_SECS: u64 = 180;

#[derive(Clone)]
pub struct IdleHandle {
    threshold: Duration,
}

impl IdleHandle {
    pub fn new(threshold: Duration) -> Self {
        Self { threshold }
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

    /// Time since the last system-wide input event. On a transient failure or
    /// unsupported platform we report zero, so the user is treated as **active**
    /// rather than being falsely marked idle.
    pub fn idle_for(&self) -> Duration {
        match UserIdle::get_time() {
            Ok(t) => Duration::from_secs(t.as_seconds()),
            Err(e) => {
                tracing::debug!("idle query failed, assuming active: {e}");
                Duration::ZERO
            }
        }
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
    fn zero_threshold_reports_idle() {
        // A zero threshold means any elapsed time counts as idle.
        let handle = IdleHandle::new(Duration::ZERO);
        assert!(handle.is_idle());
    }
}

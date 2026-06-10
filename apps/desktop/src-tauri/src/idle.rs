//! Idle detection (STEP 3) via the `device_query` crate.
//!
//! A background thread samples mouse position + pressed keys every ~2s. Any
//! change marks "activity" (updates a monotonic `Instant`). `is_idle` is true
//! once no activity has occurred for the configured threshold.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use device_query::{DeviceQuery, DeviceState};

/// Default idle threshold if `TIMETRACKER_IDLE_THRESHOLD_SECS` is unset.
/// 3 minutes of no mouse/keyboard input => idle.
const DEFAULT_THRESHOLD_SECS: u64 = 180;

#[derive(Clone)]
pub struct IdleHandle {
    last_activity: Arc<Mutex<Instant>>,
    threshold: Duration,
}

impl IdleHandle {
    pub fn new(threshold: Duration) -> Self {
        Self {
            last_activity: Arc::new(Mutex::new(Instant::now())),
            threshold,
        }
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

    pub fn mark_active(&self) {
        if let Ok(mut last) = self.last_activity.lock() {
            *last = Instant::now();
        }
    }

    pub fn idle_for(&self) -> Duration {
        self.last_activity
            .lock()
            .map(|t| t.elapsed())
            .unwrap_or_default()
    }

    pub fn is_idle(&self) -> bool {
        self.idle_for() >= self.threshold
    }
}

/// Spawn the input sampler on a dedicated OS thread (device_query is blocking).
pub fn spawn_sampler(handle: IdleHandle) {
    std::thread::spawn(move || {
        let device = DeviceState::new();
        let mut last_mouse = device.get_mouse().coords;
        let mut last_keys = device.get_keys();
        loop {
            std::thread::sleep(Duration::from_secs(2));
            let mouse = device.get_mouse().coords;
            let keys = device.get_keys();
            if mouse != last_mouse || keys != last_keys {
                handle.mark_active();
                last_mouse = mouse;
                last_keys = keys;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_idle_after_threshold_and_resets_on_activity() {
        let handle = IdleHandle::new(Duration::from_millis(60));
        assert!(!handle.is_idle(), "fresh handle is active");

        std::thread::sleep(Duration::from_millis(90));
        assert!(handle.is_idle(), "idle after threshold elapses");

        handle.mark_active();
        assert!(!handle.is_idle(), "activity resets idle state");
    }
}

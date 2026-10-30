//! Idle notice: tell the employee when their time stops counting as work.
//!
//! With the timer running, no keyboard or mouse input for the idle threshold
//! records the time as `idle`, not work — and until now nothing said so; the
//! employee only found out from their hours. Once idle sets in, the reminder
//! window (always on top, over every app) says "You're idle". When input comes
//! back while it is still up, the same window turns into "You were idle for
//! 14 minutes" and waits for Ok, so whoever returns to the desk sees it in
//! whatever app they go back to.
//!
//! Not shown on a break or in Meeting mode (idle isn't recorded then) or with
//! the timer off.

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use tauri::AppHandle;

use crate::meeting_check::describe;
use crate::reminder::{self, Reminder};
use crate::timer::DesktopState;

const TICK: Duration = Duration::from_secs(10);
const KIND: &str = "idle";

/// Where the current idle stretch stands.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct IdleNotice {
    /// When the current idle stretch began (last input), if one is under way.
    since: Option<Instant>,
}

#[derive(Debug, PartialEq, Eq)]
enum Step {
    Nothing,
    /// Idle has just set in.
    WentIdle {
        idle_for: Duration,
    },
    /// Input is back after an idle stretch of this length.
    CameBack {
        was_idle_for: Duration,
    },
    /// Idle stopped counting another way (break, meeting, timer off).
    Withdraw,
}

impl IdleNotice {
    /// `watching` — the timer runs and idle is being recorded (not on a break,
    /// not in a meeting). `idle_for` — time since the last input.
    fn step(
        &mut self,
        watching: bool,
        idle_for: Duration,
        threshold: Duration,
        now: Instant,
    ) -> Step {
        if !watching {
            return match self.since.take() {
                Some(_) => Step::Withdraw,
                None => Step::Nothing,
            };
        }
        match (self.since, idle_for >= threshold) {
            (None, true) => {
                self.since = Some(now.checked_sub(idle_for).unwrap_or(now));
                Step::WentIdle { idle_for }
            }
            (Some(since), false) => {
                self.since = None;
                // Input resumed `idle_for` ago, so the stretch ended then.
                let ended = now.checked_sub(idle_for).unwrap_or(now);
                Step::CameBack {
                    was_idle_for: ended.saturating_duration_since(since),
                }
            }
            _ => Step::Nothing,
        }
    }
}

fn went_idle(idle_for: Duration) -> Reminder {
    Reminder {
        kind: KIND,
        title: "You're idle".into(),
        body: format!(
            "No keyboard or mouse activity for {}, so TimeTracker is recording this as idle time, not work. It counts as work again as soon as you use the computer.",
            describe(idle_for)
        ),
    }
}

fn came_back(was_idle_for: Duration) -> Reminder {
    Reminder {
        kind: KIND,
        title: "Welcome back".into(),
        body: format!(
            "You were idle for {}. That time was recorded as idle, not work.",
            describe(was_idle_for)
        ),
    }
}

/// Background loop: say so when idle sets in, and how long it lasted on return.
pub async fn run(app: AppHandle, state: DesktopState) {
    let mut notice = IdleNotice::default();
    loop {
        tokio::time::sleep(TICK).await;
        let tracking = state.tracker.lock().await.is_some();
        let watching = tracking
            && !state.on_break.load(Ordering::Relaxed)
            && !state.in_meeting.load(Ordering::Relaxed);
        let step = notice.step(
            watching,
            state.idle.idle_for(),
            state.idle.threshold(),
            Instant::now(),
        );
        match step {
            Step::WentIdle { idle_for } => {
                tracing::info!(secs = idle_for.as_secs(), "idle: notifying");
                reminder::show(&app, went_idle(idle_for));
            }
            // Only update a notice that is still up: if it was already
            // dismissed, the user has seen it and doesn't need a second one.
            Step::CameBack { was_idle_for } if reminder::showing(KIND) => {
                tracing::info!(secs = was_idle_for.as_secs(), "idle: back from idle");
                reminder::show(&app, came_back(was_idle_for));
            }
            Step::Withdraw if reminder::showing(KIND) => {
                reminder::close(&app, "idle no longer recorded");
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: Duration = Duration::from_secs(300);

    fn secs(s: u64) -> Duration {
        Duration::from_secs(s)
    }

    #[test]
    fn notifies_once_when_idle_sets_in() {
        let t0 = Instant::now();
        let mut n = IdleNotice::default();
        assert_eq!(n.step(true, secs(60), T, t0), Step::Nothing);
        assert_eq!(
            n.step(true, secs(305), T, t0 + secs(245)),
            Step::WentIdle {
                idle_for: secs(305)
            }
        );
        // Still idle: no repeat.
        assert_eq!(n.step(true, secs(900), T, t0 + secs(840)), Step::Nothing);
    }

    #[test]
    fn coming_back_reports_the_whole_stretch() {
        let t0 = Instant::now();
        let mut n = IdleNotice::default();
        // Last input at t0; idle noticed 5m10s later.
        let _ = n.step(true, secs(310), T, t0 + secs(310));
        // Input resumed at t0 + 14m, noticed 8 seconds after that.
        assert_eq!(
            n.step(true, secs(8), T, t0 + secs(14 * 60 + 8)),
            Step::CameBack {
                was_idle_for: secs(14 * 60)
            }
        );
        assert_eq!(
            n.step(true, secs(18), T, t0 + secs(14 * 60 + 18)),
            Step::Nothing
        );
    }

    #[test]
    fn breaks_meetings_and_a_stopped_timer_withdraw_it() {
        let t0 = Instant::now();
        let mut n = IdleNotice::default();
        let _ = n.step(true, secs(310), T, t0);
        assert_eq!(n.step(false, secs(320), T, t0 + secs(10)), Step::Withdraw);
        // Nothing under way: nothing to withdraw, and idle while not watching
        // never notifies.
        assert_eq!(n.step(false, secs(999), T, t0 + secs(20)), Step::Nothing);
    }

    #[test]
    fn a_new_idle_stretch_notifies_again() {
        let t0 = Instant::now();
        let mut n = IdleNotice::default();
        let _ = n.step(true, secs(310), T, t0);
        let _ = n.step(true, secs(5), T, t0 + secs(600));
        assert_eq!(
            n.step(true, secs(301), T, t0 + secs(1200)),
            Step::WentIdle {
                idle_for: secs(301)
            }
        );
    }
}

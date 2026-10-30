//! "Are you still in a meeting?" — an hourly check while Meeting mode is on.
//!
//! Meeting mode suppresses idle detection (meetings are work), so a meeting
//! that ended without anyone switching the mode off keeps counting the rest of
//! the day as meeting time, keyboard or not. Once the mode has been on for
//! [`CHECK_EVERY`], the reminder window asks; "Yes" asks again an hour later,
//! "Turn it off" ends the meeting there and then.
//!
//! The question stays on screen until it is answered (see `reminder`) — it is
//! not answered for the user, and Meeting mode stays on meanwhile.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tauri::{AppHandle, State};

use crate::reminder::{self, Reminder};
use crate::timer::DesktopState;

/// How long Meeting mode runs before the first (and each later) check.
/// Overridable with `TIMETRACKER_MEETING_CHECK_SECS` for testing.
const CHECK_EVERY: Duration = Duration::from_secs(60 * 60);
/// How often the background loop looks. The question can arrive up to this
/// late, which is fine against an hour.
const TICK: Duration = Duration::from_secs(20);

fn check_every() -> Duration {
    std::env::var("TIMETRACKER_MEETING_CHECK_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|s| *s > 0)
        .map(Duration::from_secs)
        .unwrap_or(CHECK_EVERY)
}

/// Where the hourly check stands for the current meeting.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MeetingCheck {
    /// When Meeting mode was switched on (for "on for 2 hours").
    started: Option<Instant>,
    /// When to ask next. `None` while the question is on screen.
    next_ask: Option<Instant>,
    /// The question is on screen, waiting for an answer.
    asking: bool,
}

pub type SharedMeetingCheck = Arc<Mutex<MeetingCheck>>;

/// What the loop should do this tick.
#[derive(Debug, PartialEq, Eq)]
enum Step {
    Nothing,
    Ask {
        on_for: Duration,
    },
    /// The meeting ended some other way (button, timer stopped) while the
    /// question was up — take it down.
    Withdraw,
}

impl MeetingCheck {
    fn started(&mut self, now: Instant, every: Duration) {
        *self = Self {
            started: Some(now),
            next_ask: Some(now + every),
            asking: false,
        };
    }

    fn step(&mut self, in_meeting: bool, now: Instant, every: Duration) -> Step {
        if !in_meeting {
            let was_asking = self.asking;
            *self = Self::default();
            return if was_asking {
                Step::Withdraw
            } else {
                Step::Nothing
            };
        }
        // Meeting mode can be switched on by paths that don't go through
        // `set_meeting`; start the clock the first time we see it.
        let Some(started) = self.started else {
            self.started(now, every);
            return Step::Nothing;
        };
        match self.next_ask {
            Some(at) if now >= at => {
                self.next_ask = None;
                self.asking = true;
                Step::Ask {
                    on_for: now.saturating_duration_since(started),
                }
            }
            _ => Step::Nothing,
        }
    }

    fn confirmed(&mut self, now: Instant, every: Duration) {
        self.next_ask = Some(now + every);
        self.asking = false;
    }
}

/// Called whenever Meeting mode is switched on or off. Returns whether the
/// question was on screen, so the caller can take it down.
pub fn on_meeting_toggled(state: &DesktopState, on: bool) -> bool {
    let Ok(mut c) = state.meeting_check.lock() else {
        return false;
    };
    let was_asking = c.asking;
    if on {
        c.started(Instant::now(), check_every());
    } else {
        *c = MeetingCheck::default();
    }
    was_asking
}

/// "On for 1 hour", "on for 2 hours 15 minutes".
fn describe(on_for: Duration) -> String {
    let mins = on_for.as_secs() / 60;
    let (h, m) = (mins / 60, mins % 60);
    let hours = match h {
        0 => String::new(),
        1 => "1 hour".to_string(),
        n => format!("{n} hours"),
    };
    let minutes = match m {
        1 => "1 minute".to_string(),
        n => format!("{n} minutes"),
    };
    match (hours.is_empty(), m) {
        (true, _) => minutes,
        (false, 0) => hours,
        (false, _) => format!("{hours} {minutes}"),
    }
}

fn question(on_for: Duration) -> Reminder {
    Reminder {
        kind: "meeting",
        title: "Are you still in a meeting?".into(),
        body: format!(
            "Meeting mode has been on for {}. Turn it off if the meeting has ended, so your time is recorded as normal work.",
            describe(on_for)
        ),
    }
}

/// Background loop: ask once Meeting mode has run for an hour.
pub async fn run(app: AppHandle, state: DesktopState) {
    loop {
        tokio::time::sleep(TICK).await;
        let tracking = state.tracker.lock().await.is_some();
        let in_meeting = tracking && state.in_meeting.load(Ordering::Relaxed);
        let step = match state.meeting_check.lock() {
            Ok(mut c) => c.step(in_meeting, Instant::now(), check_every()),
            Err(_) => continue,
        };
        match step {
            Step::Ask { on_for } => {
                tracing::info!(
                    minutes = on_for.as_secs() / 60,
                    "asking if still in a meeting"
                );
                reminder::show(&app, question(on_for));
            }
            Step::Withdraw => reminder::close(&app, "meeting ended"),
            Step::Nothing => {}
        }
    }
}

/// "Yes" — still in the meeting. Ask again in an hour.
#[tauri::command]
pub fn still_in_meeting(app: AppHandle, state: State<'_, DesktopState>) -> Result<(), String> {
    if let Ok(mut c) = state.meeting_check.lock() {
        c.confirmed(Instant::now(), check_every());
    }
    reminder::close(&app, "still in meeting");
    Ok(())
}

/// "Turn it off" — the meeting is over; back to normal tracking.
#[tauri::command]
pub async fn end_meeting(app: AppHandle, state: State<'_, DesktopState>) -> Result<(), String> {
    state.in_meeting.store(false, Ordering::Relaxed);
    on_meeting_toggled(&state, false);
    reminder::close(&app, "meeting turned off");
    // Tell the server now rather than at the next 45s beat, as the main
    // window's own toggle does.
    crate::presence::heartbeat_now(state)
        .await
        .map_err(|e| tracing::warn!("heartbeat after ending meeting failed: {e}"))
        .ok();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: Duration = Duration::from_secs(3600);

    #[test]
    fn asks_once_after_an_hour_and_waits_for_an_answer() {
        let t0 = Instant::now();
        let mut c = MeetingCheck::default();
        c.started(t0, HOUR);
        assert_eq!(
            c.step(true, t0 + Duration::from_secs(59 * 60), HOUR),
            Step::Nothing
        );
        assert_eq!(c.step(true, t0 + HOUR, HOUR), Step::Ask { on_for: HOUR });
        // Unanswered: never asked twice, Meeting mode left alone.
        assert_eq!(c.step(true, t0 + 3 * HOUR, HOUR), Step::Nothing);
    }

    #[test]
    fn yes_asks_again_an_hour_later() {
        let t0 = Instant::now();
        let mut c = MeetingCheck::default();
        c.started(t0, HOUR);
        let _ = c.step(true, t0 + HOUR, HOUR);
        let answered = t0 + HOUR + Duration::from_secs(300);
        c.confirmed(answered, HOUR);
        assert_eq!(
            c.step(true, answered + Duration::from_secs(30 * 60), HOUR),
            Step::Nothing
        );
        assert_eq!(
            c.step(true, answered + HOUR, HOUR),
            Step::Ask {
                on_for: answered + HOUR - t0
            }
        );
    }

    #[test]
    fn ending_the_meeting_elsewhere_withdraws_the_question() {
        let t0 = Instant::now();
        let mut c = MeetingCheck::default();
        c.started(t0, HOUR);
        let _ = c.step(true, t0 + HOUR, HOUR);
        assert_eq!(c.step(false, t0 + HOUR, HOUR), Step::Withdraw);
        assert_eq!(c, MeetingCheck::default());
        // Nothing to withdraw when nothing was asked.
        assert_eq!(c.step(false, t0 + HOUR, HOUR), Step::Nothing);
    }

    #[test]
    fn a_meeting_started_by_another_path_gets_a_clock() {
        let t0 = Instant::now();
        let mut c = MeetingCheck::default();
        assert_eq!(c.step(true, t0, HOUR), Step::Nothing);
        assert_eq!(c.step(true, t0 + HOUR, HOUR), Step::Ask { on_for: HOUR });
    }

    #[test]
    fn durations_read_naturally() {
        assert_eq!(describe(Duration::from_secs(3600)), "1 hour");
        assert_eq!(
            describe(Duration::from_secs(2 * 3600 + 15 * 60)),
            "2 hours 15 minutes"
        );
        assert_eq!(describe(Duration::from_secs(61 * 60)), "1 hour 1 minute");
        assert_eq!(describe(Duration::from_secs(5 * 60)), "5 minutes");
    }
}

//! Presence status enum, mirroring the Postgres `presence_status` type.
//!
//! `working | idle | break` are client-reportable; `not_logged_in` is derived
//! by the server when a heartbeat goes stale and is never accepted from a client.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PresenceStatus {
    Working,
    Idle,
    Break,
    Meeting,
    NotWorking,
    NotLoggedIn,
}

impl PresenceStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            PresenceStatus::Working => "working",
            PresenceStatus::Idle => "idle",
            PresenceStatus::Break => "break",
            PresenceStatus::Meeting => "meeting",
            PresenceStatus::NotWorking => "not_working",
            PresenceStatus::NotLoggedIn => "not_logged_in",
        }
    }

    /// Statuses a client is allowed to report via the heartbeat (everything
    /// except `not_logged_in`, which is server-derived).
    pub fn is_reportable(&self) -> bool {
        !matches!(self, PresenceStatus::NotLoggedIn)
    }

    /// True when the timer is running (any active session — working, idle,
    /// meeting, or break), as opposed to stopped / signed out. Used to mark
    /// attendance present the moment a user starts tracking.
    pub fn is_tracking(&self) -> bool {
        matches!(
            self,
            PresenceStatus::Working
                | PresenceStatus::Idle
                | PresenceStatus::Meeting
                | PresenceStatus::Break
        )
    }
}

impl fmt::Display for PresenceStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for PresenceStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "working" => Ok(PresenceStatus::Working),
            "idle" => Ok(PresenceStatus::Idle),
            "break" => Ok(PresenceStatus::Break),
            "meeting" => Ok(PresenceStatus::Meeting),
            "not_working" => Ok(PresenceStatus::NotWorking),
            "not_logged_in" => Ok(PresenceStatus::NotLoggedIn),
            other => Err(format!("invalid presence status: {other}")),
        }
    }
}

/// Roster-only status: they tracked time today and have stopped — "End My Day", or the app
/// closed afterwards. Never stored and never accepted from a client.
pub const LOGGED_OUT: &str = "logged_out";
/// Roster-only status: an approved leave covers today and they aren't tracking.
pub const ON_LEAVE: &str = "on_leave";

/// The status the live roster shows for one person.
///
/// `live` is the heartbeat-derived status (`working` … `not_logged_in`). Someone tracking right
/// now always shows that, even on a leave day — the board says what is actually happening. Off
/// the clock there are only three answers:
///   * an approved leave today → `on_leave` (a half day off after a morning's work included);
///   * already worked today → `logged_out` ("End My Day", app open or closed);
///   * otherwise → `not_logged_in` — including the app being open with the timer never started
///     (`not_working`), which HR reads as "hasn't started", same as not having opened the app.
pub fn roster_status(live: &str, worked_today: bool, on_leave_today: bool) -> String {
    let tracking = live
        .parse::<PresenceStatus>()
        .map(|s| s.is_tracking())
        .unwrap_or(false);
    if tracking {
        live.to_string()
    } else if on_leave_today {
        ON_LEAVE.to_string()
    } else if worked_today {
        LOGGED_OUT.to_string()
    } else {
        PresenceStatus::NotLoggedIn.as_str().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracking_always_shows_as_is() {
        for live in ["working", "idle", "break", "meeting"] {
            assert_eq!(roster_status(live, true, true), live);
            assert_eq!(roster_status(live, false, false), live);
        }
    }

    #[test]
    fn finished_for_the_day_reads_logged_out() {
        // "End My Day" with the app still open, and after the app is closed.
        assert_eq!(roster_status("not_working", true, false), LOGGED_OUT);
        assert_eq!(roster_status("not_logged_in", true, false), LOGGED_OUT);
    }

    #[test]
    fn not_started_today_is_unchanged() {
        assert_eq!(
            roster_status("not_logged_in", false, false),
            "not_logged_in"
        );
        // App open, timer never started today: reads as not logged in, not "Not working".
        assert_eq!(roster_status("not_working", false, false), "not_logged_in");
    }

    #[test]
    fn leave_wins_off_the_clock() {
        assert_eq!(roster_status("not_logged_in", false, true), ON_LEAVE);
        assert_eq!(roster_status("not_working", false, true), ON_LEAVE);
        // Half day: worked the morning, off for the afternoon.
        assert_eq!(roster_status("not_logged_in", true, true), ON_LEAVE);
    }

    #[test]
    fn roster_only_statuses_are_not_reportable() {
        assert!(LOGGED_OUT.parse::<PresenceStatus>().is_err());
        assert!(ON_LEAVE.parse::<PresenceStatus>().is_err());
    }

    #[test]
    fn round_trips_and_reportability() {
        for s in [
            PresenceStatus::Working,
            PresenceStatus::Idle,
            PresenceStatus::Break,
            PresenceStatus::Meeting,
            PresenceStatus::NotWorking,
            PresenceStatus::NotLoggedIn,
        ] {
            assert_eq!(s.as_str().parse::<PresenceStatus>().unwrap(), s);
        }
        assert!(PresenceStatus::Working.is_reportable());
        assert!(PresenceStatus::NotWorking.is_reportable());
        assert!(!PresenceStatus::NotLoggedIn.is_reportable());
    }
}

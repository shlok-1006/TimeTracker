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

#[cfg(test)]
mod tests {
    use super::*;

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

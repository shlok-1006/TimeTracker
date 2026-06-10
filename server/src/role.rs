//! The system role enum — the single Rust source of truth, mirroring the
//! Postgres `user_role` type and the TypeScript `Role` union.
//!
//! Never use magic strings (CLAUDE.md). Conversions to/from the DB go through
//! `as_str` / `FromStr` so the text representation is defined in exactly one place.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserRole {
    Employee,
    ProjectManager,
    Hr,
}

impl UserRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            UserRole::Employee => "employee",
            UserRole::ProjectManager => "project_manager",
            UserRole::Hr => "hr",
        }
    }

    /// Roles permitted to use the admin dashboard (HR + project managers).
    /// Employees use the desktop app only.
    pub fn is_dashboard(&self) -> bool {
        matches!(self, UserRole::ProjectManager | UserRole::Hr)
    }
}

impl fmt::Display for UserRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for UserRole {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "employee" => Ok(UserRole::Employee),
            "project_manager" => Ok(UserRole::ProjectManager),
            "hr" => Ok(UserRole::Hr),
            other => Err(format!("invalid user role: {other}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_string() {
        for role in [UserRole::Employee, UserRole::ProjectManager, UserRole::Hr] {
            assert_eq!(role.as_str().parse::<UserRole>().unwrap(), role);
        }
    }

    #[test]
    fn dashboard_membership() {
        assert!(!UserRole::Employee.is_dashboard());
        assert!(UserRole::ProjectManager.is_dashboard());
        assert!(UserRole::Hr.is_dashboard());
    }

    #[test]
    fn rejects_unknown_role() {
        assert!("admin".parse::<UserRole>().is_err());
    }
}

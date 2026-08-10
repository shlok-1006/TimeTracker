//! The system role enum — the single Rust source of truth, mirroring the
//! Postgres `user_role` type and the TypeScript `Role` union.
//!
//! Never use magic strings (CLAUDE.md). Conversions to/from the DB go through
//! `as_str` / `FromStr` so the text representation is defined in exactly one place.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Ordered by authority: `Employee < ProjectManager < Hr < Admin`.
///
/// `Admin` is the seat above HR. It has everything HR has, and two things HR does not: it may
/// remove an HR account, and it cannot be removed by one. Nothing else distinguishes them — the
/// point is oversight of HR, not a second set of features.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserRole {
    Employee,
    ProjectManager,
    Hr,
    Admin,
}

impl UserRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            UserRole::Employee => "employee",
            UserRole::ProjectManager => "project_manager",
            UserRole::Hr => "hr",
            UserRole::Admin => "admin",
        }
    }

    /// Roles permitted to use the admin dashboard (staff: PM, HR, admin).
    /// Employees use the desktop app only.
    pub fn is_dashboard(&self) -> bool {
        matches!(
            self,
            UserRole::ProjectManager | UserRole::Hr | UserRole::Admin
        )
    }

    /// Does this role carry at least `other`'s authority?
    ///
    /// The single place ranking is expressed. Derived from the enum order rather than written
    /// out as a match, so adding a tier cannot leave one comparison behind — the bug that turns
    /// a new role into a silent privilege hole.
    pub fn at_least(self, other: UserRole) -> bool {
        self >= other
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
            "admin" => Ok(UserRole::Admin),
            other => Err(format!("invalid user role: {other}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_string() {
        for role in [
            UserRole::Employee,
            UserRole::ProjectManager,
            UserRole::Hr,
            UserRole::Admin,
        ] {
            assert_eq!(role.as_str().parse::<UserRole>().unwrap(), role);
        }
    }

    #[test]
    fn dashboard_membership() {
        assert!(!UserRole::Employee.is_dashboard());
        assert!(UserRole::ProjectManager.is_dashboard());
        assert!(UserRole::Hr.is_dashboard());
        assert!(UserRole::Admin.is_dashboard());
    }

    #[test]
    fn ordered_by_authority() {
        assert!(UserRole::Admin > UserRole::Hr);
        assert!(UserRole::Hr > UserRole::ProjectManager);
        assert!(UserRole::ProjectManager > UserRole::Employee);
        assert!(UserRole::Admin.at_least(UserRole::Hr));
        assert!(!UserRole::Hr.at_least(UserRole::Admin));
    }

    #[test]
    fn rejects_unknown_role() {
        // This used to assert "admin" was unknown, and was right until migration 0046 made it a
        // role. A parser test that names a real value is a trap for the next person to add one.
        assert!("superuser".parse::<UserRole>().is_err());
        assert!("".parse::<UserRole>().is_err());
        assert!(
            "HR".parse::<UserRole>().is_err(),
            "case is not normalised here"
        );
    }
}

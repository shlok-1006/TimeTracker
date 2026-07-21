//! The employment-type enum — an HR classification of a worker (employee,
//! contractor, or intern), orthogonal to the RBAC `UserRole`. Mirrors the
//! Postgres `employment_type` type and the TypeScript `EmploymentType` union.
//!
//! Never use magic strings (CLAUDE.md). Conversions to/from the DB go through
//! `as_str` / `FromStr` so the text representation is defined in exactly one place.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmploymentType {
    Employee,
    Contractor,
    Intern,
}

impl EmploymentType {
    pub fn as_str(&self) -> &'static str {
        match self {
            EmploymentType::Employee => "employee",
            EmploymentType::Contractor => "contractor",
            EmploymentType::Intern => "intern",
        }
    }
}

impl fmt::Display for EmploymentType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for EmploymentType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "employee" => Ok(EmploymentType::Employee),
            "contractor" => Ok(EmploymentType::Contractor),
            "intern" => Ok(EmploymentType::Intern),
            other => Err(format!("invalid employment type: {other}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_string() {
        for t in [
            EmploymentType::Employee,
            EmploymentType::Contractor,
            EmploymentType::Intern,
        ] {
            assert_eq!(t.as_str().parse::<EmploymentType>().unwrap(), t);
        }
    }

    #[test]
    fn rejects_unknown_type() {
        assert!("freelancer".parse::<EmploymentType>().is_err());
    }
}

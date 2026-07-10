//! Idempotent development seed: ensures 1 HR + 1 Employee user exist.
//!
//! Run with the database up:
//!   cargo run -p server --bin seed
//!
//! DEVELOPMENT ONLY. Refuses to run without `ALLOW_SEED=true` (SEC-32) so it can
//! never accidentally seed production with known credentials. Passwords may be
//! overridden via `SEED_HR_PASSWORD` / `SEED_EMPLOYEE_PASSWORD`; the defaults
//! below are for local dev only.
//!   HR        -> hr@timetracker.local        / $SEED_HR_PASSWORD
//!   Employee  -> employee@timetracker.local  / $SEED_EMPLOYEE_PASSWORD

use anyhow::Context;

use server::auth::hash_password;
use server::db;
use server::role::UserRole;

const HR_EMAIL: &str = "hr@timetracker.local";
const EMPLOYEE_EMAIL: &str = "employee@timetracker.local";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();

    // SEC-32: never seed known credentials unless explicitly allowed.
    if std::env::var("ALLOW_SEED").ok().as_deref() != Some("true") {
        anyhow::bail!(
            "refusing to run the seed binary without ALLOW_SEED=true — this is a \
             development-only tool and must never be run against production"
        );
    }

    let hr_password =
        std::env::var("SEED_HR_PASSWORD").unwrap_or_else(|_| "ChangeMe!HR1".to_string());
    let employee_password =
        std::env::var("SEED_EMPLOYEE_PASSWORD").unwrap_or_else(|_| "ChangeMe!Emp1".to_string());

    let database_url = std::env::var("DATABASE_URL").context("DATABASE_URL must be set")?;
    let pool = db::connect(&database_url, 5).await?;
    // Skip migrations when the target DB is already migrated (e.g. the deployed
    // server applied them). Avoids a spurious checksum mismatch when the seed is
    // built on a different platform (Windows CRLF) than the server (Linux LF).
    if std::env::var("SEED_SKIP_MIGRATIONS").ok().as_deref() != Some("true") {
        db::run_migrations(&pool).await?;
    }

    let hr = db::users::upsert(
        &pool,
        "HR Admin",
        HR_EMAIL,
        &hash_password(&hr_password)?,
        UserRole::Hr,
        None,
    )
    .await?;

    let employee = db::users::upsert(
        &pool,
        "Employee One",
        EMPLOYEE_EMAIL,
        &hash_password(&employee_password)?,
        UserRole::Employee,
        None,
    )
    .await?;

    println!("seeded HR       : {} <{}>", hr.id, hr.email);
    println!("seeded Employee : {} <{}>", employee.id, employee.email);
    println!("done.");
    Ok(())
}

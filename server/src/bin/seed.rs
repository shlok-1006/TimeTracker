//! Idempotent development seed: ensures 1 HR + 1 Employee user exist.
//!
//! Run with the database up:
//!   cargo run -p server --bin seed
//!
//! Credentials (development only — change for any real deployment):
//!   HR        -> hr@timetracker.local        / ChangeMe!HR1
//!   Employee  -> employee@timetracker.local  / ChangeMe!Emp1

use anyhow::Context;

use server::auth::hash_password;
use server::db;
use server::role::UserRole;

const HR_EMAIL: &str = "hr@timetracker.local";
const HR_PASSWORD: &str = "ChangeMe!HR1";
const EMPLOYEE_EMAIL: &str = "employee@timetracker.local";
const EMPLOYEE_PASSWORD: &str = "ChangeMe!Emp1";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();

    let database_url = std::env::var("DATABASE_URL").context("DATABASE_URL must be set")?;
    let pool = db::connect(&database_url, 5).await?;
    db::run_migrations(&pool).await?;

    let hr = db::users::upsert(
        &pool,
        "HR Admin",
        HR_EMAIL,
        &hash_password(HR_PASSWORD)?,
        UserRole::Hr,
        None,
    )
    .await?;

    let employee = db::users::upsert(
        &pool,
        "Employee One",
        EMPLOYEE_EMAIL,
        &hash_password(EMPLOYEE_PASSWORD)?,
        UserRole::Employee,
        None,
    )
    .await?;

    println!("seeded HR       : {} <{}>", hr.id, hr.email);
    println!("seeded Employee : {} <{}>", employee.id, employee.email);
    println!("done.");
    Ok(())
}

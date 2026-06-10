//! Database access layer (Rule 7: SQLx only, repository pattern, migrations).
//!
//! STEP 0 wires up the connection pool and runs migrations on startup.
//! Repositories (users, hours, screenshots, …) are added in later steps under
//! this module.

pub mod analysis_results;
pub mod audit;
pub mod intervals;
pub mod linear_repository;
pub mod presence;
pub mod refresh_tokens;
pub mod screenshots;
pub mod ticket_requests;
pub mod users;

use std::time::Duration;

use anyhow::Context;
use sqlx::{postgres::PgPoolOptions, PgPool};

/// Create the PostgreSQL connection pool.
pub async fn connect(database_url: &str, max_connections: u32) -> anyhow::Result<PgPool> {
    PgPoolOptions::new()
        .max_connections(max_connections)
        .acquire_timeout(Duration::from_secs(5))
        .connect(database_url)
        .await
        .context("failed to connect to PostgreSQL")
}

/// Apply all pending migrations from `server/migrations`.
///
/// Migrations are embedded into the binary at compile time, so a deployed
/// server can migrate itself without the source tree present.
pub async fn run_migrations(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::migrate!("./migrations")
        .run(pool)
        .await
        .context("failed to run database migrations")
}

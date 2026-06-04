//! Local SQLite access (Rule 1: local-first source of truth).
//!
//! Uses SQLx with runtime queries (the local schema differs from the server's
//! Postgres schema, so we do not share a compile-time `DATABASE_URL`).

use std::path::Path;
use std::str::FromStr;

use anyhow::Context;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;

/// Open (creating if needed) the SQLite database at `path`.
pub async fn connect(path: &Path) -> anyhow::Result<SqlitePool> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }

    let url = format!("sqlite://{}", path.to_string_lossy());
    let options = SqliteConnectOptions::from_str(&url)
        .context("invalid sqlite url")?
        .create_if_missing(true);

    SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await
        .context("failed to open local SQLite database")
}

/// Open an in-memory database (used by tests).
#[cfg(test)]
pub async fn connect_in_memory() -> anyhow::Result<SqlitePool> {
    // A single shared connection keeps the in-memory DB alive for the pool.
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .context("failed to open in-memory SQLite database")
}

/// Apply embedded migrations (creates `intervals` + `interval_sync`).
pub async fn migrate(pool: &SqlitePool) -> anyhow::Result<()> {
    sqlx::migrate!("./migrations")
        .run(pool)
        .await
        .context("failed to run local migrations")
}

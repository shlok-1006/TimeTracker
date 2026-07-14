//! Local SQLite access (Rule 1: local-first source of truth).
//!
//! Uses SQLx with runtime queries (the local schema differs from the server's
//! Postgres schema, so we do not share a compile-time `DATABASE_URL`).

use std::path::Path;
use std::str::FromStr;

use anyhow::Context;
use sha2::{Digest, Sha384};
use sqlx::migrate::{MigrationType, Migrator};
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
    let migrator = sqlx::migrate!("./migrations");
    repair_line_ending_checksums(pool, &migrator)
        .await
        .context("failed to repair migration checksums")?;
    migrator
        .run(pool)
        .await
        .context("failed to run local migrations")
}

/// Repair `_sqlx_migrations` checksums that differ from the embedded
/// migrations ONLY by line endings.
///
/// Builds prior to the `*.sql text eol=lf` rule in .gitattributes embedded the
/// migrations with CRLF line endings, so databases they created store SHA-384
/// checksums of the CRLF bytes. Newer builds embed LF, sqlx sees a checksum
/// mismatch and refuses to migrate, and every upgraded install crash-loops on
/// startup. When a stored checksum matches a line-ending variant of the same
/// SQL, rewrite it to the embedded value; any other mismatch is a genuine
/// content change and is left for the migrator to reject.
async fn repair_line_ending_checksums(
    pool: &SqlitePool,
    migrator: &Migrator,
) -> anyhow::Result<()> {
    let bookkeeping_exists: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = '_sqlx_migrations'",
    )
    .fetch_optional(pool)
    .await
    .context("check for _sqlx_migrations table")?;
    if bookkeeping_exists.is_none() {
        // Fresh database — nothing applied yet, nothing to repair.
        return Ok(());
    }

    for migration in migrator.iter() {
        if matches!(migration.migration_type, MigrationType::ReversibleDown) {
            continue;
        }

        let stored: Option<Vec<u8>> =
            sqlx::query_scalar("SELECT checksum FROM _sqlx_migrations WHERE version = ?")
                .bind(migration.version)
                .fetch_optional(pool)
                .await
                .context("read stored migration checksum")?;
        let Some(stored) = stored else {
            continue; // Not applied yet — the migrator will apply it normally.
        };
        if stored == *migration.checksum {
            continue;
        }

        let lf = migration.sql.replace("\r\n", "\n");
        let crlf = lf.replace('\n', "\r\n");
        let matches_variant = [lf, crlf]
            .iter()
            .any(|variant| *stored == *Sha384::digest(variant.as_bytes()));
        if matches_variant {
            sqlx::query("UPDATE _sqlx_migrations SET checksum = ? WHERE version = ?")
                .bind(migration.checksum.as_ref())
                .bind(migration.version)
                .execute(pool)
                .await
                .context("rewrite migration checksum")?;
            tracing::info!(
                version = migration.version,
                "repaired line-ending-only migration checksum drift"
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The first embedded up-migration (used to tamper with its bookkeeping row).
    fn first_migration(migrator: &Migrator) -> &sqlx::migrate::Migration {
        migrator
            .iter()
            .find(|m| !matches!(m.migration_type, MigrationType::ReversibleDown))
            .expect("at least one embedded migration")
    }

    #[tokio::test]
    async fn migrate_repairs_crlf_checksum_drift() {
        let pool = connect_in_memory().await.unwrap();
        migrate(&pool).await.unwrap();

        // Simulate a DB created by an old CRLF build: same SQL, CRLF checksum.
        let migrator = sqlx::migrate!("./migrations");
        let first = first_migration(&migrator);
        let crlf_sql = first.sql.replace("\r\n", "\n").replace('\n', "\r\n");
        let crlf_checksum = Sha384::digest(crlf_sql.as_bytes()).to_vec();
        assert_ne!(crlf_checksum, first.checksum.to_vec());
        sqlx::query("UPDATE _sqlx_migrations SET checksum = ? WHERE version = ?")
            .bind(&crlf_checksum)
            .bind(first.version)
            .execute(&pool)
            .await
            .unwrap();

        migrate(&pool)
            .await
            .expect("line-ending-only checksum drift must self-heal");

        let stored: Vec<u8> =
            sqlx::query_scalar("SELECT checksum FROM _sqlx_migrations WHERE version = ?")
                .bind(first.version)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(stored, first.checksum.to_vec());
    }

    #[tokio::test]
    async fn migrate_rejects_genuine_checksum_mismatch() {
        let pool = connect_in_memory().await.unwrap();
        migrate(&pool).await.unwrap();

        let migrator = sqlx::migrate!("./migrations");
        let first = first_migration(&migrator);
        sqlx::query("UPDATE _sqlx_migrations SET checksum = x'DEADBEEF' WHERE version = ?")
            .bind(first.version)
            .execute(&pool)
            .await
            .unwrap();

        let err = migrate(&pool)
            .await
            .expect_err("a genuine content change must still fail loudly");
        assert!(
            format!("{err:#}").contains("previously applied but has been modified"),
            "unexpected error: {err:#}"
        );
    }
}

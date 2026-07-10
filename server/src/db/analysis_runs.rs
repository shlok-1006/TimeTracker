//! Range-analysis run tracking (Rule 7: SQLx, repository pattern).
//!
//! A "run" is one admin-triggered pass that analyzes EVERY working screenshot
//! in an arbitrary `[from, to)` window for one employee. The background task
//! bumps the counters as it goes; the admin UI polls `get` for a progress bar.

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;

#[derive(Serialize)]
pub struct RangeRun {
    pub id: Uuid,
    pub user_id: Uuid,
    pub from_utc: DateTime<Utc>,
    pub to_utc: DateTime<Utc>,
    pub status: String,
    pub total: i32,
    pub analyzed: i32,
    pub skipped: i32,
    pub failed: i32,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

/// Create a run in `running` state with the known screenshot total.
pub async fn create(
    pool: &PgPool,
    user_id: Uuid,
    requested_by: Uuid,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    total: i32,
) -> Result<Uuid, AppError> {
    let row = sqlx::query!(
        r#"
        INSERT INTO analysis_range_runs (user_id, requested_by, from_utc, to_utc, total)
        VALUES ($1, $2, $3, $4, $5)
        RETURNING id
        "#,
        user_id,
        requested_by,
        from,
        to,
        total
    )
    .fetch_one(pool)
    .await?;
    Ok(row.id)
}

pub async fn get(pool: &PgPool, id: Uuid) -> Result<Option<RangeRun>, AppError> {
    let row = sqlx::query!(
        r#"
        SELECT id, user_id, from_utc, to_utc, status, total, analyzed, skipped,
               failed, error, created_at, finished_at
        FROM analysis_range_runs WHERE id = $1
        "#,
        id
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| RangeRun {
        id: r.id,
        user_id: r.user_id,
        from_utc: r.from_utc,
        to_utc: r.to_utc,
        status: r.status,
        total: r.total,
        analyzed: r.analyzed,
        skipped: r.skipped,
        failed: r.failed,
        error: r.error,
        created_at: r.created_at,
        finished_at: r.finished_at,
    }))
}

/// Record one screenshot's outcome. Called after each analysis attempt so
/// polling clients see live progress.
pub async fn bump(
    pool: &PgPool,
    id: Uuid,
    analyzed: i32,
    skipped: i32,
    failed: i32,
) -> Result<(), AppError> {
    sqlx::query!(
        r#"
        UPDATE analysis_range_runs
        SET analyzed = analyzed + $2, skipped = skipped + $3, failed = failed + $4
        WHERE id = $1
        "#,
        id,
        analyzed,
        skipped,
        failed
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Terminal transition: mark the run completed or failed (with a message).
pub async fn finish(pool: &PgPool, id: Uuid, error: Option<&str>) -> Result<(), AppError> {
    let status = if error.is_some() {
        "failed"
    } else {
        "completed"
    };
    sqlx::query!(
        r#"
        UPDATE analysis_range_runs
        SET status = $2, error = $3, finished_at = now()
        WHERE id = $1
        "#,
        id,
        status,
        error
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Is there already a live run for this user? Prevents an admin double-click
/// from launching two overlapping (and double-billing) runs.
pub async fn has_running_for_user(pool: &PgPool, user_id: Uuid) -> Result<bool, AppError> {
    let row = sqlx::query!(
        r#"SELECT COUNT(*) AS "count!" FROM analysis_range_runs
           WHERE user_id = $1 AND status = 'running'"#,
        user_id
    )
    .fetch_one(pool)
    .await?;
    Ok(row.count > 0)
}

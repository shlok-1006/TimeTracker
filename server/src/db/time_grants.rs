//! Manual "grace" time grants (Rule 2: kept separate from the immutable interval
//! log). HR / a project manager can add time to an employee's current week with
//! a reason; `hours_summary` folds the current week's grace into the week total.

use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;

#[derive(Debug, Clone, Serialize)]
pub struct TimeGrant {
    pub id: Uuid,
    pub user_id: Uuid,
    pub week_start: NaiveDate,
    pub seconds: i32,
    pub reason: String,
    pub granted_by: Option<Uuid>,
    pub granted_by_name: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// The Monday of the user's *current* business week — the 4 AM boundary in
/// their timezone, matching `intervals::hours_summary`. Grants attach to this.
pub async fn current_week_start(pool: &PgPool, user_id: Uuid) -> Result<NaiveDate, AppError> {
    let r = sqlx::query!(
        r#"SELECT date_trunc('week',
                    ((now() AT TIME ZONE COALESCE(u.timezone, 'UTC')) - interval '4 hours')
                  )::date AS "wk!"
           FROM users u WHERE u.id = $1"#,
        user_id
    )
    .fetch_optional(pool)
    .await?;
    r.map(|r| r.wk).ok_or(AppError::NotFound)
}

/// Record a grant. `seconds` must be positive (validated at the route).
pub async fn create(
    pool: &PgPool,
    user_id: Uuid,
    week_start: NaiveDate,
    seconds: i32,
    reason: &str,
    granted_by: Uuid,
) -> Result<TimeGrant, AppError> {
    let r = sqlx::query!(
        r#"INSERT INTO time_grants (user_id, week_start, seconds, reason, granted_by)
           VALUES ($1, $2, $3, $4, $5)
           RETURNING id, created_at"#,
        user_id,
        week_start,
        seconds,
        reason,
        granted_by
    )
    .fetch_one(pool)
    .await?;
    Ok(TimeGrant {
        id: r.id,
        user_id,
        week_start,
        seconds,
        reason: reason.to_string(),
        granted_by: Some(granted_by),
        granted_by_name: None,
        created_at: r.created_at,
    })
}

/// A user's grants for a given week, newest first (with the granter's name).
pub async fn list_for_week(
    pool: &PgPool,
    user_id: Uuid,
    week_start: NaiveDate,
) -> Result<Vec<TimeGrant>, AppError> {
    let rows = sqlx::query!(
        r#"SELECT g.id, g.user_id, g.week_start, g.seconds, g.reason, g.granted_by,
                  gb.name AS "granted_by_name?", g.created_at
           FROM time_grants g
           LEFT JOIN users gb ON gb.id = g.granted_by
           WHERE g.user_id = $1 AND g.week_start = $2
           ORDER BY g.created_at DESC"#,
        user_id,
        week_start
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| TimeGrant {
            id: r.id,
            user_id: r.user_id,
            week_start: r.week_start,
            seconds: r.seconds,
            reason: r.reason,
            granted_by: r.granted_by,
            granted_by_name: r.granted_by_name,
            created_at: r.created_at,
        })
        .collect())
}

/// The user this grant belongs to (for authorization on delete).
pub async fn owner(pool: &PgPool, id: Uuid) -> Result<Option<Uuid>, AppError> {
    let r = sqlx::query!("SELECT user_id FROM time_grants WHERE id = $1", id)
        .fetch_optional(pool)
        .await?;
    Ok(r.map(|r| r.user_id))
}

/// Delete a grant. Returns whether a row was removed.
pub async fn delete(pool: &PgPool, id: Uuid) -> Result<bool, AppError> {
    let res = sqlx::query!("DELETE FROM time_grants WHERE id = $1", id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

/// Total manually-granted seconds whose week falls inside an inclusive day range
/// (used by the monthly rollup so grace time shows in the month's total).
pub async fn sum_for_range(
    pool: &PgPool,
    user_id: Uuid,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<i64, AppError> {
    let row = sqlx::query!(
        r#"SELECT COALESCE(SUM(seconds), 0)::BIGINT AS "total!"
           FROM time_grants
           WHERE user_id = $1 AND week_start >= $2 AND week_start <= $3"#,
        user_id,
        from,
        to
    )
    .fetch_one(pool)
    .await?;
    Ok(row.total)
}

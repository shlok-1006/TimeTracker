//! Weekly hours-compliance repository (Rule 7: SQLx, compile-time checked).
//!
//! Reads the per-day attendance rollup (`attendance_days`) to compute each
//! employee's weekly working days + worked seconds, and persists a compliance
//! row per (user, week_start). Working days = Mon–Fri days classified
//! present/partial/absent — i.e. business days that were not weekend, holiday,
//! or approved leave. Worked seconds sum every day in the window (so weekend or
//! holiday work still counts toward meeting the target).

use chrono::{DateTime, NaiveDate, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;

/// One employee's aggregated activity for a week window.
#[derive(Debug, Clone)]
pub struct EmployeeWeek {
    pub user_id: Uuid,
    pub name: String,
    pub email: String,
    pub working_days: i64,
    pub worked_seconds: i64,
}

/// Result of an upsert: the row id plus its (preserved) notification stamp, so
/// the caller can tell whether HR/PM were already warned for this week.
#[derive(Debug, Clone, Copy)]
pub struct Upserted {
    pub id: Uuid,
    pub notified_at: Option<DateTime<Utc>>,
}

/// Aggregate every employee's working days + worked seconds over `[from, to]`
/// (inclusive) from the attendance rollup. `ISODOW < 6` keeps Mon–Fri only, so
/// work logged on a weekend does not inflate the required-hours count.
pub async fn week_activity(
    pool: &PgPool,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<EmployeeWeek>, AppError> {
    let rows = sqlx::query!(
        r#"
        SELECT u.id AS user_id, u.name, u.email,
          COUNT(ad.*) FILTER (
              WHERE ad.status IN ('present','partial','absent')
                AND EXTRACT(ISODOW FROM ad.day) < 6
          ) AS "working_days!",
          CAST(COALESCE(SUM(ad.worked_seconds), 0) AS BIGINT) AS "worked!"
        FROM users u
        LEFT JOIN attendance_days ad
               ON ad.user_id = u.id AND ad.day >= $1 AND ad.day <= $2
        WHERE u.role = 'employee'::user_role
        GROUP BY u.id, u.name, u.email
        ORDER BY u.name
        "#,
        from,
        to
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| EmployeeWeek {
            user_id: r.user_id,
            name: r.name,
            email: r.email,
            working_days: r.working_days,
            worked_seconds: r.worked,
        })
        .collect())
}

/// Upsert a weekly compliance row (idempotent per user/week). `notified_at` is
/// intentionally *not* overwritten, so re-running the job preserves whether a
/// warning was already sent. Returns the row id and the prior `notified_at`.
#[allow(clippy::too_many_arguments)]
pub async fn upsert(
    pool: &PgPool,
    user_id: Uuid,
    week_start: NaiveDate,
    week_end: NaiveDate,
    working_days: i32,
    required_seconds: i64,
    worked_seconds: i64,
    shortfall_seconds: i64,
    compliant: bool,
) -> Result<Upserted, AppError> {
    let row = sqlx::query!(
        r#"
        INSERT INTO weekly_hours_reports
            (user_id, week_start, week_end, working_days,
             required_seconds, worked_seconds, shortfall_seconds, compliant)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        ON CONFLICT (user_id, week_start) DO UPDATE SET
            week_end          = EXCLUDED.week_end,
            working_days      = EXCLUDED.working_days,
            required_seconds  = EXCLUDED.required_seconds,
            worked_seconds    = EXCLUDED.worked_seconds,
            shortfall_seconds = EXCLUDED.shortfall_seconds,
            compliant         = EXCLUDED.compliant,
            updated_at        = now()
        RETURNING id, notified_at
        "#,
        user_id,
        week_start,
        week_end,
        working_days,
        required_seconds,
        worked_seconds,
        shortfall_seconds,
        compliant
    )
    .fetch_one(pool)
    .await?;
    Ok(Upserted {
        id: row.id,
        notified_at: row.notified_at,
    })
}

/// Stamp `notified_at = now()` once the shortfall warning has been sent.
pub async fn mark_notified(pool: &PgPool, id: Uuid) -> Result<(), AppError> {
    sqlx::query!(
        "UPDATE weekly_hours_reports SET notified_at = now(), updated_at = now() WHERE id = $1",
        id
    )
    .execute(pool)
    .await?;
    Ok(())
}

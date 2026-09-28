//! Weekly hours-compliance repository (Rule 7: SQLx, compile-time checked).
//!
//! Reads the per-day attendance rollup (`attendance_days`) to compute each
//! employee's weekly working days + worked seconds, and persists a compliance
//! row per (user, week_start). Working days = Mon–Fri days classified
//! present/partial/absent — i.e. business days that were not weekend, holiday,
//! or approved leave. Worked seconds sum every day in the window (so weekend or
//! holiday work still counts toward meeting the target).

use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;

/// One non-compliant employee for a completed week — the row HR/PM sees in the
/// Weekly Report and can email from. `notified_at` is set once the shortfall
/// email has been sent (so the UI can show "Sent" and avoid re-sending).
#[derive(Debug, Clone, Serialize)]
pub struct ShortfallRow {
    #[serde(skip_serializing)]
    pub id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    pub email: String,
    pub week_start: NaiveDate,
    pub week_end: NaiveDate,
    pub working_days: i64,
    pub required_seconds: i64,
    pub worked_seconds: i64,
    pub shortfall_seconds: i64,
    pub notified_at: Option<DateTime<Utc>>,
}

/// The non-compliant employees for `week_start`, biggest shortfall first. `manager_id = None` for HR
/// (everyone); `Some(pm)` restricts to the people that PM is responsible for — their direct reports
/// (`user_managers`) OR members of a team they're assigned to (`team_pms` + `user_teams`), so a PM set up
/// only through team assignment doesn't see an empty report. Deactivated users are excluded.
/// `notified_at` in the result is the EMPLOYEE-email stamp (`employee_notified_at`, migration 0049).
pub async fn list_shortfalls(
    pool: &PgPool,
    week_start: NaiveDate,
    manager_id: Option<Uuid>,
) -> Result<Vec<ShortfallRow>, AppError> {
    let rows = sqlx::query!(
        r#"
        SELECT wh.id, wh.user_id, u.name AS "name!", u.email AS "email!",
               wh.week_start, wh.week_end, wh.working_days,
               wh.required_seconds, wh.worked_seconds, wh.shortfall_seconds,
               wh.employee_notified_at AS notified_at
        FROM weekly_hours_reports wh
        JOIN users u ON u.id = wh.user_id
        WHERE wh.week_start = $1
          AND wh.compliant = FALSE
          AND u.deactivated_at IS NULL
          AND ($2::uuid IS NULL
               OR EXISTS (SELECT 1 FROM user_managers um
                          WHERE um.user_id = u.id AND um.manager_id = $2)
               OR EXISTS (SELECT 1 FROM user_teams ut
                            JOIN team_pms tp ON tp.team_id = ut.team_id
                          WHERE ut.user_id = u.id AND tp.pm_user_id = $2))
        ORDER BY wh.shortfall_seconds DESC, u.name
        "#,
        week_start,
        manager_id
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| ShortfallRow {
            id: r.id,
            user_id: r.user_id,
            name: r.name,
            email: r.email,
            week_start: r.week_start,
            week_end: r.week_end,
            working_days: r.working_days as i64,
            required_seconds: r.required_seconds,
            worked_seconds: r.worked_seconds,
            shortfall_seconds: r.shortfall_seconds,
            notified_at: r.notified_at,
        })
        .collect())
}

/// One non-compliant report for (user, week), scoped like [`list_shortfalls`] — so a PM can only fetch
/// (and therefore email) their own people. `None` when the row doesn't exist, is compliant, the user is
/// deactivated, or it's out of the caller's scope.
pub async fn find_shortfall(
    pool: &PgPool,
    user_id: Uuid,
    week_start: NaiveDate,
    manager_id: Option<Uuid>,
) -> Result<Option<ShortfallRow>, AppError> {
    let row = sqlx::query!(
        r#"
        SELECT wh.id, wh.user_id, u.name AS "name!", u.email AS "email!",
               wh.week_start, wh.week_end, wh.working_days,
               wh.required_seconds, wh.worked_seconds, wh.shortfall_seconds,
               wh.employee_notified_at AS notified_at
        FROM weekly_hours_reports wh
        JOIN users u ON u.id = wh.user_id
        WHERE wh.user_id = $1
          AND wh.week_start = $2
          AND wh.compliant = FALSE
          AND u.deactivated_at IS NULL
          AND ($3::uuid IS NULL
               OR EXISTS (SELECT 1 FROM user_managers um
                          WHERE um.user_id = u.id AND um.manager_id = $3)
               OR EXISTS (SELECT 1 FROM user_teams ut
                            JOIN team_pms tp ON tp.team_id = ut.team_id
                          WHERE ut.user_id = u.id AND tp.pm_user_id = $3))
        "#,
        user_id,
        week_start,
        manager_id
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| ShortfallRow {
        id: r.id,
        user_id: r.user_id,
        name: r.name,
        email: r.email,
        week_start: r.week_start,
        week_end: r.week_end,
        working_days: r.working_days as i64,
        required_seconds: r.required_seconds,
        worked_seconds: r.worked_seconds,
        shortfall_seconds: r.shortfall_seconds,
        notified_at: r.notified_at,
    }))
}

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

/// Atomically CLAIM the employee-email slot for a report: stamps `employee_notified_at` only if it was
/// still NULL, and returns the stamp. `None` means someone else already sent it (a second HR user, the
/// pop-up and the tab open at once, a retry) — the caller must not send a second email.
pub async fn claim_employee_notify(
    pool: &PgPool,
    id: Uuid,
) -> Result<Option<DateTime<Utc>>, AppError> {
    let row = sqlx::query!(
        r#"UPDATE weekly_hours_reports
              SET employee_notified_at = now(), updated_at = now()
            WHERE id = $1 AND employee_notified_at IS NULL
        RETURNING employee_notified_at AS "at!""#,
        id
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| r.at))
}

/// Undo a claim when the email then failed to send, so HR can retry.
pub async fn release_employee_notify(pool: &PgPool, id: Uuid) -> Result<(), AppError> {
    sqlx::query!(
        "UPDATE weekly_hours_reports SET employee_notified_at = NULL, updated_at = now() WHERE id = $1",
        id
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Whether the weekly batch has computed `week_start` at all (any row, compliant or not). Lets the UI tell
/// "nobody fell short" apart from "not computed yet", and lets the scheduler catch up a missed Monday.
pub async fn week_has_rows(pool: &PgPool, week_start: NaiveDate) -> Result<bool, AppError> {
    let row = sqlx::query!(
        r#"SELECT EXISTS (SELECT 1 FROM weekly_hours_reports WHERE week_start = $1) AS "exists!""#,
        week_start
    )
    .fetch_one(pool)
    .await?;
    Ok(row.exists)
}

//! Leave repository (Rule 7): leave types, holidays, allocations, requests.

use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;

#[derive(Debug, Clone, Serialize)]
pub struct LeaveType {
    pub id: Uuid,
    pub name: String,
    pub paid: bool,
    /// Default days for the employee category (also used for PMs and HR).
    pub default_days: f64,
    pub default_days_contractor: f64,
    pub default_days_intern: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Holiday {
    pub id: Uuid,
    pub day: NaiveDate,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct LeaveRequest {
    pub id: Uuid,
    pub user_id: Uuid,
    pub leave_type_id: Uuid,
    pub leave_type_name: String,
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    pub days: f64,
    pub reason: String,
    pub status: String,
    pub approver_id: Option<Uuid>,
    pub decided_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// A pending request enriched with the requesting employee's identity (approver view).
#[derive(Debug, Clone, Serialize)]
pub struct PendingRequest {
    pub id: Uuid,
    pub user_id: Uuid,
    pub employee_name: String,
    pub employee_email: String,
    pub leave_type_name: String,
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    pub days: f64,
    pub reason: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Balance {
    pub leave_type_id: Uuid,
    pub leave_type_name: String,
    pub paid: bool,
    pub allotted_days: f64,
    pub used_days: f64,
    pub remaining_days: f64,
    /// True when `allotted_days` comes from an explicit per-user allocation
    /// (a manual override); false when it falls back to the category default.
    pub is_override: bool,
}

// ---- Leave types ----

pub async fn list_types(pool: &PgPool) -> Result<Vec<LeaveType>, AppError> {
    let rows = sqlx::query!(
        "SELECT id, name, paid, default_days, default_days_contractor, default_days_intern
         FROM leave_types ORDER BY name"
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| LeaveType {
            id: r.id,
            name: r.name,
            paid: r.paid,
            default_days: r.default_days,
            default_days_contractor: r.default_days_contractor,
            default_days_intern: r.default_days_intern,
        })
        .collect())
}

pub async fn create_type(
    pool: &PgPool,
    name: &str,
    paid: bool,
    default_days: f64,
    default_days_contractor: f64,
    default_days_intern: f64,
) -> Result<LeaveType, AppError> {
    let r = sqlx::query!(
        "INSERT INTO leave_types
             (name, paid, default_days, default_days_contractor, default_days_intern)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING id, name, paid, default_days, default_days_contractor, default_days_intern",
        name,
        paid,
        default_days,
        default_days_contractor,
        default_days_intern
    )
    .fetch_one(pool)
    .await?;
    Ok(LeaveType {
        id: r.id,
        name: r.name,
        paid: r.paid,
        default_days: r.default_days,
        default_days_contractor: r.default_days_contractor,
        default_days_intern: r.default_days_intern,
    })
}

/// Update a leave type's paid flag and its per-category default allotments.
/// Returns the updated type, or `None` if no type has that id.
pub async fn update_type(
    pool: &PgPool,
    id: Uuid,
    paid: bool,
    default_days: f64,
    default_days_contractor: f64,
    default_days_intern: f64,
) -> Result<Option<LeaveType>, AppError> {
    let row = sqlx::query!(
        "UPDATE leave_types
         SET paid = $2, default_days = $3,
             default_days_contractor = $4, default_days_intern = $5
         WHERE id = $1
         RETURNING id, name, paid, default_days, default_days_contractor, default_days_intern",
        id,
        paid,
        default_days,
        default_days_contractor,
        default_days_intern
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| LeaveType {
        id: r.id,
        name: r.name,
        paid: r.paid,
        default_days: r.default_days,
        default_days_contractor: r.default_days_contractor,
        default_days_intern: r.default_days_intern,
    }))
}

// ---- Holidays ----

pub async fn list_holidays(pool: &PgPool, year: Option<i32>) -> Result<Vec<Holiday>, AppError> {
    let rows = sqlx::query!(
        r#"SELECT id, day, name FROM holidays
           WHERE $1::int IS NULL OR EXTRACT(YEAR FROM day)::int = $1
           ORDER BY day"#,
        year
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| Holiday {
            id: r.id,
            day: r.day,
            name: r.name,
        })
        .collect())
}

/// The name of the holiday falling on `day`, if any (for attendance rollups).
pub async fn holiday_name_on_day(
    pool: &PgPool,
    day: NaiveDate,
) -> Result<Option<String>, AppError> {
    let row = sqlx::query!("SELECT name FROM holidays WHERE day = $1", day)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|r| r.name))
}

/// The leave type name of an *approved* request covering `day` for `user_id`,
/// if any (for attendance rollups).
pub async fn approved_leave_type_on_day(
    pool: &PgPool,
    user_id: Uuid,
    day: NaiveDate,
) -> Result<Option<String>, AppError> {
    let row = sqlx::query!(
        r#"SELECT lt.name
           FROM leave_requests lr
           JOIN leave_types lt ON lt.id = lr.leave_type_id
           WHERE lr.user_id = $1 AND lr.status = 'approved'
             AND lr.start_date <= $2 AND lr.end_date >= $2
           ORDER BY lr.created_at
           LIMIT 1"#,
        user_id,
        day
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| r.name))
}

/// Holiday dates within an inclusive range (for business-day counting).
pub async fn holiday_dates_between(
    pool: &PgPool,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<Vec<NaiveDate>, AppError> {
    let rows = sqlx::query!(
        "SELECT day FROM holidays WHERE day >= $1 AND day <= $2",
        start,
        end
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|r| r.day).collect())
}

pub async fn create_holiday(
    pool: &PgPool,
    day: NaiveDate,
    name: &str,
) -> Result<Holiday, AppError> {
    let r = sqlx::query!(
        "INSERT INTO holidays (day, name) VALUES ($1, $2)
         ON CONFLICT (day) DO UPDATE SET name = EXCLUDED.name
         RETURNING id, day, name",
        day,
        name
    )
    .fetch_one(pool)
    .await?;
    Ok(Holiday {
        id: r.id,
        day: r.day,
        name: r.name,
    })
}

// ---- Allocations ----

pub async fn upsert_allocation(
    pool: &PgPool,
    user_id: Uuid,
    leave_type_id: Uuid,
    year: i32,
    allotted_days: f64,
) -> Result<(), AppError> {
    sqlx::query!(
        "INSERT INTO leave_allocations (user_id, leave_type_id, year, allotted_days)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (user_id, leave_type_id, year)
         DO UPDATE SET allotted_days = EXCLUDED.allotted_days",
        user_id,
        leave_type_id,
        year,
        allotted_days
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Delete a user's explicit allocation override for a type/year, reverting the
/// balance to their category default. Returns whether a row was removed.
pub async fn delete_allocation(
    pool: &PgPool,
    user_id: Uuid,
    leave_type_id: Uuid,
    year: i32,
) -> Result<bool, AppError> {
    let res = sqlx::query!(
        "DELETE FROM leave_allocations
         WHERE user_id = $1 AND leave_type_id = $2 AND year = $3",
        user_id,
        leave_type_id,
        year
    )
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// The effective allotment for one (user, type, year): an explicit override if
/// present, otherwise the leave type's default for the user's category. PMs and
/// HR are treated as the employee category. Returns `None` if the type is unknown.
pub async fn effective_allotment(
    pool: &PgPool,
    user_id: Uuid,
    leave_type_id: Uuid,
    year: i32,
) -> Result<Option<f64>, AppError> {
    let row = sqlx::query!(
        r#"
        SELECT COALESCE(
            la.allotted_days,
            CASE
                WHEN u.role IN ('project_manager'::user_role, 'hr'::user_role)
                    THEN lt.default_days
                WHEN u.employment_type = 'contractor'::employment_type
                    THEN lt.default_days_contractor
                WHEN u.employment_type = 'intern'::employment_type
                    THEN lt.default_days_intern
                ELSE lt.default_days
            END
        ) AS "allotted!"
        FROM leave_types lt
        JOIN users u ON u.id = $1
        LEFT JOIN leave_allocations la
               ON la.leave_type_id = lt.id AND la.user_id = $1 AND la.year = $3
        WHERE lt.id = $2
        "#,
        user_id,
        leave_type_id,
        year
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| r.allotted))
}

/// Increase or decrease a user's allotment for a type/year by `delta` days,
/// writing an explicit override (starting from their current effective
/// allotment). The result is clamped to be non-negative. Returns the new
/// allotment, or `None` if the leave type is unknown.
pub async fn adjust_allocation(
    pool: &PgPool,
    user_id: Uuid,
    leave_type_id: Uuid,
    year: i32,
    delta: f64,
) -> Result<Option<f64>, AppError> {
    let Some(current) = effective_allotment(pool, user_id, leave_type_id, year).await? else {
        return Ok(None);
    };
    let next = (current + delta).max(0.0);
    upsert_allocation(pool, user_id, leave_type_id, year, next).await?;
    Ok(Some(next))
}

/// Per-type balances for a user in a given year (allotted, used [approved],
/// remaining). `allotted` falls back to the user's category default when there
/// is no explicit allocation override; `is_override` flags which is which.
pub async fn balances(pool: &PgPool, user_id: Uuid, year: i32) -> Result<Vec<Balance>, AppError> {
    let rows = sqlx::query!(
        r#"
        SELECT
            lt.id                                   AS leave_type_id,
            lt.name                                 AS leave_type_name,
            lt.paid                                 AS paid,
            COALESCE(
                la.allotted_days,
                CASE
                    WHEN u.role IN ('project_manager'::user_role, 'hr'::user_role)
                        THEN lt.default_days
                    WHEN u.employment_type = 'contractor'::employment_type
                        THEN lt.default_days_contractor
                    WHEN u.employment_type = 'intern'::employment_type
                        THEN lt.default_days_intern
                    ELSE lt.default_days
                END
            )                                       AS "allotted!",
            (la.allotted_days IS NOT NULL)          AS "is_override!",
            COALESCE((
                SELECT SUM(lr.days) FROM leave_requests lr
                WHERE lr.user_id = $1 AND lr.leave_type_id = lt.id
                  AND lr.status = 'approved'
                  AND EXTRACT(YEAR FROM lr.start_date)::int = $2
            ), 0)                                   AS "used!"
        FROM leave_types lt
        JOIN users u ON u.id = $1
        LEFT JOIN leave_allocations la
               ON la.leave_type_id = lt.id AND la.user_id = $1 AND la.year = $2
        ORDER BY lt.name
        "#,
        user_id,
        year
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| Balance {
            leave_type_id: r.leave_type_id,
            leave_type_name: r.leave_type_name,
            paid: r.paid,
            allotted_days: r.allotted,
            used_days: r.used,
            remaining_days: r.allotted - r.used,
            is_override: r.is_override,
        })
        .collect())
}

// ---- Requests ----

#[allow(clippy::too_many_arguments)]
pub async fn create_request(
    pool: &PgPool,
    user_id: Uuid,
    leave_type_id: Uuid,
    start_date: NaiveDate,
    end_date: NaiveDate,
    days: f64,
    reason: &str,
) -> Result<Uuid, AppError> {
    let r = sqlx::query!(
        "INSERT INTO leave_requests (user_id, leave_type_id, start_date, end_date, days, reason)
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
        user_id,
        leave_type_id,
        start_date,
        end_date,
        days,
        reason
    )
    .fetch_one(pool)
    .await?;
    Ok(r.id)
}

pub async fn list_requests_for_user(
    pool: &PgPool,
    user_id: Uuid,
) -> Result<Vec<LeaveRequest>, AppError> {
    let rows = sqlx::query!(
        r#"SELECT lr.id, lr.user_id, lr.leave_type_id, lt.name AS "leave_type_name!",
                  lr.start_date, lr.end_date, lr.days, lr.reason, lr.status,
                  lr.approver_id, lr.decided_at, lr.created_at
           FROM leave_requests lr
           JOIN leave_types lt ON lt.id = lr.leave_type_id
           WHERE lr.user_id = $1
           ORDER BY lr.start_date DESC"#,
        user_id
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| LeaveRequest {
            id: r.id,
            user_id: r.user_id,
            leave_type_id: r.leave_type_id,
            leave_type_name: r.leave_type_name,
            start_date: r.start_date,
            end_date: r.end_date,
            days: r.days,
            reason: r.reason,
            status: r.status,
            approver_id: r.approver_id,
            decided_at: r.decided_at,
            created_at: r.created_at,
        })
        .collect())
}

/// Pending requests for approval. `manager_id = Some(pm)` scopes to that
/// manager's team; `None` (HR) returns everyone's.
pub async fn list_pending(
    pool: &PgPool,
    manager_id: Option<Uuid>,
) -> Result<Vec<PendingRequest>, AppError> {
    let rows = sqlx::query!(
        r#"SELECT lr.id, lr.user_id, u.name AS employee_name, u.email AS employee_email,
                  lt.name AS leave_type_name, lr.start_date, lr.end_date, lr.days,
                  lr.reason, lr.created_at
           FROM leave_requests lr
           JOIN users u       ON u.id = lr.user_id
           JOIN leave_types lt ON lt.id = lr.leave_type_id
           WHERE lr.status = 'pending'
             AND ($1::uuid IS NULL
                  OR EXISTS (SELECT 1 FROM user_managers um
                             WHERE um.user_id = u.id AND um.manager_id = $1))
           ORDER BY lr.created_at"#,
        manager_id
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| PendingRequest {
            id: r.id,
            user_id: r.user_id,
            employee_name: r.employee_name,
            employee_email: r.employee_email,
            leave_type_name: r.leave_type_name,
            start_date: r.start_date,
            end_date: r.end_date,
            days: r.days,
            reason: r.reason,
            created_at: r.created_at,
        })
        .collect())
}

/// (user_id, status) of a request, for authorization + workflow checks.
pub async fn owner_and_status(pool: &PgPool, id: Uuid) -> Result<Option<(Uuid, String)>, AppError> {
    let row = sqlx::query!(
        "SELECT user_id, status FROM leave_requests WHERE id = $1",
        id
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| (r.user_id, r.status)))
}

/// Approve/reject a pending request. Returns false if it was not pending.
pub async fn decide(
    pool: &PgPool,
    id: Uuid,
    status: &str,
    approver_id: Uuid,
) -> Result<bool, AppError> {
    let res = sqlx::query!(
        "UPDATE leave_requests
         SET status = $2, approver_id = $3, decided_at = now()
         WHERE id = $1 AND status = 'pending'",
        id,
        status,
        approver_id
    )
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// Cancel a still-pending request owned by `user_id`.
pub async fn cancel(pool: &PgPool, id: Uuid, user_id: Uuid) -> Result<bool, AppError> {
    let res = sqlx::query!(
        "UPDATE leave_requests SET status = 'cancelled'
         WHERE id = $1 AND user_id = $2 AND status = 'pending'",
        id,
        user_id
    )
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

//! Attendance repository (Feature 6C, Rule 7). Rows are *derived* from the
//! interval log by the attendance service and stored here as a rollup cache;
//! reports and the calendar read from this table.

use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;

#[derive(Debug, Clone, Serialize)]
pub struct AttendanceDay {
    pub user_id: Uuid,
    pub day: NaiveDate,
    pub status: String,
    pub worked_seconds: i32,
    pub idle_seconds: i32,
    pub first_in_utc: Option<DateTime<Utc>>,
    pub last_out_utc: Option<DateTime<Utc>>,
    pub note: String,
}

/// Worked/idle seconds and clock in/out for a user on a UTC day, clipped to the
/// day window. Worked = active + meeting (Rule 2).
#[derive(Debug, Clone)]
pub struct DayActivity {
    pub worked_seconds: i64,
    pub idle_seconds: i64,
    pub first_in_utc: Option<DateTime<Utc>>,
    pub last_out_utc: Option<DateTime<Utc>>,
}

/// Aggregate a single day's interval activity for one user (clipped to
/// `[day_start, day_end)`).
pub async fn day_activity(
    pool: &PgPool,
    user_id: Uuid,
    day_start: DateTime<Utc>,
    day_end: DateTime<Utc>,
) -> Result<DayActivity, AppError> {
    let r = sqlx::query!(
        r#"
        SELECT
          CAST(COALESCE(SUM(EXTRACT(EPOCH FROM (LEAST(end_utc,$3) - GREATEST(start_utc,$2))))
               FILTER (WHERE kind IN ('active','meeting')), 0) AS BIGINT) AS "worked!",
          CAST(COALESCE(SUM(EXTRACT(EPOCH FROM (LEAST(end_utc,$3) - GREATEST(start_utc,$2))))
               FILTER (WHERE kind = 'idle'), 0) AS BIGINT) AS "idle!",
          MIN(GREATEST(start_utc,$2)) FILTER (WHERE kind IN ('active','meeting')) AS first_in,
          MAX(LEAST(end_utc,$3))      FILTER (WHERE kind IN ('active','meeting')) AS last_out
        FROM intervals
        WHERE user_id = $1 AND end_utc > $2 AND start_utc < $3
        "#,
        user_id,
        day_start,
        day_end
    )
    .fetch_one(pool)
    .await?;
    Ok(DayActivity {
        worked_seconds: r.worked,
        idle_seconds: r.idle,
        first_in_utc: r.first_in,
        last_out_utc: r.last_out,
    })
}

/// Upsert a derived attendance row (idempotent per user/day).
#[allow(clippy::too_many_arguments)]
pub async fn upsert(
    pool: &PgPool,
    user_id: Uuid,
    day: NaiveDate,
    status: &str,
    worked_seconds: i32,
    idle_seconds: i32,
    first_in_utc: Option<DateTime<Utc>>,
    last_out_utc: Option<DateTime<Utc>>,
    note: &str,
) -> Result<(), AppError> {
    sqlx::query!(
        r#"
        INSERT INTO attendance_days
            (user_id, day, status, worked_seconds, idle_seconds, first_in_utc, last_out_utc, note)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        ON CONFLICT (user_id, day) DO UPDATE SET
            status = EXCLUDED.status,
            worked_seconds = EXCLUDED.worked_seconds,
            idle_seconds = EXCLUDED.idle_seconds,
            first_in_utc = EXCLUDED.first_in_utc,
            last_out_utc = EXCLUDED.last_out_utc,
            note = EXCLUDED.note,
            updated_at = now()
        "#,
        user_id,
        day,
        status,
        worked_seconds,
        idle_seconds,
        first_in_utc,
        last_out_utc,
        note
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get(
    pool: &PgPool,
    user_id: Uuid,
    day: NaiveDate,
) -> Result<Option<AttendanceDay>, AppError> {
    let row = sqlx::query!(
        r#"SELECT user_id, day, status, worked_seconds, idle_seconds,
                  first_in_utc, last_out_utc, note
           FROM attendance_days WHERE user_id = $1 AND day = $2"#,
        user_id,
        day
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| AttendanceDay {
        user_id: r.user_id,
        day: r.day,
        status: r.status,
        worked_seconds: r.worked_seconds,
        idle_seconds: r.idle_seconds,
        first_in_utc: r.first_in_utc,
        last_out_utc: r.last_out_utc,
        note: r.note,
    }))
}

/// Days the user already has rolled up within `[from, to]` (so the service can
/// skip recomputing finalized past days).
pub async fn existing_days(
    pool: &PgPool,
    user_id: Uuid,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<NaiveDate>, AppError> {
    let rows = sqlx::query!(
        "SELECT day FROM attendance_days WHERE user_id = $1 AND day >= $2 AND day <= $3",
        user_id,
        from,
        to
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|r| r.day).collect())
}

/// The user's attendance rows in `[from, to]`, ascending (calendar feed).
pub async fn list_range(
    pool: &PgPool,
    user_id: Uuid,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<AttendanceDay>, AppError> {
    let rows = sqlx::query!(
        r#"SELECT user_id, day, status, worked_seconds, idle_seconds,
                  first_in_utc, last_out_utc, note
           FROM attendance_days
           WHERE user_id = $1 AND day >= $2 AND day <= $3
           ORDER BY day"#,
        user_id,
        from,
        to
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| AttendanceDay {
            user_id: r.user_id,
            day: r.day,
            status: r.status,
            worked_seconds: r.worked_seconds,
            idle_seconds: r.idle_seconds,
            first_in_utc: r.first_in_utc,
            last_out_utc: r.last_out_utc,
            note: r.note,
        })
        .collect())
}

/// Per-employee attendance summary over `[from, to]` (the HR/PM report).
/// `manager_id = Some(pm)` scopes to that manager's team; `None` (HR) is all
/// employees.
#[derive(Debug, Clone, Serialize)]
pub struct UserAttendanceSummary {
    pub user_id: Uuid,
    pub name: String,
    pub email: String,
    pub present: i64,
    pub absent: i64,
    pub leave: i64,
    pub holiday: i64,
    pub weekend: i64,
    pub worked_seconds: i64,
}

pub async fn report(
    pool: &PgPool,
    from: NaiveDate,
    to: NaiveDate,
    manager_id: Option<Uuid>,
) -> Result<Vec<UserAttendanceSummary>, AppError> {
    let rows = sqlx::query!(
        r#"
        SELECT u.id AS user_id, u.name, u.email,
          COUNT(ad.*) FILTER (WHERE ad.status = 'present') AS "present!",
          COUNT(ad.*) FILTER (WHERE ad.status = 'absent')  AS "absent!",
          COUNT(ad.*) FILTER (WHERE ad.status = 'leave')   AS "leave!",
          COUNT(ad.*) FILTER (WHERE ad.status = 'holiday') AS "holiday!",
          COUNT(ad.*) FILTER (WHERE ad.status = 'weekend') AS "weekend!",
          CAST(COALESCE(SUM(ad.worked_seconds), 0) AS BIGINT) AS "worked!"
        FROM users u
        LEFT JOIN attendance_days ad
               ON ad.user_id = u.id AND ad.day >= $1 AND ad.day <= $2
        WHERE u.role = 'employee'::user_role
          AND ($3::uuid IS NULL OR u.manager_id = $3)
        GROUP BY u.id, u.name, u.email
        ORDER BY u.name
        "#,
        from,
        to,
        manager_id
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| UserAttendanceSummary {
            user_id: r.user_id,
            name: r.name,
            email: r.email,
            present: r.present,
            absent: r.absent,
            leave: r.leave,
            holiday: r.holiday,
            weekend: r.weekend,
            worked_seconds: r.worked,
        })
        .collect())
}

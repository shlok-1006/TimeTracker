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
    /// True when an HR edit is pinned here — derivation leaves it untouched.
    pub is_override: bool,
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
        -- `interval_seconds` (migration 0044) unions the day's intervals instead of
        -- adding their lengths up, so two devices recording the same minute produce
        -- one minute. First-in/last-out are edges, not durations, so they are read
        -- straight from the rows.
        WITH s AS (
          SELECT * FROM interval_seconds($1, $2, $3, NULL)
        ),
        edges AS (
          SELECT MIN(GREATEST(start_utc,$2)) FILTER (WHERE kind IN ('active','meeting')) AS first_in,
                 MAX(LEAST(end_utc,$3))      FILTER (WHERE kind IN ('active','meeting')) AS last_out
          FROM intervals
          WHERE user_id = $1 AND end_utc > $2 AND start_utc < $3
        )
        SELECT
          CAST(s.active + s.meeting AS BIGINT) AS "worked!",
          CAST(s.idle               AS BIGINT) AS "idle!",
          edges.first_in,
          edges.last_out
        FROM s, edges
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

/// Worked seconds for `[day_start, day_end)` and the idle that lowers the day score (report_service::day_idle).
///
/// The idle that counts is idle INSIDE the working span — from the first to the last active/meeting minute — and
/// only seconds when no device was active, in a meeting OR ON A BREAK: `interval_seconds` ranks idle above break
/// (right for attendance, where it only decides a label), but a break paused on one device while another sat idle is
/// still a break, not idle at work.
///
/// The span stretches to the day's edge when the work carries on across midnight: to `day_start` when there was
/// active/meeting time within `bridge_seconds` before it AND the day's first active minute is within
/// `bridge_seconds` after it (and likewise at `day_end`), so idle in a night session is counted (each part on its
/// own day) while a timer left on overnight still isn't. No work in the day → (worked, 0).
pub async fn score_idle_activity(
    pool: &PgPool,
    user_id: Uuid,
    day_start: DateTime<Utc>,
    day_end: DateTime<Utc>,
    bridge_seconds: i64,
) -> Result<(i64, i64), AppError> {
    let r = sqlx::query!(
        r#"
        WITH iv AS (
          SELECT kind, start_utc, end_utc FROM intervals
          WHERE user_id = $1
            AND end_utc > $2::timestamptz - make_interval(secs => $4::float8)
            AND start_utc < $3::timestamptz + make_interval(secs => $4::float8)
        ),
        edges AS (
          SELECT
            MIN(GREATEST(start_utc, $2::timestamptz)) FILTER (WHERE kind IN ('active','meeting') AND end_utc > $2::timestamptz AND start_utc < $3::timestamptz) AS first_in,
            MAX(LEAST(end_utc, $3::timestamptz))      FILTER (WHERE kind IN ('active','meeting') AND end_utc > $2::timestamptz AND start_utc < $3::timestamptz) AS last_out,
            COALESCE(BOOL_OR(kind IN ('active','meeting') AND start_utc < $2::timestamptz
                             AND end_utc > $2::timestamptz - make_interval(secs => $4::float8)), FALSE) AS worked_before,
            COALESCE(BOOL_OR(kind IN ('active','meeting') AND end_utc > $3::timestamptz
                             AND start_utc < $3::timestamptz + make_interval(secs => $4::float8)), FALSE) AS worked_after
          FROM iv
        ),
        span AS (
          SELECT
            CASE WHEN worked_before AND first_in <= $2::timestamptz + make_interval(secs => $4::float8) THEN $2::timestamptz ELSE first_in END AS s,
            CASE WHEN worked_after  AND last_out >= $3::timestamptz - make_interval(secs => $4::float8) THEN $3::timestamptz ELSE last_out END AS e
          FROM edges
        ),
        clipped AS (
          SELECT iv.kind, tstzrange(GREATEST(iv.start_utc, span.s), LEAST(iv.end_utc, span.e), '[)') AS r
          FROM iv, span
          WHERE span.s IS NOT NULL AND span.e IS NOT NULL AND GREATEST(iv.start_utc, span.s) < LEAST(iv.end_utc, span.e)
        ),
        by_kind AS (SELECT kind, range_agg(r) AS m FROM clipped GROUP BY kind),
        pick AS (
          SELECT
            COALESCE((SELECT m FROM by_kind WHERE kind = 'meeting'), '{}'::tstzmultirange) AS mtg,
            COALESCE((SELECT m FROM by_kind WHERE kind = 'active'),  '{}'::tstzmultirange) AS act,
            COALESCE((SELECT m FROM by_kind WHERE kind = 'idle'),    '{}'::tstzmultirange) AS idl,
            COALESCE((SELECT m FROM by_kind WHERE kind = 'break'),   '{}'::tstzmultirange) AS brk
        )
        SELECT
          (SELECT CAST(s.active + s.meeting AS BIGINT) FROM interval_seconds($1, $2::timestamptz, $3::timestamptz, NULL) s) AS "worked!",
          CAST(tstz_multirange_seconds(idl - mtg - act - brk) AS BIGINT) AS "idle!"
        FROM pick
        "#,
        user_id,
        day_start,
        day_end,
        bridge_seconds as f64
    )
    .fetch_one(pool)
    .await?;
    Ok((r.worked, r.idle))
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
        WHERE attendance_days.is_override = FALSE
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
                  first_in_utc, last_out_utc, note, is_override
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
        is_override: r.is_override,
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
                  first_in_utc, last_out_utc, note, is_override
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
            is_override: r.is_override,
        })
        .collect())
}

/// Pin a manual HR override for a user/day. Marks the row `is_override` so the
/// nightly rollup and the recompute-today path leave it alone. Overwrites any
/// existing row (HR's explicit edit always wins).
#[allow(clippy::too_many_arguments)]
pub async fn upsert_override(
    pool: &PgPool,
    user_id: Uuid,
    day: NaiveDate,
    status: &str,
    worked_seconds: i32,
    idle_seconds: i32,
    first_in_utc: Option<DateTime<Utc>>,
    last_out_utc: Option<DateTime<Utc>>,
    note: &str,
    overridden_by: Uuid,
) -> Result<(), AppError> {
    sqlx::query!(
        r#"
        INSERT INTO attendance_days
            (user_id, day, status, worked_seconds, idle_seconds, first_in_utc,
             last_out_utc, note, is_override, overridden_by)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, TRUE, $9)
        ON CONFLICT (user_id, day) DO UPDATE SET
            status = EXCLUDED.status,
            worked_seconds = EXCLUDED.worked_seconds,
            idle_seconds = EXCLUDED.idle_seconds,
            first_in_utc = EXCLUDED.first_in_utc,
            last_out_utc = EXCLUDED.last_out_utc,
            note = EXCLUDED.note,
            is_override = TRUE,
            overridden_by = EXCLUDED.overridden_by,
            updated_at = now()
        "#,
        user_id,
        day,
        status,
        worked_seconds,
        idle_seconds,
        first_in_utc,
        last_out_utc,
        note,
        overridden_by
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Clear a user/day override so derivation controls it again. Returns whether a
/// row was actually un-pinned (false = no override was set).
pub async fn clear_override(
    pool: &PgPool,
    user_id: Uuid,
    day: NaiveDate,
) -> Result<bool, AppError> {
    let res = sqlx::query!(
        "UPDATE attendance_days
         SET is_override = FALSE, overridden_by = NULL, updated_at = now()
         WHERE user_id = $1 AND day = $2 AND is_override = TRUE",
        user_id,
        day
    )
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
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
    pub partial: i64,
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
          COUNT(ad.*) FILTER (WHERE ad.status = 'partial') AS "partial!",
          COUNT(ad.*) FILTER (WHERE ad.status = 'absent')  AS "absent!",
          COUNT(ad.*) FILTER (WHERE ad.status = 'leave')   AS "leave!",
          COUNT(ad.*) FILTER (WHERE ad.status = 'holiday') AS "holiday!",
          COUNT(ad.*) FILTER (WHERE ad.status = 'weekend') AS "weekend!",
          CAST(COALESCE(SUM(ad.worked_seconds), 0) AS BIGINT) AS "worked!"
        FROM users u
        LEFT JOIN attendance_days ad
               ON ad.user_id = u.id AND ad.day >= $1 AND ad.day <= $2
        -- Everyone in scope, every range. There used to be a role filter here:
        --   u.role = 'employee' OR EXISTS (attendance rows in this range)
        -- which meant a PM or HR appeared ONLY in ranges where they happened to have tracked
        -- time, and silently vanished from others. The same person showing up in Live status
        -- and the directory but missing from Attendance — and moving in and out as you change
        -- the dates — makes the headcount untrustworthy and hides real absence.
        --
        -- The LEFT JOIN already yields zeros for someone with no rows, so absence now renders
        -- as 0 rather than as a missing row. "No data" and "not a person" are different facts
        -- and should not look alike.
        WHERE ($3::uuid IS NULL
               OR EXISTS (SELECT 1 FROM user_managers um
                          WHERE um.user_id = u.id AND um.manager_id = $3))
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
            partial: r.partial,
            absent: r.absent,
            leave: r.leave,
            holiday: r.holiday,
            weekend: r.weekend,
            worked_seconds: r.worked,
        })
        .collect())
}

/// The same summary as [`report`], for the members of ONE team.
///
/// Separate from `report`'s `manager_id` scope rather than another optional parameter, because it
/// answers a different question: `report` asks "the people this PM manages", this asks "the people
/// on this team". Those are different sets — that difference is the whole point of the HRMS's ask
/// — and folding both into one function with two nullable filters would invite a caller to pass
/// neither and quietly get the company.
///
/// Membership only: `team_pms` decides who may CALL this, `user_teams` decides who appears in it.
/// A team's PMs are not counted as its staff.
pub async fn report_for_team(
    pool: &PgPool,
    from: NaiveDate,
    to: NaiveDate,
    team_id: Uuid,
) -> Result<Vec<UserAttendanceSummary>, AppError> {
    let rows = sqlx::query!(
        r#"
        SELECT u.id AS user_id, u.name, u.email,
          COUNT(ad.*) FILTER (WHERE ad.status = 'present') AS "present!",
          COUNT(ad.*) FILTER (WHERE ad.status = 'partial') AS "partial!",
          COUNT(ad.*) FILTER (WHERE ad.status = 'absent')  AS "absent!",
          COUNT(ad.*) FILTER (WHERE ad.status = 'leave')   AS "leave!",
          COUNT(ad.*) FILTER (WHERE ad.status = 'holiday') AS "holiday!",
          COUNT(ad.*) FILTER (WHERE ad.status = 'weekend') AS "weekend!",
          CAST(COALESCE(SUM(ad.worked_seconds), 0) AS BIGINT) AS "worked!"
        FROM users u
        JOIN user_teams ut ON ut.user_id = u.id AND ut.team_id = $3
        LEFT JOIN attendance_days ad
               ON ad.user_id = u.id AND ad.day >= $1 AND ad.day <= $2
        GROUP BY u.id, u.name, u.email
        ORDER BY u.name
        "#,
        from,
        to,
        team_id
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
            partial: r.partial,
            absent: r.absent,
            leave: r.leave,
            holiday: r.holiday,
            weekend: r.weekend,
            worked_seconds: r.worked,
        })
        .collect())
}

/// Users whose attendance for `day` is present or partial — the nightly analyzer's
/// selection base (changes: nightly coverage). Attendance-based selection means a
/// present employee with few/no working screenshots still gets a daily report;
/// the scheduler unions this with the has-working-shots list so weekend workers
/// (whose day stays `weekend` even when tracking) are not dropped.
pub async fn user_ids_present_on_day(pool: &PgPool, day: NaiveDate) -> Result<Vec<Uuid>, AppError> {
    let rows = sqlx::query!(
        r#"SELECT user_id FROM attendance_days
           WHERE day = $1 AND status IN ('present', 'partial')"#,
        day
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|r| r.user_id).collect())
}

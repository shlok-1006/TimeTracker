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
    /// `None` = everyone; `Some("male" | "female")` = only people whose profile gender matches.
    pub eligible_gender: Option<String>,
    /// Months of service (from `users.joined_on`) before the type is available; 0 = from day one.
    pub min_tenure_months: i32,
    /// `"working"` (weekdays minus holidays) or `"calendar"` (every day in the range).
    pub day_basis: String,
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
    /// Which half a half-day covers: `first` | `second`. `None` for a full day, and for half days
    /// booked before migration 0051 or by a client that doesn't send it.
    pub half_period: Option<String>,
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
    /// Which half a half-day covers (see [`LeaveRequest::half_period`]). Carried here so an approver
    /// can tell a morning absence from an afternoon one.
    pub half_period: Option<String>,
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
    /// Whether the person meets the type's eligibility rule (gender / tenure) on the reference date.
    /// An ineligible type with no HR override has `allotted_days = 0`.
    pub eligible: bool,
    /// Why not, in words, when `eligible` is false (e.g. "Available from 12 Mar 2027 …").
    pub eligibility_note: Option<String>,
    /// `"working"` or `"calendar"` — how a request of this type counts its days.
    pub day_basis: String,
    /// True for a type with an eligibility rule (paternity / maternity). Such types are excluded from the
    /// "leaves left" totals: they are an entitlement for a life event, not regular paid leave.
    pub special: bool,
    /// When the ONLY thing missing is service time: the first date a leave of this type may START (forms
    /// offer the type with a note, since eligibility is judged on the start date). `None` otherwise.
    pub eligible_from: Option<NaiveDate>,
}

// ---- Leave types ----

pub async fn list_types(pool: &PgPool) -> Result<Vec<LeaveType>, AppError> {
    let rows = sqlx::query!(
        "SELECT id, name, paid, default_days, default_days_contractor, default_days_intern,
                eligible_gender, min_tenure_months, day_basis
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
            eligible_gender: r.eligible_gender,
            min_tenure_months: r.min_tenure_months,
            day_basis: r.day_basis,
        })
        .collect())
}

/// The eligibility + day-basis settings of a leave type (migration 0050).
#[derive(Debug, Clone)]
pub struct TypeRules {
    pub eligible_gender: Option<String>,
    pub min_tenure_months: i32,
    pub day_basis: String,
}

impl Default for TypeRules {
    /// No rule: everyone, from day one, counted in working days (every pre-0050 type).
    fn default() -> Self {
        Self {
            eligible_gender: None,
            min_tenure_months: 0,
            day_basis: "working".into(),
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn create_type(
    pool: &PgPool,
    name: &str,
    paid: bool,
    default_days: f64,
    default_days_contractor: f64,
    default_days_intern: f64,
    rules: &TypeRules,
) -> Result<LeaveType, AppError> {
    let r = sqlx::query!(
        "INSERT INTO leave_types
             (name, paid, default_days, default_days_contractor, default_days_intern,
              eligible_gender, min_tenure_months, day_basis)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         RETURNING id, name, paid, default_days, default_days_contractor, default_days_intern,
                   eligible_gender, min_tenure_months, day_basis",
        name,
        paid,
        default_days,
        default_days_contractor,
        default_days_intern,
        rules.eligible_gender,
        rules.min_tenure_months,
        rules.day_basis
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
        eligible_gender: r.eligible_gender,
        min_tenure_months: r.min_tenure_months,
        day_basis: r.day_basis,
    })
}

/// Update a leave type's paid flag and its per-category default allotments, and — when `rules` is
/// `Some` — its eligibility + day basis. `rules: None` leaves them as they are, so an older client that
/// only edits days can never wipe a type's eligibility rule.
/// Returns the updated type, or `None` if no type has that id.
#[allow(clippy::too_many_arguments)]
pub async fn update_type(
    pool: &PgPool,
    id: Uuid,
    paid: bool,
    default_days: f64,
    default_days_contractor: f64,
    default_days_intern: f64,
    rules: Option<&TypeRules>,
) -> Result<Option<LeaveType>, AppError> {
    let set_rules = rules.is_some();
    let (gender, tenure, basis) = match rules {
        Some(r) => (
            r.eligible_gender.clone(),
            r.min_tenure_months,
            r.day_basis.clone(),
        ),
        None => (None, 0, String::new()),
    };
    let row = sqlx::query!(
        "UPDATE leave_types
         SET paid = $2, default_days = $3,
             default_days_contractor = $4, default_days_intern = $5,
             eligible_gender   = CASE WHEN $6 THEN $7 ELSE eligible_gender END,
             min_tenure_months = CASE WHEN $6 THEN $8 ELSE min_tenure_months END,
             day_basis         = CASE WHEN $6 THEN $9 ELSE day_basis END
         WHERE id = $1
         RETURNING id, name, paid, default_days, default_days_contractor, default_days_intern,
                   eligible_gender, min_tenure_months, day_basis",
        id,
        paid,
        default_days,
        default_days_contractor,
        default_days_intern,
        set_rules,
        gender,
        tenure,
        basis
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
        eligible_gender: r.eligible_gender,
        min_tenure_months: r.min_tenure_months,
        day_basis: r.day_basis,
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

/// A user's effective allotment for one type/year — exactly what their balance shows: an HR override if
/// there is one, else the category default when they're ELIGIBLE (migration 0050), else 0. The "adjust"
/// action starts from this, so +1 on an ineligible type gives 1 day, never default + 1 (which used to hand
/// a man ~180 days of maternity leave via an override that then bypassed the rule).
pub async fn effective_allotment(
    pool: &PgPool,
    user_id: Uuid,
    leave_type_id: Uuid,
    year: i32,
) -> Result<Option<f64>, AppError> {
    Ok(balances(pool, user_id, year)
        .await?
        .into_iter()
        .find(|b| b.leave_type_id == leave_type_id)
        .map(|b| b.allotted_days))
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
/// Balances for `year`, with eligibility judged as of today — or 1 January of `year` when that is later, so
/// next year's balance doesn't hide maternity from someone who reaches one year of service before then.
pub async fn balances(pool: &PgPool, user_id: Uuid, year: i32) -> Result<Vec<Balance>, AppError> {
    let today = Utc::now().date_naive();
    let as_of = NaiveDate::from_ymd_opt(year, 1, 1)
        .filter(|jan1| *jan1 > today)
        .unwrap_or(today);
    balances_as_of(pool, user_id, year, as_of).await
}

/// Balances for `year`, with each type's eligibility rule (migration 0050) judged on `as_of` — the leave's
/// start date when submitting or approving, today when just displaying. Precedence: an HR allocation for the
/// year wins outright; otherwise an eligible person gets the category default and an ineligible one gets 0.
pub async fn balances_as_of(
    pool: &PgPool,
    user_id: Uuid,
    year: i32,
    as_of: NaiveDate,
) -> Result<Vec<Balance>, AppError> {
    let rows = sqlx::query!(
        r#"
        SELECT
            lt.id                                   AS leave_type_id,
            lt.name                                 AS leave_type_name,
            lt.paid                                 AS paid,
            lt.day_basis                            AS day_basis,
            lt.eligible_gender                      AS eligible_gender,
            lt.min_tenure_months                    AS min_tenure_months,
            CASE
                WHEN u.role IN ('project_manager'::user_role, 'hr'::user_role)
                    THEN lt.default_days
                WHEN u.employment_type = 'contractor'::employment_type
                    THEN lt.default_days_contractor
                WHEN u.employment_type = 'intern'::employment_type
                    THEN lt.default_days_intern
                ELSE lt.default_days
            END                                     AS "default_allotted!",
            la.allotted_days                        AS "override_days?",
            (
                lt.eligible_gender IS NULL
                OR (lt.eligible_gender = 'male'
                    AND lower(btrim(COALESCE(ep.gender, ''), E' \t\r\n' || chr(160))) IN ('male', 'm', 'man'))
                OR (lt.eligible_gender = 'female'
                    AND lower(btrim(COALESCE(ep.gender, ''), E' \t\r\n' || chr(160))) IN ('female', 'f', 'woman'))
            )                                       AS "gender_ok!",
            (
                lt.min_tenure_months = 0
                OR (u.joined_on IS NOT NULL
                    AND (u.joined_on + make_interval(months => lt.min_tenure_months))::date <= $3)
            )                                       AS "tenure_ok!",
            (u.joined_on + make_interval(months => lt.min_tenure_months))::date AS "eligible_from?",
            (u.joined_on IS NOT NULL)               AS "has_joined_on!",
            COALESCE((
                SELECT SUM(lr.days) FROM leave_requests lr
                WHERE lr.user_id = $1 AND lr.leave_type_id = lt.id
                  AND lr.status = 'approved'
                  AND EXTRACT(YEAR FROM lr.start_date)::int = $2
            ), 0)                                   AS "used!"
        FROM leave_types lt
        JOIN users u ON u.id = $1
        LEFT JOIN employee_profiles ep ON ep.user_id = u.id
        LEFT JOIN leave_allocations la
               ON la.leave_type_id = lt.id AND la.user_id = $1 AND la.year = $2
        ORDER BY lt.name
        "#,
        user_id,
        year,
        as_of
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| {
            let eligible = r.gender_ok && r.tenure_ok;
            let is_override = r.override_days.is_some();
            let allotted = match r.override_days {
                Some(d) => d,
                None if eligible => r.default_allotted,
                None => 0.0,
            };
            let eligibility_note = if eligible {
                None
            } else if !r.gender_ok {
                Some(format!(
                    "Available to {} employees (as recorded on your profile)",
                    r.eligible_gender.as_deref().unwrap_or("eligible")
                ))
            } else if !r.has_joined_on {
                Some("Needs your joining date on record — ask HR".to_string())
            } else {
                Some(format!(
                    "Available after {} months of service, from {}",
                    r.min_tenure_months,
                    r.eligible_from
                        .map(|d| d.format("%-d %b %Y").to_string())
                        .unwrap_or_default()
                ))
            };
            Balance {
                leave_type_id: r.leave_type_id,
                leave_type_name: r.leave_type_name,
                paid: r.paid,
                allotted_days: allotted,
                used_days: r.used,
                remaining_days: allotted - r.used,
                is_override,
                eligible,
                eligibility_note,
                day_basis: r.day_basis,
                special: r.eligible_gender.is_some() || r.min_tenure_months > 0,
                eligible_from: if !eligible && r.gender_ok && r.has_joined_on {
                    r.eligible_from
                } else {
                    None
                },
            }
        })
        .collect())
}

/// Total remaining **paid** leave days per user for `year`, in ONE query — the
/// "leaves left" column of the HR monthly attendance report. Calling
/// [`balances`] per employee would be N round-trips; this rolls the same
/// allotted-minus-approved arithmetic up across every user and every paid type.
///
/// Scope mirrors [`crate::db::attendance::report`]: `manager_id = Some(pm)`
/// restricts to that PM's managed users, `None` (HR) is the whole company.
/// Deactivated users are excluded — they are off the roster (see migration 0047).
///
/// Unpaid types (e.g. leave-without-pay, which can carry a huge nominal
/// allotment) are deliberately left out: "leaves left" means the paid balance an
/// employee can still draw on. A user with no leave rows returns their full
/// allotment; a user absent from the map (no paid types at all) is treated as 0
/// by the caller.
pub async fn remaining_paid_by_user(
    pool: &PgPool,
    year: i32,
    manager_id: Option<Uuid>,
) -> Result<std::collections::HashMap<Uuid, f64>, AppError> {
    let rows = sqlx::query!(
        r#"
        SELECT u.id AS user_id,
          COALESCE(SUM(
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
            )
            - COALESCE((
                SELECT SUM(lr.days) FROM leave_requests lr
                WHERE lr.user_id = u.id AND lr.leave_type_id = lt.id
                  AND lr.status = 'approved'
                  AND EXTRACT(YEAR FROM lr.start_date)::int = $1
            ), 0)
          ), 0) AS "remaining!"
        FROM users u
        CROSS JOIN leave_types lt
        LEFT JOIN leave_allocations la
               ON la.leave_type_id = lt.id AND la.user_id = u.id AND la.year = $1
        WHERE lt.paid = TRUE
          -- regular paid leave only: paternity/maternity (an eligibility rule) are not "leaves left"
          AND lt.eligible_gender IS NULL AND lt.min_tenure_months = 0
          AND u.deactivated_at IS NULL
          AND ($2::uuid IS NULL
               OR EXISTS (SELECT 1 FROM user_managers um
                          WHERE um.user_id = u.id AND um.manager_id = $2))
        GROUP BY u.id
        "#,
        year,
        manager_id
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|r| (r.user_id, r.remaining)).collect())
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
    half_period: Option<&str>,
) -> Result<Uuid, AppError> {
    let r = sqlx::query!(
        "INSERT INTO leave_requests (user_id, leave_type_id, start_date, end_date, days, reason, half_period)
         VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING id",
        user_id,
        leave_type_id,
        start_date,
        end_date,
        days,
        reason,
        half_period
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
                  lr.start_date, lr.end_date, lr.days, lr.half_period, lr.reason, lr.status,
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
            half_period: r.half_period,
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
                  lr.half_period, lr.reason, lr.created_at
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
            half_period: r.half_period,
            reason: r.reason,
            created_at: r.created_at,
        })
        .collect())
}

/// One leave for the month-register grid: who, which type, the span, how many days, its status,
/// and the reason. Deliberately leaner than [`PendingRequest`] (no email/created_at) — the grid
/// only needs to draw a block and open a detail popover.
#[derive(Debug, Clone, Serialize)]
pub struct CalendarLeave {
    pub id: Uuid,
    pub user_id: Uuid,
    pub employee_name: String,
    pub leave_type_name: String,
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    pub days: f64,
    /// Which half a half-day covers (see [`LeaveRequest::half_period`]), so the register can show
    /// a morning absence apart from an afternoon one.
    pub half_period: Option<String>,
    pub status: String,
    pub reason: String,
}

/// Every leave that OVERLAPS `[from, to]` and is `approved` or `pending` — the leave register.
/// Rejected/cancelled requests are excluded (nobody is on leave for those). A leave overlaps the
/// window when it starts on or before `to` and ends on or after `from`, so a multi-day leave that
/// straddles either edge is still returned in full (the UI clips it to the visible days).
///
/// Same scope as [`list_pending`]: `manager_id = None` (HR) is everyone; `Some(pm)` is that PM's
/// team only.
pub async fn list_in_range(
    pool: &PgPool,
    from: NaiveDate,
    to: NaiveDate,
    manager_id: Option<Uuid>,
) -> Result<Vec<CalendarLeave>, AppError> {
    let rows = sqlx::query!(
        r#"SELECT lr.id, lr.user_id, u.name AS employee_name,
                  lt.name AS leave_type_name, lr.start_date, lr.end_date, lr.days,
                  lr.half_period, lr.status, lr.reason
           FROM leave_requests lr
           JOIN users u        ON u.id = lr.user_id
           JOIN leave_types lt ON lt.id = lr.leave_type_id
           WHERE lr.status IN ('approved', 'pending')
             AND lr.start_date <= $2 AND lr.end_date >= $1
             AND ($3::uuid IS NULL
                  OR EXISTS (SELECT 1 FROM user_managers um
                             WHERE um.user_id = u.id AND um.manager_id = $3))
           ORDER BY lr.start_date, u.name"#,
        from,
        to,
        manager_id
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| CalendarLeave {
            id: r.id,
            user_id: r.user_id,
            employee_name: r.employee_name,
            leave_type_name: r.leave_type_name,
            start_date: r.start_date,
            end_date: r.end_date,
            days: r.days,
            half_period: r.half_period,
            status: r.status,
            reason: r.reason,
        })
        .collect())
}

/// True when the user already has a PENDING or APPROVED request overlapping `[start, end]` (any type) —
/// the same days can't be booked twice.
pub async fn has_overlapping_request(
    pool: &PgPool,
    user_id: Uuid,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<bool, AppError> {
    let row = sqlx::query!(
        r#"SELECT EXISTS (
             SELECT 1 FROM leave_requests
              WHERE user_id = $1 AND status IN ('pending', 'approved')
                AND start_date <= $3 AND end_date >= $2
           ) AS "overlap!""#,
        user_id,
        start,
        end
    )
    .fetch_one(pool)
    .await?;
    Ok(row.overlap)
}

/// The facts an approval re-checks: whose request, which type, when it starts, how many days.
pub async fn request_for_decision(
    pool: &PgPool,
    id: Uuid,
) -> Result<Option<(Uuid, Uuid, NaiveDate, f64)>, AppError> {
    let row = sqlx::query!(
        "SELECT user_id, leave_type_id, start_date, days FROM leave_requests WHERE id = $1",
        id
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| (r.user_id, r.leave_type_id, r.start_date, r.days)))
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

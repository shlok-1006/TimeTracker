//! Attendance routes (Feature 6C).
//!
//!   GET  /me/attendance?from=&to=               own calendar (derived rows)
//!   GET  /admin/users/:id/attendance?from=&to=  drill-down (HR all, PM own team)
//!   GET  /admin/attendance?from=&to=            per-employee report (HR all, PM team)
//!   GET  /admin/attendance/monthly?month=YYYY-MM  monthly report + leaves left (HR all, PM team)
//!   POST /admin/attendance/rollup?day=          recompute a day for everyone (HR)
//!
//! Calendar/drill-down endpoints lazily roll up missing days (and refresh today)
//! so data appears without waiting for the nightly job.

use axum::{
    extract::{Path, Query, State},
    http::header,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use chrono::{Datelike, Duration, NaiveDate, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::attendance_service;
use crate::db::{attendance, audit, leave, users};
use crate::error::AppError;
use crate::middleware::{AuthUser, RequireHr, RequireStaff};
use crate::monthly_report_service::{month_end, month_key};
use crate::role::UserRole;
use crate::routes::admin::authorize_view;
use crate::state::AppState;

/// Cap a range so a single request can't roll up an unbounded number of days.
const MAX_RANGE_DAYS: i64 = 366;

/// The attendance statuses HR may assign (must match the DB CHECK constraint).
const VALID_STATUSES: [&str; 6] = [
    "present", "partial", "absent", "leave", "holiday", "weekend",
];

#[derive(Deserialize)]
struct RangeQuery {
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
}

/// Resolve a `[from, to]` range, defaulting to the current calendar month.
fn resolve_range(q: &RangeQuery) -> Result<(NaiveDate, NaiveDate), AppError> {
    let today = Utc::now().date_naive();
    let from = q.from.unwrap_or_else(|| today.with_day(1).unwrap_or(today));
    let to = q.to.unwrap_or(today);
    if to < from {
        return Err(AppError::BadRequest("`to` is before `from`".into()));
    }
    if (to - from).num_days() > MAX_RANGE_DAYS {
        return Err(AppError::BadRequest(format!(
            "range too large (max {MAX_RANGE_DAYS} days)"
        )));
    }
    Ok((from, to))
}

/// `GET /me/attendance` — the caller's own attendance calendar.
async fn my_attendance(
    State(state): State<AppState>,
    user: AuthUser,
    Query(q): Query<RangeQuery>,
) -> Result<Json<Value>, AppError> {
    let (from, to) = resolve_range(&q)?;
    attendance_service::ensure_range(&state.db, user.id, from, to).await?;
    let days = attendance::list_range(&state.db, user.id, from, to).await?;
    Ok(Json(json!({ "from": from, "to": to, "days": days })))
}

/// `GET /admin/users/:id/attendance` — drill-down for HR / the user's PM.
async fn user_attendance(
    State(state): State<AppState>,
    RequireStaff(viewer): RequireStaff,
    Path(target): Path<Uuid>,
    Query(q): Query<RangeQuery>,
) -> Result<Json<Value>, AppError> {
    authorize_view(&state, &viewer, target).await?;
    let (from, to) = resolve_range(&q)?;
    attendance_service::ensure_range(&state.db, target, from, to).await?;
    let days = attendance::list_range(&state.db, target, from, to).await?;
    Ok(Json(json!({ "from": from, "to": to, "days": days })))
}

/// `GET /admin/attendance` — per-employee summary report. HR sees all; a PM sees
/// only their own team.
async fn attendance_report(
    State(state): State<AppState>,
    RequireStaff(viewer): RequireStaff,
    Query(q): Query<RangeQuery>,
) -> Result<Json<Value>, AppError> {
    let (from, to) = resolve_range(&q)?;
    let scope = if viewer.role.at_least(UserRole::Hr) {
        None
    } else {
        Some(viewer.id)
    };
    let rows = attendance::report(&state.db, from, to, scope).await?;
    Ok(Json(json!({ "from": from, "to": to, "employees": rows })))
}

#[derive(Deserialize)]
struct MonthQuery {
    /// `YYYY-MM` (or any full `YYYY-MM-DD` within the month). Absent → this month.
    month: Option<String>,
    /// `csv` streams a downloadable spreadsheet; anything else (or absent) → JSON.
    format: Option<String>,
}

/// One employee's row in the monthly report: the day counts from the attendance
/// rollup plus the paid-leave balance left for the year.
struct MonthlyRow {
    user_id: Uuid,
    name: String,
    email: String,
    present: i64,
    partial: i64,
    absent: i64,
    leave: i64,
    holiday: i64,
    weekend: i64,
    worked_seconds: i64,
    leaves_remaining: f64,
}

/// Quote a CSV field iff it contains a comma, quote, CR or LF (RFC 4180).
fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// Render the report as an RFC-4180 CSV with a trailing TOTALS row.
fn to_csv(rows: &[MonthlyRow]) -> String {
    let mut out = String::from(
        "Name,Email,Present,Partial,Absent,Leave,Holiday,Weekend,Worked Hours,Leaves Remaining\r\n",
    );
    let hours = |secs: i64| format!("{:.1}", secs as f64 / 3600.0);
    let (mut p, mut pa, mut ab, mut lv, mut ho, mut we, mut wk) = (0, 0, 0, 0, 0, 0, 0i64);
    for r in rows {
        p += r.present;
        pa += r.partial;
        ab += r.absent;
        lv += r.leave;
        ho += r.holiday;
        we += r.weekend;
        wk += r.worked_seconds;
        out.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{}\r\n",
            csv_field(&r.name),
            csv_field(&r.email),
            r.present,
            r.partial,
            r.absent,
            r.leave,
            r.holiday,
            r.weekend,
            hours(r.worked_seconds),
            format_args!("{:.1}", r.leaves_remaining),
        ));
    }
    out.push_str(&format!(
        "TOTALS ({} employees),,{p},{pa},{ab},{lv},{ho},{we},{},\r\n",
        rows.len(),
        hours(wk),
    ));
    out
}

/// Parse the `month` param to the first day of that month, defaulting to the
/// current month. Accepts `YYYY-MM` and full `YYYY-MM-DD`.
fn resolve_month(q: &MonthQuery) -> Result<NaiveDate, AppError> {
    let today = Utc::now().date_naive();
    let Some(raw) = q.month.as_deref().map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(month_key(today));
    };
    // Full date first, then the bare `YYYY-MM` (normalised to the 1st).
    let day = NaiveDate::parse_from_str(raw, "%Y-%m-%d")
        .or_else(|_| NaiveDate::parse_from_str(&format!("{raw}-01"), "%Y-%m-%d"))
        .map_err(|_| AppError::BadRequest(format!("invalid month {raw:?} (expected YYYY-MM)")))?;
    Ok(month_key(day))
}

/// `GET /admin/attendance/monthly?month=YYYY-MM` — the HR monthly attendance
/// report: one row per employee with present/partial/absent/leave/holiday/weekend
/// day counts for the month, plus `leaves_remaining` (paid-leave balance left for
/// the year — it falls as leave is approved). HR sees everyone; a PM sees only
/// their team. A company `totals` row sums the columns.
///
/// Reads the rolled-up `attendance_days` cache (populated nightly), consistent
/// with the range report — it does not force a recompute.
///
/// `?format=csv` returns the same data as a downloadable spreadsheet instead of
/// JSON (Content-Disposition attachment, named for the month).
async fn monthly_attendance_report(
    State(state): State<AppState>,
    RequireStaff(viewer): RequireStaff,
    Query(q): Query<MonthQuery>,
) -> Result<Response, AppError> {
    let month = resolve_month(&q)?;
    let from = month;
    let to = month_end(month);
    let scope = if viewer.role.at_least(UserRole::Hr) {
        None
    } else {
        Some(viewer.id)
    };

    let summaries = attendance::report(&state.db, from, to, scope).await?;
    let leaves = leave::remaining_paid_by_user(&state.db, month.year(), scope).await?;

    // Merge the paid-leave balance onto each attendance row once; JSON and CSV
    // are two renderings of the same list.
    let rows: Vec<MonthlyRow> = summaries
        .into_iter()
        .map(|r| MonthlyRow {
            leaves_remaining: leaves.get(&r.user_id).copied().unwrap_or(0.0),
            user_id: r.user_id,
            name: r.name,
            email: r.email,
            present: r.present,
            partial: r.partial,
            absent: r.absent,
            leave: r.leave,
            holiday: r.holiday,
            weekend: r.weekend,
            worked_seconds: r.worked_seconds,
        })
        .collect();

    if q.format.as_deref() == Some("csv") {
        let body = to_csv(&rows);
        let filename = format!("attendance-{}.csv", month.format("%Y-%m"));
        return Ok((
            [
                (header::CONTENT_TYPE, "text/csv; charset=utf-8".to_string()),
                (
                    header::CONTENT_DISPOSITION,
                    format!("attachment; filename=\"{filename}\""),
                ),
            ],
            body,
        )
            .into_response());
    }

    let mut t = [0i64; 7];
    let employees: Vec<Value> = rows
        .iter()
        .map(|r| {
            t[0] += r.present;
            t[1] += r.partial;
            t[2] += r.absent;
            t[3] += r.leave;
            t[4] += r.holiday;
            t[5] += r.weekend;
            t[6] += r.worked_seconds;
            json!({
                "user_id": r.user_id,
                "name": r.name,
                "email": r.email,
                "present": r.present,
                "partial": r.partial,
                "absent": r.absent,
                "leave": r.leave,
                "holiday": r.holiday,
                "weekend": r.weekend,
                "worked_seconds": r.worked_seconds,
                "leaves_remaining": r.leaves_remaining,
            })
        })
        .collect();

    Ok(Json(json!({
        "month": month.format("%Y-%m").to_string(),
        "from": from,
        "to": to,
        "employees": employees,
        "totals": {
            "employees": rows.len(),
            "present": t[0],
            "partial": t[1],
            "absent": t[2],
            "leave": t[3],
            "holiday": t[4],
            "weekend": t[5],
            "worked_seconds": t[6],
        }
    }))
    .into_response())
}

#[derive(Deserialize)]
struct RollupQuery {
    day: Option<NaiveDate>,
}

/// `POST /admin/attendance/rollup?day=` — recompute a day for every employee
/// (HR only). Defaults to yesterday. Audited.
async fn rollup(
    State(state): State<AppState>,
    RequireHr(hr): RequireHr,
    Query(q): Query<RollupQuery>,
) -> Result<Json<Value>, AppError> {
    let day = q
        .day
        .unwrap_or_else(|| (Utc::now() - Duration::days(1)).date_naive());
    let count = attendance_service::rollup_all_for_day(&state.db, day).await?;
    audit::log(&state.db, hr.id, "attendance.rollup", "attendance", None).await;
    Ok(Json(json!({ "day": day, "employees": count })))
}

#[derive(Deserialize)]
struct OverrideBody {
    status: String,
    #[serde(default)]
    note: String,
}

/// `PUT /admin/users/:id/attendance/:day` — HR sets (overrides) a user's status
/// for a day. The edit is pinned so the nightly rollup won't overwrite it.
async fn set_attendance(
    State(state): State<AppState>,
    RequireHr(hr): RequireHr,
    Path((target, day)): Path<(Uuid, NaiveDate)>,
    Json(body): Json<OverrideBody>,
) -> Result<Json<Value>, AppError> {
    if !VALID_STATUSES.contains(&body.status.as_str()) {
        return Err(AppError::BadRequest(format!(
            "status must be one of: {}",
            VALID_STATUSES.join(", ")
        )));
    }
    if users::find_by_id(&state.db, target).await?.is_none() {
        return Err(AppError::NotFound);
    }
    let row = attendance_service::override_day(
        &state.db,
        target,
        day,
        &body.status,
        body.note.trim(),
        hr.id,
    )
    .await?;
    audit::log(
        &state.db,
        hr.id,
        "attendance.override",
        "user",
        Some(target),
    )
    .await;
    Ok(Json(json!(row)))
}

/// `DELETE /admin/users/:id/attendance/:day` — HR reverts a day back to the
/// automatically-derived status (recomputed from intervals).
async fn clear_attendance(
    State(state): State<AppState>,
    RequireHr(hr): RequireHr,
    Path((target, day)): Path<(Uuid, NaiveDate)>,
) -> Result<Json<Value>, AppError> {
    if users::find_by_id(&state.db, target).await?.is_none() {
        return Err(AppError::NotFound);
    }
    let row = attendance_service::clear_override(&state.db, target, day).await?;
    audit::log(
        &state.db,
        hr.id,
        "attendance.override.clear",
        "user",
        Some(target),
    )
    .await;
    Ok(Json(json!(row)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str, email: &str, present: i64, worked: i64, remaining: f64) -> MonthlyRow {
        MonthlyRow {
            user_id: Uuid::nil(),
            name: name.into(),
            email: email.into(),
            present,
            partial: 1,
            absent: 2,
            leave: 3,
            holiday: 0,
            weekend: 8,
            worked_seconds: worked,
            leaves_remaining: remaining,
        }
    }

    #[test]
    fn csv_field_quotes_only_when_needed() {
        assert_eq!(csv_field("Alice"), "Alice");
        assert_eq!(csv_field("Doe, John"), "\"Doe, John\"");
        // An embedded quote is doubled (RFC 4180), and the field is wrapped.
        assert_eq!(csv_field("a\"b"), "\"a\"\"b\"");
        assert_eq!(csv_field("two\nlines"), "\"two\nlines\"");
    }

    #[test]
    fn csv_has_header_rows_and_a_totals_line() {
        let rows = [
            row("Alice", "a@x.io", 18, 8 * 3600, 12.0),
            row("Bob, Jr.", "b@x.io", 20, 9 * 3600 + 1800, 1.5),
        ];
        let csv = to_csv(&rows);
        let lines: Vec<&str> = csv.lines().collect();

        assert!(lines[0].starts_with(
            "Name,Email,Present,Partial,Absent,Leave,Holiday,Weekend,Worked Hours,Leaves Remaining"
        ));
        // Worked seconds render as hours; the comma in "Bob, Jr." forces quoting.
        assert_eq!(lines[1], "Alice,a@x.io,18,1,2,3,0,8,8.0,12.0");
        assert_eq!(lines[2], "\"Bob, Jr.\",b@x.io,20,1,2,3,0,8,9.5,1.5");
        // Totals: present 18+20=38, worked 17.5h, and the balance column is left blank.
        let totals = lines.last().unwrap();
        assert!(totals.starts_with("TOTALS (2 employees),,38,2,4,6,0,16,17.5,"));
    }

    #[test]
    fn empty_report_is_just_the_header_and_a_zero_totals_row() {
        let csv = to_csv(&[]);
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines.len(), 2, "header + totals only");
        assert!(lines[1].starts_with("TOTALS (0 employees),,0,0,0,0,0,0,0.0,"));
    }
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/me/attendance", get(my_attendance))
        .route("/admin/users/:id/attendance", get(user_attendance))
        .route(
            "/admin/users/:id/attendance/:day",
            axum::routing::put(set_attendance).delete(clear_attendance),
        )
        .route("/admin/attendance", get(attendance_report))
        .route("/admin/attendance/monthly", get(monthly_attendance_report))
        .route("/admin/attendance/rollup", post(rollup))
}

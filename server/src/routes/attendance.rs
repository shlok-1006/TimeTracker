//! Attendance routes (Feature 6C).
//!
//!   GET  /me/attendance?from=&to=               own calendar (derived rows)
//!   GET  /admin/users/:id/attendance?from=&to=  drill-down (HR all, PM own team)
//!   GET  /admin/attendance?from=&to=            per-employee report (HR all, PM team)
//!   POST /admin/attendance/rollup?day=          recompute a day for everyone (HR)
//!
//! Calendar/drill-down endpoints lazily roll up missing days (and refresh today)
//! so data appears without waiting for the nightly job.

use axum::{
    extract::{Path, Query, State},
    routing::{get, post},
    Json, Router,
};
use chrono::{Datelike, Duration, NaiveDate, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::attendance_service;
use crate::db::{attendance, audit};
use crate::error::AppError;
use crate::middleware::{AuthUser, RequireAdmin, RequireHr};
use crate::role::UserRole;
use crate::routes::admin::authorize_view;
use crate::state::AppState;

/// Cap a range so a single request can't roll up an unbounded number of days.
const MAX_RANGE_DAYS: i64 = 366;

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
    RequireAdmin(viewer): RequireAdmin,
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
    RequireAdmin(viewer): RequireAdmin,
    Query(q): Query<RangeQuery>,
) -> Result<Json<Value>, AppError> {
    let (from, to) = resolve_range(&q)?;
    let scope = if viewer.role == UserRole::Hr { None } else { Some(viewer.id) };
    let rows = attendance::report(&state.db, from, to, scope).await?;
    Ok(Json(json!({ "from": from, "to": to, "employees": rows })))
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
    let day = q.day.unwrap_or_else(|| (Utc::now() - Duration::days(1)).date_naive());
    let count = attendance_service::rollup_all_for_day(&state.db, day).await?;
    audit::log(&state.db, hr.id, "attendance.rollup", "attendance", None).await;
    Ok(Json(json!({ "day": day, "employees": count })))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/me/attendance", get(my_attendance))
        .route("/admin/users/:id/attendance", get(user_attendance))
        .route("/admin/attendance", get(attendance_report))
        .route("/admin/attendance/rollup", post(rollup))
}

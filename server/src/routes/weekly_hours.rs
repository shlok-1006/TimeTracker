//! Weekly hours-shortfall routes — the "Weekly Report" HR/PM surface.
//!
//!   GET  /admin/weekly-hours/shortfalls?week_start=YYYY-MM-DD
//!        Employees who did NOT meet their weekly hours for a completed week.
//!        Defaults to the most recent completed Mon–Sun week. HR sees everyone;
//!        a PM sees only their own team (same scope as the pending-leave queue).
//!
//!   POST /admin/weekly-hours/notify   { user_id, week_start }
//!        Send the first-person shortfall email to that employee, at most ONCE per
//!        week: the row's `employee_notified_at` is claimed atomically before the
//!        send (and released if the send fails), so two HR users, the pop-up plus
//!        the tab, or a retry can't double-send. Scoped like the list. The weekly
//!        batch itself no longer emails anyone; this is the "Send Email" action.

use axum::{
    extract::{Query, State},
    routing::{get, post},
    Json, Router,
};
use chrono::{Duration, NaiveDate, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::db::{audit, weekly_hours};
use crate::email_service;
use crate::error::AppError;
use crate::middleware::RequireStaff;
use crate::role::UserRole;
use crate::state::AppState;
use crate::weekly_hours_service::previous_week;

#[derive(Deserialize)]
struct WeekQuery {
    week_start: Option<NaiveDate>,
}

/// `GET /admin/weekly-hours/shortfalls` — the non-compliant employees for a week.
async fn shortfalls(
    State(state): State<AppState>,
    RequireStaff(user): RequireStaff,
    Query(q): Query<WeekQuery>,
) -> Result<Json<Value>, AppError> {
    // Default to the most recent COMPLETED Mon–Sun week (the one the Monday job just ran for).
    let (week_start, week_end) = match q.week_start {
        Some(ws) => (ws, ws + Duration::days(6)),
        None => previous_week(Utc::now().date_naive()),
    };
    // HR sees everyone; a project manager only their own team.
    let scope = if user.role.at_least(UserRole::Hr) {
        None
    } else {
        Some(user.id)
    };
    let rows = weekly_hours::list_shortfalls(&state.db, week_start, scope).await?;
    // `computed` separates "nobody fell short" from "the batch hasn't run for this week yet" (both have
    // zero rows) — the UI must not show a green "everyone completed" for a week that was never computed.
    let computed = weekly_hours::week_has_rows(&state.db, week_start).await?;
    Ok(Json(json!({
        "week_start": week_start,
        "week_end": week_end,
        "computed": computed,
        "rows": rows,
    })))
}

#[derive(Deserialize)]
struct NotifyBody {
    user_id: Uuid,
    week_start: NaiveDate,
}

/// `POST /admin/weekly-hours/notify` — send the shortfall email on demand.
async fn notify(
    State(state): State<AppState>,
    RequireStaff(user): RequireStaff,
    Json(body): Json<NotifyBody>,
) -> Result<Json<Value>, AppError> {
    let scope = if user.role.at_least(UserRole::Hr) {
        None
    } else {
        Some(user.id)
    };
    // Scoped lookup: a PM can only email their own team, and only a genuine (non-compliant) row
    // can be emailed — so a stale/forged request can't send an unwarranted "you're short" mail.
    let row = weekly_hours::find_shortfall(&state.db, body.user_id, body.week_start, scope)
        .await?
        .ok_or(AppError::NotFound)?;

    // Already emailed (by anyone) → idempotent success, no second email.
    if let Some(at) = row.notified_at {
        return Ok(Json(
            json!({ "ok": true, "already_sent": true, "user_id": row.user_id, "notified_at": at }),
        ));
    }
    // Claim the slot atomically; losing the race means another request is sending / has sent it.
    let Some(claimed_at) = weekly_hours::claim_employee_notify(&state.db, row.id).await? else {
        let now =
            weekly_hours::find_shortfall(&state.db, row.user_id, row.week_start, None).await?;
        return Ok(Json(json!({
            "ok": true, "already_sent": true, "user_id": row.user_id,
            "notified_at": now.and_then(|r| r.notified_at),
        })));
    };

    if let Err(e) = email_service::send_hours_shortfall_self(
        &row.email,
        &row.name,
        row.week_start,
        row.week_end,
        row.working_days,
        row.required_seconds,
        row.worked_seconds,
        row.shortfall_seconds,
    )
    .await
    {
        // Give the slot back so HR can retry, and say it was the MAIL that failed.
        weekly_hours::release_employee_notify(&state.db, row.id).await?;
        tracing::warn!(user_id = %row.user_id, week = %row.week_start, "shortfall email failed: {e}");
        return Err(AppError::BadRequest(format!(
            "the email could not be sent: {e}"
        )));
    }

    // Audit the REPORT row (it identifies both the employee and the week).
    audit::log(
        &state.db,
        user.id,
        "weekly_hours.notify",
        "weekly_hours_report",
        Some(row.id),
    )
    .await;

    Ok(Json(json!({
        "ok": true,
        "user_id": row.user_id,
        "notified_at": claimed_at,
    })))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/weekly-hours/shortfalls", get(shortfalls))
        .route("/admin/weekly-hours/notify", post(notify))
}

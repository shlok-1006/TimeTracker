//! Report-viewing routes (Feature 1 Phase 4).
//!
//! RBAC:
//!   * employee → own report only          (`GET /me/report`)
//!   * project manager → own team only      (`GET /admin/reports`, drill-down)
//!   * HR → everyone                         (all)
//!
//! Scoping reuses `admin::team_scope` (roster filter) and `admin::authorize_view`
//! (per-employee drill-down) so it stays consistent with the other admin views.

use axum::{
    extract::{Path, Query, State},
    routing::get,
    Json, Router,
};
use chrono::{NaiveDate, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::db::analysis_reports;
use crate::error::AppError;
use crate::middleware::{AuthUser, RequireAdmin};
use crate::routes::admin::{authorize_view, team_scope};
use crate::state::AppState;

#[derive(Deserialize)]
struct DayQuery {
    day: Option<NaiveDate>,
}

fn resolve_day(q: &DayQuery) -> NaiveDate {
    q.day.unwrap_or_else(|| Utc::now().date_naive())
}

/// `GET /admin/reports?day=` — roster of reports for a day. HR: all employees;
/// project manager: only their team.
async fn admin_reports(
    State(state): State<AppState>,
    RequireAdmin(user): RequireAdmin,
    Query(q): Query<DayQuery>,
) -> Result<Json<Value>, AppError> {
    let day = resolve_day(&q);
    let reports = analysis_reports::list_for_day(&state.db, team_scope(&user), day).await?;
    Ok(Json(json!({ "day": day, "reports": reports })))
}

/// `GET /admin/users/:id/report?day=` — one employee's report (HR any; PM team).
async fn user_report(
    State(state): State<AppState>,
    RequireAdmin(user): RequireAdmin,
    Path(target): Path<Uuid>,
    Query(q): Query<DayQuery>,
) -> Result<Json<Value>, AppError> {
    authorize_view(&state, &user, target).await?;
    let day = resolve_day(&q);
    let report = analysis_reports::get(&state.db, target, day).await?;
    Ok(Json(json!({ "day": day, "report": report })))
}

/// `GET /me/report?day=` — the caller's own report.
async fn my_report(
    State(state): State<AppState>,
    user: AuthUser,
    Query(q): Query<DayQuery>,
) -> Result<Json<Value>, AppError> {
    let day = resolve_day(&q);
    let report = analysis_reports::get(&state.db, user.id, day).await?;
    Ok(Json(json!({ "day": day, "report": report })))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/reports", get(admin_reports))
        .route("/admin/users/:id/report", get(user_report))
        .route("/me/report", get(my_report))
}

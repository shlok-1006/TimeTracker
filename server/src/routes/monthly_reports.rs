//! Monthly summary report routes.
//!
//! RBAC mirrors the daily reports (routes/reports.rs):
//!   * employee        → own report only            (`GET /me/reports/monthly`)
//!   * project manager → their team (drill-down + roster)
//!   * HR              → everyone
//!
//! Generation is HR/PM only (`RequireAdmin` + `authorize_view` for the target),
//! so an employee can read their month but never (re)write it. Every generation
//! is audited — a monthly summary feeds performance conversations, so who asked
//! for it and when must be traceable.

use axum::{
    extract::{Path, Query, State},
    routing::get,
    Json, Router,
};
use chrono::NaiveDate;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::db::{audit, monthly_reports};
use crate::error::AppError;
use crate::middleware::{AuthUser, RequireAdmin};
use crate::monthly_report_service as service;
use crate::org_time;
use crate::routes::admin::{authorize_view, team_scope};
use crate::state::AppState;

/// `?month=YYYY-MM-DD` (any day in the month) — defaults to the current
/// org-local (IST) month.
#[derive(Deserialize)]
struct MonthQuery {
    month: Option<NaiveDate>,
}

fn resolve_month(q: &MonthQuery) -> NaiveDate {
    service::month_key(q.month.unwrap_or_else(org_time::today))
}

/// `GET /me/reports/monthly?month=` — the caller's own monthly summary.
/// `report: null` means it hasn't been generated for that month yet.
async fn my_monthly(
    State(state): State<AppState>,
    user: AuthUser,
    Query(q): Query<MonthQuery>,
) -> Result<Json<Value>, AppError> {
    let month = resolve_month(&q);
    let report = monthly_reports::get(&state.db, user.id, month).await?;
    Ok(Json(json!({ "month": month, "report": report })))
}

/// `GET /admin/users/:id/reports/monthly?month=` — one employee's monthly
/// summary (HR anyone; PM only their own reports).
async fn user_monthly(
    State(state): State<AppState>,
    RequireAdmin(user): RequireAdmin,
    Path(target): Path<Uuid>,
    Query(q): Query<MonthQuery>,
) -> Result<Json<Value>, AppError> {
    authorize_view(&state, &user, target).await?;
    let month = resolve_month(&q);
    let report = monthly_reports::get(&state.db, target, month).await?;
    Ok(Json(json!({ "month": month, "report": report })))
}

/// `POST /admin/users/:id/reports/monthly?month=` — generate (or regenerate)
/// on demand, for any month including the one still running.
async fn generate_user_monthly(
    State(state): State<AppState>,
    RequireAdmin(user): RequireAdmin,
    Path(target): Path<Uuid>,
    Query(q): Query<MonthQuery>,
) -> Result<Json<Value>, AppError> {
    authorize_view(&state, &user, target).await?;
    let month = resolve_month(&q);
    let report = service::build(&state.db, target, month, Some(user.id)).await?;
    audit::log(
        &state.db,
        user.id,
        "monthly_report.generate",
        "monthly_report",
        Some(report.id),
    )
    .await;
    Ok(Json(json!({ "month": month, "report": report })))
}

/// `GET /admin/reports/monthly?month=` — the roster for a month. HR sees
/// everyone; a project manager sees only their team.
async fn monthly_roster(
    State(state): State<AppState>,
    RequireAdmin(user): RequireAdmin,
    Query(q): Query<MonthQuery>,
) -> Result<Json<Value>, AppError> {
    let month = resolve_month(&q);
    let reports = monthly_reports::list_for_month(&state.db, team_scope(&user), month).await?;
    Ok(Json(json!({ "month": month, "reports": reports })))
}

/// `POST /admin/reports/monthly?month=` — generate for EVERYONE in scope in one
/// go (HR: all employees; PM: their team). This is the "give me the month for my
/// team" button; per-employee failures are reported but never abort the batch.
async fn generate_monthly_roster(
    State(state): State<AppState>,
    RequireAdmin(user): RequireAdmin,
    Query(q): Query<MonthQuery>,
) -> Result<Json<Value>, AppError> {
    let month = resolve_month(&q);
    // HR: every user. PM: only the employees they manage.
    let targets: Vec<Uuid> = match team_scope(&user) {
        None => crate::db::users::list_all(&state.db)
            .await?
            .into_iter()
            .map(|u| u.id)
            .collect(),
        Some(pm) => crate::db::users::managed_by(&state.db, pm).await?,
    };
    let mut generated = 0usize;
    let mut failed = 0usize;
    for id in &targets {
        match service::build(&state.db, *id, month, Some(user.id)).await {
            Ok(_) => generated += 1,
            Err(e) => {
                failed += 1;
                tracing::warn!(user_id = %id, %month, "monthly report generation failed: {e}");
            }
        }
    }
    audit::log(
        &state.db,
        user.id,
        "monthly_report.generate_batch",
        "monthly_report",
        None,
    )
    .await;
    Ok(Json(
        json!({ "month": month, "generated": generated, "failed": failed }),
    ))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/me/reports/monthly", get(my_monthly))
        .route(
            "/admin/users/:id/reports/monthly",
            get(user_monthly).post(generate_user_monthly),
        )
        .route(
            "/admin/reports/monthly",
            get(monthly_roster).post(generate_monthly_roster),
        )
}

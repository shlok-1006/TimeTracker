//! Leave routes: employee self-service (`/me/leave/*`), approver actions and HR
//! configuration (`/admin/leave/*`, `/admin/holidays`).
//!
//! RBAC: employees manage their own requests; project managers approve their own
//! team's requests (HR approves anyone's); only HR configures leave types,
//! allocations, and holidays.

use axum::{
    extract::{Path, Query, State},
    routing::{get, post},
    Json, Router,
};
use chrono::{Datelike, NaiveDate, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::db::{audit, leave, users};
use crate::error::AppError;
use crate::leave_service;
use crate::middleware::{AuthUser, RequireAdmin, RequireHr};
use crate::role::UserRole;
use crate::state::AppState;

#[derive(Deserialize)]
struct YearQuery {
    year: Option<i32>,
}

// ---- Employee self-service ----

async fn my_types(State(state): State<AppState>, _u: AuthUser) -> Result<Json<Value>, AppError> {
    Ok(Json(json!(leave::list_types(&state.db).await?)))
}

async fn my_balance(
    State(state): State<AppState>,
    user: AuthUser,
    Query(q): Query<YearQuery>,
) -> Result<Json<Value>, AppError> {
    let year = q.year.unwrap_or_else(|| Utc::now().year());
    Ok(Json(json!({
        "year": year,
        "balances": leave::balances(&state.db, user.id, year).await?,
    })))
}

async fn my_requests(State(state): State<AppState>, user: AuthUser) -> Result<Json<Value>, AppError> {
    Ok(Json(json!(leave::list_requests_for_user(&state.db, user.id).await?)))
}

#[derive(Deserialize)]
struct NewLeave {
    leave_type_id: Uuid,
    start_date: NaiveDate,
    end_date: NaiveDate,
    #[serde(default)]
    reason: String,
}

async fn request_leave(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<NewLeave>,
) -> Result<Json<Value>, AppError> {
    let (id, days) = leave_service::submit_request(
        &state.db,
        user.id,
        body.leave_type_id,
        body.start_date,
        body.end_date,
        &body.reason,
    )
    .await?;
    audit::log(&state.db, user.id, "leave.request", "leave_request", Some(id)).await;
    Ok(Json(json!({ "id": id, "days": days, "status": "pending" })))
}

async fn cancel_leave(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    if !leave::cancel(&state.db, id, user.id).await? {
        return Err(AppError::NotFound);
    }
    Ok(Json(json!({ "id": id, "status": "cancelled" })))
}

// ---- Approver (HR / project manager) ----

/// Ensure the caller may act on `target`'s request: HR anyone; PM only own team.
async fn authorize_approver(
    state: &AppState,
    approver: &AuthUser,
    target: Uuid,
) -> Result<(), AppError> {
    if approver.role == UserRole::Hr {
        return Ok(());
    }
    match users::manager_id_of(&state.db, target).await? {
        Some(m) if m == approver.id => Ok(()),
        _ => Err(AppError::Forbidden),
    }
}

async fn pending_requests(
    State(state): State<AppState>,
    RequireAdmin(user): RequireAdmin,
) -> Result<Json<Value>, AppError> {
    // HR sees all; a project manager sees only their team's.
    let scope = if user.role == UserRole::Hr { None } else { Some(user.id) };
    Ok(Json(json!(leave::list_pending(&state.db, scope).await?)))
}

async fn decide_request(
    state: &AppState,
    approver: &AuthUser,
    id: Uuid,
    status: &str,
) -> Result<Json<Value>, AppError> {
    let (owner, current) = leave::owner_and_status(&state.db, id)
        .await?
        .ok_or(AppError::NotFound)?;
    authorize_approver(state, approver, owner).await?;
    if current != "pending" {
        return Err(AppError::BadRequest(format!("request is already {current}")));
    }
    if !leave::decide(&state.db, id, status, approver.id).await? {
        return Err(AppError::BadRequest("request is no longer pending".into()));
    }
    audit::log(&state.db, approver.id, &format!("leave.{status}"), "leave_request", Some(id)).await;
    Ok(Json(json!({ "id": id, "status": status })))
}

async fn approve_request(
    State(state): State<AppState>,
    RequireAdmin(user): RequireAdmin,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    decide_request(&state, &user, id, "approved").await
}

async fn reject_request(
    State(state): State<AppState>,
    RequireAdmin(user): RequireAdmin,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    decide_request(&state, &user, id, "rejected").await
}

// ---- HR configuration ----

#[derive(Deserialize)]
struct NewType {
    name: String,
    #[serde(default = "default_true")]
    paid: bool,
    #[serde(default)]
    default_days: f64,
}
fn default_true() -> bool {
    true
}

async fn create_type(
    State(state): State<AppState>,
    RequireHr(hr): RequireHr,
    Json(body): Json<NewType>,
) -> Result<Json<Value>, AppError> {
    if body.name.trim().is_empty() {
        return Err(AppError::BadRequest("name is required".into()));
    }
    let t = leave::create_type(&state.db, body.name.trim(), body.paid, body.default_days).await?;
    audit::log(&state.db, hr.id, "leave.type.create", "leave_type", Some(t.id)).await;
    Ok(Json(json!(t)))
}

#[derive(Deserialize)]
struct NewAllocation {
    user_id: Uuid,
    leave_type_id: Uuid,
    year: Option<i32>,
    allotted_days: f64,
}

async fn allocate(
    State(state): State<AppState>,
    RequireHr(hr): RequireHr,
    Json(body): Json<NewAllocation>,
) -> Result<Json<Value>, AppError> {
    let year = body.year.unwrap_or_else(|| Utc::now().year());
    leave::upsert_allocation(&state.db, body.user_id, body.leave_type_id, year, body.allotted_days)
        .await?;
    audit::log(&state.db, hr.id, "leave.allocate", "user", Some(body.user_id)).await;
    Ok(Json(json!({
        "user_id": body.user_id, "leave_type_id": body.leave_type_id,
        "year": year, "allotted_days": body.allotted_days
    })))
}

async fn list_holidays(
    State(state): State<AppState>,
    RequireAdmin(_u): RequireAdmin,
    Query(q): Query<YearQuery>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(json!(leave::list_holidays(&state.db, q.year).await?)))
}

#[derive(Deserialize)]
struct NewHoliday {
    day: NaiveDate,
    name: String,
}

async fn create_holiday(
    State(state): State<AppState>,
    RequireHr(hr): RequireHr,
    Json(body): Json<NewHoliday>,
) -> Result<Json<Value>, AppError> {
    let h = leave::create_holiday(&state.db, body.day, body.name.trim()).await?;
    audit::log(&state.db, hr.id, "holiday.create", "holiday", Some(h.id)).await;
    Ok(Json(json!(h)))
}

pub fn router() -> Router<AppState> {
    Router::new()
        // Employee self-service
        .route("/me/leave/types", get(my_types))
        .route("/me/leave/balance", get(my_balance))
        .route("/me/leave/requests", get(my_requests).post(request_leave))
        .route("/me/leave/requests/:id/cancel", post(cancel_leave))
        // Approver
        .route("/admin/leave/requests", get(pending_requests))
        .route("/admin/leave/requests/:id/approve", post(approve_request))
        .route("/admin/leave/requests/:id/reject", post(reject_request))
        // HR configuration
        .route("/admin/leave/types", post(create_type))
        .route("/admin/leave/allocations", post(allocate))
        .route("/admin/holidays", get(list_holidays).post(create_holiday))
}

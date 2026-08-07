//! Employee directory routes — the onboarding form's data, read back.
//!
//! RBAC is the access matrix from the RUH HRMS "Employee & Teams" proposal,
//! enforced here rather than in the UI:
//!
//!   own personal details      employee (`GET /me/directory/profile`)
//!   team members' details     PM, only where they are the manager
//!   everyone's details        HR
//!   edit / verify             HR only — the form is the employee's claim until
//!                             HR checks it, after which it is the single truth
//!   bank details              HR only, separate routes (see below)
//!
//! The sealed tier is deliberately its own endpoint rather than a field on the
//! profile response. Nothing that merely lists people can leak an account
//! number, and a PM cannot reach it at all — `RequireHr`, not `RequireAdmin`.

use axum::{
    extract::{Path, State},
    routing::{get, post},
    Json, Router,
};
use chrono::NaiveDate;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::db::{audit, employee_directory as repo};
use crate::error::AppError;
use crate::middleware::{AuthUser, RequireAdmin, RequireHr};
use crate::routes::admin::{authorize_view, team_scope};
use crate::state::AppState;

/// `GET /me/directory/profile` — the caller's own record. Employees have no
/// other way to see what the onboarding form recorded about them.
async fn my_profile(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<Value>, AppError> {
    let bundle = repo::get_bundle(&state.db, user.id).await?;
    Ok(Json(json!({ "profile": bundle })))
}

/// `GET /admin/directory` — the roster. HR sees everyone; a PM sees only the
/// people they manage.
async fn directory(
    State(state): State<AppState>,
    RequireAdmin(user): RequireAdmin,
) -> Result<Json<Value>, AppError> {
    let people = repo::list_directory(&state.db, team_scope(&user)).await?;
    Ok(Json(json!({ "people": people })))
}

/// `GET /admin/directory/:id` — one person's full tier-2 record.
async fn user_profile(
    State(state): State<AppState>,
    RequireAdmin(user): RequireAdmin,
    Path(target): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    authorize_view(&state, &user, target).await?;
    let bundle = repo::get_bundle(&state.db, target).await?;
    Ok(Json(json!({ "profile": bundle })))
}

#[derive(Deserialize)]
struct ProfileUpdate {
    // Tier-1 employment facts (live on the core row).
    employee_code: Option<String>,
    department: Option<String>,
    designation: Option<String>,
    joined_on: Option<NaiveDate>,
    // Tier-2 personal details.
    #[serde(default)]
    profile: Option<repo::EmployeeProfile>,
    #[serde(default)]
    education: Option<Vec<repo::Education>>,
    #[serde(default)]
    prev_employment: Option<Vec<repo::PrevEmployment>>,
}

/// `PUT /admin/directory/:id` — HR corrects or completes a record.
///
/// HR-only by design: a PM can read their team's details but must not be able
/// to rewrite someone's date of birth or address. Each list field is optional —
/// omitting `education` leaves it alone, sending `[]` clears it.
async fn update_profile(
    State(state): State<AppState>,
    RequireHr(user): RequireHr,
    Path(target): Path<Uuid>,
    Json(body): Json<ProfileUpdate>,
) -> Result<Json<Value>, AppError> {
    repo::set_employment_facts(
        &state.db,
        target,
        body.employee_code.as_deref(),
        body.department.as_deref(),
        body.designation.as_deref(),
        body.joined_on,
    )
    .await?;
    if let Some(p) = &body.profile {
        repo::upsert_profile(&state.db, target, p).await?;
    }
    if let Some(rows) = &body.education {
        repo::replace_education(&state.db, target, rows).await?;
    }
    if let Some(rows) = &body.prev_employment {
        repo::replace_prev_employment(&state.db, target, rows).await?;
    }
    audit::log(
        &state.db,
        user.id,
        "employee_profile.update",
        "user",
        Some(target),
    )
    .await;
    let bundle = repo::get_bundle(&state.db, target).await?;
    Ok(Json(json!({ "profile": bundle })))
}

/// `POST /admin/directory/:id/verify` — HR confirms the form's answers are
/// checked. Audited: "who said this data is true" is exactly the kind of claim
/// that needs a name against it.
async fn verify_profile(
    State(state): State<AppState>,
    RequireHr(user): RequireHr,
    Path(target): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    repo::mark_verified(&state.db, target, user.id).await?;
    audit::log(
        &state.db,
        user.id,
        "employee_profile.verify",
        "user",
        Some(target),
    )
    .await;
    let bundle = repo::get_bundle(&state.db, target).await?;
    Ok(Json(json!({ "profile": bundle })))
}

/// `GET /admin/directory/:id/bank` — sealed tier. HR only, and every read is
/// audited: unlike the rest of the record, merely *looking* at bank details is
/// an event worth being able to reconstruct later.
async fn bank(
    State(state): State<AppState>,
    RequireHr(user): RequireHr,
    Path(target): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    let details = repo::get_bank(&state.db, target).await?;
    audit::log(
        &state.db,
        user.id,
        "employee_bank.view",
        "user",
        Some(target),
    )
    .await;
    Ok(Json(json!({ "bank": details })))
}

/// `PUT /admin/directory/:id/bank` — sealed tier, HR only, audited.
async fn set_bank(
    State(state): State<AppState>,
    RequireHr(user): RequireHr,
    Path(target): Path<Uuid>,
    Json(body): Json<repo::BankDetails>,
) -> Result<Json<Value>, AppError> {
    repo::upsert_bank(&state.db, target, &body).await?;
    audit::log(
        &state.db,
        user.id,
        "employee_bank.update",
        "user",
        Some(target),
    )
    .await;
    Ok(Json(json!({ "ok": true })))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/me/directory/profile", get(my_profile))
        .route("/admin/directory", get(directory))
        .route("/admin/directory/:id", get(user_profile).put(update_profile))
        .route("/admin/directory/:id/verify", post(verify_profile))
        .route("/admin/directory/:id/bank", get(bank).put(set_bank))
}

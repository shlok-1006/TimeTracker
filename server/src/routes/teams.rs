//! Team management routes (Feature 4 Phase 2). HR only.
//!
//! Endpoints:
//!   POST   /teams                        create a team
//!   PATCH  /teams/:id                    rename / re-describe
//!   DELETE /teams/:id                    delete (cascades membership)
//!   POST   /teams/:id/members            add an employee   { user_id }
//!   DELETE /teams/:id/members/:user_id   remove an employee
//!   GET    /teams                        list teams
//!   GET    /teams/:id/members            list a team's members
//!
//! All guarded by `RequireHr` and audit-logged.

use axum::{
    extract::{Path, State},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::db::{audit, teams};
use crate::error::AppError;
use crate::middleware::{AuthUser, RequireAdmin, RequireHr};
use crate::state::AppState;

/// `GET /admin/teams` — all teams with member counts (HR or project manager).
async fn admin_list_teams(
    State(state): State<AppState>,
    RequireAdmin(_user): RequireAdmin,
) -> Result<Json<Value>, AppError> {
    Ok(Json(json!(teams::list_with_counts(&state.db).await?)))
}

/// `GET /admin/teams/:id/summary` — team rollup: total hours, active users,
/// status breakdown, and per-member totals (HR or project manager).
async fn team_summary(
    State(state): State<AppState>,
    RequireAdmin(_user): RequireAdmin,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    let team = teams::get(&state.db, id).await?.ok_or(AppError::NotFound)?;
    let breakdown = teams::status_breakdown(&state.db, id).await?;
    let members = teams::member_totals(&state.db, id).await?;
    let active_users = members.iter().filter(|m| m.worked_seconds > 0).count();

    Ok(Json(json!({
        "team": team,
        "total_seconds": breakdown.total,
        "member_count": members.len(),
        "active_users": active_users,
        "status_breakdown": {
            "active": breakdown.active,
            "idle": breakdown.idle,
            "meeting": breakdown.meeting,
            "break": breakdown.break_,
        },
        "members": members,
    })))
}

/// `GET /me/teams` — the teams the authenticated employee belongs to (used by
/// the desktop's pre-timer team dropdown). Any authenticated user.
async fn my_teams(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<Value>, AppError> {
    Ok(Json(json!(teams::teams_for_user(&state.db, user.id).await?)))
}

/// `GET /me/team-options` — all teams the employee can choose from (self-service
/// team selection).
async fn my_team_options(
    State(state): State<AppState>,
    _user: AuthUser,
) -> Result<Json<Value>, AppError> {
    Ok(Json(json!(teams::list(&state.db).await?)))
}

/// `POST /me/teams/:id/join` — the employee joins a team themselves (idempotent).
async fn join_team(
    State(state): State<AppState>,
    user: AuthUser,
    Path(team_id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    if teams::get(&state.db, team_id).await?.is_none() {
        return Err(AppError::NotFound);
    }
    teams::add_member(&state.db, user.id, team_id).await?;
    audit::log(&state.db, user.id, "team.self_join", "team", Some(team_id)).await;
    Ok(Json(json!({ "team_id": team_id, "joined": true })))
}

/// `POST /me/teams/:id/leave` — the employee leaves a team themselves.
async fn leave_team(
    State(state): State<AppState>,
    user: AuthUser,
    Path(team_id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    let removed = teams::remove_member(&state.db, user.id, team_id).await?;
    audit::log(&state.db, user.id, "team.self_leave", "team", Some(team_id)).await;
    Ok(Json(json!({ "team_id": team_id, "left": removed })))
}

/// `GET /admin/users/:id/teams` — an employee's teams (HR any; PM own team).
async fn user_teams(
    State(state): State<AppState>,
    RequireAdmin(user): RequireAdmin,
    Path(target): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    crate::routes::admin::authorize_view(&state, &user, target).await?;
    Ok(Json(json!(teams::teams_for_user(&state.db, target).await?)))
}

#[derive(Deserialize)]
struct CreateTeam {
    name: String,
    #[serde(default)]
    description: String,
}

async fn create_team(
    State(state): State<AppState>,
    RequireHr(hr): RequireHr,
    Json(body): Json<CreateTeam>,
) -> Result<Json<Value>, AppError> {
    let name = body.name.trim();
    if name.is_empty() {
        return Err(AppError::BadRequest("name is required".into()));
    }
    let team = teams::create(&state.db, name, body.description.trim()).await?;
    audit::log(&state.db, hr.id, "team.create", "team", Some(team.id)).await;
    Ok(Json(json!(team)))
}

async fn list_teams(
    State(state): State<AppState>,
    RequireHr(_hr): RequireHr,
) -> Result<Json<Value>, AppError> {
    Ok(Json(json!(teams::list(&state.db).await?)))
}

#[derive(Deserialize)]
struct UpdateTeam {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

async fn update_team(
    State(state): State<AppState>,
    RequireHr(hr): RequireHr,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateTeam>,
) -> Result<Json<Value>, AppError> {
    // Trim provided fields; reject an explicit empty name.
    let name = body.name.as_deref().map(str::trim);
    if matches!(name, Some("")) {
        return Err(AppError::BadRequest("name cannot be empty".into()));
    }
    let description = body.description.as_deref().map(str::trim);

    let team = teams::update(&state.db, id, name, description)
        .await?
        .ok_or(AppError::NotFound)?;
    audit::log(&state.db, hr.id, "team.update", "team", Some(id)).await;
    Ok(Json(json!(team)))
}

async fn delete_team(
    State(state): State<AppState>,
    RequireHr(hr): RequireHr,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    if !teams::delete(&state.db, id).await? {
        return Err(AppError::NotFound);
    }
    audit::log(&state.db, hr.id, "team.delete", "team", Some(id)).await;
    Ok(Json(json!({ "deleted": true })))
}

async fn list_members(
    State(state): State<AppState>,
    RequireHr(_hr): RequireHr,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(json!(teams::members_of(&state.db, id).await?)))
}

#[derive(Deserialize)]
struct AddMember {
    user_id: Uuid,
}

async fn add_member(
    State(state): State<AppState>,
    RequireHr(hr): RequireHr,
    Path(team_id): Path<Uuid>,
    Json(body): Json<AddMember>,
) -> Result<Json<Value>, AppError> {
    // The team must exist (FK also guards, but this gives a clean 404).
    if teams::get(&state.db, team_id).await?.is_none() {
        return Err(AppError::NotFound);
    }
    teams::add_member(&state.db, body.user_id, team_id).await?;
    audit::log(&state.db, hr.id, "team.add_member", "team", Some(team_id)).await;
    Ok(Json(json!({ "team_id": team_id, "user_id": body.user_id, "added": true })))
}

async fn remove_member(
    State(state): State<AppState>,
    RequireHr(hr): RequireHr,
    Path((team_id, user_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Value>, AppError> {
    if !teams::remove_member(&state.db, user_id, team_id).await? {
        return Err(AppError::NotFound);
    }
    audit::log(&state.db, hr.id, "team.remove_member", "team", Some(team_id)).await;
    Ok(Json(json!({ "team_id": team_id, "user_id": user_id, "removed": true })))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/me/teams", get(my_teams))
        .route("/me/team-options", get(my_team_options))
        .route("/me/teams/:id/join", post(join_team))
        .route("/me/teams/:id/leave", post(leave_team))
        .route("/admin/teams", get(admin_list_teams))
        .route("/admin/teams/:id/summary", get(team_summary))
        .route("/admin/users/:id/teams", get(user_teams))
        .route("/teams", get(list_teams).post(create_team))
        .route(
            "/teams/:id",
            axum::routing::patch(update_team).delete(delete_team),
        )
        .route("/teams/:id/members", post(add_member).get(list_members))
        .route("/teams/:id/members/:user_id", axum::routing::delete(remove_member))
}

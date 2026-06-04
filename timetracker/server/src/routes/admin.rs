//! Admin/dashboard routes (require HR or project-manager — `RequireAdmin`).
//!
//! Scope (CLAUDE.md): HR sees everyone; a project manager sees only their own
//! team (`users.manager_id = <pm>`). Enforced on the team list (query filter)
//! and on every drill-down (explicit `authorize_view`).

use axum::{
    extract::{Path, State},
    routing::get,
    Json, Router,
};
use chrono::Utc;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::db::{intervals, presence, screenshots, users};
use crate::error::AppError;
use crate::middleware::{AuthUser, RequireAdmin};
use crate::role::UserRole;
use crate::state::AppState;

const VIEW_URL_EXPIRES_SECS: u64 = 900;

/// Which employees the caller may see in the team list (`None` = all).
fn team_scope(user: &AuthUser) -> Option<Uuid> {
    match user.role {
        UserRole::Hr => None,
        _ => Some(user.id), // project manager: own team
    }
}

/// Authorize a drill-down on `target`. HR: anyone. PM: only their team.
async fn authorize_view(
    state: &AppState,
    viewer: &AuthUser,
    target: Uuid,
) -> Result<(), AppError> {
    if viewer.role == UserRole::Hr {
        return Ok(());
    }
    match users::manager_id_of(&state.db, target).await? {
        Some(mgr) if mgr == viewer.id => Ok(()),
        _ => Err(AppError::Forbidden),
    }
}

/// `GET /admin/team` — live team statuses + today's hours.
async fn team(
    State(state): State<AppState>,
    RequireAdmin(user): RequireAdmin,
) -> Result<Json<Value>, AppError> {
    let members = presence::team(&state.db, team_scope(&user)).await?;
    let body: Vec<Value> = members
        .into_iter()
        .map(|m| {
            json!({
                "user": { "id": m.id, "name": m.name, "email": m.email, "role": m.role },
                "status": m.status,
                "last_seen_at": m.last_seen_at,
                "today_seconds": m.today_seconds,
            })
        })
        .collect();
    Ok(Json(Value::Array(body)))
}

/// `GET /admin/users/:id/hours` — drill-down hours for one employee.
async fn user_hours(
    State(state): State<AppState>,
    RequireAdmin(user): RequireAdmin,
    Path(target): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    authorize_view(&state, &user, target).await?;
    let s = intervals::hours_summary(&state.db, target).await?;
    Ok(Json(json!({
        "total_seconds": s.total_seconds,
        "today_seconds": s.today_seconds,
        "week_seconds": s.week_seconds,
        "active_seconds": s.active_seconds,
        "idle_seconds": s.idle_seconds,
    })))
}

/// `GET /admin/users/:id/screenshots` — drill-down screenshots (presigned URLs).
async fn user_screenshots(
    State(state): State<AppState>,
    RequireAdmin(user): RequireAdmin,
    Path(target): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    authorize_view(&state, &user, target).await?;
    let now = Utc::now();
    let rows = screenshots::list_for_user(&state.db, target, 60).await?;
    let items: Vec<Value> = rows
        .into_iter()
        .map(|r| {
            json!({
                "id": r.id,
                "taken_at": r.taken_at,
                "url": state.storage.presign_get(&r.storage_key, VIEW_URL_EXPIRES_SECS, now),
            })
        })
        .collect();
    Ok(Json(Value::Array(items)))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/team", get(team))
        .route("/admin/users/:id/hours", get(user_hours))
        .route("/admin/users/:id/screenshots", get(user_screenshots))
}

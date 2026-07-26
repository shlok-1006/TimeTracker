//! Manual "grace" time grants (HR / project manager). Adds time to an
//! employee's current week with a required reason; the amount is folded into the
//! week total by `intervals::hours_summary` and tagged in the UI.
//!
//!   POST   /admin/users/:id/time-grants   add grace  { hours, minutes, reason }
//!   GET    /admin/users/:id/time-grants   list the current week's grants
//!   DELETE /admin/time-grants/:id          remove a grant
//!
//! Scope (CLAUDE.md Rule 11): HR may grant to anyone; a PM only to employees
//! they manage (via `authorize_view`). Every action is audited.

use axum::{
    extract::{Path, State},
    routing::{delete, get},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::db::{audit, time_grants, users};
use crate::error::AppError;
use crate::middleware::RequireAdmin;
use crate::routes::admin::authorize_view;
use crate::state::AppState;

/// Sanity cap so a typo can't grant an absurd amount (one full week).
const MAX_GRANT_SECONDS: i64 = 7 * 24 * 3600;

#[derive(Deserialize)]
struct NewGrant {
    #[serde(default)]
    hours: i64,
    #[serde(default)]
    minutes: i64,
    #[serde(default)]
    reason: String,
}

async fn add_grant(
    State(state): State<AppState>,
    RequireAdmin(actor): RequireAdmin,
    Path(target): Path<Uuid>,
    Json(body): Json<NewGrant>,
) -> Result<Json<Value>, AppError> {
    authorize_view(&state, &actor, target).await?;
    if users::find_by_id(&state.db, target).await?.is_none() {
        return Err(AppError::NotFound);
    }
    let seconds = body.hours.saturating_mul(3600) + body.minutes.saturating_mul(60);
    if seconds <= 0 {
        return Err(AppError::BadRequest("time to add must be positive".into()));
    }
    if seconds > MAX_GRANT_SECONDS {
        return Err(AppError::BadRequest("time to add is unreasonably large".into()));
    }
    let reason = body.reason.trim();
    if reason.is_empty() {
        return Err(AppError::BadRequest("a reason is required".into()));
    }

    let week_start = time_grants::current_week_start(&state.db, target).await?;
    let grant =
        time_grants::create(&state.db, target, week_start, seconds as i32, reason, actor.id).await?;
    audit::log(&state.db, actor.id, "time.grant", "user", Some(target)).await;
    Ok(Json(json!(grant)))
}

async fn list_grants(
    State(state): State<AppState>,
    RequireAdmin(actor): RequireAdmin,
    Path(target): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    authorize_view(&state, &actor, target).await?;
    let week_start = time_grants::current_week_start(&state.db, target).await?;
    let grants = time_grants::list_for_week(&state.db, target, week_start).await?;
    Ok(Json(json!({ "week_start": week_start, "grants": grants })))
}

async fn delete_grant(
    State(state): State<AppState>,
    RequireAdmin(actor): RequireAdmin,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    let owner = time_grants::owner(&state.db, id)
        .await?
        .ok_or(AppError::NotFound)?;
    authorize_view(&state, &actor, owner).await?;
    time_grants::delete(&state.db, id).await?;
    audit::log(&state.db, actor.id, "time.grant.delete", "user", Some(owner)).await;
    Ok(Json(json!({ "deleted": true })))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/admin/users/:id/time-grants",
            get(list_grants).post(add_grant),
        )
        .route("/admin/time-grants/:id", delete(delete_grant))
}

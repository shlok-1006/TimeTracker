//! Canonical employee directory for the cross-system HRMS integration.
//!
//!   GET /directory/users  →  [{ id, name, email, role, teams: [name] }]
//!
//! Read-only, HR-scoped. `id` is the canonical user UUID (identical to the JWT
//! `sub`), so another platform can reconcile its own employee records against
//! ours by email once and thereafter key everything to this id. The response is
//! deliberately minimal (no secrets, no internal columns). This is the ONE place
//! we expose the whole roster; every other user endpoint is per-user + scoped.

use axum::{extract::State, routing::get, Json, Router};
use serde_json::{json, Value};

use crate::db::users;
use crate::error::AppError;
use crate::middleware::RequireHr;
use crate::state::AppState;

/// `GET /directory/users` — the full directory (HR only).
async fn list_directory(
    State(state): State<AppState>,
    RequireHr(_hr): RequireHr,
) -> Result<Json<Value>, AppError> {
    Ok(Json(json!(users::list_directory(&state.db).await?)))
}

pub fn router() -> Router<AppState> {
    Router::new().route("/directory/users", get(list_directory))
}

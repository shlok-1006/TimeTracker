//! Interval sync + hours routes (protected by `auth_middleware`).

use axum::{extract::State, routing::get, routing::post, Json, Router};
use serde_json::{json, Value};

use crate::db::intervals::{self, IntervalDto};
use crate::error::AppError;
use crate::middleware::AuthUser;
use crate::state::AppState;

/// `POST /intervals` — sync a batch of intervals for the authenticated user.
/// `user_id` is taken from the token, not the body. Idempotent.
async fn create_intervals(
    State(state): State<AppState>,
    user: AuthUser,
    Json(items): Json<Vec<IntervalDto>>,
) -> Result<Json<Value>, AppError> {
    let accepted = intervals::insert_batch(&state.db, user.id, &items).await?;
    Ok(Json(
        json!({ "accepted": accepted, "received": items.len() }),
    ))
}

/// `GET /me/hours` — worked-time summary for the authenticated user, computed
/// from intervals (Rule 2: derived, never a stored counter).
async fn my_hours(State(state): State<AppState>, user: AuthUser) -> Result<Json<Value>, AppError> {
    let s = intervals::hours_summary(&state.db, user.id).await?;
    Ok(Json(json!({
        "total_seconds": s.total_seconds,
        "today_seconds": s.today_seconds,
        "week_seconds": s.week_seconds,
        "active_seconds": s.active_seconds,
        "idle_seconds": s.idle_seconds,
        "meeting_seconds": s.meeting_seconds,
        "break_seconds": s.break_seconds,
    })))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/intervals", post(create_intervals))
        .route("/me/hours", get(my_hours))
}

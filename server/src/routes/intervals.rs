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
        "today_seconds": s.today_seconds,
        "today_active_seconds": s.today_active_seconds,
        "today_idle_seconds": s.today_idle_seconds,
        "today_meeting_seconds": s.today_meeting_seconds,
        "week_seconds": s.week_seconds,
        "week_active_seconds": s.week_active_seconds,
        "week_idle_seconds": s.week_idle_seconds,
        "week_meeting_seconds": s.week_meeting_seconds,
        "total_seconds": s.total_seconds,
    })))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/intervals", post(create_intervals))
        .route("/me/hours", get(my_hours))
}

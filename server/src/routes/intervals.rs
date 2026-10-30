//! Interval sync + hours routes (protected by `auth_middleware`).

use axum::extract::DefaultBodyLimit;
use axum::{extract::State, routing::get, routing::post, Json, Router};
use chrono::Utc;
use serde_json::{json, Value};

use crate::attendance_service;
use crate::db::intervals::{self, IntervalDto};
use crate::error::AppError;
use crate::middleware::AuthUser;
use crate::state::AppState;

/// `POST /intervals` — sync a batch of intervals for the authenticated user.
/// `user_id` is taken from the token, not the body. Idempotent.
///
/// When the batch brings new time for days that are already over, those days'
/// attendance is re-derived on the spot: the desktop syncs whenever it can reach
/// us, which for someone on a VPN that blocks the app may be days later.
async fn create_intervals(
    State(state): State<AppState>,
    user: AuthUser,
    Json(items): Json<Vec<IntervalDto>>,
) -> Result<Json<Value>, AppError> {
    let accepted = intervals::insert_batch(&state.db, user.id, &items).await?;
    if accepted > 0 {
        let days = attendance_service::completed_days_touched(
            items.iter().map(|i| (i.start_utc, i.end_utc)),
            Utc::now().date_naive(),
        );
        if !days.is_empty() {
            // The intervals are saved; a failed recompute must not make the
            // desktop resend them. The nightly job and the calendar's own
            // refresh are still there for days it missed.
            match attendance_service::refresh_days(&state.db, user.id, &days).await {
                Ok(()) => {
                    tracing::info!(user_id = %user.id, ?days, "late intervals: attendance re-derived")
                }
                Err(e) => {
                    tracing::warn!(user_id = %user.id, ?days, "late intervals: attendance refresh failed: {e}")
                }
            }
        }
    }
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
        "week_grace_seconds": s.week_grace_seconds,
        "total_seconds": s.total_seconds,
    })))
}

pub fn router() -> Router<AppState> {
    Router::new()
        // Raise the body limit well above Axum's 2 MB default: a client draining
        // a large offline backlog can POST many thousands of intervals at once,
        // and a 413 here would stall its sync forever (never acked → retried).
        .route(
            "/intervals",
            post(create_intervals).layer(DefaultBodyLimit::max(32 * 1024 * 1024)),
        )
        .route("/me/hours", get(my_hours))
}

//! Activity routes: the desktop syncs per-app foreground seconds and 10-minute
//! input-activity blocks (app NAMES only — no window titles, no keystrokes);
//! employees read their own breakdown, admins read per-employee (PM team-scoped).

use axum::{
    extract::{Path, Query, State},
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, NaiveDate, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::db::activity;
use crate::error::AppError;
use crate::middleware::{AuthUser, RequireAdmin};
use crate::routes::admin::authorize_view;
use crate::state::AppState;
use crate::validate::sanitize_line;

/// Upper bound on rows per sync batch (the desktop aggregates locally, so a
/// legitimate day is tens of apps and ≤144 blocks — far below this).
const MAX_BATCH: usize = 500;
/// App names are attacker-controllable client input — cap their length.
const MAX_APP_NAME: usize = 120;

#[derive(Deserialize)]
struct AppUsageDto {
    day: NaiveDate,
    app_name: String,
    seconds: i32,
}

#[derive(Deserialize)]
struct BlockDto {
    block_start: DateTime<Utc>,
    active_seconds: i32,
    total_seconds: i32,
}

#[derive(Deserialize)]
struct ActivityBatch {
    #[serde(default)]
    apps: Vec<AppUsageDto>,
    #[serde(default)]
    blocks: Vec<BlockDto>,
}

/// `POST /activity` — desktop sync (at-least-once; upserts are monotonic so
/// retries never double-count). `user_id` comes from the JWT, never the body.
async fn sync_activity(
    State(state): State<AppState>,
    user: AuthUser,
    Json(batch): Json<ActivityBatch>,
) -> Result<Json<Value>, AppError> {
    if batch.apps.len() > MAX_BATCH || batch.blocks.len() > MAX_BATCH {
        return Err(AppError::BadRequest(format!(
            "batch too large (max {MAX_BATCH})"
        )));
    }

    let mut accepted = 0usize;
    for a in &batch.apps {
        let name = sanitize_line(&a.app_name, MAX_APP_NAME);
        if name.is_empty() || a.seconds < 0 {
            continue; // skip garbage rows rather than failing the batch
        }
        activity::upsert_app_usage(&state.db, user.id, a.day, &name, a.seconds).await?;
        accepted += 1;
    }
    for b in &batch.blocks {
        if b.active_seconds < 0 || b.total_seconds < 0 || b.active_seconds > b.total_seconds {
            continue;
        }
        activity::upsert_block(
            &state.db,
            user.id,
            b.block_start,
            b.active_seconds,
            b.total_seconds,
        )
        .await?;
        accepted += 1;
    }
    Ok(Json(json!({ "accepted": accepted })))
}

#[derive(Deserialize)]
struct DayQuery {
    #[serde(default)]
    day: Option<NaiveDate>,
}

/// Shared read: one user's activity for a UTC day.
async fn activity_payload(
    state: &AppState,
    user_id: Uuid,
    day: NaiveDate,
) -> Result<Value, AppError> {
    let apps = activity::apps_for_day(&state.db, user_id, day).await?;
    let blocks = activity::blocks_for_day(&state.db, user_id, day).await?;
    let pct = activity::activity_pct(&blocks);
    Ok(json!({
        "day": day,
        "activity_pct": pct,
        "apps": apps,
        "blocks": blocks,
    }))
}

/// `GET /me/activity?day=` — the caller's own activity breakdown.
async fn my_activity(
    State(state): State<AppState>,
    user: AuthUser,
    Query(q): Query<DayQuery>,
) -> Result<Json<Value>, AppError> {
    let day = q.day.unwrap_or_else(|| Utc::now().date_naive());
    Ok(Json(activity_payload(&state, user.id, day).await?))
}

/// `GET /admin/users/:id/activity?day=` — drill-down (PM team-scoped, HR all).
async fn user_activity(
    State(state): State<AppState>,
    RequireAdmin(user): RequireAdmin,
    Path(target): Path<Uuid>,
    Query(q): Query<DayQuery>,
) -> Result<Json<Value>, AppError> {
    authorize_view(&state, &user, target).await?;
    let day = q.day.unwrap_or_else(|| Utc::now().date_naive());
    Ok(Json(activity_payload(&state, target, day).await?))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/activity", post(sync_activity))
        .route("/me/activity", get(my_activity))
        .route("/admin/users/:id/activity", get(user_activity))
}

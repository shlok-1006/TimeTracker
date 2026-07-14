//! Heartbeat route (protected). The desktop posts the user's live status here
//! every ~45 seconds.

use axum::{extract::State, routing::post, Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::db::{presence, users};
use crate::error::AppError;
use crate::middleware::AuthUser;
use crate::presence::PresenceStatus;
use crate::state::AppState;

#[derive(Deserialize)]
struct HeartbeatBody {
    status: PresenceStatus,
    #[serde(default)]
    current_interval_id: Option<Uuid>,
    /// The desktop's IANA timezone (e.g. "Asia/Kolkata"), for the 4 AM local
    /// business-day boundary on the hours display. Optional; validated + stored.
    #[serde(default)]
    timezone: Option<String>,
}

/// `POST /presence` — record a heartbeat for the authenticated user.
/// Rejects `not_logged_in` (that status is server-derived only).
async fn heartbeat(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<HeartbeatBody>,
) -> Result<Json<Value>, AppError> {
    if !body.status.is_reportable() {
        return Err(AppError::BadRequest("invalid status".into()));
    }
    presence::heartbeat(&state.db, user.id, body.status, body.current_interval_id).await?;
    if let Some(tz) = body.timezone.as_deref() {
        users::set_timezone(&state.db, user.id, tz).await?;
    }
    Ok(Json(json!({ "ok": true })))
}

pub fn router() -> Router<AppState> {
    Router::new().route("/presence", post(heartbeat))
}

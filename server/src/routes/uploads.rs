//! Upload + screenshot-metadata routes (protected).

use axum::{
    extract::{Query, State},
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, NaiveDate, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::db::screenshots;
use crate::db::screenshots::DayScreenshot;
use crate::error::AppError;
use crate::middleware::AuthUser;
use crate::state::AppState;
use crate::storage::StorageClient;
use crate::upload_service;

/// Lifetime of presigned view (GET) URLs.
const VIEW_URL_EXPIRES_SECS: u64 = 900;

#[derive(Deserialize)]
pub(crate) struct DayQuery {
    pub day: Option<NaiveDate>,
}

/// Build one day-listing item: the screenshot, its verdict, a meeting flag, and
/// a short-lived presigned view URL (Rule 5). Shared by the `/me` and admin
/// listings. Top-level `id`/`taken_at`/`url` are kept as back-compat aliases.
pub(crate) fn day_item(
    storage: &StorageClient,
    s: &DayScreenshot,
    now: DateTime<Utc>,
) -> Result<Value, AppError> {
    let url = storage.presign_get(&s.storage_key, VIEW_URL_EXPIRES_SECS, now)?;
    Ok(json!({
        "screenshot": {
            "id": s.id,
            "taken_at": s.taken_at,
            "captured_status": s.captured_status,
        },
        "verdict": s.verdict,
        "meeting_flag": s.captured_status == "meeting",
        "presigned_url": url,
        // back-compat aliases for the existing gallery
        "id": s.id,
        "taken_at": s.taken_at,
        "url": url,
    }))
}

/// `POST /uploads/presign` — get a short-lived presigned PUT URL + storage key.
async fn presign(State(state): State<AppState>, user: AuthUser) -> Result<Json<Value>, AppError> {
    let p = upload_service::presign_screenshot(&state.storage, user.id, Utc::now())?;
    Ok(Json(json!({
        "url": p.url,
        "method": p.method,
        "storage_key": p.storage_key,
        "expires_in": p.expires_in,
    })))
}

#[derive(Deserialize)]
struct SaveScreenshot {
    storage_key: String,
    taken_at: DateTime<Utc>,
    #[serde(default)]
    interval_id: Option<Uuid>,
    /// Presence status at capture time (Feature 2). Defaults to "working" for
    /// older desktop builds that don't send it (they only capture while working).
    #[serde(default = "default_captured_status")]
    captured_status: String,
}

fn default_captured_status() -> String {
    "working".to_string()
}

/// `POST /screenshots` — store metadata only (Rule 5). The storage key must be
/// within the caller's own namespace (defends against writing others' rows).
async fn save_screenshot(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<SaveScreenshot>,
) -> Result<Json<Value>, AppError> {
    if !body.storage_key.starts_with(&format!("{}/", user.id)) {
        return Err(AppError::BadRequest(
            "storage_key outside user namespace".into(),
        ));
    }
    if !screenshots::is_valid_captured_status(&body.captured_status) {
        return Err(AppError::BadRequest(format!(
            "invalid captured_status: {:?}",
            body.captured_status
        )));
    }
    let id = screenshots::insert(
        &state.db,
        user.id,
        &body.storage_key,
        body.taken_at,
        body.interval_id,
        &body.captured_status,
    )
    .await?;
    Ok(Json(json!({ "id": id })))
}

/// `GET /me/screenshots?day=` — the caller's own screenshots for a day, each
/// with its verdict, meeting flag, and a short-lived presigned view URL (Rule 5).
/// `day` defaults to today (UTC).
async fn my_screenshots(
    State(state): State<AppState>,
    user: AuthUser,
    Query(q): Query<DayQuery>,
) -> Result<Json<Value>, AppError> {
    let day = q.day.unwrap_or_else(|| Utc::now().date_naive());
    let now = Utc::now();
    let rows = screenshots::list_for_day(&state.db, user.id, day).await?;
    let items = rows
        .iter()
        .map(|r| day_item(&state.storage, r, now))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json(Value::Array(items)))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/uploads/presign", post(presign))
        .route("/screenshots", post(save_screenshot))
        .route("/me/screenshots", get(my_screenshots))
}

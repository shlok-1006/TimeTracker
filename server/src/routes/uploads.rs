//! Upload + screenshot-metadata routes (protected).

use axum::{
    extract::State,
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::db::screenshots;
use crate::error::AppError;
use crate::middleware::AuthUser;
use crate::state::AppState;
use crate::upload_service;

/// Lifetime of presigned view (GET) URLs.
const VIEW_URL_EXPIRES_SECS: u64 = 900;

/// `POST /uploads/presign` — get a short-lived presigned PUT URL + storage key.
async fn presign(State(state): State<AppState>, user: AuthUser) -> Result<Json<Value>, AppError> {
    let p = upload_service::presign_screenshot(&state.storage, user.id, Utc::now());
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
    let id = screenshots::insert(
        &state.db,
        user.id,
        &body.storage_key,
        body.taken_at,
        body.interval_id,
    )
    .await?;
    Ok(Json(json!({ "id": id })))
}

/// `GET /me/screenshots` — the caller's own recent screenshots, each with a
/// short-lived presigned view URL (Rule 5: never expose raw storage keys).
async fn my_screenshots(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<Value>, AppError> {
    let now = Utc::now();
    let rows = screenshots::list_for_user(&state.db, user.id, 60).await?;
    let items: Vec<Value> = rows
        .into_iter()
        .map(|r| {
            json!({
                "id": r.id,
                "taken_at": r.taken_at,
                "interval_id": r.interval_id,
                "url": state.storage.presign_get(&r.storage_key, VIEW_URL_EXPIRES_SECS, now),
            })
        })
        .collect();
    Ok(Json(Value::Array(items)))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/uploads/presign", post(presign))
        .route("/screenshots", post(save_screenshot))
        .route("/me/screenshots", get(my_screenshots))
}

//! Company rulebook (OKF) — HR-only read/edit of the single policy document.
//!
//!   GET /admin/okf   the current rulebook (+ last editor / timestamp)
//!   PUT /admin/okf   replace it  { content }
//!
//! HR-only (`RequireHr`); every save is audited. The document is a single row
//! (migration 0037); see `db::okf`.

use axum::{extract::State, routing::get, Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::db::{audit, okf};
use crate::error::AppError;
use crate::middleware::RequireHr;
use crate::state::AppState;

/// Guard against an accidental huge paste (the rulebook is a few tens of KB).
const MAX_OKF_BYTES: usize = 256 * 1024;

#[derive(Deserialize)]
struct UpdateOkf {
    content: String,
}

async fn get_okf(State(state): State<AppState>, _hr: RequireHr) -> Result<Json<Value>, AppError> {
    let doc = okf::get(&state.db).await?;
    Ok(Json(json!(doc)))
}

async fn put_okf(
    State(state): State<AppState>,
    RequireHr(actor): RequireHr,
    Json(body): Json<UpdateOkf>,
) -> Result<Json<Value>, AppError> {
    if body.content.trim().is_empty() {
        return Err(AppError::BadRequest("the rulebook cannot be empty".into()));
    }
    if body.content.len() > MAX_OKF_BYTES {
        return Err(AppError::BadRequest("the rulebook is too large".into()));
    }
    let doc = okf::update(&state.db, &body.content, actor.id).await?;
    audit::log(&state.db, actor.id, "okf.update", "okf_document", None).await;
    Ok(Json(json!(doc)))
}

pub fn router() -> Router<AppState> {
    Router::new().route("/admin/okf", get(get_okf).put(put_okf))
}

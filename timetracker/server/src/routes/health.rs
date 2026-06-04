//! Health and readiness endpoints.

use axum::{routing::get, Json, Router};
use serde::Serialize;

use crate::state::AppState;

#[derive(Serialize)]
struct Health {
    status: &'static str,
}

/// `GET /health` — liveness probe. Always returns `200 { "status": "ok" }`.
///
/// Intentionally does NOT touch the database: it answers "is the process up",
/// which load balancers and `tauri`/CI smoke tests rely on.
async fn health() -> Json<Health> {
    Json(Health { status: "ok" })
}

#[derive(Serialize)]
struct Readiness {
    status: &'static str,
    database: &'static str,
}

/// `GET /ready` — readiness probe. Verifies the database is reachable.
async fn ready(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> Result<Json<Readiness>, crate::error::AppError> {
    sqlx::query("SELECT 1").execute(&state.db).await?;
    Ok(Json(Readiness {
        status: "ok",
        database: "ok",
    }))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
}

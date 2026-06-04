//! HTTP routing. Each feature area owns a submodule that exposes a `router()`;
//! they are merged here into the application router.
//!
//! Route groups:
//!   * public    — no auth (`/health`, `/ready`, `/auth/login`)
//!   * protected — `auth_middleware` validates the JWT; handlers add role guards

pub mod admin;
pub mod auth;
pub mod health;
pub mod intervals;
pub mod presence;
pub mod uploads;

use axum::{routing::get, Json, Router};
use serde_json::{json, Value};
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

use crate::middleware::{auth_middleware, AuthUser, RequireAdmin, RequireEmployee, RequireHr};
use crate::state::AppState;

/// Current authenticated principal (any role).
async fn me(user: AuthUser) -> Json<Value> {
    Json(json!({
        "id": user.id,
        "role": user.role,
        "team": user.team,
    }))
}

/// Employee-only resource (desktop app). Wrong role => 403.
async fn desktop_ping(_guard: RequireEmployee) -> Json<Value> {
    Json(json!({ "ok": true, "scope": "employee" }))
}

/// Admin-dashboard resource (HR or project manager). Wrong role => 403.
async fn dashboard_ping(_guard: RequireAdmin) -> Json<Value> {
    Json(json!({ "ok": true, "scope": "dashboard" }))
}

/// HR-only resource. Wrong role => 403.
async fn hr_ping(_guard: RequireHr) -> Json<Value> {
    Json(json!({ "ok": true, "scope": "hr" }))
}

/// Build the full application router with shared middleware.
pub fn build(state: AppState) -> Router {
    // Permissive CORS for local development (admin dashboard on :3001).
    // Tightened to explicit origins in a later step.
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let public = Router::new().merge(health::router()).merge(auth::router());

    let protected = Router::new()
        .route("/me", get(me))
        .route("/desktop/ping", get(desktop_ping))
        .route("/dashboard/ping", get(dashboard_ping))
        .route("/hr/ping", get(hr_ping))
        .merge(intervals::router())
        .merge(presence::router())
        .merge(uploads::router())
        .merge(admin::router())
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    Router::new()
        .merge(public)
        .merge(protected)
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .with_state(state)
}

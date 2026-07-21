//! HTTP routing. Each feature area owns a submodule that exposes a `router()`;
//! they are merged here into the application router.
//!
//! Route groups:
//!   * public    — no auth (`/health`, `/ready`, `/auth/login`)
//!   * protected — `auth_middleware` validates the JWT; handlers add role guards

pub mod activity;
pub mod admin;
pub mod attendance;
pub mod auth;
pub mod health;
pub mod intervals;
pub mod jwks;
pub mod leave;
pub mod linear;
pub mod onboarding;
pub mod presence;
pub mod reports;
pub mod tasks;
pub mod teams;
pub mod ticket_requests;
pub mod uploads;

use axum::http::{header, HeaderValue, Method};
use axum::{routing::get, Json, Router};
use serde_json::{json, Value};
use tower_http::cors::CorsLayer;
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
///
/// CORS is restricted to the exact origins in `CORS_ALLOWED_ORIGINS` — no
/// wildcard (SEC-02). Methods and headers are restricted to what the SPA uses.
pub fn build(state: AppState) -> Router {
    let allow_origins: Vec<HeaderValue> = crate::config::cors_allowed_origins()
        .iter()
        .filter_map(|o| match o.parse::<HeaderValue>() {
            Ok(v) => Some(v),
            Err(_) => {
                tracing::warn!("ignoring invalid CORS origin: {o}");
                None
            }
        })
        .collect();

    // RA-10: refuse to boot a deny-all API on a misconfigured allowlist rather
    // than silently rejecting every cross-origin request.
    assert!(
        !allow_origins.is_empty(),
        "CORS_ALLOWED_ORIGINS produced no valid origins — set it to your dashboard origin(s)"
    );

    let cors = CorsLayer::new()
        .allow_origin(allow_origins)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::PATCH,
            Method::OPTIONS,
        ])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE]);

    // SEC-08: rate-limit the auth endpoints (login/refresh/logout) per client IP.
    let auth_limiter = std::sync::Arc::new(crate::rate_limit::RateLimiter::from_env());
    let auth_routes = auth::router().route_layer(axum::middleware::from_fn_with_state(
        auth_limiter,
        crate::rate_limit::rate_limit,
    ));

    let public = Router::new()
        .merge(health::router())
        .merge(auth_routes)
        .merge(jwks::router())
        .merge(ticket_requests::router());

    let protected = Router::new()
        .route("/me", get(me))
        .route("/desktop/ping", get(desktop_ping))
        .route("/dashboard/ping", get(dashboard_ping))
        .route("/hr/ping", get(hr_ping))
        .merge(intervals::router())
        .merge(presence::router())
        .merge(uploads::router())
        .merge(linear::router())
        .merge(leave::router())
        .merge(reports::router())
        .merge(teams::router())
        .merge(tasks::router())
        .merge(onboarding::router())
        .merge(attendance::router())
        .merge(activity::router())
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

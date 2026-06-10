//! Authentication routes.

use axum::{extract::State, routing::post, Json, Router};
use serde_json::{json, Value};

use crate::auth::{self, LoginRequest, LoginResponse, RefreshRequest, TokenPair};
use crate::error::AppError;
use crate::state::AppState;

/// `POST /auth/login` — email + password → access token + refresh token.
async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, AppError> {
    Ok(Json(auth::login(&state, req).await?))
}

/// `POST /auth/refresh` — rotate a refresh token for a fresh token pair.
async fn refresh(
    State(state): State<AppState>,
    Json(req): Json<RefreshRequest>,
) -> Result<Json<TokenPair>, AppError> {
    Ok(Json(auth::refresh(&state, req).await?))
}

/// `POST /auth/logout` — revoke a refresh token. Idempotent.
async fn logout(
    State(state): State<AppState>,
    Json(req): Json<RefreshRequest>,
) -> Result<Json<Value>, AppError> {
    auth::logout(&state, req).await?;
    Ok(Json(json!({ "ok": true })))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/auth/login", post(login))
        .route("/auth/refresh", post(refresh))
        .route("/auth/logout", post(logout))
}

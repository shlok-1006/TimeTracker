//! Authentication routes.

use axum::{extract::State, routing::post, Json, Router};

use crate::auth::{self, LoginRequest, LoginResponse};
use crate::error::AppError;
use crate::state::AppState;

/// `POST /auth/login` — exchange email + password for a JWT access token.
async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, AppError> {
    let resp = auth::login(&state, req).await?;
    Ok(Json(resp))
}

pub fn router() -> Router<AppState> {
    Router::new().route("/auth/login", post(login))
}

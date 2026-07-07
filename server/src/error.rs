//! Centralized error type for the API (Rule 8: anyhow + thiserror, never unwrap).
//!
//! `AppError` implements `IntoResponse` so any handler can return
//! `Result<T, AppError>` and failures become well-formed JSON responses.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    BadRequest(String),

    #[error("resource not found")]
    NotFound,

    #[error("unauthorized")]
    Unauthorized,

    #[error("forbidden")]
    Forbidden,

    #[error(transparent)]
    Database(#[from] sqlx::Error),

    /// Catch-all for unexpected failures. The internal message is logged but
    /// never leaked to clients.
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

/// A SQLx error that means the database is unreachable (rather than a bad query)
/// — pool acquisition timed out/closed, or a connection-level I/O failure. These
/// map to 503 so an outage (e.g. Postgres down / disk full) fails honestly as
/// "service temporarily unavailable" instead of a generic 500.
fn db_unavailable(e: &sqlx::Error) -> bool {
    matches!(
        e,
        sqlx::Error::PoolTimedOut | sqlx::Error::PoolClosed | sqlx::Error::Io(_)
    )
}

impl AppError {
    fn status(&self) -> StatusCode {
        match self {
            AppError::BadRequest(_) => StatusCode::BAD_REQUEST,
            AppError::NotFound => StatusCode::NOT_FOUND,
            AppError::Unauthorized => StatusCode::UNAUTHORIZED,
            AppError::Forbidden => StatusCode::FORBIDDEN,
            AppError::Database(e) if db_unavailable(e) => StatusCode::SERVICE_UNAVAILABLE,
            AppError::Database(_) | AppError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status();

        // Log full detail server-side (any 5xx) — return a safe message to the client.
        if status.is_server_error() {
            tracing::error!(error = %self, status = %status, "server error");
        }

        let message = match status {
            StatusCode::INTERNAL_SERVER_ERROR => "internal server error".to_string(),
            StatusCode::SERVICE_UNAVAILABLE => {
                "service temporarily unavailable — please try again shortly".to_string()
            }
            _ => self.to_string(),
        };

        (status, Json(json!({ "error": message }))).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_connectivity_errors_map_to_503() {
        assert_eq!(
            AppError::Database(sqlx::Error::PoolTimedOut).status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(
            AppError::Database(sqlx::Error::PoolClosed).status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }

    #[test]
    fn other_internal_errors_stay_500() {
        assert_eq!(
            AppError::Internal(anyhow::anyhow!("boom")).status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(AppError::NotFound.status(), StatusCode::NOT_FOUND);
    }
}

pub type AppResult<T> = Result<T, AppError>;

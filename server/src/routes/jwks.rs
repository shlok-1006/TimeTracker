//! Public JWKS endpoint (HRMS integration).
//!
//! `GET /.well-known/jwks.json` publishes the RS256 PUBLIC key so external
//! services (the HRMS dashboard) can verify our access tokens without holding
//! any secret — they can check tokens but never mint them. Public by design:
//! it is mounted on the public router, outside `auth_middleware`, and serves
//! only key material that is public by definition (RFC 7517).
//!
//! When RS256 is not configured the document is an empty key set, so a
//! verifier polling before the key ships gets valid JSON rather than a 404.

use axum::extract::State;
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/.well-known/jwks.json", get(jwks))
}

async fn jwks(State(state): State<AppState>) -> impl IntoResponse {
    let body = state
        .jwt
        .jwks_json()
        .unwrap_or(r#"{"keys":[]}"#)
        .to_string();
    (
        [
            (header::CONTENT_TYPE, "application/json"),
            // Cacheable for an hour: rotations bump the kid and overlap keys,
            // so a stale-by-an-hour copy can never break verification.
            (header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        body,
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn empty_jwks_fallback_is_valid_json() {
        let v: serde_json::Value = serde_json::from_str(r#"{"keys":[]}"#).unwrap();
        assert!(v["keys"].as_array().unwrap().is_empty());
    }
}

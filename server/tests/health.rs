//! Integration test for the health endpoint (Rule 9).
//!
//! Builds the *real* application router and asserts that `GET /health` returns
//! `200 { "status": "ok" }` without any database connectivity.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::Value;
use tower::ServiceExt; // for `oneshot`

use server::jwt::JwtKeys;
use server::linear_service::LinearService;
use server::storage::{S3Config, StorageClient};
use server::AppState;

#[tokio::test]
async fn health_returns_ok() {
    // A lazily-created pool never opens a connection until first queried, and the
    // `/health` handler does not touch the database — so no Postgres is required.
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://localhost/timetracker")
        .expect("lazy pool should construct");

    let app = server::build_router(AppState::new(
        pool,
        JwtKeys::new("test-secret", 900),
        StorageClient::new(S3Config::from_env()),
        LinearService::from_env(),
        server::claude_provider::ClaudeProvider::from_env(),
        2_592_000,
    ));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["status"], "ok");
}

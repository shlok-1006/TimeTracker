//! Auth-gating tests for the Linear routes (Rule 9). End-to-end ticket fetch is
//! exercised against a real Linear workspace when `LINEAR_API_KEY` is set; the
//! parsing + cache logic is unit-tested in the crate.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use server::jwt::JwtKeys;
use server::linear_service::LinearService;
use server::storage::{S3Config, StorageClient};
use server::AppState;

fn app() -> axum::Router {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://localhost/timetracker")
        .expect("lazy pool");
    server::build_router(AppState::new(
        pool,
        JwtKeys::new("test-secret", 900),
        StorageClient::new(S3Config::from_env()),
        LinearService::from_env(),
        server::gemini_provider::GeminiProvider::from_env(),
        2_592_000,
    ))
}

#[tokio::test]
async fn my_tickets_requires_auth() {
    let req = Request::builder()
        .uri("/me/tickets")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app().oneshot(req).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn link_requires_auth() {
    let req = Request::builder()
        .method("POST")
        .uri("/me/linear/link")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app().oneshot(req).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
}

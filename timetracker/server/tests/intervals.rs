//! Integration tests for the interval routes' auth gating (Rule 9).
//!
//! These assert the endpoints require authentication without needing a DB
//! (the middleware rejects before any handler/DB access). End-to-end insert +
//! totals are exercised against a live DB in the manual verification flow.

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
        2_592_000,
    ))
}

#[tokio::test]
async fn post_intervals_requires_auth() {
    let req = Request::builder()
        .method("POST")
        .uri("/intervals")
        .header("content-type", "application/json")
        .body(Body::from("[]"))
        .unwrap();
    assert_eq!(
        app().oneshot(req).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn my_hours_requires_auth() {
    let req = Request::builder()
        .uri("/me/hours")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app().oneshot(req).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
}

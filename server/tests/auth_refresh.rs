//! Refresh-token rotation resilience: a client that re-presents a just-rotated
//! token (because it never received its successor — e.g. a dropped response on a
//! server restart) must RECOVER with a fresh pair, not be logged out. Live HTTP
//! round-trip; skips if no DATABASE_URL.

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use axum::Router;
use serde_json::{json, Value};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tower::ServiceExt;

use server::jwt::JwtKeys;
use server::linear_service::LinearService;
use server::storage::{S3Config, StorageClient};
use server::AppState;

const SECRET: &str = "auth-refresh-test-secret";

fn app_with(pool: PgPool) -> Router {
    server::build_router(AppState::new(
        pool,
        JwtKeys::new(SECRET, 900),
        StorageClient::new(S3Config::insecure_local()),
        LinearService::from_env(),
        server::claude_provider::ClaudeProvider::from_env(),
        2_592_000,
    ))
}

async fn send(app: Router, method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
    let b = Request::builder().method(method).uri(path);
    let req = match body {
        Some(j) => b
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&j).unwrap()))
            .unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let v = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, v)
}

async fn real_pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .ok()
}

#[tokio::test]
async fn re_presenting_a_just_rotated_token_recovers_not_logs_out() {
    let Some(pool) = real_pool().await else {
        eprintln!("skipping auth refresh test: DATABASE_URL not set");
        return;
    };

    // Log in as the seed HR to obtain a real refresh token.
    let (s, login) = send(
        app_with(pool.clone()),
        "POST",
        "/auth/login",
        Some(json!({ "email": "hr@timetracker.local", "password": "ChangeMe!HR1" })),
    )
    .await;
    if s != StatusCode::OK {
        eprintln!("skipping: seed HR login failed ({s})");
        return;
    }
    let r1 = login["refresh_token"].as_str().unwrap().to_string();

    // Normal rotation: R1 -> R2. R1 is now revoked.
    let (s, pair) = send(
        app_with(pool.clone()),
        "POST",
        "/auth/refresh",
        Some(json!({ "refresh_token": r1 })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "first refresh: {pair}");
    let r2 = pair["refresh_token"].as_str().unwrap().to_string();
    assert_ne!(r1, r2, "rotation issues a new token");

    // Client never received R2 (dropped response) and retries with R1 while it's
    // still within the grace window. Must RECOVER (200 + a fresh pair), not 401.
    let (s, recovered) = send(
        app_with(pool.clone()),
        "POST",
        "/auth/refresh",
        Some(json!({ "refresh_token": r1 })),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::OK,
        "re-presenting a just-rotated token must recover, not log out: {recovered}"
    );
    let r3 = recovered["refresh_token"].as_str().unwrap().to_string();

    // The recovered token works for a subsequent rotation.
    let (s, _) = send(
        app_with(pool.clone()),
        "POST",
        "/auth/refresh",
        Some(json!({ "refresh_token": r3 })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "recovered token should be usable");

    // A completely unknown token is still rejected.
    let (s, _) = send(
        app_with(pool.clone()),
        "POST",
        "/auth/refresh",
        Some(json!({ "refresh_token": "deadbeef-not-a-real-token" })),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::UNAUTHORIZED,
        "unknown token must be rejected"
    );
}

//! Team management API tests (Feature 4 Phase 2). HR-only gating (no DB) plus a
//! live HTTP round-trip (skips if DATABASE_URL is unset).

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use axum::Router;
use serde_json::{json, Value};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use server::db::users;
use server::jwt::JwtKeys;
use server::linear_service::LinearService;
use server::role::UserRole;
use server::storage::{S3Config, StorageClient};
use server::AppState;

const SECRET: &str = "teams-api-test-secret";

fn app_with(pool: PgPool) -> Router {
    server::build_router(AppState::new(
        pool,
        JwtKeys::new(SECRET, 900),
        StorageClient::new(S3Config::from_env()),
        LinearService::from_env(),
        server::claude_provider::ClaudeProvider::from_env(),
        2_592_000,
    ))
}

fn lazy_app() -> Router {
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://localhost/timetracker")
        .expect("lazy pool");
    app_with(pool)
}

fn token(role: UserRole) -> String {
    JwtKeys::new(SECRET, 900).issue(Uuid::new_v4(), role, None).unwrap()
}

async fn send(
    app: Router,
    method: &str,
    path: &str,
    tok: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut b = Request::builder().method(method).uri(path);
    if let Some(t) = tok {
        b = b.header("authorization", format!("Bearer {t}"));
    }
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
    let v = if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).unwrap_or(Value::Null) };
    (status, v)
}

async fn real_pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    PgPoolOptions::new().max_connections(2).connect(&url).await.ok()
}

#[tokio::test]
async fn team_management_is_hr_only() {
    // No token → 401.
    let (s, _) = send(lazy_app(), "POST", "/teams", None, Some(json!({ "name": "X" }))).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);

    // Employee and project manager → 403 (HR only).
    for role in [UserRole::Employee, UserRole::ProjectManager] {
        let t = token(role);
        let (s, _) = send(lazy_app(), "POST", "/teams", Some(&t), Some(json!({ "name": "X" }))).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{role:?} must be forbidden");
        let id = Uuid::new_v4();
        let (s2, _) = send(lazy_app(), "DELETE", &format!("/teams/{id}"), Some(&t), None).await;
        assert_eq!(s2, StatusCode::FORBIDDEN);
    }
}

#[tokio::test]
async fn team_management_roundtrip_over_http() {
    let Some(pool) = real_pool().await else {
        eprintln!("skipping teams_api round-trip: DATABASE_URL not set");
        return;
    };
    let hr = token(UserRole::Hr);
    let tag = Uuid::new_v4();

    // A temp employee to add as a member.
    let emp = users::create(
        &pool, "Team API Emp", &format!("teamapi-{tag}@t.local"), "h", UserRole::Employee, None,
    ).await.unwrap();

    // Create.
    let (s, body) = send(
        app_with(pool.clone()), "POST", "/teams", Some(&hr),
        Some(json!({ "name": format!("Squad-{tag}"), "description": "first" })),
    ).await;
    assert_eq!(s, StatusCode::OK, "create: {body}");
    let team_id = body["id"].as_str().unwrap().to_string();

    // Patch (rename only; description preserved).
    let (s, body) = send(
        app_with(pool.clone()), "PATCH", &format!("/teams/{team_id}"), Some(&hr),
        Some(json!({ "name": format!("Squad-{tag}-renamed") })),
    ).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(body["name"], format!("Squad-{tag}-renamed"));
    assert_eq!(body["description"], "first", "PATCH preserves unset fields");

    // Add member.
    let (s, _) = send(
        app_with(pool.clone()), "POST", &format!("/teams/{team_id}/members"), Some(&hr),
        Some(json!({ "user_id": emp.id })),
    ).await;
    assert_eq!(s, StatusCode::OK);

    // List members → the employee is there.
    let (s, body) = send(
        app_with(pool.clone()), "GET", &format!("/teams/{team_id}/members"), Some(&hr), None,
    ).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(body.as_array().unwrap().len(), 1);
    assert_eq!(body[0]["id"], emp.id.to_string());

    // Patch a missing team → 404.
    let (s, _) = send(
        app_with(pool.clone()), "PATCH", &format!("/teams/{}", Uuid::new_v4()), Some(&hr),
        Some(json!({ "name": "nope" })),
    ).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // Delete (cascades membership).
    let (s, _) = send(
        app_with(pool.clone()), "DELETE", &format!("/teams/{team_id}"), Some(&hr), None,
    ).await;
    assert_eq!(s, StatusCode::OK);

    users::delete(&pool, emp.id).await.unwrap();
}

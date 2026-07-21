//! Employment-type API tests: HR-only gating (no DB) plus a live HTTP round-trip
//! — create a user with a type, change it, read it back (skips if no DATABASE_URL).

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

const SECRET: &str = "employment-type-test-secret";

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

fn lazy_app() -> Router {
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://localhost/timetracker")
        .expect("lazy pool");
    app_with(pool)
}

fn token(role: UserRole) -> String {
    JwtKeys::new(SECRET, 900)
        .issue(Uuid::new_v4(), role, None, None)
        .unwrap()
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
    let v = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
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
async fn set_employment_type_is_hr_only() {
    let path = format!("/admin/users/{}/employment-type", Uuid::new_v4());
    // No token → 401.
    let (s, _) = send(
        lazy_app(),
        "PUT",
        &path,
        None,
        Some(json!({ "employment_type": "contractor" })),
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    // Employee + project manager → 403 (HR only), decided by role before any DB.
    for role in [UserRole::Employee, UserRole::ProjectManager] {
        let t = token(role);
        let (s, _) = send(
            lazy_app(),
            "PUT",
            &path,
            Some(&t),
            Some(json!({ "employment_type": "contractor" })),
        )
        .await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{role:?} must be forbidden");
    }
}

#[tokio::test]
async fn create_and_update_employment_type_over_http() {
    let Some(pool) = real_pool().await else {
        eprintln!("skipping employment_type round-trip: DATABASE_URL not set");
        return;
    };

    // Log in as the seed HR.
    let (s, login) = send(
        app_with(pool.clone()),
        "POST",
        "/auth/login",
        None,
        Some(json!({ "email": "hr@timetracker.local", "password": "ChangeMe!HR1" })),
    )
    .await;
    if s != StatusCode::OK {
        eprintln!("skipping: seed HR login failed ({s})");
        return;
    }
    let hr = login["access_token"].as_str().unwrap().to_string();

    let tag = Uuid::new_v4();
    let email = format!("contractor-{tag}@t.local");

    // Create a user as a contractor.
    let (s, created) = send(
        app_with(pool.clone()),
        "POST",
        "/admin/users",
        Some(&hr),
        Some(json!({
            "name": "Casey Contractor",
            "email": email,
            "password": "TempPass!123",
            "role": "employee",
            "employment_type": "contractor"
        })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "create: {created}");
    assert_eq!(created["employment_type"], "contractor");
    let user_id = created["id"].as_str().unwrap().to_string();

    // Reclassify as intern.
    let (s, updated) = send(
        app_with(pool.clone()),
        "PUT",
        &format!("/admin/users/{user_id}/employment-type"),
        Some(&hr),
        Some(json!({ "employment_type": "intern" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "update: {updated}");
    assert_eq!(updated["employment_type"], "intern");

    // The list reflects the new type.
    let (s, list) = send(app_with(pool.clone()), "GET", "/admin/users", Some(&hr), None).await;
    assert_eq!(s, StatusCode::OK);
    let row = list
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["id"] == user_id)
        .expect("created user in list");
    assert_eq!(row["employment_type"], "intern");

    // An invalid type is rejected.
    let (s, _) = send(
        app_with(pool.clone()),
        "PUT",
        &format!("/admin/users/{user_id}/employment-type"),
        Some(&hr),
        Some(json!({ "employment_type": "freelancer" })),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // Cleanup (the created user is not an audit actor, so a hard delete is fine).
    users::delete(&pool, Uuid::parse_str(&user_id).unwrap())
        .await
        .unwrap();
}

//! `POST /admin/directory` — the onboarding "create employee" hand-off.
//!
//! The two claims that matter for the HRMS integration:
//!   * a brand-new hire is CREATED (row + profile), and
//!   * re-sending the same person (same Razorpay id) UPSERTS — same user, never a duplicate.
//! Plus the access rule: HR/admin only.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::json;
use tower::ServiceExt;
use uuid::Uuid;

use server::jwt::JwtKeys;
use server::linear_service::LinearService;
use server::role::UserRole;
use server::storage::{S3Config, StorageClient};
use server::AppState;

const SECRET: &str = "create-employee-test-secret";

async fn real_pool() -> Option<sqlx::PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .ok()
}

fn app() -> axum::Router {
    let url =
        std::env::var("DATABASE_URL").unwrap_or_else(|_| "postgres://localhost/timetracker".into());
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy(&url)
        .expect("lazy pool");
    server::build_router(AppState::new(
        pool,
        JwtKeys::new(SECRET, 900),
        StorageClient::new(S3Config::insecure_local()),
        LinearService::from_env(),
        server::claude_provider::ClaudeProvider::from_env(),
        2_592_000,
    ))
}

fn token(role: UserRole) -> String {
    JwtKeys::new(SECRET, 900)
        .issue(Uuid::new_v4(), role, None, None)
        .unwrap()
}

async fn post(body: serde_json::Value, who: Option<UserRole>) -> (StatusCode, serde_json::Value) {
    let mut b = Request::builder().method("POST").uri("/admin/directory");
    b = b.header("content-type", "application/json");
    if let Some(role) = who {
        b = b.header("Authorization", format!("Bearer {}", token(role)));
    }
    let resp = app()
        .oneshot(b.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let v = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, v)
}

#[tokio::test]
async fn creates_then_upserts_on_the_same_razorpay_id() {
    let Some(pool) = real_pool().await else {
        eprintln!("DATABASE_URL unset — skipping create-employee test");
        return;
    };

    let code = format!("RUH-{}", &Uuid::new_v4().to_string()[..8]);
    let email = format!("hire-{}@test.local", &Uuid::new_v4().to_string()[..8]);
    let payload = json!({
        "name": "New Hire",
        "email": email,
        "employee_code": code,
        "department": "Engineering",
        "designation": "Backend Engineer",
        "joined_on": "2026-09-01",
        "profile": { "date_of_birth": "1998-04-12", "phone": "+91 90000 00000" },
        "education": [{ "degree": "B.Tech", "institute": "IIT", "year": "2020" }],
    });

    // 1) Create.
    let (s1, v1) = post(payload.clone(), Some(UserRole::Hr)).await;
    assert_eq!(s1, StatusCode::OK, "create failed: {v1}");
    assert_eq!(v1["created"], json!(true));
    let user_id = v1["user_id"]
        .as_str()
        .expect("user_id returned")
        .to_string();
    assert!(
        v1["temp_password"].as_str().is_some(),
        "a generated password is returned once for a new hire"
    );

    // The row is really there, keyed by the Razorpay id, with the profile applied.
    let uid = Uuid::parse_str(&user_id).unwrap();
    let got_code: Option<String> =
        sqlx::query_scalar("SELECT employee_code FROM users WHERE id = $1")
            .bind(uid)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(got_code.as_deref(), Some(code.as_str()));
    let has_profile: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM employee_profiles WHERE user_id = $1)")
            .bind(uid)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(has_profile, "the profile row was written");

    // 2) Re-send the same person (double-click / re-sync) — upsert, not a duplicate.
    let (s2, v2) = post(payload.clone(), Some(UserRole::Hr)).await;
    assert_eq!(s2, StatusCode::OK);
    assert_eq!(v2["created"], json!(false), "second send must UPSERT");
    assert_eq!(
        v2["user_id"].as_str(),
        Some(user_id.as_str()),
        "same user id"
    );

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE employee_code = $1")
        .bind(&code)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        count, 1,
        "exactly one row for the Razorpay id — never duplicated"
    );

    // cleanup
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(uid)
        .execute(&pool)
        .await
        .ok();
}

#[tokio::test]
async fn create_is_staff_gated() {
    let body = json!({ "name": "X", "email": "x@test.local" });
    assert_eq!(post(body.clone(), None).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(
        post(body, Some(UserRole::Employee)).await.0,
        StatusCode::FORBIDDEN,
        "an employee cannot create directory entries"
    );
}

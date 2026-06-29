//! Auth-gating tests for the presence + admin routes (Rule 9).
//! End-to-end heartbeat/derivation is covered in the live verification flow.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;
use uuid::Uuid;

use server::jwt::JwtKeys;
use server::linear_service::LinearService;
use server::role::UserRole;
use server::storage::{GcsConfig, StorageClient};
use server::AppState;

const SECRET: &str = "presence-test-secret";

fn app() -> axum::Router {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://localhost/timetracker")
        .expect("lazy pool");
    server::build_router(AppState::new(
        pool,
        JwtKeys::new(SECRET, 900),
        StorageClient::new(GcsConfig::from_env()),
        LinearService::from_env(),
        server::gemini_provider::GeminiProvider::from_env(),
        2_592_000,
    ))
}

fn token(role: UserRole) -> String {
    JwtKeys::new(SECRET, 900)
        .issue(Uuid::new_v4(), role, None)
        .unwrap()
}

#[tokio::test]
async fn presence_requires_auth() {
    let req = Request::builder()
        .method("POST")
        .uri("/presence")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"status":"working"}"#))
        .unwrap();
    assert_eq!(
        app().oneshot(req).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn admin_team_forbidden_for_employee() {
    let req = Request::builder()
        .uri("/admin/team")
        .header(
            "Authorization",
            format!("Bearer {}", token(UserRole::Employee)),
        )
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app().oneshot(req).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn admin_team_unauthorized_without_token() {
    let req = Request::builder()
        .uri("/admin/team")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app().oneshot(req).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
}

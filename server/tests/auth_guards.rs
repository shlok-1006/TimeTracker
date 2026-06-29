//! Integration tests for the auth middleware + role guards (Rule 9).
//!
//! Builds the real router and drives it with signed tokens. The guarded
//! endpoints (`/me`, `/desktop/ping`, `/dashboard/ping`, `/hr/ping`) do not
//! touch the database, so these run without Postgres. Asserts the STEP 1
//! acceptance criterion: a token with the wrong role receives 403.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;
use uuid::Uuid;

use server::jwt::JwtKeys;
use server::linear_service::LinearService;
use server::role::UserRole;
use server::storage::{GcsConfig, StorageClient};
use server::AppState;

const SECRET: &str = "integration-test-secret";

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

fn token_for(role: UserRole) -> String {
    JwtKeys::new(SECRET, 900)
        .issue(Uuid::new_v4(), role, None)
        .expect("issue token")
}

async fn status(role: Option<UserRole>, path: &str) -> StatusCode {
    let mut builder = Request::builder().uri(path);
    if let Some(r) = role {
        builder = builder.header("Authorization", format!("Bearer {}", token_for(r)));
    }
    let req = builder.body(Body::empty()).unwrap();
    app().oneshot(req).await.unwrap().status()
}

#[tokio::test]
async fn missing_token_is_unauthorized() {
    assert_eq!(status(None, "/me").await, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn malformed_token_is_unauthorized() {
    let req = Request::builder()
        .uri("/me")
        .header("Authorization", "Bearer not.a.jwt")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app().oneshot(req).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn any_authenticated_user_can_read_me() {
    assert_eq!(
        status(Some(UserRole::Employee), "/me").await,
        StatusCode::OK
    );
    assert_eq!(status(Some(UserRole::Hr), "/me").await, StatusCode::OK);
}

#[tokio::test]
async fn employee_allowed_on_desktop_denied_on_dashboard() {
    assert_eq!(
        status(Some(UserRole::Employee), "/desktop/ping").await,
        StatusCode::OK
    );
    // Wrong role => 403.
    assert_eq!(
        status(Some(UserRole::Employee), "/dashboard/ping").await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        status(Some(UserRole::Employee), "/hr/ping").await,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn hr_allowed_on_dashboard_and_hr_denied_on_desktop() {
    assert_eq!(
        status(Some(UserRole::Hr), "/dashboard/ping").await,
        StatusCode::OK
    );
    assert_eq!(status(Some(UserRole::Hr), "/hr/ping").await, StatusCode::OK);
    // Wrong role => 403.
    assert_eq!(
        status(Some(UserRole::Hr), "/desktop/ping").await,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn project_manager_is_dashboard_admin_but_not_hr() {
    assert_eq!(
        status(Some(UserRole::ProjectManager), "/dashboard/ping").await,
        StatusCode::OK
    );
    assert_eq!(
        status(Some(UserRole::ProjectManager), "/hr/ping").await,
        StatusCode::FORBIDDEN
    );
}

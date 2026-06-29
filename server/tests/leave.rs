//! Auth-gating tests for the leave routes (Rule 9). The day-counting and
//! approval workflow are unit-tested in the crate and verified live.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;
use uuid::Uuid;

use server::jwt::JwtKeys;
use server::linear_service::LinearService;
use server::role::UserRole;
use server::storage::{GcsConfig, StorageClient};
use server::AppState;

const SECRET: &str = "leave-test-secret";

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

async fn req(method: &str, path: &str, role: Option<UserRole>) -> StatusCode {
    let mut b = Request::builder().method(method).uri(path);
    if let Some(r) = role {
        b = b.header("Authorization", format!("Bearer {}", token(r)));
    }
    if method == "POST" {
        b = b.header("content-type", "application/json");
    }
    let body = if method == "POST" { Body::from("{}") } else { Body::empty() };
    app().oneshot(b.body(body).unwrap()).await.unwrap().status()
}

#[tokio::test]
async fn employee_self_service_requires_auth() {
    assert_eq!(req("GET", "/me/leave/balance", None).await, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn approver_routes_forbidden_for_employee() {
    assert_eq!(
        req("GET", "/admin/leave/requests", Some(UserRole::Employee)).await,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn hr_config_forbidden_for_non_hr() {
    // leave-type creation is HR-only: a project manager (admin-tier) is still rejected.
    assert_eq!(
        req("POST", "/admin/leave/types", Some(UserRole::ProjectManager)).await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        req("POST", "/admin/leave/types", Some(UserRole::Employee)).await,
        StatusCode::FORBIDDEN
    );
}

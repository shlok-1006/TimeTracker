//! Auth-gating tests for the admin drill-down routes (Rule 9).
//! PM-scope enforcement (team vs non-team) is exercised against a live DB in
//! the manual verification flow.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;
use uuid::Uuid;

use server::jwt::JwtKeys;
use server::linear_service::LinearService;
use server::role::UserRole;
use server::storage::{S3Config, StorageClient};
use server::AppState;

const SECRET: &str = "admin-test-secret";

fn app() -> axum::Router {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://localhost/timetracker")
        .expect("lazy pool");
    server::build_router(AppState::new(
        pool,
        JwtKeys::new(SECRET, 900),
        StorageClient::new(S3Config::from_env()),
        LinearService::from_env(),
        2_592_000,
    ))
}

fn token(role: UserRole) -> String {
    JwtKeys::new(SECRET, 900)
        .issue(Uuid::new_v4(), role, None)
        .unwrap()
}

async fn status(path: &str, role: Option<UserRole>) -> StatusCode {
    let mut b = Request::builder().uri(path);
    if let Some(r) = role {
        b = b.header("Authorization", format!("Bearer {}", token(r)));
    }
    app()
        .oneshot(b.body(Body::empty()).unwrap())
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn drilldown_requires_auth() {
    let id = Uuid::new_v4();
    assert_eq!(
        status(&format!("/admin/users/{id}/hours"), None).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        status(&format!("/admin/users/{id}/screenshots"), None).await,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn drilldown_forbidden_for_employee() {
    let id = Uuid::new_v4();
    assert_eq!(
        status(
            &format!("/admin/users/{id}/hours"),
            Some(UserRole::Employee)
        )
        .await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        status(
            &format!("/admin/users/{id}/screenshots"),
            Some(UserRole::Employee)
        )
        .await,
        StatusCode::FORBIDDEN
    );
}

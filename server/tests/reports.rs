//! Auth-gating tests for the report-viewing routes (Feature 1 Phase 4, Rule 9).
//! The PM/HR scoping logic is exercised against a live DB in analysis_reports.rs.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;
use uuid::Uuid;

use server::jwt::JwtKeys;
use server::linear_service::LinearService;
use server::role::UserRole;
use server::storage::{S3Config, StorageClient};
use server::AppState;

const SECRET: &str = "reports-test-secret";

fn app() -> axum::Router {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://localhost/timetracker")
        .expect("lazy pool");
    server::build_router(AppState::new(
        pool,
        JwtKeys::new(SECRET, 900),
        StorageClient::new(S3Config::from_env()),
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
async fn me_report_requires_auth() {
    assert_eq!(status("/me/report", None).await, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn admin_reports_forbidden_for_employee() {
    assert_eq!(
        status("/admin/reports", Some(UserRole::Employee)).await,
        StatusCode::FORBIDDEN
    );
    let id = Uuid::new_v4();
    assert_eq!(
        status(&format!("/admin/users/{id}/report"), Some(UserRole::Employee)).await,
        StatusCode::FORBIDDEN
    );
}

//! `/me/celebrations` — the self endpoint that lets an EMPLOYEE dashboard show birthdays and
//! work anniversaries. The point of this test is the access rule: any signed-in user reaches it
//! (unlike `/admin/directory/celebrations`, which is staff-only), and an anonymous caller does not.
//!
//! The window/attribution logic itself is covered by the pure-function unit tests in
//! `routes::employee_directory`; here we only pin the RBAC of the two routes.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;
use uuid::Uuid;

use server::jwt::JwtKeys;
use server::linear_service::LinearService;
use server::role::UserRole;
use server::storage::{S3Config, StorageClient};
use server::AppState;

const SECRET: &str = "celebrations-test-secret";

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

async fn status(path: &str, who: Option<(Uuid, UserRole)>) -> StatusCode {
    let mut b = Request::builder().uri(path);
    if let Some((id, role)) = who {
        let token = JwtKeys::new(SECRET, 900).issue(id, role, None, None).unwrap();
        b = b.header("Authorization", format!("Bearer {token}"));
    }
    app()
        .oneshot(b.body(Body::empty()).unwrap())
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn self_celebrations_need_a_session() {
    assert_eq!(
        status("/me/celebrations?days=7", None).await,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn an_employee_may_read_self_celebrations() {
    // The whole reason this route exists: an employee (who is refused by /admin/*) can read it.
    // We assert it is NOT refused by RBAC — status may be 200 (DB up) or 500 (no DB in CI), but
    // never 401/403.
    let s = status(
        "/me/celebrations?days=7",
        Some((Uuid::new_v4(), UserRole::Employee)),
    )
    .await;
    assert_ne!(s, StatusCode::UNAUTHORIZED, "an employee session must pass");
    assert_ne!(s, StatusCode::FORBIDDEN, "self celebrations are not staff-gated");
}

#[tokio::test]
async fn admin_celebrations_stay_staff_only() {
    // The companion assertion: the /admin feed still refuses an employee.
    assert_eq!(
        status(
            "/admin/directory/celebrations?days=7",
            Some((Uuid::new_v4(), UserRole::Employee))
        )
        .await,
        StatusCode::FORBIDDEN,
    );
    assert_eq!(
        status("/admin/directory/celebrations?days=7", None).await,
        StatusCode::UNAUTHORIZED,
    );
}

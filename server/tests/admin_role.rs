//! The `admin` tier (migration 0046) — the seat above HR.
//!
//! It exists for oversight of HR, not for a second set of features, so almost all of it is about
//! two rules holding:
//!
//!   * an admin can do everything HR can — otherwise the top of the hierarchy would somehow see
//!     less than the tier beneath it, which is the failure mode a rank ordering invites when
//!     checks are written `== Hr` instead of `>= Hr`;
//!   * HR cannot remove an admin, and cannot mint one. Without the second, any HR account could
//!     step past its own oversight by creating a new seat, and the tier would be decorative.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;
use uuid::Uuid;

use server::jwt::JwtKeys;
use server::linear_service::LinearService;
use server::middleware::{require_admin, require_employee, require_hr, require_staff};
use server::role::UserRole;
use server::storage::{S3Config, StorageClient};
use server::AppState;

const SECRET: &str = "admin-role-test-secret";

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

fn token_for(id: Uuid, role: UserRole) -> String {
    JwtKeys::new(SECRET, 900)
        .issue(id, role, None, None)
        .unwrap()
}

// ---- the ranking itself ----

#[tokio::test]
async fn the_roles_are_ordered_by_authority() {
    assert!(UserRole::Admin > UserRole::Hr);
    assert!(UserRole::Hr > UserRole::ProjectManager);
    assert!(UserRole::ProjectManager > UserRole::Employee);
    assert!(UserRole::Admin.at_least(UserRole::Hr));
    assert!(!UserRole::Hr.at_least(UserRole::Admin));
}

#[tokio::test]
async fn admin_passes_every_guard_hr_passes() {
    // The inheritance rule, at the level it is actually enforced.
    assert!(require_staff(UserRole::Admin).is_ok());
    assert!(
        require_hr(UserRole::Admin).is_ok(),
        "admin must have HR's reach"
    );
    assert!(require_admin(UserRole::Admin).is_ok());

    // And HR does not gain admin's.
    assert!(require_hr(UserRole::Hr).is_ok());
    assert!(require_admin(UserRole::Hr).is_err());
    assert!(require_admin(UserRole::ProjectManager).is_err());
    assert!(require_admin(UserRole::Employee).is_err());

    // Admin is staff, not an employee — it belongs on the dashboard, not the desktop app.
    assert!(UserRole::Admin.is_dashboard());
    assert!(require_employee(UserRole::Admin).is_err());
}

#[tokio::test]
async fn the_role_round_trips_through_its_string() {
    assert_eq!(UserRole::Admin.as_str(), "admin");
    assert_eq!("admin".parse::<UserRole>().unwrap(), UserRole::Admin);
}

// ---- over HTTP ----

async fn status(method: &str, path: &str, body: &str, who: (Uuid, UserRole)) -> StatusCode {
    let req = Request::builder()
        .method(method)
        .uri(path)
        .header(
            "Authorization",
            format!("Bearer {}", token_for(who.0, who.1)),
        )
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    app().oneshot(req).await.unwrap().status()
}

#[tokio::test]
async fn hr_cannot_mint_an_admin() {
    let hr = (Uuid::new_v4(), UserRole::Hr);
    let body =
        r#"{"name":"X","email":"x@test.local","password":"averylongpassword","role":"admin"}"#;
    assert_eq!(
        status("POST", "/admin/users", body, hr).await,
        StatusCode::FORBIDDEN,
        "an HR account creating an admin would be stepping past its own oversight"
    );
}

#[tokio::test]
async fn an_admin_reaches_the_hr_only_routes() {
    // Not asserting 200 — these touch a DB this test does not seed. Asserting they are not
    // REFUSED is the whole claim: admin inherits HR's reach.
    let admin = (Uuid::new_v4(), UserRole::Admin);
    for (method, path) in [("GET", "/admin/users"), ("GET", "/admin/audit")] {
        let s = status(method, path, "", admin).await;
        assert_ne!(s, StatusCode::FORBIDDEN, "{path} must admit an admin");
        assert_ne!(s, StatusCode::UNAUTHORIZED, "{path}");
    }
}

// ---- DB-backed: the deletion rule ----

async fn real_pool() -> Option<sqlx::PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .ok()
}

async fn seed(pool: &sqlx::PgPool, role: &str) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO users (id, name, email, password_hash, role)
         VALUES ($1, 'Role Test', $2, 'x', $3::user_role)",
    )
    .bind(id)
    .bind(format!("role-{id}@test.local"))
    .bind(role)
    .execute(pool)
    .await
    .expect("seed user");
    id
}

#[tokio::test]
async fn hr_cannot_delete_an_admin_but_an_admin_can() {
    let Some(pool) = real_pool().await else {
        eprintln!("DATABASE_URL unset — skipping admin deletion test");
        return;
    };

    let hr = seed(&pool, "hr").await;
    let victim = seed(&pool, "admin").await;

    assert_eq!(
        status(
            "DELETE",
            &format!("/admin/users/{victim}"),
            "",
            (hr, UserRole::Hr)
        )
        .await,
        StatusCode::FORBIDDEN,
        "HR removing the seat that oversees HR is the exact thing this tier prevents"
    );

    // Still there.
    let alive: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE id = $1)")
        .bind(victim)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(alive, "the refused delete must not have removed anything");

    // An admin may remove another admin — a seat nobody can revoke is worse than one that needs
    // a peer to revoke it.
    let boss = seed(&pool, "admin").await;
    assert_eq!(
        status(
            "DELETE",
            &format!("/admin/users/{victim}"),
            "",
            (boss, UserRole::Admin)
        )
        .await,
        StatusCode::OK,
        "an admin must be able to remove another admin"
    );

    sqlx::query("DELETE FROM users WHERE id = ANY($1)")
        .bind(vec![hr, victim, boss])
        .execute(&pool)
        .await
        .ok();
}

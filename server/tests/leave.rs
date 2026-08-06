//! Auth-gating tests for the leave routes (Rule 9). The day-counting and
//! approval workflow are unit-tested in the crate and verified live.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;
use uuid::Uuid;

use server::jwt::JwtKeys;
use server::linear_service::LinearService;
use server::role::UserRole;
use server::storage::{S3Config, StorageClient};
use server::AppState;

const SECRET: &str = "leave-test-secret";

fn app() -> axum::Router {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://localhost/timetracker")
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

async fn req(method: &str, path: &str, role: Option<UserRole>) -> StatusCode {
    let mut b = Request::builder().method(method).uri(path);
    if let Some(r) = role {
        b = b.header("Authorization", format!("Bearer {}", token(r)));
    }
    if method == "POST" {
        b = b.header("content-type", "application/json");
    }
    let body = if method == "POST" {
        Body::from("{}")
    } else {
        Body::empty()
    };
    app().oneshot(b.body(body).unwrap()).await.unwrap().status()
}

#[tokio::test]
async fn employee_self_service_requires_auth() {
    assert_eq!(
        req("GET", "/me/leave/balance", None).await,
        StatusCode::UNAUTHORIZED
    );
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

#[tokio::test]
async fn holidays_readable_by_every_signed_in_role() {
    // The company calendar is published, not personal: any authenticated user may read
    // it (the HRMS employee dashboard shows upcoming holidays). Anything but 401/403.
    for role in [UserRole::Employee, UserRole::ProjectManager, UserRole::Hr] {
        let status = req("GET", "/me/holidays", Some(role)).await;
        assert_ne!(
            status,
            StatusCode::FORBIDDEN,
            "{role:?} must not be forbidden"
        );
        assert_ne!(
            status,
            StatusCode::UNAUTHORIZED,
            "{role:?} must not be unauthorized"
        );
    }
}

#[tokio::test]
async fn holidays_still_require_a_session() {
    assert_eq!(
        req("GET", "/me/holidays", None).await,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn creating_a_holiday_stays_hr_only() {
    // Opening the READ must not have widened the write.
    assert_eq!(
        req("POST", "/admin/holidays", Some(UserRole::Employee)).await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        req("POST", "/admin/holidays", Some(UserRole::ProjectManager)).await,
        StatusCode::FORBIDDEN
    );
}

// ---- DB-backed: category defaults + manual override / adjust / delete ----

async fn real_pool() -> Option<sqlx::PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .ok()
}

#[tokio::test]
async fn category_defaults_and_manual_overrides() {
    use server::db::{leave, users};
    use server::employment_type::EmploymentType;

    let Some(pool) = real_pool().await else {
        eprintln!("skipping leave category test: DATABASE_URL not set");
        return;
    };
    let tag = Uuid::new_v4();
    let year = 2020;

    // A leave type with distinct per-category defaults.
    let lt = leave::create_type(&pool, &format!("cat-{tag}"), true, 20.0, 10.0, 5.0)
        .await
        .unwrap();

    let emp = users::create(
        &pool,
        "emp",
        &format!("emp-{tag}@t.local"),
        "h",
        UserRole::Employee,
        None,
    )
    .await
    .unwrap();
    let con = users::create(
        &pool,
        "con",
        &format!("con-{tag}@t.local"),
        "h",
        UserRole::Employee,
        None,
    )
    .await
    .unwrap();
    users::set_employment_type(&pool, con.id, EmploymentType::Contractor)
        .await
        .unwrap();
    let intern = users::create(
        &pool,
        "int",
        &format!("int-{tag}@t.local"),
        "h",
        UserRole::Employee,
        None,
    )
    .await
    .unwrap();
    users::set_employment_type(&pool, intern.id, EmploymentType::Intern)
        .await
        .unwrap();
    let pm = users::create(
        &pool,
        "pm",
        &format!("pm-{tag}@t.local"),
        "h",
        UserRole::ProjectManager,
        None,
    )
    .await
    .unwrap();

    // Fetch this type's balance row for a user.
    async fn row(pool: &sqlx::PgPool, uid: Uuid, lt: Uuid, year: i32) -> leave::Balance {
        leave::balances(pool, uid, year)
            .await
            .unwrap()
            .into_iter()
            .find(|b| b.leave_type_id == lt)
            .expect("type present in balances")
    }

    // Category defaults apply with no explicit allocation.
    let b = row(&pool, emp.id, lt.id, year).await;
    assert_eq!(b.allotted_days, 20.0);
    assert!(!b.is_override);
    assert_eq!(row(&pool, con.id, lt.id, year).await.allotted_days, 10.0);
    assert_eq!(row(&pool, intern.id, lt.id, year).await.allotted_days, 5.0);
    // PM is treated as the employee category.
    assert_eq!(row(&pool, pm.id, lt.id, year).await.allotted_days, 20.0);

    // Adjust +5 from the effective default of 20 → 25 (now an override).
    let n = leave::adjust_allocation(&pool, emp.id, lt.id, year, 5.0)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(n, 25.0);
    let b = row(&pool, emp.id, lt.id, year).await;
    assert_eq!(b.allotted_days, 25.0);
    assert!(b.is_override);

    // A large decrease clamps at 0.
    let n = leave::adjust_allocation(&pool, emp.id, lt.id, year, -100.0)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(n, 0.0);

    // Absolute set, then delete the override → back to the category default.
    leave::upsert_allocation(&pool, emp.id, lt.id, year, 12.0)
        .await
        .unwrap();
    assert_eq!(row(&pool, emp.id, lt.id, year).await.allotted_days, 12.0);
    assert!(leave::delete_allocation(&pool, emp.id, lt.id, year)
        .await
        .unwrap());
    let b = row(&pool, emp.id, lt.id, year).await;
    assert_eq!(b.allotted_days, 20.0);
    assert!(!b.is_override);

    // Cleanup (deleting users cascades their allocations).
    for u in [emp.id, con.id, intern.id, pm.id] {
        users::delete(&pool, u).await.unwrap();
    }
    sqlx::query("DELETE FROM leave_types WHERE id = $1")
        .bind(lt.id)
        .execute(&pool)
        .await
        .unwrap();
}

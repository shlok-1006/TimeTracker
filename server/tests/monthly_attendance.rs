//! HR monthly attendance report: per-employee present/partial/absent counts for a
//! month, joined with the paid-leave balance left for the year.
//!
//! Two claims matter and are tested here against a real database:
//!   * the month's day counts come straight from the `attendance_days` rollup, and
//!   * `leaves_remaining` is the paid balance that FALLS as leave is approved —
//!     the "total leaves left that changes" the feature is about.
//!
//! The balance is asserted as a DELTA (before vs. after approving a 2-day leave)
//! so the test does not depend on whatever leave types the database is seeded with.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::NaiveDate;
use tower::ServiceExt;
use uuid::Uuid;

use server::db::{attendance, leave};
use server::jwt::JwtKeys;
use server::linear_service::LinearService;
use server::role::UserRole;
use server::storage::{S3Config, StorageClient};
use server::AppState;

const SECRET: &str = "monthly-attendance-test-secret";

async fn real_pool() -> Option<sqlx::PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .ok()
}

async fn seed_user(pool: &sqlx::PgPool) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, name, email, password_hash, role) VALUES ($1,'Month Test',$2,'x','employee')")
        .bind(id)
        .bind(format!("month-{id}@test.local"))
        .execute(pool)
        .await
        .expect("seed user");
    id
}

async fn seed_day(pool: &sqlx::PgPool, uid: Uuid, day: NaiveDate, status: &str, worked: i32) {
    sqlx::query(
        "INSERT INTO attendance_days (user_id, day, status, worked_seconds, idle_seconds, note)
         VALUES ($1,$2,$3,$4,0,'')
         ON CONFLICT (user_id, day) DO UPDATE SET status = EXCLUDED.status,
             worked_seconds = EXCLUDED.worked_seconds",
    )
    .bind(uid)
    .bind(day)
    .bind(status)
    .bind(worked)
    .execute(pool)
    .await
    .expect("seed attendance day");
}

async fn cleanup(pool: &sqlx::PgPool, uid: Uuid) {
    sqlx::query("DELETE FROM leave_requests WHERE user_id = $1")
        .bind(uid)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM attendance_days WHERE user_id = $1")
        .bind(uid)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(uid)
        .execute(pool)
        .await
        .ok();
}

#[tokio::test]
async fn monthly_counts_come_from_the_rollup() {
    let Some(pool) = real_pool().await else {
        eprintln!("DATABASE_URL unset — skipping monthly attendance test");
        return;
    };
    let uid = seed_user(&pool).await;

    // A long-past month so no live data is touched. 2 present, 1 partial, 1 absent, 1 leave.
    let d = |n: u32| NaiveDate::from_ymd_opt(2020, 3, n).unwrap();
    seed_day(&pool, uid, d(2), "present", 8 * 3600).await;
    seed_day(&pool, uid, d(3), "present", 7 * 3600).await;
    seed_day(&pool, uid, d(4), "partial", 3 * 3600).await;
    seed_day(&pool, uid, d(5), "absent", 0).await;
    seed_day(&pool, uid, d(6), "leave", 0).await;

    let rows = attendance::report(&pool, d(1), d(31), None)
        .await
        .expect("report");
    let me = rows
        .iter()
        .find(|r| r.user_id == uid)
        .expect("employee present in the company report");

    assert_eq!(me.present, 2, "two present days");
    assert_eq!(me.partial, 1, "one partial day");
    assert_eq!(me.absent, 1, "one absent day");
    assert_eq!(me.leave, 1, "one leave day");
    assert_eq!(me.worked_seconds, (8 + 7 + 3) * 3600, "summed worked seconds");

    cleanup(&pool, uid).await;
}

#[tokio::test]
async fn leaves_remaining_falls_when_a_leave_is_approved() {
    let Some(pool) = real_pool().await else {
        eprintln!("DATABASE_URL unset — skipping leave-balance delta test");
        return;
    };
    let uid = seed_user(&pool).await;
    let year = 2020;

    // A paid leave type must exist for the balance to be non-trivial. Reuse any
    // paid type already in the DB; if there is none, this DB can't express the
    // feature and the delta below would be zero — seed one for the test.
    let paid_type: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM leave_types WHERE paid = TRUE ORDER BY name LIMIT 1")
            .fetch_optional(&pool)
            .await
            .expect("query leave types");
    let (leave_type_id, made_type) = match paid_type {
        Some(id) => (id, false),
        None => {
            let id = Uuid::new_v4();
            sqlx::query("INSERT INTO leave_types (id, name, paid, default_days) VALUES ($1,$2,TRUE,12)")
                .bind(id)
                .bind(format!("Test Paid {id}"))
                .execute(&pool)
                .await
                .expect("seed leave type");
            (id, true)
        }
    };

    let before = *leave::remaining_paid_by_user(&pool, year, None)
        .await
        .expect("balances")
        .get(&uid)
        .expect("new employee has a paid allotment");
    assert!(before > 0.0, "a fresh employee should have paid leave to spend");

    // Approve two days of that paid type in the year.
    sqlx::query(
        "INSERT INTO leave_requests (user_id, leave_type_id, start_date, end_date, days, status)
         VALUES ($1,$2,$3,$4,2,'approved')",
    )
    .bind(uid)
    .bind(leave_type_id)
    .bind(NaiveDate::from_ymd_opt(year, 3, 10).unwrap())
    .bind(NaiveDate::from_ymd_opt(year, 3, 11).unwrap())
    .execute(&pool)
    .await
    .expect("approve leave");

    let after = *leave::remaining_paid_by_user(&pool, year, None)
        .await
        .expect("balances")
        .get(&uid)
        .unwrap();

    assert!(
        (before - after - 2.0).abs() < 1e-9,
        "approving 2 leave days must drop the remaining balance by exactly 2 (before {before}, after {after})"
    );

    cleanup(&pool, uid).await;
    if made_type {
        sqlx::query("DELETE FROM leave_types WHERE id = $1")
            .bind(leave_type_id)
            .execute(&pool)
            .await
            .ok();
    }
}

// ---- the endpoint is staff-only ----

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

async fn status(who: Option<(Uuid, UserRole)>) -> StatusCode {
    let mut b = Request::builder().uri("/admin/attendance/monthly?month=2020-03");
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
async fn the_monthly_report_is_staff_only() {
    assert_eq!(status(None).await, StatusCode::UNAUTHORIZED);
    assert_eq!(
        status(Some((Uuid::new_v4(), UserRole::Employee))).await,
        StatusCode::FORBIDDEN,
        "an employee must not read the company attendance report"
    );
}

//! `/admin/leave/calendar` — leave across a date range for the HRMS month register.
//!
//! Three things must hold, and they are exactly what the register depends on:
//!   * OVERLAP, not containment — a multi-day leave that straddles either edge of the window is
//!     still returned in full, and a leave entirely outside the window is not;
//!   * only `approved`/`pending` count — a rejected or cancelled request means nobody is on leave;
//!   * team scope matches the pending queue — HR sees everyone, a PM only their own team.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::NaiveDate;
use tower::ServiceExt;
use uuid::Uuid;

use server::db::leave;
use server::jwt::JwtKeys;
use server::linear_service::LinearService;
use server::role::UserRole;
use server::storage::{S3Config, StorageClient};
use server::AppState;

const SECRET: &str = "leave-calendar-test-secret";

async fn real_pool() -> Option<sqlx::PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .ok()
}

fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

async fn seed_user(pool: &sqlx::PgPool, role: &str) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, name, email, password_hash, role) VALUES ($1,$2,$3,'x',$4::user_role)")
        .bind(id)
        .bind(format!("Cal {role}"))
        .bind(format!("cal-{id}@test.local"))
        .bind(role)
        .execute(pool)
        .await
        .expect("seed user");
    id
}

async fn paid_type(pool: &sqlx::PgPool) -> (Uuid, bool) {
    if let Some(id) = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM leave_types WHERE paid = TRUE ORDER BY name LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .expect("query types")
    {
        return (id, false);
    }
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO leave_types (id, name, paid, default_days) VALUES ($1,$2,TRUE,12)")
        .bind(id)
        .bind(format!("Cal Type {id}"))
        .execute(pool)
        .await
        .expect("seed type");
    (id, true)
}

async fn seed_leave(
    pool: &sqlx::PgPool,
    user: Uuid,
    lt: Uuid,
    start: NaiveDate,
    end: NaiveDate,
    status: &str,
) {
    sqlx::query(
        "INSERT INTO leave_requests (user_id, leave_type_id, start_date, end_date, days, status)
         VALUES ($1,$2,$3,$4,$5,$6)",
    )
    .bind(user)
    .bind(lt)
    .bind(start)
    .bind(end)
    .bind(((end - start).num_days() + 1) as f64)
    .bind(status)
    .execute(pool)
    .await
    .expect("seed leave");
}

#[tokio::test]
async fn range_returns_overlapping_approved_and_pending_scoped_to_the_team() {
    let Some(pool) = real_pool().await else {
        eprintln!("DATABASE_URL unset — skipping leave-calendar test");
        return;
    };

    let pm = seed_user(&pool, "project_manager").await;
    let on_team = seed_user(&pool, "employee").await;
    let off_team = seed_user(&pool, "employee").await;
    let (lt, made_type) = paid_type(&pool).await;

    sqlx::query("INSERT INTO user_managers (user_id, manager_id) VALUES ($1,$2)")
        .bind(on_team)
        .bind(pm)
        .execute(&pool)
        .await
        .expect("assign manager");

    let (from, to) = (ymd(2026, 8, 1), ymd(2026, 8, 31));
    // on_team: inside (approved), straddling the start edge (pending), rejected (excluded),
    // entirely before the window (excluded).
    seed_leave(
        &pool,
        on_team,
        lt,
        ymd(2026, 8, 4),
        ymd(2026, 8, 6),
        "approved",
    )
    .await;
    seed_leave(
        &pool,
        on_team,
        lt,
        ymd(2026, 7, 30),
        ymd(2026, 8, 2),
        "pending",
    )
    .await;
    seed_leave(
        &pool,
        on_team,
        lt,
        ymd(2026, 8, 10),
        ymd(2026, 8, 12),
        "rejected",
    )
    .await;
    seed_leave(
        &pool,
        on_team,
        lt,
        ymd(2026, 7, 1),
        ymd(2026, 7, 5),
        "approved",
    )
    .await;
    // off_team: an approved leave in-window — visible to HR, not to this PM.
    seed_leave(
        &pool,
        off_team,
        lt,
        ymd(2026, 8, 20),
        ymd(2026, 8, 22),
        "approved",
    )
    .await;

    // PM scope: only their team member's overlapping approved + pending.
    let pm_rows = leave::list_in_range(&pool, from, to, Some(pm))
        .await
        .unwrap();
    let mine: Vec<_> = pm_rows.iter().filter(|r| r.user_id == on_team).collect();
    assert_eq!(
        mine.len(),
        2,
        "approved + pending that overlap, nothing else"
    );
    assert!(mine
        .iter()
        .any(|r| r.status == "approved" && r.start_date == ymd(2026, 8, 4)));
    assert!(
        mine.iter()
            .any(|r| r.status == "pending" && r.start_date == ymd(2026, 7, 30)),
        "a leave straddling the start edge must still be returned in full"
    );
    assert!(
        !mine.iter().any(|r| r.status == "rejected"),
        "rejected is not on leave"
    );
    assert!(
        !mine.iter().any(|r| r.start_date == ymd(2026, 7, 1)),
        "a leave entirely before the window must be excluded"
    );
    assert!(
        !pm_rows.iter().any(|r| r.user_id == off_team),
        "a PM must not see a leave outside their team"
    );

    // HR scope (None): sees the off-team leave too.
    let hr_rows = leave::list_in_range(&pool, from, to, None).await.unwrap();
    assert!(
        hr_rows
            .iter()
            .any(|r| r.user_id == off_team && r.start_date == ymd(2026, 8, 20)),
        "HR sees everyone"
    );

    sqlx::query("DELETE FROM leave_requests WHERE user_id = ANY($1)")
        .bind(vec![on_team, off_team])
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM user_managers WHERE user_id = $1")
        .bind(on_team)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM users WHERE id = ANY($1)")
        .bind(vec![pm, on_team, off_team])
        .execute(&pool)
        .await
        .ok();
    if made_type {
        sqlx::query("DELETE FROM leave_types WHERE id = $1")
            .bind(lt)
            .execute(&pool)
            .await
            .ok();
    }
}

// ---- RBAC over HTTP ----

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
    let path = "/admin/leave/calendar?from=2026-08-01&to=2026-08-31";
    let mut b = Request::builder().uri(path);
    if let Some((id, role)) = who {
        let token = JwtKeys::new(SECRET, 900)
            .issue(id, role, None, None)
            .unwrap();
        b = b.header("Authorization", format!("Bearer {token}"));
    }
    app()
        .oneshot(b.body(Body::empty()).unwrap())
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn the_calendar_is_staff_only() {
    assert_eq!(status(None).await, StatusCode::UNAUTHORIZED);
    assert_eq!(
        status(Some((Uuid::new_v4(), UserRole::Employee))).await,
        StatusCode::FORBIDDEN,
        "an employee cannot read the company leave calendar"
    );
}

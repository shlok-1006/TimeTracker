//! `GET /me/leave/out` — who is off work in a window, for ANY signed-in user (the HRMS "On Leave" card).
//!
//! What must hold:
//!   * every role may read it, and it is company-wide (an employee sees a colleague outside their team);
//!   * approved and pending count; rejected, cancelled and expired do not; a deactivated person is not listed;
//!   * OVERLAP with the window, like the leave calendar (a leave straddling an edge is returned in full);
//!   * it never carries the leave type or the reason;
//!   * the window is bounded (at most 62 days, `to` not before `from`), and signing in is required.

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

const SECRET: &str = "leave-out-test-secret";

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

async fn seed_user(pool: &sqlx::PgPool, role: &str, name: &str) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, name, email, password_hash, role) VALUES ($1,$2,$3,'x',$4::user_role)")
        .bind(id)
        .bind(name)
        .bind(format!("out-{id}@test.local"))
        .bind(role)
        .execute(pool)
        .await
        .expect("seed user");
    id
}

async fn any_type(pool: &sqlx::PgPool) -> (Uuid, bool) {
    if let Some(id) =
        sqlx::query_scalar::<_, Uuid>("SELECT id FROM leave_types ORDER BY name LIMIT 1")
            .fetch_optional(pool)
            .await
            .expect("query types")
    {
        return (id, false);
    }
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO leave_types (id, name, paid, default_days) VALUES ($1,$2,TRUE,12)")
        .bind(id)
        .bind(format!("Out Type {id}"))
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
        "INSERT INTO leave_requests (user_id, leave_type_id, start_date, end_date, days, status, reason)
         VALUES ($1,$2,$3,$4,$5,$6,'a private reason')",
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

fn app(pool: sqlx::PgPool) -> axum::Router {
    server::build_router(AppState::new(
        pool,
        JwtKeys::new(SECRET, 900),
        StorageClient::new(S3Config::insecure_local()),
        LinearService::from_env(),
        server::claude_provider::ClaudeProvider::from_env(),
        2_592_000,
    ))
}

async fn get(
    pool: &sqlx::PgPool,
    path: &str,
    who: Option<(Uuid, UserRole)>,
) -> (StatusCode, serde_json::Value) {
    let mut b = Request::builder().uri(path);
    if let Some((id, role)) = who {
        let token = JwtKeys::new(SECRET, 900)
            .issue(id, role, None, None)
            .unwrap();
        b = b.header("Authorization", format!("Bearer {token}"));
    }
    let res = app(pool.clone())
        .oneshot(b.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}

#[tokio::test]
async fn anyone_signed_in_sees_who_is_out_company_wide_without_type_or_reason() {
    let Some(pool) = real_pool().await else {
        eprintln!("DATABASE_URL unset — skipping leave-out test");
        return;
    };
    let viewer = seed_user(&pool, "employee", "Out Viewer").await;
    let colleague = seed_user(&pool, "employee", "Out Colleague").await;
    let gone = seed_user(&pool, "employee", "Out Gone").await;
    let (lt, made_type) = any_type(&pool).await;

    // An August 2031 window keeps clear of real data.
    seed_leave(
        &pool,
        colleague,
        lt,
        ymd(2031, 8, 4),
        ymd(2031, 8, 6),
        "approved",
    )
    .await;
    seed_leave(
        &pool,
        colleague,
        lt,
        ymd(2031, 7, 30),
        ymd(2031, 8, 2),
        "pending",
    )
    .await; // straddles `from`
    seed_leave(
        &pool,
        colleague,
        lt,
        ymd(2031, 8, 10),
        ymd(2031, 8, 10),
        "rejected",
    )
    .await;
    seed_leave(
        &pool,
        colleague,
        lt,
        ymd(2031, 8, 11),
        ymd(2031, 8, 11),
        "cancelled",
    )
    .await;
    seed_leave(
        &pool,
        colleague,
        lt,
        ymd(2031, 7, 1),
        ymd(2031, 7, 3),
        "approved",
    )
    .await; // before the window
    seed_leave(
        &pool,
        gone,
        lt,
        ymd(2031, 8, 5),
        ymd(2031, 8, 5),
        "approved",
    )
    .await;
    sqlx::query("UPDATE users SET deactivated_at = now() WHERE id = $1")
        .bind(gone)
        .execute(&pool)
        .await
        .expect("deactivate");

    let rows = leave::list_out_in_range(&pool, ymd(2031, 8, 1), ymd(2031, 8, 31))
        .await
        .unwrap();
    let theirs: Vec<_> = rows.iter().filter(|r| r.user_id == colleague).collect();
    assert_eq!(
        theirs.len(),
        2,
        "approved + pending overlapping the window, nothing else"
    );
    assert!(
        theirs
            .iter()
            .any(|r| r.status == "pending" && r.start_date == ymd(2031, 7, 30)),
        "a leave straddling the window's start is returned in full"
    );
    assert!(
        !rows.iter().any(|r| r.user_id == gone),
        "a deactivated person is not on leave"
    );

    // Over HTTP, as a plain employee who manages nobody: 200, company-wide, no type, no reason.
    let (status, body) = get(
        &pool,
        "/me/leave/out?from=2031-08-01&to=2031-08-31",
        Some((viewer, UserRole::Employee)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let list = body.as_array().expect("a JSON array");
    let mine: Vec<_> = list
        .iter()
        .filter(|r| r["user_id"] == colleague.to_string())
        .collect();
    assert_eq!(
        mine.len(),
        2,
        "an employee sees a colleague's leave: {body}"
    );
    for r in &mine {
        assert_eq!(r["name"], "Out Colleague");
        assert!(
            r.get("reason").is_none() && r.get("leave_type_name").is_none(),
            "never the type or the reason: {r}"
        );
        assert!(
            r["start_date"].is_string() && r["end_date"].is_string() && r["status"].is_string()
        );
    }

    for id in [colleague, gone] {
        sqlx::query("DELETE FROM leave_requests WHERE user_id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .ok();
    }
    sqlx::query("DELETE FROM users WHERE id = ANY($1)")
        .bind(vec![viewer, colleague, gone])
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

#[tokio::test]
async fn the_window_is_bounded_and_signing_in_is_required() {
    let Some(pool) = real_pool().await else {
        eprintln!("DATABASE_URL unset — skipping leave-out test");
        return;
    };
    let viewer = seed_user(&pool, "employee", "Out Bounds").await;
    let me = Some((viewer, UserRole::Employee));
    assert_eq!(
        get(&pool, "/me/leave/out?from=2031-08-01&to=2031-08-31", None)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        get(&pool, "/me/leave/out?from=2031-08-31&to=2031-08-01", me)
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        get(&pool, "/me/leave/out?from=2031-01-01&to=2031-06-30", me)
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        get(&pool, "/me/leave/out?from=2031-08-01&to=2031-10-02", me)
            .await
            .0,
        StatusCode::OK,
        "62 days is allowed"
    );
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(viewer)
        .execute(&pool)
        .await
        .ok();
}

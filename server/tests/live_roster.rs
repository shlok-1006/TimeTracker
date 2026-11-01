//! The live roster's off-the-clock statuses (`GET /admin/team`, `GET /admin/teams/:id/live`).
//!
//! What must hold:
//!   * someone who worked today and stopped ("End My Day", app open or closed) reads `logged_out`;
//!   * an approved leave covering today reads `on_leave` — including a half day after a morning's work;
//!   * anyone tracking right now shows their live status, even on a leave day;
//!   * someone who hasn't started today reads `not_logged_in` — app open with the timer never
//!     started included — and a pending leave changes nothing.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;
use uuid::Uuid;

use server::db::presence;
use server::jwt::JwtKeys;
use server::linear_service::LinearService;
use server::presence::PresenceStatus;
use server::role::UserRole;
use server::storage::{S3Config, StorageClient};
use server::AppState;

const SECRET: &str = "live-roster-test-secret";

async fn real_pool() -> Option<sqlx::PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .ok()
}

async fn seed_user(pool: &sqlx::PgPool, role: &str, name: &str) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, name, email, password_hash, role) VALUES ($1,$2,$3,'x',$4::user_role)")
        .bind(id)
        .bind(name)
        .bind(format!("live-{id}@test.local"))
        .bind(role)
        .execute(pool)
        .await
        .expect("seed user");
    id
}

/// A few minutes of tracked work earlier today (inside today's UTC window, like `today_seconds`).
async fn worked_today(pool: &sqlx::PgPool, user: Uuid) {
    sqlx::query(
        "INSERT INTO intervals (id, user_id, start_utc, end_utc, idle, kind)
         VALUES ($1, $2, GREATEST(date_trunc('day', now()), now() - interval '10 minutes'), now(), false, 'active')",
    )
    .bind(Uuid::new_v4())
    .bind(user)
    .execute(pool)
    .await
    .expect("seed interval");
}

/// A heartbeat long enough ago that the server derives `not_logged_in`.
async fn stale_heartbeat(pool: &sqlx::PgPool, user: Uuid) {
    sqlx::query(
        "INSERT INTO presence (user_id, status, last_seen_at)
         VALUES ($1, 'not_working'::presence_status, now() - interval '10 minutes')",
    )
    .bind(user)
    .execute(pool)
    .await
    .expect("seed stale presence");
}

async fn leave_type(pool: &sqlx::PgPool) -> Uuid {
    if let Some(id) =
        sqlx::query_scalar::<_, Uuid>("SELECT id FROM leave_types ORDER BY name LIMIT 1")
            .fetch_optional(pool)
            .await
            .expect("query types")
    {
        return id;
    }
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO leave_types (id, name, paid, default_days) VALUES ($1,$2,TRUE,12)")
        .bind(id)
        .bind(format!("Live Type {id}"))
        .execute(pool)
        .await
        .expect("seed type");
    id
}

async fn leave_today(pool: &sqlx::PgPool, user: Uuid, lt: Uuid, status: &str) {
    let today = server::org_time::today();
    sqlx::query(
        "INSERT INTO leave_requests (user_id, leave_type_id, start_date, end_date, days, status, reason)
         VALUES ($1,$2,$3,$3,1,$4,'')",
    )
    .bind(user)
    .bind(lt)
    .bind(today)
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

async fn get(pool: &sqlx::PgPool, path: &str, viewer: Uuid) -> serde_json::Value {
    let token = JwtKeys::new(SECRET, 900)
        .issue(viewer, UserRole::Hr, None, None)
        .unwrap();
    let res = app(pool.clone())
        .oneshot(
            Request::builder()
                .uri(path)
                .header("Authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK, "{path}");
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

/// `user.id` → `status` from either roster shape (`[...]` or `{ "members": [...] }`).
fn statuses(body: &serde_json::Value) -> std::collections::HashMap<String, String> {
    let rows = body
        .as_array()
        .or_else(|| body.get("members").and_then(|m| m.as_array()))
        .expect("roster rows");
    rows.iter()
        .map(|r| {
            (
                r["user"]["id"].as_str().unwrap().to_string(),
                r["status"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

#[tokio::test]
async fn off_the_clock_reads_logged_out_or_on_leave() {
    let Some(pool) = real_pool().await else {
        eprintln!("DATABASE_URL unset — skipping live-roster test");
        return;
    };
    let hr = seed_user(&pool, "hr", "Live HR").await;
    let lt = leave_type(&pool).await;

    // Ended the day with the app still open (heartbeat says not_working).
    let ended_open = seed_user(&pool, "employee", "Live Ended Open").await;
    worked_today(&pool, ended_open).await;
    presence::heartbeat(&pool, ended_open, PresenceStatus::NotWorking, None, None)
        .await
        .unwrap();

    // Ended the day and closed the app (heartbeat went stale).
    let ended_closed = seed_user(&pool, "employee", "Live Ended Closed").await;
    worked_today(&pool, ended_closed).await;
    stale_heartbeat(&pool, ended_closed).await;

    // Hasn't started: never seen, and app open but no time today.
    let not_started = seed_user(&pool, "employee", "Live Not Started").await;
    let app_open = seed_user(&pool, "employee", "Live App Open").await;
    presence::heartbeat(&pool, app_open, PresenceStatus::NotWorking, None, None)
        .await
        .unwrap();

    // Leave today: off, half day after a morning's work, working anyway, and merely pending.
    let on_leave = seed_user(&pool, "employee", "Live On Leave").await;
    leave_today(&pool, on_leave, lt, "approved").await;
    let half_day = seed_user(&pool, "employee", "Live Half Day").await;
    worked_today(&pool, half_day).await;
    stale_heartbeat(&pool, half_day).await;
    leave_today(&pool, half_day, lt, "approved").await;
    let working_on_leave = seed_user(&pool, "employee", "Live Working On Leave").await;
    leave_today(&pool, working_on_leave, lt, "approved").await;
    presence::heartbeat(&pool, working_on_leave, PresenceStatus::Working, None, None)
        .await
        .unwrap();
    let pending = seed_user(&pool, "employee", "Live Pending").await;
    leave_today(&pool, pending, lt, "pending").await;

    let expected = [
        (ended_open, "logged_out"),
        (ended_closed, "logged_out"),
        (not_started, "not_logged_in"),
        (app_open, "not_logged_in"),
        (on_leave, "on_leave"),
        (half_day, "on_leave"),
        (working_on_leave, "working"),
        (pending, "not_logged_in"),
    ];

    // HR's whole-company board.
    let all = statuses(&get(&pool, "/admin/team", hr).await);
    for (id, want) in expected {
        assert_eq!(
            all.get(&id.to_string()).map(String::as_str),
            Some(want),
            "/admin/team {id}"
        );
    }

    // The same people through one team's live board.
    let team = Uuid::new_v4();
    sqlx::query("INSERT INTO teams (id, name, description) VALUES ($1,$2,'')")
        .bind(team)
        .bind(format!("Live Team {team}"))
        .execute(&pool)
        .await
        .unwrap();
    for (id, _) in expected {
        sqlx::query("INSERT INTO user_teams (user_id, team_id) VALUES ($1,$2)")
            .bind(id)
            .bind(team)
            .execute(&pool)
            .await
            .unwrap();
    }
    let live = statuses(&get(&pool, &format!("/admin/teams/{team}/live"), hr).await);
    for (id, want) in expected {
        assert_eq!(
            live.get(&id.to_string()).map(String::as_str),
            Some(want),
            "team live {id}"
        );
    }

    // Clean up everything this test made.
    let ids: Vec<Uuid> = expected.iter().map(|(id, _)| *id).chain([hr]).collect();
    for sql in [
        "DELETE FROM user_teams WHERE user_id = ANY($1)",
        "DELETE FROM leave_requests WHERE user_id = ANY($1)",
        "DELETE FROM intervals WHERE user_id = ANY($1)",
        "DELETE FROM presence WHERE user_id = ANY($1)",
        "DELETE FROM users WHERE id = ANY($1)",
    ] {
        sqlx::query(sql).bind(&ids).execute(&pool).await.unwrap();
    }
    sqlx::query("DELETE FROM teams WHERE id = $1")
        .bind(team)
        .execute(&pool)
        .await
        .unwrap();
}

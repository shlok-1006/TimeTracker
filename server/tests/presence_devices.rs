//! One person, several devices: the live badge shows the person, not the last machine to beat.
//!
//! Working on two devices at once is allowed. Before migration 0058 every heartbeat overwrote a
//! single `presence` row, so a laptop on a break and a desktop in active use made the badge flip
//! between "On break" and "Working" every ~45 seconds. What must hold now:
//!   * any fresh device working ⇒ working, whichever device beat last;
//!   * order when devices disagree: meeting > working > idle > break > not_working;
//!   * a device not heard from within the grace window no longer counts, and none ⇒ not_logged_in;
//!   * a device that sends its id switches at once (a break on the only machine shows straight away);
//!   * older builds without an id still stop flickering;
//!   * the team roster (`/admin/team`, team members) uses the same derivation;
//!   * rows for machines silent for a day are dropped on the owner's next beat, never the beating one.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;
use uuid::Uuid;

use server::db::presence::{self, device_key};
use server::jwt::JwtKeys;
use server::linear_service::LinearService;
use server::presence::PresenceStatus::{self, Break, Idle, Meeting, NotWorking, Working};
use server::role::UserRole;
use server::storage::{S3Config, StorageClient};
use server::AppState;

const SECRET: &str = "presence-devices-test-secret";

async fn real_pool() -> Option<sqlx::PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .ok()
}

async fn seed_user(pool: &sqlx::PgPool, role: &str) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO users (id, name, email, password_hash, role) VALUES ($1,$2,$3,'x',$4::user_role)",
    )
    .bind(id)
    .bind(format!("Devices {id}"))
    .bind(format!("devices-{id}@test.local"))
    .bind(role)
    .execute(pool)
    .await
    .expect("seed user");
    id
}

async fn cleanup(pool: &sqlx::PgPool, ids: &[Uuid]) {
    for id in ids {
        sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(id)
            .execute(pool)
            .await
            .ok();
    }
}

async fn beat(pool: &sqlx::PgPool, user: Uuid, device: Option<Uuid>, status: PresenceStatus) {
    presence::heartbeat(pool, user, status, None, device)
        .await
        .expect("heartbeat");
}

async fn live(pool: &sqlx::PgPool, user: Uuid) -> String {
    presence::live_status(pool, user)
        .await
        .expect("live status")
}

/// Pretend `key` was last heard from `secs` seconds ago.
async fn age(pool: &sqlx::PgPool, user: Uuid, key: &str, secs: i64) {
    let n = sqlx::query(
        "UPDATE presence_devices SET last_seen_at = now() - make_interval(secs => $3)
         WHERE user_id = $1 AND device_key = $2",
    )
    .bind(user)
    .bind(key)
    .bind(secs as f64)
    .execute(pool)
    .await
    .expect("age device")
    .rows_affected();
    assert_eq!(n, 1, "no row for {key}");
}

async fn device_rows(pool: &sqlx::PgPool, user: Uuid) -> Vec<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT device_key FROM presence_devices WHERE user_id = $1 ORDER BY device_key",
    )
    .bind(user)
    .fetch_all(pool)
    .await
    .expect("device rows")
}

#[test]
fn device_keys() {
    let id = Uuid::parse_str("6f1c8a52-0d1e-4b7a-9c3e-2a4b5c6d7e8f").unwrap();
    assert_eq!(
        device_key(Some(id), Break),
        "device:6f1c8a520d1e4b7a9c3e2a4b5c6d7e8f"
    );
    // The id decides, not the status: one machine keeps one row as it changes.
    assert_eq!(device_key(Some(id), Working), device_key(Some(id), Break));
    assert_eq!(device_key(None, Working), "legacy:working");
    assert_eq!(device_key(None, NotWorking), "legacy:not_working");
}

#[tokio::test]
async fn break_on_one_device_working_on_another_reads_working_whichever_beats_last() {
    let Some(pool) = real_pool().await else {
        eprintln!("DATABASE_URL unset — skipping");
        return;
    };
    let u = seed_user(&pool, "employee").await;
    let (laptop, desktop) = (Some(Uuid::new_v4()), Some(Uuid::new_v4()));

    beat(&pool, u, desktop, Working).await;
    beat(&pool, u, laptop, Break).await;
    assert_eq!(live(&pool, u).await, "working", "laptop beat last");
    beat(&pool, u, desktop, Working).await;
    assert_eq!(live(&pool, u).await, "working", "desktop beat last");
    beat(&pool, u, laptop, Idle).await;
    assert_eq!(live(&pool, u).await, "working", "laptop idle now");

    cleanup(&pool, &[u]).await;
}

#[tokio::test]
async fn disagreeing_devices_resolve_in_a_fixed_order() {
    let Some(pool) = real_pool().await else {
        return;
    };
    let u = seed_user(&pool, "employee").await;
    let (a, b) = (Some(Uuid::new_v4()), Some(Uuid::new_v4()));

    for (x, y, want) in [
        (Meeting, Working, "meeting"),
        (Working, Idle, "working"),
        (Idle, Break, "idle"),
        (Break, NotWorking, "break"),
        (NotWorking, NotWorking, "not_working"),
    ] {
        beat(&pool, u, a, x).await;
        beat(&pool, u, b, y).await;
        assert_eq!(live(&pool, u).await, want, "{x} + {y}");
        // Same answer whichever device spoke last.
        beat(&pool, u, a, x).await;
        assert_eq!(live(&pool, u).await, want, "{y} + {x}");
    }

    cleanup(&pool, &[u]).await;
}

#[tokio::test]
async fn one_identified_device_switches_at_once() {
    let Some(pool) = real_pool().await else {
        return;
    };
    let u = seed_user(&pool, "employee").await;
    let only = Some(Uuid::new_v4());

    beat(&pool, u, only, Working).await;
    assert_eq!(live(&pool, u).await, "working");
    beat(&pool, u, only, Break).await;
    assert_eq!(live(&pool, u).await, "break", "no lingering 'working'");
    beat(&pool, u, only, Working).await;
    assert_eq!(live(&pool, u).await, "working");
    assert_eq!(device_rows(&pool, u).await.len(), 1);

    cleanup(&pool, &[u]).await;
}

#[tokio::test]
async fn a_silent_device_stops_counting_and_none_left_means_not_logged_in() {
    let Some(pool) = real_pool().await else {
        return;
    };
    let u = seed_user(&pool, "employee").await;
    let (laptop, desktop) = (Uuid::new_v4(), Uuid::new_v4());

    beat(&pool, u, Some(desktop), Working).await;
    beat(&pool, u, Some(laptop), Break).await;
    // The desktop was shut 2 minutes ago without signing out: past the 90-second grace.
    age(&pool, u, &device_key(Some(desktop), Working), 120).await;
    assert_eq!(live(&pool, u).await, "break");
    // Just inside the grace still counts.
    age(&pool, u, &device_key(Some(desktop), Working), 80).await;
    assert_eq!(live(&pool, u).await, "working");

    age(&pool, u, &device_key(Some(desktop), Working), 120).await;
    age(&pool, u, &device_key(Some(laptop), Break), 120).await;
    assert_eq!(live(&pool, u).await, "not_logged_in");

    // Never heard from at all.
    let nobody = seed_user(&pool, "employee").await;
    assert_eq!(live(&pool, nobody).await, "not_logged_in");

    cleanup(&pool, &[u, nobody]).await;
}

#[tokio::test]
async fn old_builds_without_an_id_stop_flickering_too() {
    let Some(pool) = real_pool().await else {
        return;
    };
    let u = seed_user(&pool, "employee").await;

    beat(&pool, u, None, Break).await;
    assert_eq!(live(&pool, u).await, "break");
    // Two old-build machines alternating, exactly what used to flip the badge.
    for _ in 0..3 {
        beat(&pool, u, None, Working).await;
        assert_eq!(live(&pool, u).await, "working");
        beat(&pool, u, None, Break).await;
        assert_eq!(
            live(&pool, u).await,
            "working",
            "the break machine beat last"
        );
    }
    assert_eq!(
        device_rows(&pool, u).await,
        vec!["legacy:break".to_string(), "legacy:working".to_string()]
    );

    // Once the working machine goes quiet past the grace, the break shows.
    age(&pool, u, "legacy:working", 120).await;
    assert_eq!(live(&pool, u).await, "break");

    cleanup(&pool, &[u]).await;
}

#[tokio::test]
async fn rows_for_machines_silent_a_day_are_dropped_but_never_the_beating_one() {
    let Some(pool) = real_pool().await else {
        return;
    };
    let u = seed_user(&pool, "employee").await;
    let (old, current) = (Uuid::new_v4(), Uuid::new_v4());

    beat(&pool, u, Some(old), Working).await;
    beat(&pool, u, Some(current), Working).await;
    beat(&pool, u, None, Idle).await;
    age(&pool, u, &device_key(Some(old), Working), 86_400 + 60).await;
    age(&pool, u, "legacy:idle", 3_600).await;
    // The beating device's own row is a day old too; it must be refreshed, not deleted.
    age(&pool, u, &device_key(Some(current), Working), 86_400 + 60).await;

    beat(&pool, u, Some(current), Break).await;
    assert_eq!(
        device_rows(&pool, u).await,
        vec![device_key(Some(current), Break), "legacy:idle".to_string()]
    );
    assert_eq!(live(&pool, u).await, "break");

    cleanup(&pool, &[u]).await;
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

fn bearer(id: Uuid, role: UserRole) -> String {
    format!(
        "Bearer {}",
        JwtKeys::new(SECRET, 900)
            .issue(id, role, None, None)
            .unwrap()
    )
}

async fn post_beat(pool: &sqlx::PgPool, user: Uuid, body: serde_json::Value) -> StatusCode {
    let req = Request::builder()
        .method("POST")
        .uri("/presence")
        .header("content-type", "application/json")
        .header("Authorization", bearer(user, UserRole::Employee))
        .body(Body::from(body.to_string()))
        .unwrap();
    app(pool.clone()).oneshot(req).await.unwrap().status()
}

#[tokio::test]
async fn the_heartbeat_route_and_both_rosters_show_the_person() {
    let Some(pool) = real_pool().await else {
        return;
    };
    let u = seed_user(&pool, "employee").await;
    let hr = seed_user(&pool, "hr").await;
    let laptop = Uuid::new_v4();

    // Today's desktop build: no device_id. A new build: with one.
    assert_eq!(
        post_beat(&pool, u, serde_json::json!({ "status": "working" })).await,
        StatusCode::OK
    );
    assert_eq!(
        post_beat(
            &pool,
            u,
            serde_json::json!({ "status": "break", "device_id": laptop })
        )
        .await,
        StatusCode::OK
    );
    // A malformed id is rejected rather than silently becoming "no id".
    assert_eq!(
        post_beat(
            &pool,
            u,
            serde_json::json!({ "status": "break", "device_id": "not-a-uuid" })
        )
        .await,
        StatusCode::UNPROCESSABLE_ENTITY
    );

    let req = Request::builder()
        .uri("/admin/team")
        .header("Authorization", bearer(hr, UserRole::Hr))
        .body(Body::empty())
        .unwrap();
    let res = app(pool.clone()).oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let rows: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let me = rows
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["user"]["id"] == serde_json::json!(u))
        .expect("listed");
    assert_eq!(me["status"], "working", "the laptop's break beat last");
    assert!(me["last_seen_at"].is_string());

    let team = Uuid::new_v4();
    sqlx::query("INSERT INTO teams (id, name) VALUES ($1, $2)")
        .bind(team)
        .bind(format!("Devices team {team}"))
        .execute(&pool)
        .await
        .expect("seed team");
    sqlx::query("INSERT INTO user_teams (user_id, team_id) VALUES ($1, $2)")
        .bind(u)
        .bind(team)
        .execute(&pool)
        .await
        .expect("seed membership");
    let members = presence::team_members(&pool, team).await.expect("members");
    assert_eq!(members.len(), 1);
    assert_eq!(members[0].status, "working");

    sqlx::query("DELETE FROM teams WHERE id = $1")
        .bind(team)
        .execute(&pool)
        .await
        .ok();
    cleanup(&pool, &[u, hr]).await;
}

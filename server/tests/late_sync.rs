//! Data that reaches the server days late — a laptop on a VPN that blocked the
//! app syncs once the VPN drops. Hits a live DB via DATABASE_URL; skips if unset.
//!
//! Uses year-2020 dates for a fresh user so it never collides with real data.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::{Datelike, Duration, NaiveDate, TimeZone, Utc, Weekday};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use server::attendance_service;
use server::db::{screenshots, users};
use server::jwt::JwtKeys;
use server::linear_service::LinearService;
use server::role::UserRole;
use server::storage::{S3Config, StorageClient};
use server::AppState;

const SECRET: &str = "late-sync-test-secret";

async fn pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .ok()
}

fn app(pool: PgPool) -> axum::Router {
    server::build_router(AppState::new(
        pool,
        JwtKeys::new(SECRET, 900),
        StorageClient::new(S3Config::insecure_local()),
        LinearService::from_env(),
        server::claude_provider::ClaudeProvider::from_env(),
        2_592_000,
    ))
}

async fn employee(pool: &PgPool, label: &str) -> Uuid {
    let emp = users::create(
        pool,
        "Late Sync",
        &format!("late-{label}-{}@t.local", Uuid::new_v4()),
        "h",
        UserRole::Employee,
        None,
    )
    .await
    .unwrap();
    sqlx::query("UPDATE users SET created_at = '2020-01-01T00:00:00Z' WHERE id = $1")
        .bind(emp.id)
        .execute(pool)
        .await
        .unwrap();
    emp.id
}

fn monday_2020() -> NaiveDate {
    let mut day = NaiveDate::from_ymd_opt(2020, 6, 1).unwrap();
    while day.weekday() != Weekday::Mon {
        day += Duration::days(1);
    }
    day
}

#[tokio::test]
async fn intervals_synced_after_the_rollup_fix_that_days_attendance() {
    let Some(pool) = pool().await else {
        eprintln!("skipping late-sync test: DATABASE_URL not set");
        return;
    };
    let emp = employee(&pool, "att").await;
    let monday = monday_2020();

    // The nightly rollup ran while the laptop couldn't reach us: absent.
    let before = attendance_service::rollup_day(&pool, emp, monday)
        .await
        .unwrap();
    assert_eq!(before.status, "absent");

    // The VPN drops; the desktop syncs the day's 5 hours.
    let start = Utc.from_utc_datetime(&monday.and_hms_opt(4, 0, 0).unwrap());
    let body = serde_json::json!([{
        "id": Uuid::new_v4(),
        "start_utc": start,
        "end_utc": start + Duration::hours(5),
        "kind": "active",
    }]);
    let token = JwtKeys::new(SECRET, 900)
        .issue(emp, UserRole::Employee, None, None)
        .unwrap();
    let resp = app(pool.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/intervals")
                .header("content-type", "application/json")
                .header("Authorization", format!("Bearer {token}"))
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let (status, worked): (String, i64) = sqlx::query_as(
        "SELECT status, worked_seconds::bigint FROM attendance_days WHERE user_id = $1 AND day = $2",
    )
    .bind(emp)
    .bind(monday)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(status, "present", "the late sync re-derives the day");
    assert_eq!(worked, 5 * 3600);

    let received: Option<chrono::DateTime<Utc>> =
        sqlx::query_scalar("SELECT received_at FROM intervals WHERE user_id = $1")
            .bind(emp)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        received.is_some(),
        "new intervals are stamped with received_at"
    );

    users::delete(&pool, emp).await.unwrap();
}

#[tokio::test]
async fn late_intervals_leave_an_hr_override_alone() {
    let Some(pool) = pool().await else {
        eprintln!("skipping late-sync override test: DATABASE_URL not set");
        return;
    };
    let emp = employee(&pool, "ovr").await;
    let monday = monday_2020() + Duration::days(7);
    attendance_service::override_day(&pool, emp, monday, "leave", "approved offline", emp)
        .await
        .unwrap();

    let start = Utc.from_utc_datetime(&monday.and_hms_opt(4, 0, 0).unwrap());
    server::db::intervals::insert_batch(
        &pool,
        emp,
        &[server::db::intervals::IntervalDto {
            id: Uuid::new_v4(),
            start_utc: start,
            end_utc: start + Duration::hours(5),
            kind: "active".into(),
            team_id: None,
        }],
    )
    .await
    .unwrap();
    attendance_service::refresh_days(&pool, emp, &[monday])
        .await
        .unwrap();

    let status: String =
        sqlx::query_scalar("SELECT status FROM attendance_days WHERE user_id = $1 AND day = $2")
            .bind(emp)
            .bind(monday)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(status, "leave", "an HR override survives a late sync");

    users::delete(&pool, emp).await.unwrap();
}

#[tokio::test]
async fn retried_screenshot_metadata_returns_the_same_row() {
    let Some(pool) = pool().await else {
        eprintln!("skipping screenshot idempotency test: DATABASE_URL not set");
        return;
    };
    let emp = employee(&pool, "shot").await;
    let key = format!("{emp}/20200601/{}.jpg", Uuid::new_v4());
    let taken = Utc.with_ymd_and_hms(2020, 6, 1, 6, 0, 0).unwrap();

    let first = screenshots::insert(&pool, emp, &key, taken, None, "working")
        .await
        .unwrap();
    // The response was lost on a flaky VPN; the desktop sends it again.
    let again = screenshots::insert(&pool, emp, &key, taken, None, "working")
        .await
        .unwrap();
    assert_eq!(first, again);
    assert_eq!(screenshots::count_for_user(&pool, emp).await.unwrap(), 1);

    users::delete(&pool, emp).await.unwrap();
}

#[tokio::test]
async fn days_reviewed_before_their_screenshots_arrived_are_found() {
    let Some(pool) = pool().await else {
        eprintln!("skipping late-screenshot selection test: DATABASE_URL not set");
        return;
    };
    let emp = employee(&pool, "review").await;
    let day = monday_2020() + Duration::days(14);
    let (from, _) = server::org_time::day_bounds_utc(day);

    // The nightly review ran with nothing to look at: a job, no samples.
    let job = server::sampler::create_daily_job(&pool, emp, day)
        .await
        .unwrap();
    // Then the queued screenshot arrives.
    let shot = screenshots::insert(
        &pool,
        emp,
        &format!("{emp}/20200615/{}.jpg", Uuid::new_v4()),
        from + Duration::hours(5),
        None,
        "working",
    )
    .await
    .unwrap();
    let found = screenshots::users_with_unsampled_working_shots(&pool, day)
        .await
        .unwrap();
    assert!(
        found.contains(&emp),
        "a day reviewed empty is picked up again"
    );

    // Once the day has a sampled set it is never picked again.
    sqlx::query(
        "INSERT INTO analysis_job_samples (job_id, screenshot_id, bucket) VALUES ($1, $2, 'slot_01')",
    )
    .bind(job.id)
    .bind(shot)
    .execute(&pool)
    .await
    .unwrap();
    let found = screenshots::users_with_unsampled_working_shots(&pool, day)
        .await
        .unwrap();
    assert!(!found.contains(&emp));

    users::delete(&pool, emp).await.unwrap();
}

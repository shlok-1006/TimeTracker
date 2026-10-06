//! Adversarial cases for "holidays are never a work day": the edges the happy-path test
//! (holiday_work.rs) doesn't reach — HR overrides, leave on a holiday, a holiday on a weekend, the
//! tracker heartbeat on a holiday, and a holiday HR enters AFTER the days were already rolled up.
//! Hits a live DB via DATABASE_URL; skips if unset. Each test uses its own 2018 week and fresh users.

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use chrono::{Duration, NaiveDate, TimeZone, Utc};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use server::attendance_service;
use server::db::intervals::IntervalDto;
use server::db::{attendance, intervals, leave, users, weekly_hours};
use server::jwt::JwtKeys;
use server::linear_service::LinearService;
use server::role::UserRole;
use server::storage::{S3Config, StorageClient};
use server::weekly_hours_service;
use server::AppState;

const SECRET: &str = "holiday-adversarial-secret";

async fn pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .ok()
}

fn state(pool: PgPool) -> AppState {
    AppState::new(
        pool,
        JwtKeys::new(SECRET, 900),
        StorageClient::new(S3Config::insecure_local()),
        LinearService::from_env(),
        server::claude_provider::ClaudeProvider::from_env(),
        2_592_000,
    )
}

async fn employee(pool: &PgPool, tag: &str) -> Uuid {
    let u = users::create(
        pool,
        tag,
        &format!("{tag}-{}@t.local", Uuid::new_v4()),
        "h",
        UserRole::Employee,
        None,
    )
    .await
    .unwrap();
    sqlx::query("UPDATE users SET created_at = '2018-01-01T00:00:00Z' WHERE id = $1")
        .bind(u.id)
        .execute(pool)
        .await
        .unwrap();
    u.id
}

async fn track(pool: &PgPool, user: Uuid, day: NaiveDate, hours: i64) {
    let start = Utc.from_utc_datetime(&day.and_hms_opt(9, 0, 0).unwrap());
    intervals::insert_batch(
        pool,
        user,
        &[IntervalDto {
            id: Uuid::new_v4(),
            start_utc: start,
            end_utc: start + Duration::hours(hours),
            kind: "active".into(),
            team_id: None,
        }],
    )
    .await
    .unwrap();
}

/// Remove a holiday this file created (an earlier run that panicked may have left one behind).
/// Only `Adv …` names: a real holiday on a shared DB is never touched.
async fn drop_holiday(pool: &PgPool, day: NaiveDate) {
    sqlx::query("DELETE FROM holidays WHERE day = $1 AND name LIKE 'Adv %'")
        .bind(day)
        .execute(pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn hr_override_on_a_holiday_is_left_alone() {
    let Some(pool) = pool().await else { return };
    let (emp, hr) = (
        employee(&pool, "ovr").await,
        employee(&pool, "ovr-hr").await,
    );
    let day = NaiveDate::from_ymd_opt(2018, 3, 9).unwrap(); // Friday
    drop_holiday(&pool, day).await;
    leave::create_holiday(&pool, day, "Adv Override Day")
        .await
        .unwrap();
    track(&pool, emp, day, 6).await;
    attendance_service::override_day(&pool, emp, day, "present", "came in for a release", hr)
        .await
        .unwrap();
    let after = attendance_service::rollup_day(&pool, emp, day)
        .await
        .unwrap();
    assert_eq!(
        (after.status.as_str(), after.is_override),
        ("present", true),
        "an HR override must survive the holiday rule"
    );
    users::delete(&pool, emp).await.unwrap();
    users::delete(&pool, hr).await.unwrap();
    drop_holiday(&pool, day).await;
}

#[tokio::test]
async fn leave_and_weekend_on_a_holiday() {
    let Some(pool) = pool().await else { return };
    let (worked, rested) = (employee(&pool, "lv-w").await, employee(&pool, "lv-r").await);
    let friday = NaiveDate::from_ymd_opt(2018, 4, 13).unwrap();
    let saturday = friday + Duration::days(1);
    drop_holiday(&pool, friday).await;
    drop_holiday(&pool, saturday).await;
    leave::create_holiday(&pool, friday, "Adv Fri Holiday")
        .await
        .unwrap();
    leave::create_holiday(&pool, saturday, "Adv Sat Holiday")
        .await
        .unwrap();
    let lt = leave::create_type(
        &pool,
        &format!("Adv-{}", Uuid::new_v4()),
        true,
        20.0,
        0.0,
        0.0,
        &leave::TypeRules::default(),
    )
    .await
    .unwrap();
    // Approved leave inserted directly (as monthly_attendance.rs does): this test is about how
    // attendance READS a leave day, not about booking one — and it keeps the test independent of
    // `create_request`'s signature, which the half-day-leave PR extends.
    for u in [worked, rested] {
        sqlx::query(
            "INSERT INTO leave_requests (user_id, leave_type_id, start_date, end_date, days, status)
             VALUES ($1, $2, $3, $3, 1, 'approved')",
        )
        .bind(u)
        .bind(lt.id)
        .bind(friday)
        .execute(&pool)
        .await
        .unwrap();
    }
    // Leave + holiday + worked anyway → a holiday (the work happened; it's still not a work day).
    track(&pool, worked, friday, 3).await;
    let w = attendance_service::rollup_day(&pool, worked, friday)
        .await
        .unwrap();
    assert_eq!(w.status, "holiday");
    // Leave + holiday, nothing tracked → leave explains the day first, as before.
    let r = attendance_service::rollup_day(&pool, rested, friday)
        .await
        .unwrap();
    assert_eq!(r.status, "leave");
    // A holiday on a Saturday, worked → holiday, and never a required day either way.
    track(&pool, worked, saturday, 2).await;
    let s = attendance_service::rollup_day(&pool, worked, saturday)
        .await
        .unwrap();
    assert_eq!(s.status, "holiday");

    for u in [worked, rested] {
        users::delete(&pool, u).await.unwrap();
    }
    sqlx::query("DELETE FROM leave_types WHERE id = $1")
        .bind(lt.id)
        .execute(&pool)
        .await
        .unwrap();
    drop_holiday(&pool, friday).await;
    drop_holiday(&pool, saturday).await;
}

#[tokio::test]
async fn heartbeat_does_not_mark_present_on_a_holiday() {
    let Some(pool) = pool().await else { return };
    let today = Utc::now().date_naive();
    // Never clobber a real holiday on a shared DB: only run when today is free.
    if leave::holiday_name_on_day(&pool, today)
        .await
        .unwrap()
        .is_some()
    {
        eprintln!("skipping: today is already a holiday in this DB");
        return;
    }
    let emp = employee(&pool, "hb").await;
    leave::create_holiday(&pool, today, "Adv Today Holiday")
        .await
        .unwrap();
    track(&pool, emp, today, 1).await;
    attendance_service::mark_present_today(&pool, emp)
        .await
        .unwrap();
    let row = attendance::get(&pool, emp, today).await.unwrap();
    assert!(
        row.map_or(true, |r| r.status != "present"),
        "the tracker heartbeat must not mark a holiday present"
    );
    users::delete(&pool, emp).await.unwrap();
    drop_holiday(&pool, today).await;
}

/// HR usually enters a holiday late — after the day (and often the week's report) was computed.
/// Creating it must re-derive that day for everyone and refresh an already-computed week.
#[tokio::test]
async fn a_holiday_entered_late_fixes_the_day_and_the_week() {
    let Some(pool) = pool().await else { return };
    let emp = employee(&pool, "late").await;
    let monday = NaiveDate::from_ymd_opt(2018, 5, 7).unwrap();
    let sunday = monday + Duration::days(6);
    let friday = monday + Duration::days(4);
    drop_holiday(&pool, friday).await;
    for i in 0..5 {
        track(&pool, emp, monday + Duration::days(i), 8).await;
    }
    let st = state(pool.clone());
    weekly_hours_service::run_for_week(&st, monday, sunday)
        .await
        .unwrap();
    let before = attendance::get(&pool, emp, friday).await.unwrap().unwrap();
    assert_eq!(
        before.status, "present",
        "precondition: Friday was a work day"
    );

    // HR adds Friday as a holiday afterwards, through the real API.
    let hr = users::create(
        &pool,
        "Late HR",
        &format!("late-hr-{}@t.local", Uuid::new_v4()),
        "h",
        UserRole::Hr,
        None,
    )
    .await
    .unwrap();
    let tok = JwtKeys::new(SECRET, 900)
        .issue(hr.id, UserRole::Hr, None, None)
        .unwrap();
    let resp = server::build_router(state(pool.clone()))
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/holidays")
                .header("authorization", format!("Bearer {tok}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "day": friday, "name": "Adv Late Holiday" }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let _ = to_bytes(resp.into_body(), usize::MAX).await;
    assert_eq!(status, StatusCode::OK);

    // The refresh runs in the background; give it a few seconds.
    let mut fixed = None;
    for _ in 0..50 {
        let day = attendance::get(&pool, emp, friday).await.unwrap().unwrap();
        let week = weekly_hours::find_shortfall(&pool, emp, monday, None)
            .await
            .unwrap();
        let wd = sqlx::query_scalar::<_, i32>(
            "SELECT working_days FROM weekly_hours_reports WHERE user_id = $1 AND week_start = $2",
        )
        .bind(emp)
        .bind(monday)
        .fetch_one(&pool)
        .await
        .unwrap();
        if day.status == "holiday" && wd == 4 {
            fixed = Some((day, week, wd));
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let (day, week, wd) = fixed.expect("late holiday must re-derive the day and refresh the week");
    assert_eq!(day.note, "Adv Late Holiday");
    assert_eq!(wd, 4);
    assert!(
        week.is_none(),
        "40h worked against 32h required → not a shortfall"
    );

    users::delete(&pool, emp).await.unwrap();
    users::delete(&pool, hr.id).await.unwrap();
    drop_holiday(&pool, friday).await;
}

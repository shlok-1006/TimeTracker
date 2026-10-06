//! Holidays are never a work day — the same rule weekends already follow. An employee who tracks
//! time on a company holiday keeps a `holiday` day (not `present`), so the Weekly Report doesn't add
//! 8h to that week's requirement; the hours they did work still count toward the week, exactly as
//! weekend hours do. Hits a live DB via DATABASE_URL; skips if unset. Uses a 2019 week and a fresh
//! user so it never collides with real data.

use chrono::{Duration, NaiveDate, TimeZone, Utc};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use uuid::Uuid;

use server::attendance_service;
use server::db::intervals::IntervalDto;
use server::db::{intervals, leave, users, weekly_hours};
use server::role::UserRole;
use server::weekly_hours_service;

async fn pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .ok()
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

#[tokio::test]
async fn working_on_a_holiday_does_not_make_it_a_work_day() {
    let Some(pool) = pool().await else {
        eprintln!("skipping holiday-work test: DATABASE_URL not set");
        return;
    };
    let emp = users::create(
        &pool,
        "Holiday Worker",
        &format!("holiday-{}@t.local", Uuid::new_v4()),
        "h",
        UserRole::Employee,
        None,
    )
    .await
    .unwrap();
    sqlx::query("UPDATE users SET created_at = '2019-01-01T00:00:00Z' WHERE id = $1")
        .bind(emp.id)
        .execute(&pool)
        .await
        .unwrap();

    // Mon 2019-07-08 .. Sun 2019-07-14; Friday 2019-07-12 is a company holiday.
    let monday = NaiveDate::from_ymd_opt(2019, 7, 8).unwrap();
    let friday = monday + Duration::days(4);
    let saturday = monday + Duration::days(5);
    let sunday = monday + Duration::days(6);
    leave::create_holiday(&pool, friday, "Test Founders' Day")
        .await
        .unwrap();

    for i in 0..4 {
        track(&pool, emp.id, monday + Duration::days(i), 8).await; // Mon–Thu: 8h each
    }
    track(&pool, emp.id, friday, 5).await; // worked 5h on the holiday
    track(&pool, emp.id, saturday, 2).await; // and 2h on Saturday

    let mut day = monday;
    while day <= sunday {
        attendance_service::rollup_day(&pool, emp.id, day)
            .await
            .unwrap();
        day += Duration::days(1);
    }

    // The holiday stays a holiday despite the 5h, and keeps the hours — just like Saturday.
    let fri = attendance_service::rollup_day(&pool, emp.id, friday)
        .await
        .unwrap();
    assert_eq!(
        fri.status, "holiday",
        "worked holiday must not become present"
    );
    assert_eq!(fri.note, "Test Founders' Day");
    assert_eq!(
        fri.worked_seconds,
        5 * 3600,
        "the hours themselves are kept"
    );
    let sat = attendance_service::rollup_day(&pool, emp.id, saturday)
        .await
        .unwrap();
    assert_eq!(sat.status, "weekend");

    // Weekly Report: 4 working days (Mon–Thu) ⇒ 32h required, not 40h; all 39h worked count.
    let week = weekly_hours::week_activity(&pool, monday, sunday)
        .await
        .unwrap()
        .into_iter()
        .find(|w| w.user_id == emp.id)
        .unwrap();
    assert_eq!(week.working_days, 4, "the holiday is not a required day");
    assert_eq!(week.worked_seconds, (32 + 5 + 2) * 3600);
    let h = weekly_hours_service::evaluate(week.working_days, week.worked_seconds);
    assert_eq!(h.required_seconds, 32 * 3600);
    assert!(h.compliant);

    users::delete(&pool, emp.id).await.unwrap();
    sqlx::query("DELETE FROM holidays WHERE day = $1")
        .bind(friday)
        .execute(&pool)
        .await
        .unwrap();
}

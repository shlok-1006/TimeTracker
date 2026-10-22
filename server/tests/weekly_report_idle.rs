//! The Weekly Report counts all tracked time toward the 40h: active + meeting + idle (HR policy,
//! 19 Oct 2026). Breaks are not tracked time and stay out. Attendance (present/absent) still counts
//! active + meeting only, so the day rows keep idle in their own column. Hits a live DB via
//! DATABASE_URL; skips if unset. Uses a 2019 week and a fresh user so it never collides with real data.

use chrono::{Duration, NaiveDate, TimeZone, Utc};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use uuid::Uuid;

use server::attendance_service;
use server::db::intervals::IntervalDto;
use server::db::{intervals, users, weekly_hours};
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

/// One day back to back from 09:00 UTC: each (kind, minutes) follows the previous one.
async fn day_of(pool: &PgPool, user: Uuid, day: NaiveDate, parts: &[(&str, i64)]) {
    let mut at = Utc.from_utc_datetime(&day.and_hms_opt(9, 0, 0).unwrap());
    let mut batch = Vec::new();
    for (kind, minutes) in parts {
        let end = at + Duration::minutes(*minutes);
        batch.push(IntervalDto {
            id: Uuid::new_v4(),
            start_utc: at,
            end_utc: end,
            kind: (*kind).into(),
            team_id: None,
        });
        at = end;
    }
    intervals::insert_batch(pool, user, &batch).await.unwrap();
}

#[tokio::test]
async fn idle_time_counts_toward_the_weekly_hours() {
    let Some(pool) = pool().await else {
        eprintln!("skipping weekly-report idle test: DATABASE_URL not set");
        return;
    };
    let emp = users::create(
        &pool,
        "Idle Counter",
        &format!("weekly-idle-{}@t.local", Uuid::new_v4()),
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

    // Mon 2019-08-05 .. Sun 2019-08-11. Each weekday: 5h active, 1h meeting, 2h idle, 1h break.
    let monday = NaiveDate::from_ymd_opt(2019, 8, 5).unwrap();
    let sunday = monday + Duration::days(6);
    for i in 0..5 {
        let day = monday + Duration::days(i);
        day_of(
            &pool,
            emp.id,
            day,
            &[
                ("active", 180),
                ("idle", 60),
                ("meeting", 60),
                ("break", 60),
                ("active", 120),
                ("idle", 60),
            ],
        )
        .await;
        let d = attendance_service::rollup_day(&pool, emp.id, day)
            .await
            .unwrap();
        assert_eq!(d.worked_seconds, 6 * 3600, "attendance: active + meeting");
        assert_eq!(d.idle_seconds, 2 * 3600, "idle kept in its own column");
    }

    // Weekly Report: 5 × (5h + 1h + 2h) = 40h; the 5h of breaks never count.
    let week = weekly_hours::week_activity(&pool, monday, sunday)
        .await
        .unwrap()
        .into_iter()
        .find(|w| w.user_id == emp.id)
        .unwrap();
    assert_eq!(week.working_days, 5);
    assert_eq!(
        week.worked_seconds,
        40 * 3600,
        "active + meeting + idle, no breaks"
    );
    let h = weekly_hours_service::evaluate(week.working_days, week.worked_seconds);
    assert!(h.compliant, "40h of tracked time meets 40h");
    assert_eq!(h.shortfall_seconds, 0);

    users::delete(&pool, emp.id).await.unwrap();
}

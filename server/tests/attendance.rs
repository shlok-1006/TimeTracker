//! Attendance rollup round-trip (Feature 6C). Derives status from real
//! intervals + leave + holidays. Hits a live DB via DATABASE_URL; skips if unset.
//!
//! Uses year-2020 dates (past, so `ensure_range` fills them) for a fresh user,
//! so it never collides with seeded/real data.

use chrono::{Datelike, Duration, NaiveDate, TimeZone, Utc, Weekday};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use uuid::Uuid;

use server::attendance_service;
use server::db::{attendance, intervals, leave, users};
use server::db::intervals::IntervalDto;
use server::role::UserRole;

async fn pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    PgPoolOptions::new().max_connections(2).connect(&url).await.ok()
}

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

/// First date on/after `base` with the given weekday.
fn next_weekday(base: NaiveDate, wd: Weekday) -> NaiveDate {
    let mut day = base;
    while day.weekday() != wd {
        day += Duration::days(1);
    }
    day
}

#[tokio::test]
async fn attendance_rollup_derives_all_statuses() {
    let Some(pool) = pool().await else {
        eprintln!("skipping attendance test: DATABASE_URL not set");
        return;
    };
    let tag = Uuid::new_v4();
    let emp = users::create(
        &pool, "Att Emp", &format!("att-{tag}@t.local"), "h", UserRole::Employee, None,
    ).await.unwrap();

    // Pick a clear weekday (Mon) and weekend (Sat) in 2020 (past → fillable).
    let monday = next_weekday(d(2020, 3, 2), Weekday::Mon);
    let tuesday = monday + Duration::days(1);
    let wednesday = monday + Duration::days(2);
    let thursday = monday + Duration::days(3);
    let saturday = next_weekday(monday, Weekday::Sat);

    // PRESENT: a 7h worked interval on Monday (>= 6h threshold).
    let start = Utc.from_utc_datetime(&monday.and_hms_opt(9, 0, 0).unwrap());
    intervals::insert_batch(
        &pool,
        emp.id,
        &[IntervalDto {
            id: Uuid::new_v4(),
            start_utc: start,
            end_utc: start + Duration::hours(7),
            kind: "active".into(),
            team_id: None,
        }],
    )
    .await
    .unwrap();
    let present = attendance_service::rollup_day(&pool, emp.id, monday).await.unwrap();
    assert_eq!(present.status, "present");
    assert_eq!(present.worked_seconds, 7 * 3600);
    assert!(present.first_in_utc.is_some() && present.last_out_utc.is_some());

    // LEAVE: an approved leave request covering Tuesday.
    let lt = leave::create_type(&pool, &format!("Annual-{tag}"), true, 20.0).await.unwrap();
    let req = leave::create_request(&pool, emp.id, lt.id, tuesday, tuesday, 1.0, "vacation")
        .await
        .unwrap();
    assert!(leave::decide(&pool, req, "approved", emp.id).await.unwrap());
    let on_leave = attendance_service::rollup_day(&pool, emp.id, tuesday).await.unwrap();
    assert_eq!(on_leave.status, "leave");
    assert_eq!(on_leave.note, lt.name);

    // HOLIDAY: a company holiday on Wednesday, no work.
    leave::create_holiday(&pool, wednesday, "Test Holiday").await.unwrap();
    let holiday = attendance_service::rollup_day(&pool, emp.id, wednesday).await.unwrap();
    assert_eq!(holiday.status, "holiday");
    assert_eq!(holiday.note, "Test Holiday");

    // ABSENT: a plain weekday with no work / leave / holiday.
    let absent = attendance_service::rollup_day(&pool, emp.id, thursday).await.unwrap();
    assert_eq!(absent.status, "absent");
    assert_eq!(absent.worked_seconds, 0);

    // WEEKEND: Saturday, no work.
    let weekend = attendance_service::rollup_day(&pool, emp.id, saturday).await.unwrap();
    assert_eq!(weekend.status, "weekend");

    // ensure_range fills the whole span and the report counts line up.
    attendance_service::ensure_range(&pool, emp.id, monday, saturday).await.unwrap();
    let rows = attendance::list_range(&pool, emp.id, monday, saturday).await.unwrap();
    assert_eq!(rows.len(), 6); // Mon..Sat inclusive

    let report = attendance::report(&pool, monday, saturday, None).await.unwrap();
    let mine = report.into_iter().find(|r| r.user_id == emp.id).unwrap();
    assert_eq!(mine.present, 1);
    assert_eq!(mine.leave, 1);
    assert_eq!(mine.holiday, 1);
    assert_eq!(mine.weekend, 1);
    assert!(mine.absent >= 2); // Thursday + Friday (Friday filled by ensure_range)
    assert_eq!(mine.worked_seconds, 7 * 3600);

    // Cleanup (attendance + intervals + leave_requests cascade on user delete).
    users::delete(&pool, emp.id).await.unwrap();
    sqlx::query("DELETE FROM holidays WHERE day = $1").bind(wednesday).execute(&pool).await.unwrap();
    sqlx::query("DELETE FROM leave_types WHERE id = $1").bind(lt.id).execute(&pool).await.unwrap();
}

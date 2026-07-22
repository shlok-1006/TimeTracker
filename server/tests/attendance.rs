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
use server::db::intervals::IntervalDto;
use server::db::{attendance, intervals, leave, users};
use server::role::UserRole;

async fn pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .ok()
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
        &pool,
        "Att Emp",
        &format!("att-{tag}@t.local"),
        "h",
        UserRole::Employee,
        None,
    )
    .await
    .unwrap();
    // Backdate the account: attendance never predates `users.created_at`
    // (ensure_range clamps to it), and this test rolls up year-2020 days.
    sqlx::query!(
        "UPDATE users SET created_at = '2020-01-01T00:00:00Z' WHERE id = $1",
        emp.id
    )
    .execute(&pool)
    .await
    .unwrap();

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
    let present = attendance_service::rollup_day(&pool, emp.id, monday)
        .await
        .unwrap();
    assert_eq!(present.status, "present");
    assert_eq!(present.worked_seconds, 7 * 3600);
    assert!(present.first_in_utc.is_some() && present.last_out_utc.is_some());

    // LEAVE: an approved leave request covering Tuesday.
    let lt = leave::create_type(&pool, &format!("Annual-{tag}"), true, 20.0, 0.0, 0.0)
        .await
        .unwrap();
    let req = leave::create_request(&pool, emp.id, lt.id, tuesday, tuesday, 1.0, "vacation")
        .await
        .unwrap();
    assert!(leave::decide(&pool, req, "approved", emp.id).await.unwrap());
    let on_leave = attendance_service::rollup_day(&pool, emp.id, tuesday)
        .await
        .unwrap();
    assert_eq!(on_leave.status, "leave");
    assert_eq!(on_leave.note, lt.name);

    // HOLIDAY: a company holiday on Wednesday, no work.
    leave::create_holiday(&pool, wednesday, "Test Holiday")
        .await
        .unwrap();
    let holiday = attendance_service::rollup_day(&pool, emp.id, wednesday)
        .await
        .unwrap();
    assert_eq!(holiday.status, "holiday");
    assert_eq!(holiday.note, "Test Holiday");

    // ABSENT: a plain weekday with no work / leave / holiday.
    let absent = attendance_service::rollup_day(&pool, emp.id, thursday)
        .await
        .unwrap();
    assert_eq!(absent.status, "absent");
    assert_eq!(absent.worked_seconds, 0);

    // WEEKEND: Saturday, no work.
    let weekend = attendance_service::rollup_day(&pool, emp.id, saturday)
        .await
        .unwrap();
    assert_eq!(weekend.status, "weekend");

    // ensure_range fills the whole span and the report counts line up.
    attendance_service::ensure_range(&pool, emp.id, monday, saturday)
        .await
        .unwrap();
    let rows = attendance::list_range(&pool, emp.id, monday, saturday)
        .await
        .unwrap();
    assert_eq!(rows.len(), 6); // Mon..Sat inclusive

    let report = attendance::report(&pool, monday, saturday, None)
        .await
        .unwrap();
    let mine = report.into_iter().find(|r| r.user_id == emp.id).unwrap();
    assert_eq!(mine.present, 1);
    assert_eq!(mine.leave, 1);
    assert_eq!(mine.holiday, 1);
    assert_eq!(mine.weekend, 1);
    assert!(mine.absent >= 2); // Thursday + Friday (Friday filled by ensure_range)
    assert_eq!(mine.worked_seconds, 7 * 3600);

    // Cleanup (attendance + intervals + leave_requests cascade on user delete).
    users::delete(&pool, emp.id).await.unwrap();
    sqlx::query("DELETE FROM holidays WHERE day = $1")
        .bind(wednesday)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM leave_types WHERE id = $1")
        .bind(lt.id)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn attendance_override_survives_rollup_and_reverts() {
    let Some(pool) = pool().await else {
        eprintln!("skipping attendance override test: DATABASE_URL not set");
        return;
    };
    let tag = Uuid::new_v4();
    let emp = users::create(
        &pool,
        "Att Ovr",
        &format!("att-ovr-{tag}@t.local"),
        "h",
        UserRole::Employee,
        None,
    )
    .await
    .unwrap();
    sqlx::query!(
        "UPDATE users SET created_at = '2020-01-01T00:00:00Z' WHERE id = $1",
        emp.id
    )
    .execute(&pool)
    .await
    .unwrap();

    // A plain weekday with no work derives "absent".
    let monday = next_weekday(d(2020, 4, 6), Weekday::Mon);
    let derived = attendance_service::rollup_day(&pool, emp.id, monday)
        .await
        .unwrap();
    assert_eq!(derived.status, "absent");
    assert!(!derived.is_override);

    // HR overrides it to "leave".
    let ovr =
        attendance_service::override_day(&pool, emp.id, monday, "leave", "approved offline", emp.id)
            .await
            .unwrap();
    assert_eq!(ovr.status, "leave");
    assert_eq!(ovr.note, "approved offline");
    assert!(ovr.is_override);

    // The rollup (nightly job / recompute-today path) must NOT clobber it.
    let after = attendance_service::rollup_day(&pool, emp.id, monday)
        .await
        .unwrap();
    assert_eq!(after.status, "leave", "override must survive the rollup");
    assert!(after.is_override);

    // Clearing the override reverts to the derived status.
    let reverted = attendance_service::clear_override(&pool, emp.id, monday)
        .await
        .unwrap();
    assert_eq!(reverted.status, "absent", "revert recomputes derived status");
    assert!(!reverted.is_override);

    users::delete(&pool, emp.id).await.unwrap();
}

#[tokio::test]
async fn completed_day_under_four_hours_is_partial() {
    let Some(pool) = pool().await else {
        eprintln!("skipping partial-day test: DATABASE_URL not set");
        return;
    };
    let tag = Uuid::new_v4();
    let emp = users::create(
        &pool,
        "Att Partial",
        &format!("att-partial-{tag}@t.local"),
        "h",
        UserRole::Employee,
        None,
    )
    .await
    .unwrap();
    sqlx::query!(
        "UPDATE users SET created_at = '2020-01-01T00:00:00Z' WHERE id = $1",
        emp.id
    )
    .execute(&pool)
    .await
    .unwrap();

    let short_day = next_weekday(d(2020, 7, 6), Weekday::Mon);
    let full_day = short_day + Duration::days(1);

    // 2h of work on a completed weekday → partial (under the 4h threshold).
    let s1 = Utc.from_utc_datetime(&short_day.and_hms_opt(9, 0, 0).unwrap());
    intervals::insert_batch(
        &pool,
        emp.id,
        &[IntervalDto {
            id: Uuid::new_v4(),
            start_utc: s1,
            end_utc: s1 + Duration::hours(2),
            kind: "active".into(),
            team_id: None,
        }],
    )
    .await
    .unwrap();
    let partial = attendance_service::rollup_day(&pool, emp.id, short_day)
        .await
        .unwrap();
    assert_eq!(partial.status, "partial");

    // 5h of work → full present day.
    let s2 = Utc.from_utc_datetime(&full_day.and_hms_opt(9, 0, 0).unwrap());
    intervals::insert_batch(
        &pool,
        emp.id,
        &[IntervalDto {
            id: Uuid::new_v4(),
            start_utc: s2,
            end_utc: s2 + Duration::hours(5),
            kind: "active".into(),
            team_id: None,
        }],
    )
    .await
    .unwrap();
    let present = attendance_service::rollup_day(&pool, emp.id, full_day)
        .await
        .unwrap();
    assert_eq!(present.status, "present");

    users::delete(&pool, emp.id).await.unwrap();
}

#[tokio::test]
async fn mark_present_today_materializes_without_section_visit() {
    let Some(pool) = pool().await else {
        eprintln!("skipping mark-present test: DATABASE_URL not set");
        return;
    };
    let tag = Uuid::new_v4();
    let emp = users::create(
        &pool,
        "Att Live",
        &format!("att-live-{tag}@t.local"),
        "h",
        UserRole::Employee,
        None,
    )
    .await
    .unwrap();
    let today = Utc::now().date_naive();

    // No calendar view, no rollup — just the "started tracking" signal marks
    // the day present immediately (even with nothing synced yet).
    attendance_service::mark_present_today(&pool, emp.id)
        .await
        .unwrap();
    let row = attendance::get(&pool, emp.id, today)
        .await
        .unwrap()
        .expect("attendance row created on start");
    assert_eq!(row.status, "present");
    assert!(!row.is_override);

    // It must never stomp an HR override.
    attendance_service::override_day(&pool, emp.id, today, "leave", "wfh", emp.id)
        .await
        .unwrap();
    attendance_service::mark_present_today(&pool, emp.id)
        .await
        .unwrap();
    let after = attendance::get(&pool, emp.id, today).await.unwrap().unwrap();
    assert_eq!(after.status, "leave", "HR override must be preserved");
    assert!(after.is_override);

    users::delete(&pool, emp.id).await.unwrap();
}

#[tokio::test]
async fn attendance_never_predates_account_creation() {
    let Some(pool) = pool().await else {
        eprintln!("skipping attendance test: DATABASE_URL not set");
        return;
    };
    // Fresh user created NOW: ensure_range over a 2020 week must not create
    // any rows (attendance is clamped to the account-creation date, which is
    // also what makes an admin "fresh start" — bump created_at + delete rows —
    // stick instead of being re-derived).
    let tag = Uuid::new_v4();
    let emp = users::create(
        &pool,
        "Att Fresh",
        &format!("att-fresh-{tag}@t.local"),
        "h",
        UserRole::Employee,
        None,
    )
    .await
    .unwrap();

    let monday = next_weekday(d(2020, 6, 1), Weekday::Mon);
    let saturday = next_weekday(monday, Weekday::Sat);
    attendance_service::ensure_range(&pool, emp.id, monday, saturday)
        .await
        .unwrap();

    let rows = attendance::list_range(&pool, emp.id, monday, saturday)
        .await
        .unwrap();
    assert!(rows.is_empty(), "pre-creation days must not be derived");

    users::delete(&pool, emp.id).await.unwrap();
}

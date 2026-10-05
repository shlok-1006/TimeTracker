//! Which half a half-day leave covers (migration 0051).
//!
//! The rule under test: `half_period` is accepted ONLY on a single-day request of exactly 0.5 days,
//! it round-trips to the employee's own list, and omitting it keeps the previous behaviour intact
//! (the desktop app and employee-web still book half days without it).
//! Needs a database: skipped when DATABASE_URL is unset. Uses year-2032 dates (a Monday, so the
//! working-day count is never confused by a weekend), a fresh user and type, and cleans up after.

use chrono::{Datelike, NaiveDate};
use uuid::Uuid;

use server::db::{leave, users};
use server::leave_service;
use server::role::UserRole;

async fn real_pool() -> Option<sqlx::PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .ok()
}

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn err_text<T>(r: Result<T, server::error::AppError>) -> String {
    match r {
        Ok(_) => "OK".into(),
        Err(e) => format!("{e:?}"),
    }
}

#[tokio::test]
async fn half_period_is_scoped_to_a_single_half_day() {
    let Some(pool) = real_pool().await else {
        eprintln!("skipping half-period test: DATABASE_URL not set");
        return;
    };
    let tag = Uuid::new_v4().simple().to_string();

    let user = users::create(
        &pool,
        "half-period",
        &format!("half-{tag}@t.local"),
        "h",
        UserRole::Employee,
        None,
    )
    .await
    .unwrap();

    let lt = leave::create_type(
        &pool,
        &format!("casual-{tag}"),
        true,
        20.0,
        20.0,
        20.0,
        &leave::TypeRules::default(),
    )
    .await
    .unwrap();

    // 2032-03-01 is a Monday. Every case below uses dates it does not share with another case:
    // the overlap guard runs BEFORE the half-period checks, so a reused date would mask the
    // refusal we actually want to assert.
    let mon = d(2032, 3, 1);
    assert_eq!(mon.weekday().num_days_from_monday(), 0, "fixture sanity");

    // ---- accepted: one day, half a day, each half ----
    let (first_id, days) = leave_service::submit_request(
        &pool,
        user.id,
        lt.id,
        mon,
        mon,
        "morning",
        Some(0.5),
        Some("first"),
    )
    .await
    .expect("first half on a single day is allowed");
    assert_eq!(days, 0.5);

    let tue = d(2032, 3, 2);
    leave_service::submit_request(
        &pool,
        user.id,
        lt.id,
        tue,
        tue,
        "afternoon",
        Some(0.5),
        Some("second"),
    )
    .await
    .expect("second half on a single day is allowed");

    // Case-insensitive, and whitespace is not a value.
    let thu = d(2032, 3, 4);
    leave_service::submit_request(
        &pool,
        user.id,
        lt.id,
        thu,
        thu,
        "",
        Some(0.5),
        Some("  First  "),
    )
    .await
    .expect("half period is normalized, not rejected on case/padding");

    // ---- refused: a half period that doesn't describe a single half-day ----
    let fri = d(2032, 3, 5);
    let whole_day = err_text(
        leave_service::submit_request(&pool, user.id, lt.id, fri, fri, "", None, Some("first"))
            .await,
    );
    assert!(
        whole_day.contains("half-day"),
        "a full day must not carry a half period, got: {whole_day}"
    );

    // A clean week, so the overlap guard can't fire first and hide the refusal under test.
    let next_mon = d(2032, 3, 15);
    let next_wed = d(2032, 3, 17);
    let over_range = err_text(
        leave_service::submit_request(
            &pool,
            user.id,
            lt.id,
            next_mon,
            next_wed,
            "",
            Some(2.5),
            Some("second"),
        )
        .await,
    );
    assert!(
        over_range.contains("half-day") || over_range.contains("single day"),
        "a multi-day request must not carry a half period, got: {over_range}"
    );

    let bogus = err_text(
        leave_service::submit_request(
            &pool,
            user.id,
            lt.id,
            fri,
            fri,
            "",
            Some(0.5),
            Some("morning"),
        )
        .await,
    );
    assert!(
        bogus.contains("first") && bogus.contains("second"),
        "an unknown half period must say what is allowed, got: {bogus}"
    );

    // ---- unchanged for callers that don't send it (desktop, employee-web) ----
    let (legacy_id, legacy_days) =
        leave_service::submit_request(&pool, user.id, lt.id, fri, fri, "", Some(0.5), None)
            .await
            .expect("a half day without a half period still books");
    assert_eq!(legacy_days, 0.5);

    // ---- it round-trips to the employee's own list ----
    let mine = leave::list_requests_for_user(&pool, user.id).await.unwrap();
    let got = |id: Uuid| {
        mine.iter()
            .find(|r| r.id == id)
            .unwrap_or_else(|| panic!("request {id} missing from the list"))
            .half_period
            .clone()
    };
    assert_eq!(got(first_id), Some("first".to_string()));
    assert_eq!(got(legacy_id), None, "a legacy half day reads back as None");

    // A blank string is treated as absent, not stored as a value.
    let blank_day = d(2032, 3, 22);
    let (blank_id, _) = leave_service::submit_request(
        &pool,
        user.id,
        lt.id,
        blank_day,
        blank_day,
        "",
        Some(0.5),
        Some("   "),
    )
    .await
    .expect("a blank half period is absent, not invalid");
    let after = leave::list_requests_for_user(&pool, user.id).await.unwrap();
    assert_eq!(
        after
            .iter()
            .find(|r| r.id == blank_id)
            .expect("blank-period request present")
            .half_period,
        None,
        "a blank half period must not be stored"
    );

    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user.id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM leave_types WHERE id = $1")
        .bind(lt.id)
        .execute(&pool)
        .await
        .unwrap();
}

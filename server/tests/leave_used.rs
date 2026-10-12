//! HR sets how many days of a leave type someone has USED (migration 0053).
//!
//! "Used" is the sum of approved requests; `leave::set_used` records the difference HR adds on top (leave taken
//! outside TimeTracker). Under test: the total HR sets is what the balance reports, it can't go below what
//! approved requests already account for, a later approved request adds on top, setting it back to the
//! approved total removes the adjustment, and the "leaves left" roll-up follows the same arithmetic.
//! Needs a database: skipped when DATABASE_URL is unset. Uses year-2033 dates, a fresh user and type, and
//! cleans up after.

use chrono::NaiveDate;
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

async fn balance_of(pool: &sqlx::PgPool, user: Uuid, lt: Uuid, year: i32) -> leave::Balance {
    leave::balances(pool, user, year)
        .await
        .unwrap()
        .into_iter()
        .find(|b| b.leave_type_id == lt)
        .expect("the test type is in the balance")
}

#[tokio::test]
async fn hr_sets_used_days_on_top_of_approved_requests() {
    let Some(pool) = real_pool().await else {
        eprintln!("skipping leave-used test: DATABASE_URL not set");
        return;
    };
    let tag = Uuid::new_v4().simple().to_string();
    let year = 2033;

    let hr = users::create(
        &pool,
        "hr-used",
        &format!("hr-used-{tag}@t.local"),
        "h",
        UserRole::Hr,
        None,
    )
    .await
    .unwrap();
    let user = users::create(
        &pool,
        "used",
        &format!("used-{tag}@t.local"),
        "h",
        UserRole::Employee,
        None,
    )
    .await
    .unwrap();
    let lt = leave::create_type(
        &pool,
        &format!("casual-used-{tag}"),
        true,
        12.0,
        12.0,
        12.0,
        &leave::TypeRules::default(),
    )
    .await
    .unwrap();

    // Nothing used yet.
    let b = balance_of(&pool, user.id, lt.id, year).await;
    assert_eq!(
        (
            b.allotted_days,
            b.used_days,
            b.remaining_days,
            b.used_adjustment
        ),
        (12.0, 0.0, 12.0, 0.0)
    );

    // HR records 3.5 days used outside TimeTracker.
    let r = leave::set_used(&pool, user.id, lt.id, year, 3.5, hr.id)
        .await
        .unwrap();
    assert_eq!(
        r,
        leave::SetUsed::Saved {
            approved: 0.0,
            adjustment: 3.5
        }
    );
    let b = balance_of(&pool, user.id, lt.id, year).await;
    assert_eq!(
        (b.used_days, b.remaining_days, b.used_adjustment),
        (3.5, 8.5, 3.5)
    );

    // An approved 1-day request (2033-03-07 is a Monday) adds on top: 4.5 used.
    let (req, _) = leave_service::submit_request(
        &pool,
        user.id,
        lt.id,
        d(year, 3, 7),
        d(year, 3, 7),
        "",
        None,
        None,
    )
    .await
    .expect("a one-day request");
    assert!(leave::decide(&pool, req, "approved", hr.id).await.unwrap());
    let b = balance_of(&pool, user.id, lt.id, year).await;
    assert_eq!(
        (b.used_days, b.remaining_days, b.used_adjustment),
        (4.5, 7.5, 3.5)
    );

    // Can't go below what approved requests already account for.
    let r = leave::set_used(&pool, user.id, lt.id, year, 0.5, hr.id)
        .await
        .unwrap();
    assert_eq!(r, leave::SetUsed::BelowApproved { approved: 1.0 });
    assert_eq!(
        balance_of(&pool, user.id, lt.id, year).await.used_days,
        4.5,
        "a refused set changes nothing"
    );

    // Setting it to exactly the approved total removes the adjustment row.
    let r = leave::set_used(&pool, user.id, lt.id, year, 1.0, hr.id)
        .await
        .unwrap();
    assert_eq!(
        r,
        leave::SetUsed::Saved {
            approved: 1.0,
            adjustment: 0.0
        }
    );
    let rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM leave_used_adjustments WHERE user_id = $1")
            .bind(user.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(rows, 0, "a zero difference leaves no row behind");

    // The "leaves left" roll-up follows the same arithmetic (12 allotted − 1 approved − 2 recorded).
    leave::set_used(&pool, user.id, lt.id, year, 3.0, hr.id)
        .await
        .unwrap();
    let left = leave::remaining_paid_by_user(&pool, year, None)
        .await
        .unwrap();
    let this_type_only = left.get(&user.id).copied().unwrap_or_default()
        - leave::balances(&pool, user.id, year)
            .await
            .unwrap()
            .iter()
            .filter(|b| b.paid && !b.special && b.leave_type_id != lt.id)
            .map(|b| b.remaining_days)
            .sum::<f64>();
    assert_eq!(this_type_only, 9.0);

    // Unknown user / type.
    assert_eq!(
        leave::set_used(&pool, Uuid::new_v4(), lt.id, year, 1.0, hr.id)
            .await
            .unwrap(),
        leave::SetUsed::NotFound
    );
    assert_eq!(
        leave::set_used(&pool, user.id, Uuid::new_v4(), year, 1.0, hr.id)
            .await
            .unwrap(),
        leave::SetUsed::NotFound
    );

    for id in [user.id, hr.id] {
        sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
    }
    sqlx::query("DELETE FROM leave_types WHERE id = $1")
        .bind(lt.id)
        .execute(&pool)
        .await
        .unwrap();
}

//! Pending leave requests nobody decided in time expire (migration 0056, `leave_expiry`).
//!
//! Under test: a request still pending more than the grace days after its leave ended becomes `expired`;
//! one inside the grace window, a future one, and anything already decided are untouched; an expired request
//! can no longer be approved; a second run changes nothing; grace 0 expires the day after the leave.
//!
//! The expiry UPDATE touches every pending row older than its cutoff, so these tests use a "today" in January
//! 2001 and requests dated around it: nothing real in any database is that old. Needs a database (skipped
//! when DATABASE_URL is unset); cleans up after itself.

use chrono::NaiveDate;
use uuid::Uuid;

use server::db::{leave, users};
use server::leave_expiry::{expire_pending, DEFAULT_EXPIRY_DAYS};
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

async fn request(
    pool: &sqlx::PgPool,
    user: Uuid,
    lt: Uuid,
    start: NaiveDate,
    end: NaiveDate,
    status: &str,
) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO leave_requests (user_id, leave_type_id, start_date, end_date, days, reason, status)
         VALUES ($1, $2, $3, $4, 1, 'expiry test', $5) RETURNING id",
    )
    .bind(user)
    .bind(lt)
    .bind(start)
    .bind(end)
    .bind(status)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn status_of(pool: &sqlx::PgPool, id: Uuid) -> (String, bool) {
    sqlx::query_as::<_, (String, bool)>(
        "SELECT status, decided_at IS NOT NULL FROM leave_requests WHERE id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn pending_requests_expire_after_the_grace_days() {
    let Some(pool) = real_pool().await else {
        eprintln!("skipping leave-expiry test: DATABASE_URL not set");
        return;
    };
    assert_eq!(DEFAULT_EXPIRY_DAYS, 3, "OKF LV-13 says 3 days");
    let tag = Uuid::new_v4().simple().to_string();
    let hr = users::create(
        &pool,
        "hr-exp",
        &format!("hr-exp-{tag}@t.local"),
        "h",
        UserRole::Hr,
        None,
    )
    .await
    .unwrap();
    let user = users::create(
        &pool,
        "exp",
        &format!("exp-{tag}@t.local"),
        "h",
        UserRole::Employee,
        None,
    )
    .await
    .unwrap();
    let lt = leave::create_type(
        &pool,
        &format!("exp-{tag}"),
        true,
        12.0,
        12.0,
        12.0,
        &leave::TypeRules::default(),
    )
    .await
    .unwrap();

    // Today is Sat 20 Jan 2001; with 3 grace days, a leave that ended before the 17th has waited too long.
    let today = d(2001, 1, 20);
    let old = request(
        &pool,
        user.id,
        lt.id,
        d(2001, 1, 15),
        d(2001, 1, 16),
        "pending",
    )
    .await; // ended 16th: expires
    let edge = request(
        &pool,
        user.id,
        lt.id,
        d(2001, 1, 17),
        d(2001, 1, 17),
        "pending",
    )
    .await; // ended 17th: 3 days, still waits
    let later = request(
        &pool,
        user.id,
        lt.id,
        d(2001, 1, 25),
        d(2001, 1, 26),
        "pending",
    )
    .await; // not taken yet
    let approved = request(
        &pool,
        user.id,
        lt.id,
        d(2001, 1, 2),
        d(2001, 1, 2),
        "approved",
    )
    .await;
    let rejected = request(
        &pool,
        user.id,
        lt.id,
        d(2001, 1, 3),
        d(2001, 1, 3),
        "rejected",
    )
    .await;

    let done = expire_pending(&pool, today, 3).await.unwrap();
    let ours: Vec<Uuid> = done
        .iter()
        .map(|e| e.id)
        .filter(|id| [old, edge, later, approved, rejected].contains(id))
        .collect();
    assert_eq!(
        ours,
        vec![old],
        "only the request past its grace days expires"
    );
    assert_eq!(
        status_of(&pool, old).await,
        ("expired".to_string(), true),
        "expired, with when"
    );
    assert_eq!(status_of(&pool, edge).await.0, "pending");
    assert_eq!(status_of(&pool, later).await.0, "pending");
    assert_eq!(
        status_of(&pool, approved).await.0,
        "approved",
        "a decision is never overwritten"
    );
    assert_eq!(status_of(&pool, rejected).await.0, "rejected");

    // An expired request can't be approved any more (approve only decides pending ones).
    assert!(!leave::decide(&pool, old, "approved", hr.id).await.unwrap());
    assert!(!leave_service::approve(&pool, old, hr.id)
        .await
        .unwrap_or(false));
    assert_eq!(status_of(&pool, old).await.0, "expired");

    // A second run changes nothing of ours.
    let again = expire_pending(&pool, today, 3).await.unwrap();
    assert!(again.iter().all(|e| ![old, edge, later].contains(&e.id)));

    // A day later the edge request has waited its 3 days too.
    let next_day = expire_pending(&pool, d(2001, 1, 21), 3).await.unwrap();
    assert!(next_day.iter().any(|e| e.id == edge));
    assert_eq!(status_of(&pool, later).await.0, "pending");

    // Grace 0: expires the day after the leave.
    let zero = request(
        &pool,
        user.id,
        lt.id,
        d(2001, 1, 19),
        d(2001, 1, 19),
        "pending",
    )
    .await;
    assert!(
        expire_pending(&pool, d(2001, 1, 19), 0)
            .await
            .unwrap()
            .iter()
            .all(|e| e.id != zero),
        "not on the day itself"
    );
    assert!(expire_pending(&pool, d(2001, 1, 20), 0)
        .await
        .unwrap()
        .iter()
        .any(|e| e.id == zero));

    sqlx::query("DELETE FROM leave_requests WHERE user_id = $1")
        .bind(user.id)
        .execute(&pool)
        .await
        .unwrap();
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

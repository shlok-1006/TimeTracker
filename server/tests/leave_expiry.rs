//! Pending leave requests nobody decided in time expire (migration 0056, `leave_expiry`).
//!
//! Under test: a request still pending more than the grace days after its leave ended becomes `expired`;
//! one inside the grace window, a future one, and anything already decided are untouched; an expired request
//! can no longer be approved; a second run changes nothing; grace 0 expires the day after the leave. Leave
//! applied for after the fact counts its grace days from the day it was submitted (an IST date), not from
//! the end date; and each expiry leaves a `leave.expired` audit row.
//!
//! The expiry UPDATE touches every pending row older than its cutoff, so these tests use a "today" in January
//! 2001 and requests dated around it: nothing real in any database is that old. Needs a database (skipped
//! when DATABASE_URL is unset); cleans up after itself.

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
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

/// Noon IST (06:30 UTC) on `day`: a submit time that is unambiguously that IST date.
fn noon_ist(day: NaiveDate) -> DateTime<Utc> {
    Utc.from_utc_datetime(&day.and_hms_opt(6, 30, 0).unwrap())
}

/// A request submitted on 1 Jan 2001 — before every leave date the main test uses — so only its end date
/// decides when it expires. (`created_at` must be set: its default, now(), would keep 2001 leave pending.)
async fn request(
    pool: &sqlx::PgPool,
    user: Uuid,
    lt: Uuid,
    start: NaiveDate,
    end: NaiveDate,
    status: &str,
) -> Uuid {
    request_submitted(pool, user, lt, start, end, status, noon_ist(d(2001, 1, 1))).await
}

async fn request_submitted(
    pool: &sqlx::PgPool,
    user: Uuid,
    lt: Uuid,
    start: NaiveDate,
    end: NaiveDate,
    status: &str,
    created_at: DateTime<Utc>,
) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO leave_requests (user_id, leave_type_id, start_date, end_date, days, reason, status, created_at)
         VALUES ($1, $2, $3, $4, 1, 'expiry test', $5, $6) RETURNING id",
    )
    .bind(user)
    .bind(lt)
    .bind(start)
    .bind(end)
    .bind(status)
    .bind(created_at)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn expired_audit_rows(pool: &sqlx::PgPool, id: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM audit_logs
          WHERE action = 'leave.expired' AND entity_type = 'leave_request' AND entity_id = $1
            AND actor_id IS NULL",
    )
    .bind(id)
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
    assert_eq!(
        expired_audit_rows(&pool, old).await,
        1,
        "the expiry is on the audit trail"
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
    assert_eq!(
        expired_audit_rows(&pool, old).await,
        1,
        "no second audit row"
    );

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

    // Applied for after the fact: leave 1–2 Jan, submitted on the 20th (IST). The grace counts from the
    // submit day, so it waits through the 23rd and expires from the 24th — not on the next run.
    let late = request_submitted(
        &pool,
        user.id,
        lt.id,
        d(2001, 1, 1),
        d(2001, 1, 2),
        "pending",
        noon_ist(d(2001, 1, 20)),
    )
    .await;
    for day in [20, 21, 22, 23] {
        assert!(
            expire_pending(&pool, d(2001, 1, day), 3)
                .await
                .unwrap()
                .iter()
                .all(|e| e.id != late),
            "a late application still waits on the {day}th"
        );
        assert_eq!(status_of(&pool, late).await.0, "pending");
    }
    assert!(expire_pending(&pool, d(2001, 1, 24), 3)
        .await
        .unwrap()
        .iter()
        .any(|e| e.id == late));
    assert_eq!(status_of(&pool, late).await.0, "expired");

    // The submit day is the IST date: 19:00 UTC on the 21st is already 00:30 on the 22nd in IST, so the
    // request waits through the 25th (a UTC date would have expired it on the 25th).
    let late_night = request_submitted(
        &pool,
        user.id,
        lt.id,
        d(2001, 1, 1),
        d(2001, 1, 2),
        "pending",
        Utc.with_ymd_and_hms(2001, 1, 21, 19, 0, 0).unwrap(),
    )
    .await;
    assert!(expire_pending(&pool, d(2001, 1, 25), 3)
        .await
        .unwrap()
        .iter()
        .all(|e| e.id != late_night));
    assert!(expire_pending(&pool, d(2001, 1, 26), 3)
        .await
        .unwrap()
        .iter()
        .any(|e| e.id == late_night));

    // Grace 0, submitted the same day the job runs: not today, from tomorrow.
    let same_day = request_submitted(
        &pool,
        user.id,
        lt.id,
        d(2001, 1, 3),
        d(2001, 1, 3),
        "pending",
        noon_ist(d(2001, 1, 28)),
    )
    .await;
    assert!(expire_pending(&pool, d(2001, 1, 28), 0)
        .await
        .unwrap()
        .iter()
        .all(|e| e.id != same_day));
    assert!(expire_pending(&pool, d(2001, 1, 29), 0)
        .await
        .unwrap()
        .iter()
        .any(|e| e.id == same_day));

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

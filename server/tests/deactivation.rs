//! Deactivate instead of delete (migration 0047).
//!
//! `DELETE /admin/users/:id` used to really delete, and every foreign key cascaded: the person's
//! intervals, screenshots, attendance and reports went with them. A month-end report covering a
//! period someone left in quietly lost their hours, and an audit of last quarter could not explain
//! its own numbers.
//!
//! So the test that matters is not "are they gone" but "is their WORK still here". The rest is
//! about a leaver actually being gone in the ways that count: no login, off the rosters, no longer
//! accruing attendance.

use chrono::{Duration, TimeZone, Utc};
use uuid::Uuid;

use server::db::users;

async fn real_pool() -> Option<sqlx::PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .ok()
}

/// A real HR row to act as the remover. `deactivated_by` is a foreign key, so this cannot be a
/// throwaway UUID — the UPDATE would fail the constraint and (with `.ok()`) look like success.
async fn seed_hr(pool: &sqlx::PgPool) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO users (id, name, email, password_hash, role) VALUES ($1,'HR',$2,'x','hr')",
    )
    .bind(id)
    .bind(format!("hr-{id}@test.local"))
    .execute(pool)
    .await
    .expect("seed hr");
    id
}

async fn seed(pool: &sqlx::PgPool) -> (Uuid, String) {
    let id = Uuid::new_v4();
    let email = format!("leaver-{id}@test.local");
    sqlx::query("INSERT INTO users (id, name, email, password_hash, role) VALUES ($1,'Leaver',$2,'x','employee')")
        .bind(id)
        .bind(&email)
        .execute(pool)
        .await
        .expect("seed user");
    (id, email)
}

#[tokio::test]
async fn deactivating_keeps_every_trace_of_their_work() {
    let Some(pool) = real_pool().await else {
        eprintln!("DATABASE_URL unset — skipping deactivation test");
        return;
    };
    let (id, _) = seed(&pool).await;
    let hr = seed_hr(&pool).await;

    // An hour of tracked work in a long-past month.
    let base = Utc.with_ymd_and_hms(2020, 7, 6, 9, 0, 0).unwrap();
    sqlx::query("INSERT INTO intervals (id, user_id, start_utc, end_utc, idle, kind) VALUES ($1,$2,$3,$4,false,'active')")
        .bind(Uuid::new_v4())
        .bind(id)
        .bind(base)
        .bind(base + Duration::hours(1))
        .execute(&pool)
        .await
        .expect("seed interval");

    assert!(users::deactivate(&pool, id, hr).await.expect("deactivate"));

    // THE point of the change: the row and its history are still there.
    let still: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM intervals WHERE user_id = $1")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        still, 1,
        "their work must survive — a hard delete cascaded this away and silently changed \
         every past report that included them"
    );
    assert!(
        users::find_by_id(&pool, id).await.unwrap().is_some(),
        "the user row itself must remain, so reports can still name them"
    );

    sqlx::query("DELETE FROM users WHERE id = ANY($1)")
        .bind(vec![id, hr])
        .execute(&pool)
        .await
        .ok();
}

#[tokio::test]
async fn a_leaver_cannot_sign_in_and_is_off_the_rosters() {
    let Some(pool) = real_pool().await else {
        eprintln!("DATABASE_URL unset — skipping leaver-visibility test");
        return;
    };
    let (id, email) = seed(&pool).await;
    let hr = seed_hr(&pool).await;

    assert!(
        users::find_by_email(&pool, &email).await.unwrap().is_some(),
        "resolvable while active"
    );
    assert!(users::employee_ids(&pool).await.unwrap().contains(&id));

    assert!(users::deactivate(&pool, id, hr).await.expect("deactivate"));

    assert!(
        users::find_by_email(&pool, &email).await.unwrap().is_none(),
        "login resolves by email — a deactivated account must not resolve at all, so no caller \
         can forget to check"
    );
    assert!(
        !users::employee_ids(&pool).await.unwrap().contains(&id),
        "and they stop accruing attendance the day they leave"
    );
    assert!(
        !users::list_all(&pool)
            .await
            .unwrap()
            .iter()
            .any(|u| u.id == id),
        "off the roster"
    );

    sqlx::query("DELETE FROM users WHERE id = ANY($1)")
        .bind(vec![id, hr])
        .execute(&pool)
        .await
        .ok();
}

#[tokio::test]
async fn reactivating_continues_the_record_rather_than_restarting_it() {
    let Some(pool) = real_pool().await else {
        eprintln!("DATABASE_URL unset — skipping reactivation test");
        return;
    };
    let (id, email) = seed(&pool).await;
    let hr = seed_hr(&pool).await;

    assert!(users::deactivate(&pool, id, hr).await.expect("deactivate"));
    assert!(users::list_deactivated(&pool)
        .await
        .unwrap()
        .iter()
        .any(|u| u.user_id == id));

    assert!(users::reactivate(&pool, id).await.expect("reactivate"));
    assert!(
        users::find_by_email(&pool, &email).await.unwrap().is_some(),
        "they can sign in again"
    );
    assert!(
        !users::list_deactivated(&pool)
            .await
            .unwrap()
            .iter()
            .any(|u| u.user_id == id),
        "and they are off the alumni list"
    );

    // Both idempotent: a double-click is the state the caller asked for, not an error.
    assert!(
        !users::reactivate(&pool, id).await.unwrap(),
        "already active"
    );
    assert!(users::deactivate(&pool, id, hr).await.unwrap());
    assert!(
        !users::deactivate(&pool, id, hr).await.unwrap(),
        "a second deactivate must NOT re-stamp the date they left"
    );

    sqlx::query("DELETE FROM users WHERE id = ANY($1)")
        .bind(vec![id, hr])
        .execute(&pool)
        .await
        .ok();
}

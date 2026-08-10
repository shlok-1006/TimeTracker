//! Two devices, one person: a wall-clock second must be counted once.
//!
//! Interval ids are minted by the desktop app and the sync insert dedupes on that id
//! alone, so two installs signed in as the same person both land their own minute rows
//! for the same minute. Nothing rejects them — which is defensible, both recordings did
//! happen — so the derivation has to be the thing that refuses to count twice.
//!
//! The fixtures below reproduce what production actually looked like on 10 Aug 2026:
//! two series of 60-second intervals about 11 seconds out of phase, summing to 1.7x the
//! elapsed time. Year-2020 dates and a throwaway user keep this clear of live data.

use chrono::{DateTime, Duration, TimeZone, Utc};
use uuid::Uuid;

use server::db::{attendance, intervals};

async fn real_pool() -> Option<sqlx::PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .ok()
}

async fn seed_user(pool: &sqlx::PgPool) -> Uuid {
    let uid = Uuid::new_v4();
    sqlx::query!(
        "INSERT INTO users (id, name, email, password_hash, role)
         VALUES ($1, 'Overlap Test', $2, 'x', 'employee')",
        uid,
        format!("overlap-{uid}@test.local")
    )
    .execute(pool)
    .await
    .expect("seed user");
    uid
}

/// `count` back-to-back one-minute intervals of `kind`, starting at `from`.
async fn series(pool: &sqlx::PgPool, uid: Uuid, from: DateTime<Utc>, count: i64, kind: &str) {
    for i in 0..count {
        let s = from + Duration::minutes(i);
        sqlx::query!(
            "INSERT INTO intervals (id, user_id, start_utc, end_utc, idle, kind)
             VALUES ($1, $2, $3, $4, false, $5)",
            Uuid::new_v4(),
            uid,
            s,
            s + Duration::minutes(1),
            kind
        )
        .execute(pool)
        .await
        .expect("seed interval");
    }
}

async fn cleanup(pool: &sqlx::PgPool, uid: Uuid) {
    sqlx::query!("DELETE FROM users WHERE id = $1", uid)
        .execute(pool)
        .await
        .ok();
}

#[tokio::test]
async fn a_second_device_does_not_double_the_day() {
    let Some(pool) = real_pool().await else {
        eprintln!("DATABASE_URL unset — skipping overlap test");
        return;
    };
    let uid = seed_user(&pool).await;

    // 09:00–10:00, recorded twice: the second install 11 seconds out of phase, exactly
    // the pattern seen in production.
    let base = Utc.with_ymd_and_hms(2020, 5, 4, 9, 0, 0).unwrap();
    series(&pool, uid, base, 60, "active").await;
    series(&pool, uid, base + Duration::seconds(11), 60, "active").await;

    let day_start = Utc.with_ymd_and_hms(2020, 5, 4, 0, 0, 0).unwrap();
    let day_end = day_start + Duration::days(1);
    let a = attendance::day_activity(&pool, uid, day_start, day_end)
        .await
        .expect("day activity");

    // 120 rows x 60s = 7200s if added up; the truth is 09:00:00 -> 10:00:11.
    assert_eq!(
        a.worked_seconds, 3611,
        "two devices recording the same hour must yield one hour, not two \
         (got {}s, naive summation would give 7200s)",
        a.worked_seconds
    );
    assert_eq!(a.idle_seconds, 0);

    cleanup(&pool, uid).await;
}

#[tokio::test]
async fn a_single_device_is_completely_unaffected() {
    let Some(pool) = real_pool().await else {
        eprintln!("DATABASE_URL unset — skipping single-device test");
        return;
    };
    let uid = seed_user(&pool).await;

    // The ordinary case: one recorder, no overlap. Merging must be a no-op here, or the
    // fix would quietly change everybody's numbers rather than only the broken ones.
    let base = Utc.with_ymd_and_hms(2020, 5, 5, 9, 0, 0).unwrap();
    series(&pool, uid, base, 30, "active").await;
    series(&pool, uid, base + Duration::minutes(30), 10, "idle").await;
    series(&pool, uid, base + Duration::minutes(40), 20, "meeting").await;

    let day_start = Utc.with_ymd_and_hms(2020, 5, 5, 0, 0, 0).unwrap();
    let a = attendance::day_activity(&pool, uid, day_start, day_start + Duration::days(1))
        .await
        .expect("day activity");

    assert_eq!(
        a.worked_seconds,
        (30 + 20) * 60,
        "active + meeting, untouched"
    );
    assert_eq!(a.idle_seconds, 10 * 60, "idle, untouched");
    assert_eq!(
        a.first_in_utc,
        Some(base),
        "first-in is an edge, not a duration — merging must not move it"
    );

    cleanup(&pool, uid).await;
}

#[tokio::test]
async fn disagreeing_devices_attribute_each_second_once() {
    let Some(pool) = real_pool().await else {
        eprintln!("DATABASE_URL unset — skipping precedence test");
        return;
    };
    let uid = seed_user(&pool).await;

    // Same ten minutes, one device calling it active and the other idle. Positive
    // observation wins, and — the point of the test — the two must not BOTH be counted,
    // because the breakdown is what the UI adds up to make a total.
    let base = Utc.with_ymd_and_hms(2020, 5, 6, 9, 0, 0).unwrap();
    series(&pool, uid, base, 10, "active").await;
    series(&pool, uid, base, 10, "idle").await;

    let day_start = Utc.with_ymd_and_hms(2020, 5, 6, 0, 0, 0).unwrap();
    let a = attendance::day_activity(&pool, uid, day_start, day_start + Duration::days(1))
        .await
        .expect("day activity");

    assert_eq!(a.worked_seconds, 600, "the ten minutes count as active");
    assert_eq!(
        a.idle_seconds, 0,
        "and NOT also as idle — a second belongs to exactly one kind"
    );

    cleanup(&pool, uid).await;
}

#[tokio::test]
async fn hours_summary_reports_the_union_not_the_sum() {
    let Some(pool) = real_pool().await else {
        eprintln!("DATABASE_URL unset — skipping hours-summary test");
        return;
    };
    let uid = seed_user(&pool).await;

    // The all-time figure is the one line that never depends on today's clock, so it is
    // the part of `hours_summary` a test can pin down without controlling `now()`.
    let base = Utc.with_ymd_and_hms(2020, 5, 7, 9, 0, 0).unwrap();
    series(&pool, uid, base, 20, "active").await;
    series(&pool, uid, base + Duration::seconds(11), 20, "active").await;

    let s = intervals::hours_summary(&pool, uid).await.expect("summary");
    assert_eq!(
        s.total_seconds, 1211,
        "all-time worked must be the union (20 min + 11 s), not 2400 s of added-up rows"
    );

    cleanup(&pool, uid).await;
}

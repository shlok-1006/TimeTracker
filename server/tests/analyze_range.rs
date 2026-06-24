//! Range window selection (analyze-by-time-range feature). Verifies the
//! timezone-aware wall-clock filter and the working-only rule. Hits a live DB
//! via DATABASE_URL; skips if unset. (The Gemini analysis loop itself is covered
//! by the unit-tested vision/report layers.)

use chrono::{NaiveDate, NaiveTime, TimeZone, Utc};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use uuid::Uuid;

use server::db::{screenshots, users};
use server::role::UserRole;

async fn pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    PgPoolOptions::new().max_connections(2).connect(&url).await.ok()
}

#[tokio::test]
async fn window_filters_by_local_time_and_working_only() {
    let Some(pool) = pool().await else {
        eprintln!("skipping analyze_range test: DATABASE_URL not set");
        return;
    };
    let tag = Uuid::new_v4();
    let emp = users::create(
        &pool, "Win Emp", &format!("win-{tag}@t.local"), "h", UserRole::Employee, None,
    ).await.unwrap();

    // IST: local = UTC + 330 min. Admin's window is local 15:00–19:00,
    // i.e. UTC 09:30–13:30 on 2026-06-08.
    let day = NaiveDate::from_ymd_opt(2026, 6, 8).unwrap();
    let key = |h: u32| format!("{}/win/{}-{:02}.jpg", emp.id, tag, h);
    let at = |h: u32, m: u32| Utc.with_ymd_and_hms(2026, 6, 8, h, m, 0).unwrap();

    // working, local 14:30 — BEFORE window
    let before = screenshots::insert(&pool, emp.id, &key(9), at(9, 0), None, "working").await.unwrap();
    // working, local 15:30 — in window
    let in1 = screenshots::insert(&pool, emp.id, &key(10), at(10, 0), None, "working").await.unwrap();
    // meeting, local 16:30 — in window but NOT working → excluded
    let meeting = screenshots::insert(&pool, emp.id, &key(11), at(11, 0), None, "meeting").await.unwrap();
    // working, local 17:30 — in window
    let in2 = screenshots::insert(&pool, emp.id, &key(12), at(12, 0), None, "working").await.unwrap();
    // working, local 19:30 — AFTER window
    let after = screenshots::insert(&pool, emp.id, &key(14), at(14, 0), None, "working").await.unwrap();

    let shots = screenshots::list_working_in_window(
        &pool,
        emp.id,
        day,
        NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
        NaiveTime::from_hms_opt(19, 0, 0).unwrap(),
        330, // IST
    )
    .await
    .unwrap();

    let ids: Vec<Uuid> = shots.iter().map(|s| s.screenshot_id).collect();
    assert_eq!(ids, vec![in1, in2], "only the two in-window working shots, in order");
    assert!(!ids.contains(&meeting), "meeting shots are excluded");
    assert!(!ids.contains(&before) && !ids.contains(&after), "out-of-window shots excluded");

    // Same shots, but interpreted in UTC (offset 0): the UTC times are 09:00–14:00,
    // so a UTC window of 09:30–13:30 selects only the 10:00 and 12:00 shots too.
    let utc_shots = screenshots::list_working_in_window(
        &pool,
        emp.id,
        day,
        NaiveTime::from_hms_opt(9, 30, 0).unwrap(),
        NaiveTime::from_hms_opt(13, 30, 0).unwrap(),
        0,
    )
    .await
    .unwrap();
    assert_eq!(
        utc_shots.iter().map(|s| s.screenshot_id).collect::<Vec<_>>(),
        vec![in1, in2],
        "UTC-offset window selects the same two shots"
    );

    users::delete(&pool, emp.id).await.unwrap();
}

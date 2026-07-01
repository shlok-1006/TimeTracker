//! Day-based screenshot listing tests (Feature 3 Phase 1). Hits a live DB via
//! DATABASE_URL; skips if unset. Verifies day filtering, the verdict LEFT JOIN,
//! and the meeting flag (captured_status).

use chrono::{NaiveDate, TimeZone, Utc};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use uuid::Uuid;

use server::db::{analysis_results, screenshots, users};
use server::role::UserRole;
use server::sampler;
use server::vision_analyzer::AnalysisResult;

async fn pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    PgPoolOptions::new().max_connections(2).connect(&url).await.ok()
}

fn verdict_result(verdict: &str) -> AnalysisResult {
    AnalysisResult {
        verdict: verdict.into(),
        matched_ticket_id: None,
        confidence: 0.9,
        observed: "x".into(),
        rationale: "y".into(),
        inconclusive_reason: None,
        model: "claude-haiku-4-5-20251001".into(),
    }
}

#[tokio::test]
async fn list_for_day_filters_with_verdict_and_meeting_flag() {
    let Some(pool) = pool().await else {
        eprintln!("skipping screenshots_day test: DATABASE_URL not set");
        return;
    };

    let email = format!("shots-day-{}@timetracker.local", Uuid::new_v4());
    let user = users::create(&pool, "Shots Day", &email, "h", UserRole::Employee, None)
        .await
        .unwrap();

    let day_a = NaiveDate::from_ymd_opt(2099, 3, 1).unwrap();
    let day_b = NaiveDate::from_ymd_opt(2099, 3, 2).unwrap();
    let at = |d: NaiveDate, h: u32| Utc.from_utc_datetime(&d.and_hms_opt(h, 0, 0).unwrap());
    let job = sampler::create_daily_job(&pool, user.id, day_a).await.unwrap();

    // Day A: a working shot (analysed → aligned) at 09:00 and a meeting shot at 11:00.
    let working = screenshots::insert(
        &pool, user.id, &format!("{}/a/work.jpg", user.id), at(day_a, 9), None, "working",
    ).await.unwrap();
    screenshots::insert(
        &pool, user.id, &format!("{}/a/meet.jpg", user.id), at(day_a, 11), None, "meeting",
    ).await.unwrap();
    // Day B: one working shot.
    screenshots::insert(
        &pool, user.id, &format!("{}/b/work.jpg", user.id), at(day_b, 9), None, "working",
    ).await.unwrap();

    analysis_results::upsert(&pool, job.id, working, &verdict_result("aligned"))
        .await
        .unwrap();

    // Day A → exactly its two shots, oldest first.
    let a = screenshots::list_for_day(&pool, user.id, day_a).await.unwrap();
    assert_eq!(a.len(), 2, "day A should have 2 screenshots");
    assert_eq!(a[0].captured_status, "working");
    assert_eq!(a[0].verdict.as_deref(), Some("aligned"));
    assert_eq!(a[1].captured_status, "meeting");
    assert_eq!(a[1].verdict, None, "meeting shot is not analysed");

    // Day B → only its one shot (day filtering).
    let b = screenshots::list_for_day(&pool, user.id, day_b).await.unwrap();
    assert_eq!(b.len(), 1, "day B should have 1 screenshot");

    // A day with nothing → empty.
    let empty = screenshots::list_for_day(&pool, user.id, NaiveDate::from_ymd_opt(2099, 3, 3).unwrap())
        .await
        .unwrap();
    assert!(empty.is_empty());

    users::delete(&pool, user.id).await.unwrap();
}

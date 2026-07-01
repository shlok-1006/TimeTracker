//! Repository round-trip tests for analysis_reports (Feature 1 Phase 1).
//!
//! Hits a real PostgreSQL via DATABASE_URL. If it is unset (e.g. offline build),
//! the test skips rather than fails — consistent with the rest of the suite,
//! which never assumes a live DB.

use chrono::{NaiveDate, Utc};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use uuid::Uuid;

use server::db::analysis_reports::ReportInput;
use server::db::{analysis_reports, analysis_results, screenshots, users};
use server::report_service;
use server::role::UserRole;
use server::sampler;
use server::vision_analyzer::AnalysisResult;

async fn pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .ok()
}

fn input(user_id: Uuid, day: NaiveDate, job_id: Uuid) -> ReportInput {
    ReportInput {
        user_id,
        day,
        job_id,
        total_analyzed: 5,
        aligned_count: 3,
        partially_count: 1,
        not_aligned_count: 1,
        inconclusive_count: 0,
        alignment_score: 70.0,
        summary_text: "Mostly aligned with assigned work.".into(),
        model: "claude-haiku-4-5-20251001".into(),
    }
}

#[tokio::test]
async fn upsert_get_and_unique_per_user_day() {
    let Some(pool) = pool().await else {
        eprintln!("skipping analysis_reports test: DATABASE_URL not set");
        return;
    };

    // Prerequisites: a temp employee + that day's analysis job (FK targets).
    let email = format!("report-test-{}@timetracker.local", Uuid::new_v4());
    let user = users::create(&pool, "Report Test", &email, "hash", UserRole::Employee, None)
        .await
        .expect("create temp user");
    let day = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
    let job = sampler::create_daily_job(&pool, user.id, day)
        .await
        .expect("create job");

    // Insert.
    let created = analysis_reports::upsert(&pool, &input(user.id, day, job.id))
        .await
        .expect("upsert insert");
    assert_eq!(created.total_analyzed, 5);
    assert_eq!(created.aligned_count, 3);
    assert!((created.alignment_score - 70.0).abs() < 1e-9);

    // Get round-trips the stored values.
    let got = analysis_reports::get(&pool, user.id, day)
        .await
        .expect("get")
        .expect("report exists");
    assert_eq!(got.id, created.id);
    assert_eq!(got.summary_text, "Mostly aligned with assigned work.");
    assert_eq!(got.model, "claude-haiku-4-5-20251001");

    // Upsert again for the same (user, day): UPDATE, not a duplicate.
    let mut second = input(user.id, day, job.id);
    second.total_analyzed = 4;
    second.aligned_count = 4;
    second.partially_count = 0;
    second.not_aligned_count = 0;
    second.alignment_score = 100.0;
    second.summary_text = "Fully aligned.".into();
    let updated = analysis_reports::upsert(&pool, &second).await.expect("upsert update");
    assert_eq!(updated.id, created.id, "same row updated in place");
    assert_eq!(updated.total_analyzed, 4);
    assert_eq!(updated.aligned_count, 4);
    assert!((updated.alignment_score - 100.0).abs() < 1e-9);

    let list = analysis_reports::list_for_user(&pool, user.id).await.expect("list");
    assert_eq!(list.len(), 1, "UNIQUE(user_id, day) must prevent duplicates");

    // Cleanup (cascades the report + job). The temp user performed no audited
    // actions, so deletion is unaffected by the audit-log FK.
    users::delete(&pool, user.id).await.expect("cleanup user");
}

fn result_with(verdict: &str) -> AnalysisResult {
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
async fn build_report_computes_score_from_results() {
    let Some(pool) = pool().await else {
        eprintln!("skipping build_report test: DATABASE_URL not set");
        return;
    };

    let email = format!("build-report-{}@timetracker.local", Uuid::new_v4());
    let user = users::create(&pool, "Build Report", &email, "hash", UserRole::Employee, None)
        .await
        .expect("create temp user");
    let day = NaiveDate::from_ymd_opt(2026, 6, 2).unwrap();
    let job = sampler::create_daily_job(&pool, user.id, day).await.expect("job");

    // Verdicts: aligned, aligned, partially_aligned, not_aligned, inconclusive
    // → weighted (1+1+0.5+0)=2.5 over scored=4 → 62.5; inconclusive excluded.
    let verdicts = ["aligned", "aligned", "partially_aligned", "not_aligned", "inconclusive"];
    for (i, verdict) in verdicts.iter().enumerate() {
        let sid = screenshots::insert(
            &pool,
            user.id,
            &format!("{}/20260602/{i}.jpg", user.id),
            Utc::now(),
            None,
            "working",
        )
        .await
        .expect("insert screenshot");
        analysis_results::upsert(&pool, job.id, sid, &result_with(verdict))
            .await
            .expect("insert result");
    }

    // Unconfigured provider → AI summary errors → deterministic fallback. The
    // counts/score assertions are independent of the summary text.
    let provider = server::claude_provider::ClaudeProvider::from_env();
    let report = report_service::build_report(&pool, user.id, day, job.id, &provider)
        .await
        .expect("build report");

    assert_eq!(report.total_analyzed, 5);
    assert_eq!(report.aligned_count, 2);
    assert_eq!(report.partially_count, 1);
    assert_eq!(report.not_aligned_count, 1);
    assert_eq!(report.inconclusive_count, 1);
    assert!((report.alignment_score - 62.5).abs() < 1e-9, "score was {}", report.alignment_score);
    assert_eq!(report.model, "claude-haiku-4-5-20251001");

    users::delete(&pool, user.id).await.expect("cleanup user");
}

#[tokio::test]
async fn list_for_day_scopes_to_manager_team() {
    let Some(pool) = pool().await else {
        eprintln!("skipping list_for_day scope test: DATABASE_URL not set");
        return;
    };

    // A future day with no other data, so the roster is exactly what we create.
    let day = NaiveDate::from_ymd_opt(2099, 1, 1).unwrap();
    let tag = Uuid::new_v4();

    let pm = users::create(
        &pool, "PM", &format!("pm-{tag}@t.local"), "h", UserRole::ProjectManager, None,
    ).await.unwrap();
    let on_team = users::create(
        &pool, "On Team", &format!("on-{tag}@t.local"), "h", UserRole::Employee, Some(pm.id),
    ).await.unwrap();
    let off_team = users::create(
        &pool, "Off Team", &format!("off-{tag}@t.local"), "h", UserRole::Employee, None,
    ).await.unwrap();

    for u in [&on_team, &off_team] {
        let job = sampler::create_daily_job(&pool, u.id, day).await.unwrap();
        analysis_reports::upsert(&pool, &input(u.id, day, job.id)).await.unwrap();
    }

    // PM scope → only their team member.
    let pm_view = analysis_reports::list_for_day(&pool, Some(pm.id), day).await.unwrap();
    assert_eq!(pm_view.len(), 1, "PM sees only their team");
    assert_eq!(pm_view[0].user_id, on_team.id);

    // HR scope (None) → both employees on that day.
    let hr_view = analysis_reports::list_for_day(&pool, None, day).await.unwrap();
    let ids: Vec<Uuid> = hr_view.iter().map(|r| r.user_id).collect();
    assert!(ids.contains(&on_team.id) && ids.contains(&off_team.id), "HR sees all");

    for u in [on_team, off_team, pm] {
        users::delete(&pool, u.id).await.expect("cleanup");
    }
}

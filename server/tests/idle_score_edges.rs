//! The idle day score's edge cases (migration 0061), each found by an adversarial test of the first version:
//! breaks across devices, night sessions across IST midnight (and a timer left on overnight, which must stay free),
//! the re-score's summary, a re-score racing a rebuild, and columns left stale by a rollback.
//!
//! Requires DATABASE_URL pointing at a migrated database; each test skips when it isn't set.

use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use uuid::Uuid;

use server::db::intervals::IntervalDto;
use server::db::{analysis_reports, analysis_results, intervals, screenshots, users};
use server::report_service::{self, pct};
use server::role::UserRole;
use server::sampler;
use server::vision_analyzer::AnalysisResult;

async fn pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .ok()
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

async fn user(pool: &PgPool) -> Uuid {
    let email = format!("idle-edges-{}@timetracker.local", Uuid::new_v4());
    users::create(pool, "Idle Edges", &email, "hash", UserRole::Employee, None)
        .await
        .expect("user")
        .id
}

/// Add verdicts to the (user, day) job; returns the job id.
async fn verdicts(pool: &PgPool, user_id: Uuid, day: NaiveDate, vs: &[&str]) -> Uuid {
    let job = sampler::create_daily_job(pool, user_id, day)
        .await
        .expect("job");
    for v in vs {
        let sid = screenshots::insert(
            pool,
            user_id,
            &format!("{}/{}/{}.jpg", user_id, day, Uuid::new_v4()),
            Utc::now(),
            None,
            "working",
        )
        .await
        .expect("shot");
        analysis_results::upsert(pool, job.id, sid, &result_with(v))
            .await
            .expect("result");
    }
    job.id
}

/// `h:m` IST on `day`, in UTC.
fn ist(day: NaiveDate, h: u32, m: u32) -> DateTime<Utc> {
    let local = day.and_hms_opt(h, m, 0).expect("time");
    Utc.from_utc_datetime(&local) - Duration::minutes(330)
}

fn iv(t0: DateTime<Utc>, from_min: i64, to_min: i64, kind: &str) -> IntervalDto {
    IntervalDto {
        id: Uuid::new_v4(),
        start_utc: t0 + Duration::minutes(from_min),
        end_utc: t0 + Duration::minutes(to_min),
        kind: kind.into(),
        team_id: None,
    }
}

async fn build(
    pool: &PgPool,
    u: Uuid,
    day: NaiveDate,
    job: Uuid,
) -> analysis_reports::AnalysisReport {
    let provider = server::claude_provider::ClaudeProvider::from_env();
    report_service::build_report(pool, u, day, job, &provider)
        .await
        .expect("build")
}

#[test]
fn percentages_round_half_up_like_the_apps() {
    assert_eq!(pct(62.5), 63);
    assert_eq!(pct(34.5), 35);
    assert_eq!(pct(62.4), 62);
    assert_eq!(pct(0.0), 0);
}

#[tokio::test]
async fn a_break_on_one_device_is_not_idle_while_another_sits_idle() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let u = user(&pool).await;
    let day = NaiveDate::from_ymd_opt(2099, 6, 10).expect("day");
    let job = verdicts(&pool, u, day, &["aligned", "aligned"]).await; // 100
    let t0 = ist(day, 9, 0);
    intervals::insert_batch(
        &pool,
        u,
        &[
            // laptop: 4h active, 1h lunch paused, 4h active
            iv(t0, 0, 240, "active"),
            iv(t0, 240, 300, "break"),
            iv(t0, 300, 540, "active"),
            // desktop, same person, left running idle all day
            iv(t0, 0, 540, "idle"),
        ],
    )
    .await
    .expect("intervals");
    let r = build(&pool, u, day, job).await;
    users::delete(&pool, u).await.ok();
    assert_eq!(r.worked_seconds, Some(8 * 3600));
    assert_eq!(
        r.idle_seconds,
        Some(0),
        "the lunch break was a break, not idle"
    );
    assert_eq!(r.alignment_score, 100.0);
}

#[tokio::test]
async fn idle_in_a_night_session_counts_on_each_side_of_midnight() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let u = user(&pool).await;
    let d1 = NaiveDate::from_ymd_opt(2099, 6, 20).expect("day");
    let d2 = d1 + Duration::days(1);
    let (job1, job2) = (
        verdicts(&pool, u, d1, &["aligned"]).await,
        verdicts(&pool, u, d2, &["aligned"]).await,
    );
    // 22:00–23:00 active, 23:00–01:00 idle, 01:00–03:00 active
    let t = ist(d1, 22, 0);
    intervals::insert_batch(
        &pool,
        u,
        &[
            iv(t, 0, 60, "active"),
            iv(t, 60, 180, "idle"),
            iv(t, 180, 300, "active"),
        ],
    )
    .await
    .expect("intervals");
    let (r1, r2) = (
        build(&pool, u, d1, job1).await,
        build(&pool, u, d2, job2).await,
    );
    users::delete(&pool, u).await.ok();
    assert_eq!(r1.idle_seconds, Some(3600), "23:00–24:00 on the first day");
    assert_eq!(r2.idle_seconds, Some(3600), "00:00–01:00 on the second");
}

#[tokio::test]
async fn a_timer_left_on_overnight_is_still_free() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    for (label, stop_h, stop_m) in [("left at 18:00", 18, 0), ("left late, at 22:30", 22, 30)] {
        let u = user(&pool).await;
        let d1 = NaiveDate::from_ymd_opt(2099, 7, 1).expect("day");
        let d2 = d1 + Duration::days(1);
        let (job1, job2) = (
            verdicts(&pool, u, d1, &["aligned"]).await,
            verdicts(&pool, u, d2, &["aligned"]).await,
        );
        let start = ist(d1, 9, 0);
        let stop = ist(d1, stop_h, stop_m);
        let resume = ist(d2, 9, 0);
        let mins = |a: DateTime<Utc>, b: DateTime<Utc>| (b - a).num_minutes();
        intervals::insert_batch(
            &pool,
            u,
            &[
                iv(start, 0, mins(start, stop), "active"),
                iv(stop, 0, mins(stop, resume), "idle"), // the timer kept running all night
                iv(resume, 0, 180, "active"),
            ],
        )
        .await
        .expect("intervals");
        let (r1, r2) = (
            build(&pool, u, d1, job1).await,
            build(&pool, u, d2, job2).await,
        );
        users::delete(&pool, u).await.ok();
        assert_eq!(
            r1.idle_seconds,
            Some(0),
            "{label}: evening after the last active minute"
        );
        assert_eq!(
            r2.idle_seconds,
            Some(0),
            "{label}: night before the first active minute"
        );
    }
}

#[tokio::test]
async fn a_rescore_refreshes_the_fallback_summary_and_takes_the_screen_score_from_the_counts() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let u = user(&pool).await;
    let day = NaiveDate::from_ymd_opt(2099, 7, 10).expect("day");
    let job = verdicts(
        &pool,
        u,
        day,
        &["aligned", "aligned", "not_aligned", "not_aligned"],
    )
    .await; // 50
    let t0 = ist(day, 9, 0);
    intervals::insert_batch(
        &pool,
        u,
        &[
            iv(t0, 0, 120, "active"),
            iv(t0, 120, 300, "idle"),
            iv(t0, 300, 480, "active"),
        ],
    )
    .await
    .expect("intervals");
    let built = build(&pool, u, day, job).await;
    assert!(built.alignment_score < 50.0);
    if !built.summary_text.contains("Day score") {
        // An AI provider is configured here, so the summary isn't the deterministic one; nothing to refresh.
        users::delete(&pool, u).await.ok();
        eprintln!("skipping the summary half: the summary came from an AI provider");
        return;
    }
    // A late sync: the idle stretch was a meeting on a second device.
    intervals::insert_batch(&pool, u, &[iv(t0, 120, 300, "meeting")])
        .await
        .expect("late meeting");
    let r = report_service::rescore_idle(&pool, &built)
        .await
        .expect("rescore")
        .expect("not rebuilt meanwhile");
    users::delete(&pool, u).await.ok();
    assert_eq!(r.alignment_score, 50.0);
    assert_eq!(r.screen_alignment_score, Some(50.0));
    assert!(
        !r.summary_text.contains("Day score"),
        "the summary still described the old idle: {}",
        r.summary_text
    );
}

#[tokio::test]
async fn a_rescore_never_overwrites_a_rebuild_that_happened_meanwhile() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let u = user(&pool).await;
    let day = NaiveDate::from_ymd_opt(2099, 7, 20).expect("day");
    let job = verdicts(&pool, u, day, &["aligned", "aligned"]).await; // 100
    let t0 = ist(day, 9, 0);
    intervals::insert_batch(&pool, u, &[iv(t0, 0, 480, "active")])
        .await
        .expect("intervals");
    let snapshot = build(&pool, u, day, job).await; // what the nightly job read
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    verdicts(&pool, u, day, &["not_aligned", "not_aligned"]).await;
    let rebuilt = build(&pool, u, day, job).await; // an admin re-analysis in between: 50
    let after = report_service::rescore_idle(&pool, &snapshot)
        .await
        .expect("rescore");
    let stored = analysis_reports::get(&pool, u, day)
        .await
        .expect("get")
        .expect("row");
    users::delete(&pool, u).await.ok();
    assert!(after.is_none(), "the stale re-score must be skipped");
    assert_eq!(
        stored.screen_alignment_score,
        rebuilt.screen_alignment_score
    );
    assert_eq!(stored.alignment_score, 50.0);
}

#[tokio::test]
async fn columns_left_stale_by_a_rollback_read_as_unknown() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let u = user(&pool).await;
    let day = NaiveDate::from_ymd_opt(2099, 7, 25).expect("day");
    let job = verdicts(&pool, u, day, &["aligned", "aligned"]).await; // 100
    let t0 = ist(day, 9, 0);
    intervals::insert_batch(&pool, u, &[iv(t0, 0, 480, "active")])
        .await
        .expect("intervals");
    build(&pool, u, day, job).await;
    // The pre-0061 binary's upsert (it never touches the idle columns), rebuilding the day at 40%.
    sqlx::query(
        r#"INSERT INTO analysis_reports
            (user_id, day, job_id, total_analyzed, aligned_count, partially_count,
             not_aligned_count, inconclusive_count, alignment_score, summary_text, model)
        VALUES ($1, $2, $3, 5, 2, 0, 3, 0, 40.0, 'old', 'm')
        ON CONFLICT (user_id, day) DO UPDATE SET
            job_id = EXCLUDED.job_id, total_analyzed = EXCLUDED.total_analyzed,
            aligned_count = EXCLUDED.aligned_count, partially_count = EXCLUDED.partially_count,
            not_aligned_count = EXCLUDED.not_aligned_count, inconclusive_count = EXCLUDED.inconclusive_count,
            alignment_score = EXCLUDED.alignment_score, summary_text = EXCLUDED.summary_text,
            model = EXCLUDED.model, created_at = now()"#,
    )
    .bind(u)
    .bind(day)
    .bind(job)
    .execute(&pool)
    .await
    .expect("old upsert");
    let r = analysis_reports::get(&pool, u, day)
        .await
        .expect("get")
        .expect("row");
    assert_eq!(r.alignment_score, 40.0);
    assert_eq!(
        r.screen_alignment_score, None,
        "the stale 100% must not be shown"
    );
    assert_eq!(r.worked_seconds, None);
    assert_eq!(r.idle_seconds, None);
    // A re-score takes the screenshot score from the counts, not the stale column.
    let again = report_service::rescore_idle(&pool, &r)
        .await
        .expect("rescore")
        .expect("not rebuilt meanwhile");
    users::delete(&pool, u).await.ok();
    assert_eq!(again.screen_alignment_score, Some(40.0));
    assert_eq!(again.alignment_score, 40.0);
}

#[tokio::test]
async fn a_report_is_stamped_with_the_time_its_activity_was_read() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let u = user(&pool).await;
    let day = NaiveDate::from_ymd_opt(2099, 8, 1).expect("day");
    let job = verdicts(&pool, u, day, &["aligned"]).await;
    let before = Utc::now();
    let r = build(&pool, u, day, job).await;
    let after = Utc::now();
    users::delete(&pool, u).await.ok();
    // Stamped at the activity read (inside build_report), never later than the write; activity received after it is
    // then "late" for stale_for_late_activity instead of being silently missed.
    assert!(r.created_at >= before - Duration::milliseconds(1) && r.created_at <= after);
}

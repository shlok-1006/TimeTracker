//! Analysis orchestration (shared by the on-demand admin route and the nightly
//! scheduler): sample the day's screenshots → vision-analyze the working ones →
//! persist verdicts → build the daily report.
//!
//! Also hosts the admin *range* analysis: exhaustively verify EVERY working
//! screenshot in an arbitrary `[from, to)` window (bypassing the 4–5/day
//! sampler), with live progress recorded in `analysis_range_runs`.

use std::collections::BTreeMap;

use chrono::{DateTime, NaiveDate, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::claude_provider::ClaudeProvider;
use crate::db::analysis_reports::AnalysisReport;
use crate::db::{analysis_results, analysis_runs, manual_tasks, screenshots};
use crate::error::AppError;
use crate::linear_service::LinearService;
use crate::report_service;
use crate::sampler;
use crate::storage::StorageClient;
use crate::ticket_cache::Ticket;
use crate::vision_analyzer::{self, AnalysisOutcome};

/// Cap on a manual task's description in the analyzer context.
const EXCERPT_CHARS: usize = 200;

fn excerpt(s: &str) -> String {
    if s.chars().count() <= EXCERPT_CHARS {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(EXCERPT_CHARS).collect();
        out.push('…');
        out
    }
}

/// Map an HR-assigned manual task into the same shape the analyzer uses for
/// Linear tickets. The id is prefixed `task:` so verdicts/`matched_ticket` can
/// distinguish a manual task from a Linear ticket (and it never touches Linear).
fn manual_to_context(t: &manual_tasks::ManualTask) -> Ticket {
    Ticket {
        id: format!("task:{}", t.id),
        title: t.title.clone(),
        state: "manual".into(),
        project: None,
        labels: vec!["manual task".into()],
        description_excerpt: excerpt(&t.description),
    }
}

/// Build the unified analyzer context for a user: their assigned Linear tickets
/// (open only — already filtered upstream) PLUS their OPEN manual tasks. The
/// vision analyzer compares screenshots against this combined list.
pub async fn build_context(
    db: &PgPool,
    linear: &LinearService,
    user_id: Uuid,
) -> Result<Vec<Ticket>, AppError> {
    let mut ctx = linear.fetch_assigned_tickets(db, user_id).await?;
    for t in manual_tasks::list_for_user(db, user_id).await? {
        if t.status == "open" {
            ctx.push(manual_to_context(&t));
        }
    }
    Ok(ctx)
}

/// Counts + the stored report from one analyze run.
pub struct AnalyzeOutcome {
    pub analyzed: usize,
    pub skipped: usize,
    pub report: AnalysisReport,
}

/// Analyze one employee's day end-to-end and build their report. Per-screenshot
/// failures are logged and skipped (they don't abort the run); the report is
/// always built from whatever verdicts were stored.
pub async fn analyze_user_day(
    db: &PgPool,
    storage: &StorageClient,
    claude: &ClaudeProvider,
    linear: &LinearService,
    user_id: Uuid,
    day: NaiveDate,
) -> Result<AnalyzeOutcome, AppError> {
    let shots = sampler::sample_screenshots(db, user_id, day).await?;
    let job = sampler::create_daily_job(db, user_id, day).await?;
    // Unified context: assigned Linear tickets + open HR-assigned manual tasks.
    let tickets = build_context(db, linear, user_id).await?;

    let mut analyzed = 0usize;
    let mut skipped = 0usize;
    for s in shots {
        let image = match storage.fetch_object(&s.storage_key).await {
            Ok(bytes) => bytes,
            Err(e) => {
                tracing::warn!(screenshot = %s.screenshot_id, "fetch failed: {e}");
                continue;
            }
        };
        match vision_analyzer::analyze_screenshot(claude, &image, "image/jpeg", &s.captured_status, &tickets)
            .await
        {
            Ok(AnalysisOutcome::Analyzed(a)) => {
                analysis_results::upsert(db, job.id, s.screenshot_id, &a).await?;
                analyzed += 1;
            }
            Ok(AnalysisOutcome::SkippedMeetingScreenshot) => skipped += 1,
            Err(e) => tracing::warn!(screenshot = %s.screenshot_id, "analysis failed: {e}"),
        }
    }

    let report = report_service::build_report(db, user_id, day, job.id, claude).await?;
    Ok(AnalyzeOutcome { analyzed, skipped, report })
}

/// Group screenshots by the UTC calendar day they were taken on. Verdicts are
/// stored under each day's `analysis_jobs` row (the `analysis_results.job_id`
/// FK), so a multi-day range fans out into per-day jobs and reports.
fn group_by_day(shots: Vec<screenshots::ScreenshotRow>) -> BTreeMap<NaiveDate, Vec<screenshots::ScreenshotRow>> {
    let mut by_day: BTreeMap<NaiveDate, Vec<screenshots::ScreenshotRow>> = BTreeMap::new();
    for s in shots {
        by_day.entry(s.taken_at.date_naive()).or_default().push(s);
    }
    by_day
}

/// Analyze EVERY working screenshot for one employee in `[from, to)`.
///
/// Unlike [`analyze_user_day`], this does not sample: each working shot in the
/// window is fetched and verified. Progress is written to `analysis_range_runs`
/// (`run_id`) after every screenshot so the admin UI can poll a live bar.
/// Per-screenshot failures increment `failed` and never abort the run; each
/// touched day's report is rebuilt at the end from ALL of that day's verdicts.
async fn analyze_user_range(
    db: &PgPool,
    storage: &StorageClient,
    claude: &ClaudeProvider,
    linear: &LinearService,
    user_id: Uuid,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    run_id: Uuid,
) -> Result<(), AppError> {
    let shots = screenshots::list_working_in_range(db, user_id, from, to).await?;
    // One context for the whole run: today's assigned tickets + open manual
    // tasks. (Historical shots are judged against current assignments — the
    // system has no record of past assignment states.)
    let tickets = build_context(db, linear, user_id).await?;

    for (day, day_shots) in group_by_day(shots) {
        let job = sampler::create_daily_job(db, user_id, day).await?;
        for s in day_shots {
            let image = match storage.fetch_object(&s.storage_key).await {
                Ok(bytes) => bytes,
                Err(e) => {
                    tracing::warn!(screenshot = %s.id, "range analysis: fetch failed: {e}");
                    analysis_runs::bump(db, run_id, 0, 0, 1).await?;
                    continue;
                }
            };
            match vision_analyzer::analyze_screenshot(claude, &image, "image/jpeg", &s.captured_status, &tickets)
                .await
            {
                Ok(AnalysisOutcome::Analyzed(a)) => {
                    analysis_results::upsert(db, job.id, s.id, &a).await?;
                    analysis_runs::bump(db, run_id, 1, 0, 0).await?;
                }
                Ok(AnalysisOutcome::SkippedMeetingScreenshot) => {
                    analysis_runs::bump(db, run_id, 0, 1, 0).await?;
                }
                Err(e) => {
                    tracing::warn!(screenshot = %s.id, "range analysis failed: {e}");
                    analysis_runs::bump(db, run_id, 0, 0, 1).await?;
                }
            }
        }
        // Refresh the day's report from everything now stored under its job.
        if let Err(e) = report_service::build_report(db, user_id, day, job.id, claude).await {
            tracing::warn!(%user_id, %day, "range analysis: report rebuild failed: {e}");
        }
    }
    Ok(())
}

/// Entry point for the spawned background task: run the range analysis and
/// always leave the run row in a terminal state (`completed` or `failed`),
/// so the admin UI's polling never hangs on a dead run.
pub async fn run_range_analysis(
    db: PgPool,
    storage: std::sync::Arc<StorageClient>,
    claude: std::sync::Arc<ClaudeProvider>,
    linear: std::sync::Arc<LinearService>,
    user_id: Uuid,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    run_id: Uuid,
) {
    let result =
        analyze_user_range(&db, &storage, &claude, &linear, user_id, from, to, run_id).await;
    let error = result.as_ref().err().map(|e| e.to_string());
    if let Err(e) = analysis_runs::finish(&db, run_id, error.as_deref()).await {
        tracing::error!(%run_id, "failed to finalize range run: {e}");
    }
    match error {
        None => tracing::info!(%run_id, %user_id, "range analysis completed"),
        Some(err) => tracing::warn!(%run_id, %user_id, "range analysis failed: {err}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn task(id: Uuid, title: &str, description: &str, status: &str) -> manual_tasks::ManualTask {
        manual_tasks::ManualTask {
            id,
            user_id: Uuid::new_v4(),
            created_by: None,
            title: title.into(),
            description: description.into(),
            status: status.into(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn manual_task_maps_to_task_prefixed_context() {
        let id = Uuid::new_v4();
        let c = manual_to_context(&task(id, "Fix the gateway", "retry logic", "open"));
        assert_eq!(c.id, format!("task:{id}"));
        assert_eq!(c.title, "Fix the gateway");
        assert_eq!(c.state, "manual");
        assert!(c.labels.contains(&"manual task".to_string()));
        assert_eq!(c.description_excerpt, "retry logic");
    }

    #[test]
    fn long_description_is_truncated() {
        let c = manual_to_context(&task(Uuid::new_v4(), "t", &"x".repeat(250), "open"));
        // 200 chars + the ellipsis.
        assert_eq!(c.description_excerpt.chars().count(), EXCERPT_CHARS + 1);
        assert!(c.description_excerpt.ends_with('…'));
    }

    #[test]
    fn excerpt_keeps_short_text() {
        assert_eq!(excerpt("short"), "short");
    }

    fn shot(taken_at: &str) -> screenshots::ScreenshotRow {
        screenshots::ScreenshotRow {
            id: Uuid::new_v4(),
            storage_key: format!("k/{taken_at}"),
            taken_at: taken_at.parse().unwrap(),
            interval_id: None,
            captured_status: "working".into(),
        }
    }

    #[test]
    fn range_shots_group_into_utc_days_in_order() {
        let shots = vec![
            shot("2020-06-02T09:00:00Z"),
            shot("2020-06-01T23:59:59Z"),
            shot("2020-06-01T08:00:00Z"),
            shot("2020-06-03T00:00:00Z"),
        ];
        let by_day = group_by_day(shots);
        let days: Vec<NaiveDate> = by_day.keys().copied().collect();
        assert_eq!(
            days,
            vec![
                NaiveDate::from_ymd_opt(2020, 6, 1).unwrap(),
                NaiveDate::from_ymd_opt(2020, 6, 2).unwrap(),
                NaiveDate::from_ymd_opt(2020, 6, 3).unwrap(),
            ]
        );
        assert_eq!(by_day[&NaiveDate::from_ymd_opt(2020, 6, 1).unwrap()].len(), 2);
        // A 23:59:59Z shot belongs to that day, not the next (UTC day bounds).
        assert_eq!(by_day[&NaiveDate::from_ymd_opt(2020, 6, 3).unwrap()].len(), 1);
    }

    #[test]
    fn empty_range_groups_to_no_days() {
        assert!(group_by_day(vec![]).is_empty());
    }
}

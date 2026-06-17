//! Analysis orchestration (shared by the on-demand admin route and the nightly
//! scheduler): sample the day's screenshots → vision-analyze the working ones →
//! persist verdicts → build the daily report.

use chrono::NaiveDate;
use sqlx::PgPool;
use uuid::Uuid;

use crate::db::analysis_reports::AnalysisReport;
use crate::db::{analysis_results, manual_tasks};
use crate::error::AppError;
use crate::gemini_provider::GeminiProvider;
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
    gemini: &GeminiProvider,
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
        match vision_analyzer::analyze_screenshot(gemini, &image, "image/jpeg", &s.captured_status, &tickets)
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

    let report = report_service::build_report(db, user_id, day, job.id, gemini).await?;
    Ok(AnalyzeOutcome { analyzed, skipped, report })
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
}

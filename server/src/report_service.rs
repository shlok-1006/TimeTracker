//! Daily report aggregation (Feature 1 Phase 2).
//!
//! Reads a job's per-screenshot verdicts (`analysis_results`) and produces the
//! aggregate persisted in `analysis_reports`: verdict counts plus a 0–100
//! alignment score.
//!
//! Scoring weights: `aligned` = 1, `partially_aligned` = 0.5, `not_aligned` = 0.
//! `inconclusive` verdicts are EXCLUDED from the score (they still count toward
//! `total_analyzed`). The score is the weighted average of the *scored* verdicts
//! multiplied by 100.

use chrono::NaiveDate;
use sqlx::PgPool;
use uuid::Uuid;

use crate::db::analysis_reports::{self, AnalysisReport, ReportInput};
use crate::db::analysis_results;
use crate::error::AppError;
use crate::summary_generator::{self, SummaryProvider};

/// Alignment score (0–100) below which HR is alerted about a daily report.
/// Overridable via `TIMETRACKER_LOW_SCORE_THRESHOLD`.
pub const DEFAULT_LOW_SCORE_THRESHOLD: f64 = 35.0;

/// The configured low-score alert threshold (0–100), env-overridable.
pub fn low_score_threshold() -> f64 {
    std::env::var("TIMETRACKER_LOW_SCORE_THRESHOLD")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v >= 0.0)
        .unwrap_or(DEFAULT_LOW_SCORE_THRESHOLD)
}

/// Whether a report should trigger a low-score alert: it has at least one
/// *scored* verdict (aligned / partially / not aligned) and the alignment score
/// is below `threshold`. Reports with no scored verdicts (no screenshots scored,
/// or all inconclusive) score 0 but reflect *no signal*, so they are NOT
/// flagged — that avoids false alarms for days with nothing to measure.
pub fn is_low_score(report: &AnalysisReport, threshold: f64) -> bool {
    let scored = report.aligned_count + report.partially_count + report.not_aligned_count;
    scored > 0 && report.alignment_score < threshold
}

/// Verdict counts + alignment score for a set of verdicts.
#[derive(Debug, Clone, PartialEq)]
pub struct ReportAggregate {
    pub total_analyzed: i32,
    pub aligned_count: i32,
    pub partially_count: i32,
    pub not_aligned_count: i32,
    pub inconclusive_count: i32,
    pub alignment_score: f64,
}

/// Pure scoring: tally verdicts and compute the 0–100 alignment score.
/// `inconclusive` is counted but excluded from the weighted average; when there
/// are no scored verdicts (empty, or all inconclusive) the score is 0.
pub fn aggregate(verdicts: &[String]) -> ReportAggregate {
    let (mut aligned, mut partial, mut not_aligned, mut inconclusive) = (0i32, 0i32, 0i32, 0i32);
    let (mut weighted_sum, mut scored) = (0.0_f64, 0.0_f64);

    for v in verdicts {
        match v.as_str() {
            "aligned" => {
                aligned += 1;
                weighted_sum += 1.0;
                scored += 1.0;
            }
            "partially_aligned" => {
                partial += 1;
                weighted_sum += 0.5;
                scored += 1.0;
            }
            "not_aligned" => {
                not_aligned += 1;
                // weight 0 — counts toward the denominator only.
                scored += 1.0;
            }
            "inconclusive" => {
                inconclusive += 1; // excluded from the score entirely
            }
            _ => {} // unknown verdicts ignored (DB CHECK prevents these anyway)
        }
    }

    let alignment_score = if scored > 0.0 {
        (weighted_sum / scored) * 100.0
    } else {
        0.0
    };

    ReportAggregate {
        total_analyzed: verdicts.len() as i32,
        aligned_count: aligned,
        partially_count: partial,
        not_aligned_count: not_aligned,
        inconclusive_count: inconclusive,
        alignment_score,
    }
}

/// A short, deterministic summary line derived from the aggregate.
fn summarize(a: &ReportAggregate) -> String {
    if a.total_analyzed == 0 {
        return "No screenshots were analysed for this day.".to_string();
    }
    format!(
        "{} screenshot(s) analysed: {} aligned, {} partially aligned, {} not aligned, \
         {} inconclusive. Alignment score {:.0}%.",
        a.total_analyzed,
        a.aligned_count,
        a.partially_count,
        a.not_aligned_count,
        a.inconclusive_count,
        a.alignment_score,
    )
}

/// Build (or refresh) the daily report for a job from its stored verdicts and
/// persist it. Idempotent via `analysis_reports.upsert` (UNIQUE(user, day)).
pub async fn build_report(
    pool: &PgPool,
    user_id: Uuid,
    day: NaiveDate,
    job_id: Uuid,
    provider: &dyn SummaryProvider,
) -> Result<AnalysisReport, AppError> {
    let results = analysis_results::list_for_job(pool, job_id).await?;
    let verdicts: Vec<String> = results.iter().map(|r| r.verdict.clone()).collect();
    let agg = aggregate(&verdicts);

    // Attribute the report to the model that actually produced the verdicts.
    let model = results
        .iter()
        .map(|r| r.model.as_str())
        .find(|m| !m.is_empty())
        .unwrap_or_else(|| provider.model())
        .to_string();

    // Prefer an AI-generated summary from the rationales; on any failure (model
    // not configured, transport error, repeated malformed output) fall back to
    // the deterministic count line so a report is always produced.
    let rationales: Vec<String> = results.iter().map(|r| r.rationale.clone()).collect();
    let summary_text = match summary_generator::generate_summary(provider, &rationales).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("AI summary unavailable, using deterministic summary: {e}");
            summarize(&agg)
        }
    };

    let input = ReportInput {
        user_id,
        day,
        job_id,
        total_analyzed: agg.total_analyzed,
        aligned_count: agg.aligned_count,
        partially_count: agg.partially_count,
        not_aligned_count: agg.not_aligned_count,
        inconclusive_count: agg.inconclusive_count,
        alignment_score: agg.alignment_score,
        summary_text,
        model,
    };
    analysis_reports::upsert(pool, &input).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn score(items: &[&str]) -> f64 {
        aggregate(&v(items)).alignment_score
    }

    #[test]
    fn empty_scores_zero() {
        let a = aggregate(&[]);
        assert_eq!(a.total_analyzed, 0);
        assert_eq!(a.alignment_score, 0.0);
    }

    #[test]
    fn all_aligned_is_100() {
        assert_eq!(score(&["aligned", "aligned", "aligned"]), 100.0);
    }

    #[test]
    fn all_not_aligned_is_0() {
        assert_eq!(score(&["not_aligned", "not_aligned"]), 0.0);
    }

    #[test]
    fn aligned_and_not_aligned_is_50() {
        // (1 + 0) / 2 * 100
        assert_eq!(score(&["aligned", "not_aligned"]), 50.0);
    }

    #[test]
    fn aligned_and_partial_is_75() {
        // (1 + 0.5) / 2 * 100
        assert_eq!(score(&["aligned", "partially_aligned"]), 75.0);
    }

    #[test]
    fn two_aligned_one_not_is_two_thirds() {
        // (1 + 1 + 0) / 3 * 100 = 66.66…
        assert!((score(&["aligned", "aligned", "not_aligned"]) - 66.6666).abs() < 1e-3);
    }

    #[test]
    fn inconclusive_is_excluded_from_score() {
        // [aligned, inconclusive] → scored set is just the aligned one → 100.
        let a = aggregate(&v(&["aligned", "inconclusive"]));
        assert_eq!(a.alignment_score, 100.0);
        assert_eq!(a.total_analyzed, 2);
        assert_eq!(a.inconclusive_count, 1);
        assert_eq!(a.aligned_count, 1);
    }

    #[test]
    fn all_inconclusive_scores_zero_no_div_by_zero() {
        let a = aggregate(&v(&["inconclusive", "inconclusive"]));
        assert_eq!(a.alignment_score, 0.0);
        assert_eq!(a.inconclusive_count, 2);
    }

    #[test]
    fn full_mix_counts_and_score() {
        // weighted = 1 + 1 + 0.5 + 0 = 2.5 ; scored = 4 → 62.5
        let a = aggregate(&v(&[
            "aligned",
            "aligned",
            "partially_aligned",
            "not_aligned",
            "inconclusive",
        ]));
        assert_eq!(a.total_analyzed, 5);
        assert_eq!(a.aligned_count, 2);
        assert_eq!(a.partially_count, 1);
        assert_eq!(a.not_aligned_count, 1);
        assert_eq!(a.inconclusive_count, 1);
        assert_eq!(a.alignment_score, 62.5);
    }

    #[test]
    fn summary_reflects_counts() {
        let a = aggregate(&v(&["aligned", "not_aligned"]));
        let s = summarize(&a);
        assert!(s.contains("2 screenshot"));
        assert!(s.contains("50%"));
        assert_eq!(
            summarize(&aggregate(&[])),
            "No screenshots were analysed for this day."
        );
    }

    /// Build an `AnalysisReport` from an aggregate for predicate tests.
    fn report_from(verdicts: &[&str]) -> AnalysisReport {
        let a = aggregate(&v(verdicts));
        AnalysisReport {
            id: Uuid::nil(),
            user_id: Uuid::nil(),
            day: chrono::NaiveDate::from_ymd_opt(2026, 7, 2).unwrap(),
            job_id: Uuid::nil(),
            total_analyzed: a.total_analyzed,
            aligned_count: a.aligned_count,
            partially_count: a.partially_count,
            not_aligned_count: a.not_aligned_count,
            inconclusive_count: a.inconclusive_count,
            alignment_score: a.alignment_score,
            summary_text: String::new(),
            model: String::new(),
            created_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn low_score_flags_below_threshold_with_real_signal() {
        // Two not_aligned + one aligned → 33.3% < 35% and has scored verdicts.
        let r = report_from(&["not_aligned", "not_aligned", "aligned"]);
        assert!(is_low_score(&r, 35.0));
    }

    #[test]
    fn at_or_above_threshold_is_not_flagged() {
        // 50% is above 35%.
        assert!(!is_low_score(
            &report_from(&["aligned", "not_aligned"]),
            35.0
        ));
        // Exactly at the threshold is not "below".
        let mut r = report_from(&["aligned", "not_aligned"]);
        r.alignment_score = 35.0;
        assert!(!is_low_score(&r, 35.0));
    }

    #[test]
    fn no_scored_verdicts_is_never_flagged() {
        // All inconclusive → score 0 but no signal → not an alert.
        assert!(!is_low_score(
            &report_from(&["inconclusive", "inconclusive"]),
            35.0
        ));
        // Empty day → not an alert.
        assert!(!is_low_score(&report_from(&[]), 35.0));
    }

    #[test]
    fn zero_percent_with_real_signal_is_flagged() {
        // All not_aligned → genuine 0% low score.
        assert!(is_low_score(
            &report_from(&["not_aligned", "not_aligned"]),
            35.0
        ));
    }
}

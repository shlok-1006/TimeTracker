//! Daily report aggregation (Feature 1 Phase 2).
//!
//! Reads a job's per-screenshot verdicts (`analysis_results`) and produces the
//! aggregate persisted in `analysis_reports`: verdict counts plus a 0–100
//! alignment score.
//!
//! Scoring weights: `aligned` = 1, `partially_aligned` = `PARTIAL_WEIGHT`,
//! `not_aligned` = 0. `inconclusive` verdicts are EXCLUDED from the score (they
//! still count toward `total_analyzed`). The score is the weighted average of the
//! *scored* verdicts multiplied by 100.
//!
//! Partial credit is deliberately generous (0.75, not 0.5): the analyzer emits
//! `partially_aligned` for genuine work it cannot pin to a specific ticket — a
//! sub-task, research, or supporting activity — and that is real work, so it
//! should score close to fully aligned rather than halfway to unproductive.
//!
//! IDLE (migration 0061, HR 4 Nov 2026): the day score also accounts for idle time. Only idle INSIDE the working
//! span counts — between the day's first and last active/meeting minute — so a timer left running after someone
//! leaves (the desktop has no idle auto-stop) or a second machine idling overnight doesn't sink the day. Of that,
//! the first `idle_allowance_seconds()` (30 min) are free; beyond it the screenshot alignment is scaled by
//! worked / (worked + extra idle) — see `idle_adjusted` and `day_idle`. The stored `alignment_score` is that day score (the Daily
//! Report, the low-score alert and the monthly average all read it); `screen_alignment_score` keeps the
//! screenshot-only figure beside it.

use chrono::NaiveDate;
use sqlx::PgPool;
use uuid::Uuid;

use crate::db::analysis_reports::{self, AnalysisReport, ReportInput};
use crate::db::{analysis_results, attendance};
use crate::error::AppError;
use crate::org_time;
use crate::summary_generator::{self, SummaryProvider};

/// Alignment score (0–100) below which HR is alerted about a daily report.
/// Overridable via `TIMETRACKER_LOW_SCORE_THRESHOLD`.
pub const DEFAULT_LOW_SCORE_THRESHOLD: f64 = 35.0;

/// Score credit for a `partially_aligned` verdict — genuine work the analyzer
/// could not tie to a specific ticket (a sub-task or supporting activity).
/// Set generously in the employee's favour: much closer to aligned than to zero.
pub const PARTIAL_WEIGHT: f64 = 0.75;

/// Idle a day may have before it lowers the score — the same 30 minutes the HRMS Daily Report's idle list uses.
/// Overridable via `TIMETRACKER_IDLE_ALLOWANCE_MINUTES`.
pub const DEFAULT_IDLE_ALLOWANCE_SECONDS: i64 = 30 * 60;

/// The configured idle allowance in seconds (≥ 0), env-overridable in whole minutes.
pub fn idle_allowance_seconds() -> i64 {
    std::env::var("TIMETRACKER_IDLE_ALLOWANCE_MINUTES")
        .ok()
        .and_then(|s| s.trim().parse::<i64>().ok())
        .filter(|m| *m >= 0)
        .map(|m| m.min(24 * 60) * 60) // a day at most — and no overflow from a silly value
        .unwrap_or(DEFAULT_IDLE_ALLOWANCE_SECONDS)
}

/// The day score: the screenshot alignment scaled down by idle beyond the allowance,
///
/// ```text
/// screen × worked / (worked + max(0, idle − allowance))
/// ```
///
/// e.g. 80% with 8h worked and 1h30m idle (1h over) → 80 × 8 / 9 = 71.1. Idle inside the allowance changes
/// nothing. With no worked time there is nothing to weigh idle against, so the screenshot score stands (a day is
/// only analysed when there were working screenshots, so this is the "attendance not rolled up yet" case).
/// Never above the screenshot score, never below 0.
pub fn idle_adjusted(
    screen: f64,
    worked_seconds: i64,
    idle_seconds: i64,
    allowance_seconds: i64,
) -> f64 {
    let extra = (idle_seconds - allowance_seconds).max(0);
    if worked_seconds <= 0 || extra == 0 {
        return screen;
    }
    let worked = worked_seconds as f64;
    (screen * worked / (worked + extra as f64)).clamp(0.0, screen.max(0.0))
}

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
                weighted_sum += PARTIAL_WEIGHT;
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

/// The deterministic summary, plus how much idle took off when it did.
fn summarize_with_idle(
    a: &ReportAggregate,
    day_score: f64,
    idle_seconds: i64,
    allowance_seconds: i64,
) -> String {
    let base = summarize(a);
    if a.total_analyzed == 0 || day_score >= a.alignment_score {
        return base;
    }
    format!(
        "{base} Day score {:.0}% after {} idle ({} over the {} allowance).",
        day_score,
        hm(idle_seconds),
        hm(idle_seconds - allowance_seconds),
        hm(allowance_seconds)
    )
}

/// "1h 05m".
fn hm(secs: i64) -> String {
    let secs = secs.max(0);
    format!("{}h {:02}m", secs / 3600, (secs % 3600) / 60)
}

/// Worked seconds (active + meeting) for the org/IST `day`, and the idle that counts: idle INSIDE the working span —
/// from the first to the last active/meeting minute. Attendance's own rule and multi-device de-overlap
/// (`attendance::day_activity`, interval_seconds). Idle before anyone started or after they stopped (a timer left
/// on, a second machine idling) is not "idle at work", so it doesn't lower the score. No work → (0, 0).
pub async fn day_idle(
    pool: &PgPool,
    user_id: Uuid,
    day: NaiveDate,
) -> Result<(i64, i64), AppError> {
    let (day_start, day_end) = org_time::day_bounds_utc(day);
    let whole = attendance::day_activity(pool, user_id, day_start, day_end).await?;
    let (Some(first_in), Some(last_out)) = (whole.first_in_utc, whole.last_out_utc) else {
        return Ok((whole.worked_seconds, 0));
    };
    if last_out <= first_in {
        return Ok((whole.worked_seconds, 0));
    }
    let span = attendance::day_activity(pool, user_id, first_in, last_out).await?;
    Ok((whole.worked_seconds, span.idle_seconds))
}

/// Re-score an existing report's idle without re-running the analysis or the AI summary — for days whose activity
/// reached the server after the report was built (a late or VPN sync). Keeps the screenshot score and summary;
/// recomputes worked, in-span idle and the day score, and stamps `created_at` so the same late data isn't picked up
/// again. Returns the refreshed report.
pub async fn rescore_idle(
    pool: &PgPool,
    report: &AnalysisReport,
) -> Result<AnalysisReport, AppError> {
    let screen = report
        .screen_alignment_score
        .unwrap_or(report.alignment_score);
    let (worked, idle) = day_idle(pool, report.user_id, report.day).await?;
    let day_score = idle_adjusted(screen, worked, idle, idle_allowance_seconds());
    analysis_reports::set_idle_score(
        pool,
        report.user_id,
        report.day,
        day_score,
        screen,
        clamp_i32(worked),
        clamp_i32(idle),
    )
    .await
}

/// Seconds as the INTEGER column stores them (a day never comes near i32::MAX; saturate rather than wrap).
fn clamp_i32(v: i64) -> i32 {
    i32::try_from(v.max(0)).unwrap_or(i32::MAX)
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
    // Worked and in-span idle for the day. A failed read must not lose the report: score on screenshots alone, and
    // store the worked/idle columns as NULL ("unknown"), not as a real zero.
    let activity = match day_idle(pool, user_id, day).await {
        Ok(a) => Some(a),
        Err(e) => {
            tracing::warn!(
                "day activity unavailable for {user_id} on {day}, scoring screenshots only: {e}"
            );
            None
        }
    };
    let (worked, idle) = activity.unwrap_or((0, 0));
    let day_score = idle_adjusted(agg.alignment_score, worked, idle, idle_allowance_seconds());

    let rationales: Vec<String> = results.iter().map(|r| r.rationale.clone()).collect();
    let summary_text = match summary_generator::generate_summary(provider, &rationales).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("AI summary unavailable, using deterministic summary: {e}");
            summarize_with_idle(&agg, day_score, idle, idle_allowance_seconds())
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
        alignment_score: day_score,
        screen_alignment_score: agg.alignment_score,
        worked_seconds: activity.map(|(w, _)| clamp_i32(w)),
        idle_seconds: activity.map(|(_, i)| clamp_i32(i)),
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
    fn aligned_and_partial_is_875() {
        // (1 + 0.75) / 2 * 100 — partial credit is the generous 0.75.
        assert_eq!(score(&["aligned", "partially_aligned"]), 87.5);
    }

    #[test]
    fn one_partial_alone_is_the_partial_weight() {
        // A day of nothing but unattributed-but-genuine work still scores well.
        assert_eq!(score(&["partially_aligned"]), PARTIAL_WEIGHT * 100.0);
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
        // weighted = 1 + 1 + 0.75 + 0 = 2.75 ; scored = 4 → 68.75
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
        assert_eq!(a.alignment_score, 68.75);
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
            screen_alignment_score: Some(a.alignment_score),
            worked_seconds: None,
            idle_seconds: None,
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

    const H: i64 = 3600;

    #[test]
    fn idle_inside_the_allowance_changes_nothing() {
        assert_eq!(idle_adjusted(80.0, 8 * H, 20 * 60, 30 * 60), 80.0);
        assert_eq!(idle_adjusted(80.0, 8 * H, 30 * 60, 30 * 60), 80.0);
    }

    #[test]
    fn idle_beyond_the_allowance_scales_the_score() {
        // 1h30m idle → 1h over: 80 × 8 / 9
        let s = idle_adjusted(80.0, 8 * H, 90 * 60, 30 * 60);
        assert!((s - 80.0 * 8.0 / 9.0).abs() < 1e-9, "{s}");
        // 3h idle → 2h30 over: 80 × 8 / 10.5
        let s = idle_adjusted(80.0, 8 * H, 3 * H, 30 * 60);
        assert!((s - 80.0 * 8.0 / 10.5).abs() < 1e-9, "{s}");
    }

    #[test]
    fn idle_adjustment_edges() {
        // No worked time to weigh idle against → screenshot score stands.
        assert_eq!(idle_adjusted(70.0, 0, 5 * H, 30 * 60), 70.0);
        // A zero day stays zero; never above the screenshot score, never negative.
        assert_eq!(idle_adjusted(0.0, 8 * H, 5 * H, 30 * 60), 0.0);
        let s = idle_adjusted(100.0, 1, 10 * H, 0);
        assert!((0.0..=100.0).contains(&s));
        // No allowance: every idle second counts.
        let s = idle_adjusted(80.0, 8 * H, 2 * H, 0);
        assert!((s - 64.0).abs() < 1e-9, "{s}");
    }

    #[test]
    fn idle_allowance_defaults_to_thirty_minutes() {
        assert_eq!(DEFAULT_IDLE_ALLOWANCE_SECONDS, 1800);
    }

    #[test]
    fn deterministic_summary_mentions_idle_only_when_it_lowered_the_score() {
        let a = aggregate(&v(&["aligned", "not_aligned"]));
        assert!(!summarize_with_idle(&a, a.alignment_score, 10 * 60, 30 * 60).contains("idle"));
        let s = summarize_with_idle(&a, 40.0, 2 * H + 5 * 60, 30 * 60);
        assert!(
            s.contains("Day score 40% after 2h 05m idle (1h 35m over the 0h 30m allowance)."),
            "{s}"
        );
    }
}

//! Nightly analysis scheduler. Once a day (at `RUN_HOUR_UTC`), it samples,
//! analyzes, and builds reports for the *previous* day for every employee who
//! captured working screenshots — so reports appear automatically without anyone
//! calling the on-demand endpoint.
//!
//! Idempotent: sampling never resamples a day and `build_report` upserts, so a
//! repeated run (e.g. after a restart) is safe.

use chrono::{Duration, TimeZone, Utc};

use crate::analysis_service;
use crate::db::analysis_reports::{self, AnalysisReport};
use crate::db::{screenshots, users};
use crate::email_service;
use crate::report_service;
use crate::role::UserRole;
use crate::state::AppState;

/// Hour of day (UTC) to run the nightly batch.
const RUN_HOUR_UTC: u32 = 2;

/// Background loop: sleep until the next run time, then process yesterday.
pub async fn run(state: AppState) {
    loop {
        let wait = duration_until_next_run();
        tracing::info!(
            secs = wait.as_secs(),
            "nightly analysis: sleeping until next run"
        );
        tokio::time::sleep(wait).await;
        run_once(&state).await;
    }
}

fn duration_until_next_run() -> std::time::Duration {
    let now = Utc::now();
    let at = now
        .date_naive()
        .and_hms_opt(RUN_HOUR_UTC, 0, 0)
        .expect("valid run time");
    let today_run = Utc.from_utc_datetime(&at);
    let next = if now < today_run {
        today_run
    } else {
        today_run + Duration::days(1)
    };
    (next - now)
        .to_std()
        .unwrap_or_else(|_| std::time::Duration::from_secs(3600))
}

/// Build yesterday's reports for every employee with working screenshots.
async fn run_once(state: &AppState) {
    if !state.claude.is_configured() {
        tracing::info!("nightly analysis skipped: Claude not configured");
        return;
    }
    let yesterday = (Utc::now() - Duration::days(1)).date_naive();
    let users = match screenshots::working_user_ids_on_day(&state.db, yesterday).await {
        Ok(u) => u,
        Err(e) => {
            tracing::warn!("nightly analysis: could not list users: {e}");
            return;
        }
    };
    tracing::info!(day = %yesterday, employees = users.len(), "nightly analysis: starting");
    for user_id in users {
        match analysis_service::analyze_user_day(
            &state.db,
            &state.storage,
            &state.claude,
            &state.linear,
            user_id,
            yesterday,
        )
        .await
        {
            Ok(o) => {
                tracing::info!(
                    %user_id,
                    analyzed = o.analyzed,
                    skipped = o.skipped,
                    score = o.report.alignment_score,
                    "nightly report built"
                );
                maybe_alert_low_score(state, &o.report).await;
            }
            Err(e) => tracing::warn!(%user_id, "nightly analysis failed: {e}"),
        }
    }
    tracing::info!(day = %yesterday, "nightly analysis: done");
}

/// If a freshly built daily report is below the low-score threshold (and has
/// real scored signal), email HR — once per (employee, day). Best-effort: any
/// failure is logged and never aborts the nightly batch.
async fn maybe_alert_low_score(state: &AppState, report: &AnalysisReport) {
    let threshold = report_service::low_score_threshold();
    if !report_service::is_low_score(report, threshold) {
        return;
    }

    // Idempotent: skip if HR was already alerted for this employee/day.
    match analysis_reports::low_score_notified(&state.db, report.user_id, report.day).await {
        Ok(true) => return,
        Ok(false) => {}
        Err(e) => {
            tracing::warn!(user_id = %report.user_id, "low-score notify check failed: {e}");
            return;
        }
    }

    let employee = match users::find_by_id(&state.db, report.user_id).await {
        Ok(Some(u)) => u,
        Ok(None) => return,
        Err(e) => {
            tracing::warn!(user_id = %report.user_id, "low-score alert: user lookup failed: {e}");
            return;
        }
    };

    // Recipients: all HR + every project manager assigned to the employee
    // (an employee may have several managers, one, or none).
    let mut recipients: Vec<String> = match users::contacts_with_role(&state.db, UserRole::Hr).await
    {
        Ok(hr) => hr.into_iter().map(|(_, email)| email).collect(),
        Err(e) => {
            tracing::warn!("low-score alert: could not load HR recipients: {e}");
            return;
        }
    };
    match users::managers_of(&state.db, report.user_id).await {
        Ok(managers) => recipients.extend(managers.into_iter().map(|(_, _, email)| email)),
        Err(e) => {
            tracing::warn!(user_id = %report.user_id, "low-score alert: PM lookup failed: {e}")
        }
    }
    recipients.sort();
    recipients.dedup();
    if recipients.is_empty() {
        tracing::warn!(user_id = %report.user_id, "low daily score but no HR/PM recipients configured");
        return;
    }

    let email = email_service::LowScoreEmail {
        recipients: &recipients,
        employee_name: &employee.name,
        employee_email: &employee.email,
        day: report.day,
        score: report.alignment_score,
        threshold,
        total_analyzed: report.total_analyzed,
        summary: &report.summary_text,
    };
    match email_service::send_low_score_alert(email).await {
        Ok(()) => {
            if let Err(e) =
                analysis_reports::mark_low_score_notified(&state.db, report.user_id, report.day)
                    .await
            {
                tracing::warn!(user_id = %report.user_id, "low-score notify stamp failed: {e}");
            }
            tracing::info!(
                user_id = %report.user_id,
                day = %report.day,
                score = report.alignment_score,
                "HR alerted: low daily score"
            );
        }
        Err(e) => tracing::warn!(user_id = %report.user_id, "low-score alert email failed: {e}"),
    }
}

//! Nightly analysis scheduler. Once a day (at `RUN_HOUR_UTC`), it samples,
//! analyzes, and builds reports for the *previous* day for every employee who
//! captured working screenshots — so reports appear automatically without anyone
//! calling the on-demand endpoint.
//!
//! Idempotent: sampling never resamples a day and `build_report` upserts, so a
//! repeated run (e.g. after a restart) is safe.

use chrono::{Duration, TimeZone, Utc};

use crate::analysis_service;
use crate::db::screenshots;
use crate::state::AppState;

/// Hour of day (UTC) to run the nightly batch.
const RUN_HOUR_UTC: u32 = 2;

/// Background loop: sleep until the next run time, then process yesterday.
pub async fn run(state: AppState) {
    loop {
        let wait = duration_until_next_run();
        tracing::info!(secs = wait.as_secs(), "nightly analysis: sleeping until next run");
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
    let next = if now < today_run { today_run } else { today_run + Duration::days(1) };
    (next - now).to_std().unwrap_or_else(|_| std::time::Duration::from_secs(3600))
}

/// Build yesterday's reports for every employee with working screenshots.
async fn run_once(state: &AppState) {
    if !state.gemini.is_configured() {
        tracing::info!("nightly analysis skipped: Gemini not configured");
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
            &state.gemini,
            &state.linear,
            user_id,
            yesterday,
        )
        .await
        {
            Ok(o) => tracing::info!(
                %user_id,
                analyzed = o.analyzed,
                skipped = o.skipped,
                score = o.report.alignment_score,
                "nightly report built"
            ),
            Err(e) => tracing::warn!(%user_id, "nightly analysis failed: {e}"),
        }
    }
    tracing::info!(day = %yesterday, "nightly analysis: done");
}

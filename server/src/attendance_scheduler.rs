//! Nightly attendance rollup. Once a day (at `RUN_HOUR_UTC`), derives the
//! *previous* day's attendance for every employee from the interval log, so
//! reports and calendars are ready without anyone hitting an endpoint.
//!
//! Runs an hour after the analysis scheduler to avoid overlapping load.
//! Idempotent: the rollup upserts, so a repeated run is safe.

use chrono::{Duration, TimeZone, Utc};

use crate::attendance_service;
use crate::state::AppState;

/// Hour of day (UTC) to run the nightly rollup.
const RUN_HOUR_UTC: u32 = 3;

pub async fn run(state: AppState) {
    loop {
        let wait = duration_until_next_run();
        tracing::info!(secs = wait.as_secs(), "nightly attendance: sleeping until next run");
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

async fn run_once(state: &AppState) {
    let yesterday = (Utc::now() - Duration::days(1)).date_naive();
    match attendance_service::rollup_all_for_day(&state.db, yesterday).await {
        Ok(n) => tracing::info!(day = %yesterday, employees = n, "nightly attendance rollup done"),
        Err(e) => tracing::warn!(day = %yesterday, "nightly attendance rollup failed: {e}"),
    }
}

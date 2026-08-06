//! Month-end scheduler: once the org-local (IST) month closes, generate every
//! employee's monthly summary automatically so it's waiting for them (and for
//! HR/PMs) without anyone pressing a button.
//!
//! Cadence: the loop wakes hourly and only acts on the configured run hour of
//! the FIRST day of a new month, generating the month that just ended. Hourly
//! wakeups (rather than one long sleep to the next month) mean a restart or a
//! deploy can't cause a month to be skipped — whenever the process is up during
//! the window, the batch runs.
//!
//! Idempotent: `monthly_report_service::build` upserts, and a per-month marker
//! keeps a single process from regenerating the same month on every wakeup
//! inside the run hour. Generating twice would be harmless anyway (same inputs,
//! same numbers) — HR/PM on-demand generation writes the same row.

use chrono::{Datelike, Duration, NaiveDate, Timelike, Utc};
use tokio::sync::Mutex;

use crate::db::users;
use crate::monthly_report_service as service;
use crate::org_time;
use crate::state::AppState;

/// Hour of the org-local day to run the month-end batch. 03:00 IST — after the
/// nightly analyzer (02:00 UTC = 07:30 IST covers the previous day) has had a
/// full cycle, so the final day's report is already in place.
const RUN_HOUR_LOCAL: u32 = 3;

/// Wake this often to check whether the month-end window is open.
const POLL: std::time::Duration = std::time::Duration::from_secs(3600);

/// Background loop.
pub async fn run(state: AppState) {
    // Guards against re-running within the same window in one process.
    let last_done: Mutex<Option<NaiveDate>> = Mutex::new(None);
    loop {
        let now_local = Utc::now() + Duration::minutes(org_time::offset_minutes());
        let today = now_local.date_naive();
        let in_window = today.day() == 1 && now_local.hour() == RUN_HOUR_LOCAL;

        if in_window {
            // The month that just ended = the month containing yesterday.
            let target = service::month_key(today - Duration::days(1));
            let mut guard = last_done.lock().await;
            if *guard != Some(target) {
                drop(guard);
                run_once(&state, target).await;
                guard = last_done.lock().await;
                *guard = Some(target);
            }
        }
        tokio::time::sleep(POLL).await;
    }
}

/// Generate `month` for every user. Per-user failures are logged and never
/// abort the batch — one employee's bad data must not cost everyone their report.
pub async fn run_once(state: &AppState, month: NaiveDate) {
    let users = match users::list_all(&state.db).await {
        Ok(u) => u,
        Err(e) => {
            tracing::warn!("monthly reports: could not list users: {e}");
            return;
        }
    };
    tracing::info!(%month, employees = users.len(), "monthly reports: starting");
    let (mut ok, mut failed) = (0usize, 0usize);
    for u in users {
        // generated_by = None marks it as produced by the scheduler.
        match service::build(&state.db, u.id, month, None).await {
            Ok(_) => ok += 1,
            Err(e) => {
                failed += 1;
                tracing::warn!(user_id = %u.id, %month, "monthly report failed: {e}");
            }
        }
    }
    tracing::info!(%month, generated = ok, failed, "monthly reports: done");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_month_is_the_one_that_just_ended() {
        // On 1 Sep the batch must produce August, and across a year boundary
        // 1 Jan must produce the previous December.
        let sep1 = NaiveDate::from_ymd_opt(2026, 9, 1).unwrap();
        assert_eq!(
            service::month_key(sep1 - Duration::days(1)),
            NaiveDate::from_ymd_opt(2026, 8, 1).unwrap()
        );
        let jan1 = NaiveDate::from_ymd_opt(2027, 1, 1).unwrap();
        assert_eq!(
            service::month_key(jan1 - Duration::days(1)),
            NaiveDate::from_ymd_opt(2026, 12, 1).unwrap()
        );
    }
}

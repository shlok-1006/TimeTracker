//! Weekly hours-compliance job. Every Monday at `RUN_HOUR_UTC` it evaluates the
//! week that just ended (previous Mon–Sun) for every employee and warns HR +
//! the employee's project manager about anyone who fell short of
//! `working_days × 8h`.
//!
//! Runs after the nightly attendance rollup (03:00 UTC) so the whole week is
//! finalized. Idempotent: results upsert and each shortfall warns at most once.

use chrono::{Datelike, Duration, NaiveTime, TimeZone, Utc};

use crate::state::AppState;
use crate::weekly_hours_service;

/// Hour of day (UTC) on Monday to run the weekly check (after the 03:00 rollup).
const RUN_HOUR_UTC: u32 = 6;

pub async fn run(state: AppState) {
    loop {
        let wait = duration_until_next_run(Utc::now());
        tracing::info!(
            secs = wait.as_secs(),
            "weekly hours: sleeping until next Monday run"
        );
        tokio::time::sleep(wait).await;
        run_once(&state).await;
    }
}

/// Duration until the next Monday `RUN_HOUR_UTC`. If called on a Monday before
/// the run hour, that's today; otherwise the following Monday.
fn duration_until_next_run(now: chrono::DateTime<Utc>) -> std::time::Duration {
    let run_time = NaiveTime::from_hms_opt(RUN_HOUR_UTC, 0, 0).expect("valid run time");
    // Days ahead to reach Monday (0 if today is Monday).
    let days_until_monday = (7 - now.weekday().num_days_from_monday() as i64) % 7;
    let candidate_date = now.date_naive() + Duration::days(days_until_monday);
    let candidate = Utc.from_utc_datetime(&candidate_date.and_time(run_time));
    let next = if candidate > now {
        candidate
    } else {
        candidate + Duration::days(7)
    };
    (next - now)
        .to_std()
        .unwrap_or_else(|_| std::time::Duration::from_secs(3600))
}

async fn run_once(state: &AppState) {
    let (week_start, week_end) = weekly_hours_service::previous_week(Utc::now().date_naive());
    match weekly_hours_service::run_for_week(state, week_start, week_end).await {
        Ok(s) => tracing::info!(
            %week_start,
            %week_end,
            evaluated = s.evaluated,
            shortfalls = s.shortfalls,
            warned = s.warned,
            "weekly hours check complete"
        ),
        Err(e) => tracing::warn!(%week_start, %week_end, "weekly hours check failed: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Timelike;

    #[test]
    fn schedules_within_a_week_and_at_the_run_hour() {
        // Wednesday 2026-07-01 12:00 UTC → next run is Monday 2026-07-06 06:00.
        let now = Utc.with_ymd_and_hms(2026, 7, 1, 12, 0, 0).unwrap();
        let wait = duration_until_next_run(now);
        let fire = now + Duration::from_std(wait).unwrap();
        assert_eq!(fire.weekday(), chrono::Weekday::Mon);
        assert_eq!(fire.hour(), RUN_HOUR_UTC);
        assert!(wait.as_secs() > 0 && wait.as_secs() <= 7 * 24 * 3600);
    }

    #[test]
    fn monday_before_run_hour_fires_today() {
        // Monday 2026-07-06 05:00 UTC → fires the same day at 06:00.
        let now = Utc.with_ymd_and_hms(2026, 7, 6, 5, 0, 0).unwrap();
        let fire = now + Duration::from_std(duration_until_next_run(now)).unwrap();
        assert_eq!(fire.date_naive(), now.date_naive());
        assert_eq!(fire.hour(), RUN_HOUR_UTC);
    }

    #[test]
    fn monday_after_run_hour_waits_a_week() {
        // Monday 2026-07-06 07:00 UTC → next fire is Monday 2026-07-13 06:00.
        let now = Utc.with_ymd_and_hms(2026, 7, 6, 7, 0, 0).unwrap();
        let fire = now + Duration::from_std(duration_until_next_run(now)).unwrap();
        assert_eq!(fire.weekday(), chrono::Weekday::Mon);
        assert_eq!(fire.date_naive(), now.date_naive() + Duration::days(7));
    }
}

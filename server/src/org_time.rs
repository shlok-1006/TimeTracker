//! Org-timezone day handling (OKF: org time zone is **Asia/Kolkata**; storage is UTC).
//!
//! A `day` key on reports, sampling, and screenshot day-queries means the ORG-LOCAL
//! calendar day — [00:00, 24:00) IST — not the UTC day (changes: nightly coverage,
//! Tapan sync). IST has no DST, so a fixed offset is exact; `ORG_TZ_OFFSET_MINUTES`
//! (default 330 = UTC+5:30) overrides for other deployments.
//!
//! Attendance keeps its own (UTC-midnight) boundary for now — that drift is tracked
//! separately in the OKF as ATT-05 and must move in lockstep with the rollup.

use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};

const DEFAULT_OFFSET_MINUTES: i64 = 330; // Asia/Kolkata, UTC+5:30

pub fn offset_minutes() -> i64 {
    std::env::var("ORG_TZ_OFFSET_MINUTES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_OFFSET_MINUTES)
}

/// UTC window covering the org-local calendar day `day`: [day 00:00 org, +24h).
/// With the IST default, day X = [X-1 18:30 UTC, X 18:30 UTC).
pub fn day_bounds_utc(day: NaiveDate) -> (DateTime<Utc>, DateTime<Utc>) {
    let midnight_local = day.and_hms_opt(0, 0, 0).expect("valid midnight");
    let start = Utc.from_utc_datetime(&midnight_local) - Duration::minutes(offset_minutes());
    (start, start + Duration::days(1))
}

/// Today's date in the org timezone.
pub fn today() -> NaiveDate {
    (Utc::now() + Duration::minutes(offset_minutes())).date_naive()
}

/// Yesterday's date in the org timezone — the nightly batch's target day.
pub fn yesterday() -> NaiveDate {
    today() - Duration::days(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ist_day_window_is_shifted_back_530() {
        // Can't set the env var safely in parallel tests; assert with the default.
        let day = NaiveDate::from_ymd_opt(2026, 8, 4).unwrap();
        let (from, to) = day_bounds_utc(day);
        assert_eq!(from.to_rfc3339(), "2026-08-03T18:30:00+00:00");
        assert_eq!(to.to_rfc3339(), "2026-08-04T18:30:00+00:00");
    }
}

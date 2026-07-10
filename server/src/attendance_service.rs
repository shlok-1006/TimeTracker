//! Attendance business logic (Feature 6C): derive a day's attendance status
//! from the interval log, integrating approved leave and company holidays.
//!
//! Precedence: worked time wins — at/above the present threshold the day is
//! `present` (migration 0021 removed the "partial" tier: a started timer counts
//! as present). Below it, we explain the day as leave → holiday → weekend →
//! absent. `status` must stay within the `attendance_days_status_check`
//! constraint: present | absent | leave | holiday | weekend.

use chrono::{Datelike, Duration, NaiveDate, TimeZone, Utc, Weekday};
use sqlx::PgPool;
use uuid::Uuid;

use crate::db::{attendance, leave};
use crate::error::AppError;

/// Minimum worked seconds for a day to count as **present**. Policy: simply
/// running the tracker for the day (a couple of minutes) marks it present — so
/// this is 2 minutes, not a full work day. Overridable via
/// `TIMETRACKER_ATTENDANCE_PRESENT_SECONDS`.
pub const DEFAULT_PRESENT_THRESHOLD_SECONDS: i64 = 120;

/// The present threshold (seconds), from env or the 2-minute default.
fn present_threshold_seconds() -> i64 {
    std::env::var("TIMETRACKER_ATTENDANCE_PRESENT_SECONDS")
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
        .filter(|s| *s > 0)
        .unwrap_or(DEFAULT_PRESENT_THRESHOLD_SECONDS)
}

/// UTC `[start, end)` bounds for a calendar day.
fn day_bounds(day: NaiveDate) -> (chrono::DateTime<Utc>, chrono::DateTime<Utc>) {
    let start = Utc.from_utc_datetime(&day.and_hms_opt(0, 0, 0).expect("valid midnight"));
    (start, start + Duration::days(1))
}

fn is_weekend(day: NaiveDate) -> bool {
    matches!(day.weekday(), Weekday::Sat | Weekday::Sun)
}

/// Derive (status, note) from worked time + leave/holiday/weekend context.
fn derive_status(
    worked_seconds: i64,
    present_threshold: i64,
    leave_type: Option<&str>,
    holiday_name: Option<&str>,
    day: NaiveDate,
) -> (&'static str, String) {
    // No "partial" tier — migration 0021 dropped it from the CHECK constraint,
    // so producing it here would 500 on insert. At/above the threshold => present;
    // otherwise explain the (sub-threshold) day as leave/holiday/weekend/absent.
    if worked_seconds >= present_threshold {
        ("present", String::new())
    } else if let Some(lt) = leave_type {
        ("leave", lt.to_string())
    } else if let Some(h) = holiday_name {
        ("holiday", h.to_string())
    } else if is_weekend(day) {
        ("weekend", String::new())
    } else {
        ("absent", String::new())
    }
}

/// Recompute and persist one employee's attendance for a single UTC day.
pub async fn rollup_day(
    pool: &PgPool,
    user_id: Uuid,
    day: NaiveDate,
) -> Result<attendance::AttendanceDay, AppError> {
    let (start, end) = day_bounds(day);
    let activity = attendance::day_activity(pool, user_id, start, end).await?;

    // Only look up leave/holiday when there's no work to explain (saves queries).
    let (leave_type, holiday_name) = if activity.worked_seconds > 0 {
        (None, None)
    } else {
        (
            leave::approved_leave_type_on_day(pool, user_id, day).await?,
            leave::holiday_name_on_day(pool, day).await?,
        )
    };

    let (status, note) = derive_status(
        activity.worked_seconds,
        present_threshold_seconds(),
        leave_type.as_deref(),
        holiday_name.as_deref(),
        day,
    );

    attendance::upsert(
        pool,
        user_id,
        day,
        status,
        activity.worked_seconds as i32,
        activity.idle_seconds as i32,
        activity.first_in_utc,
        activity.last_out_utc,
        &note,
    )
    .await?;

    attendance::get(pool, user_id, day)
        .await?
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("attendance row vanished after upsert")))
}

/// Ensure `[from, to]` (capped at today) is rolled up for a user: compute any
/// missing past days once, and always refresh today (it's still live). Lets the
/// calendar show data immediately without waiting for the nightly job.
///
/// The range is also clamped to the user's account-creation date: attendance
/// never predates the account, so new hires don't get "absent" backfill — and
/// an admin can grant a fresh start by bumping `users.created_at` (old rows,
/// once deleted, are never re-derived).
pub async fn ensure_range(
    pool: &PgPool,
    user_id: Uuid,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<(), AppError> {
    let today = Utc::now().date_naive();
    let from = match crate::db::users::find_by_id(pool, user_id).await? {
        Some(u) => from.max(u.created_at.date_naive()),
        None => from,
    };
    let end = to.min(today);
    if end < from {
        return Ok(());
    }
    let existing: std::collections::HashSet<NaiveDate> =
        attendance::existing_days(pool, user_id, from, end)
            .await?
            .into_iter()
            .collect();

    let mut day = from;
    while day <= end {
        if day == today || !existing.contains(&day) {
            rollup_day(pool, user_id, day).await?;
        }
        day += Duration::days(1);
    }
    Ok(())
}

/// Roll up a single day for every employee — plus any HR/PM who tracked time
/// that day (the desktop app accepts all roles) — the nightly batch.
pub async fn rollup_all_for_day(pool: &PgPool, day: NaiveDate) -> Result<usize, AppError> {
    let (start, end) = day_bounds(day);
    let ids = crate::db::users::attendance_rollup_ids(pool, start, end).await?;
    let mut done = 0;
    for user_id in ids {
        match rollup_day(pool, user_id, day).await {
            Ok(_) => done += 1,
            Err(e) => tracing::warn!(%user_id, %day, "attendance rollup failed: {e}"),
        }
    }
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    const T: i64 = DEFAULT_PRESENT_THRESHOLD_SECONDS; // 2 minutes

    #[test]
    fn tracking_at_least_two_minutes_is_present() {
        // Exactly the threshold, and well over it, both count as present.
        assert_eq!(derive_status(T, T, None, None, d(2026, 6, 8)).0, "present");
        assert_eq!(
            derive_status(5 * 3600, T, None, None, d(2026, 6, 8)).0,
            "present"
        );
    }

    #[test]
    fn tracking_under_two_minutes_is_not_present() {
        // No "partial" tier (migration 0021). Sub-threshold on a weekday with no
        // leave/holiday falls through to absent — never "partial", which the
        // attendance_days_status_check constraint would reject.
        assert_eq!(derive_status(30, T, None, None, d(2026, 6, 8)).0, "absent");
        assert_eq!(
            derive_status(T - 1, T, None, None, d(2026, 6, 8)).0,
            "absent"
        );
    }

    #[test]
    fn status_never_partial() {
        // Guard against reintroducing a status the DB constraint rejects.
        const ALLOWED: [&str; 5] = ["present", "absent", "leave", "holiday", "weekend"];
        for worked in [0i64, 1, 59, T - 1, T, T + 1, 10 * 3600] {
            for (lt, hol) in [(None, None), (Some("Annual"), None), (None, Some("NY"))] {
                for day in [d(2026, 6, 8), d(2026, 6, 13)] {
                    let (s, _) = derive_status(worked, T, lt, hol, day);
                    assert!(ALLOWED.contains(&s), "disallowed status {s:?}");
                }
            }
        }
    }

    #[test]
    fn work_overrides_leave_and_holiday() {
        // A weekend day with work still counts as present.
        let (s, _) = derive_status(T, T, Some("Annual"), Some("X"), d(2026, 6, 13));
        assert_eq!(s, "present");
    }

    #[test]
    fn no_work_prefers_leave_then_holiday_then_weekend_then_absent() {
        assert_eq!(
            derive_status(0, T, Some("Sick"), Some("NY"), d(2026, 6, 8)).0,
            "leave"
        );
        assert_eq!(
            derive_status(0, T, None, Some("New Year"), d(2026, 6, 8)).0,
            "holiday"
        );
        assert_eq!(derive_status(0, T, None, None, d(2026, 6, 13)).0, "weekend"); // Saturday
        assert_eq!(derive_status(0, T, None, None, d(2026, 6, 8)).0, "absent"); // Monday
    }

    #[test]
    fn leave_note_carries_type_name() {
        let (s, note) = derive_status(0, T, Some("Annual Leave"), None, d(2026, 6, 8));
        assert_eq!(s, "leave");
        assert_eq!(note, "Annual Leave");
    }
}

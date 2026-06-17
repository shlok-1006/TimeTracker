//! Attendance business logic (Feature 6C): derive a day's attendance status
//! from the interval log, integrating approved leave and company holidays.
//!
//! Precedence: actual worked time always wins (present/partial). Only when there
//! is *no* work do we explain the day as leave → holiday → weekend → absent.

use chrono::{Datelike, Duration, NaiveDate, TimeZone, Utc, Weekday};
use sqlx::PgPool;
use uuid::Uuid;

use crate::db::{attendance, leave};
use crate::error::AppError;

/// Worked seconds at/above which a day counts as a full "present" day.
pub const FULL_DAY_SECONDS: i64 = 6 * 3600;

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
    leave_type: Option<&str>,
    holiday_name: Option<&str>,
    day: NaiveDate,
) -> (&'static str, String) {
    if worked_seconds >= FULL_DAY_SECONDS {
        ("present", String::new())
    } else if worked_seconds > 0 {
        ("partial", String::new())
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

    let (status, note) =
        derive_status(activity.worked_seconds, leave_type.as_deref(), holiday_name.as_deref(), day);

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
pub async fn ensure_range(
    pool: &PgPool,
    user_id: Uuid,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<(), AppError> {
    let today = Utc::now().date_naive();
    let end = to.min(today);
    if end < from {
        return Ok(());
    }
    let existing: std::collections::HashSet<NaiveDate> =
        attendance::existing_days(pool, user_id, from, end).await?.into_iter().collect();

    let mut day = from;
    while day <= end {
        if day == today || !existing.contains(&day) {
            rollup_day(pool, user_id, day).await?;
        }
        day += Duration::days(1);
    }
    Ok(())
}

/// Roll up a single day for every employee (the nightly batch).
pub async fn rollup_all_for_day(pool: &PgPool, day: NaiveDate) -> Result<usize, AppError> {
    let ids = crate::db::users::employee_ids(pool).await?;
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

    #[test]
    fn full_day_is_present() {
        let (s, _) = derive_status(FULL_DAY_SECONDS, None, None, d(2026, 6, 8));
        assert_eq!(s, "present");
    }

    #[test]
    fn some_work_is_partial() {
        let (s, _) = derive_status(3600, None, None, d(2026, 6, 8));
        assert_eq!(s, "partial");
    }

    #[test]
    fn work_overrides_leave_and_holiday() {
        // A weekend day with work still counts as worked.
        let (s, _) = derive_status(FULL_DAY_SECONDS, Some("Annual"), Some("X"), d(2026, 6, 13));
        assert_eq!(s, "present");
    }

    #[test]
    fn no_work_prefers_leave_then_holiday_then_weekend_then_absent() {
        assert_eq!(derive_status(0, Some("Sick"), Some("NY"), d(2026, 6, 8)).0, "leave");
        assert_eq!(derive_status(0, None, Some("New Year"), d(2026, 6, 8)).0, "holiday");
        assert_eq!(derive_status(0, None, None, d(2026, 6, 13)).0, "weekend"); // Saturday
        assert_eq!(derive_status(0, None, None, d(2026, 6, 8)).0, "absent"); // Monday
    }

    #[test]
    fn leave_note_carries_type_name() {
        let (s, note) = derive_status(0, Some("Annual Leave"), None, d(2026, 6, 8));
        assert_eq!(s, "leave");
        assert_eq!(note, "Annual Leave");
    }
}

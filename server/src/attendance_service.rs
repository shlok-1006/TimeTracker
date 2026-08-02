//! Attendance business logic (Feature 6C): derive a day's attendance status
//! from the interval log, integrating approved leave and company holidays.
//!
//! Precedence: approved leave and holidays explain the day first; **weekends are
//! never a work day** — a Sat/Sun stays `weekend` even if the employee tracked
//! time, so it never counts as present or partial. Otherwise tracked time wins:
//! a day **in progress** (today) with any tracked time is `present` (starting the
//! tracker is enough — they may still reach a full day); once **complete**, under
//! the full-day threshold (default 4h) it is `partial` (a half day), at/above it
//! `present`. With none of the above the day is `absent`. `status` must stay
//! within the `attendance_days_status_check` constraint: present | partial |
//! absent | leave | holiday | weekend.

use chrono::{Datelike, Duration, NaiveDate, TimeZone, Utc, Weekday};
use sqlx::PgPool;
use uuid::Uuid;

use crate::db::{attendance, leave};
use crate::error::AppError;

/// Tracked seconds at/above which a completed day is a full `present` day;
/// below it (but > 0) the completed day is `partial`. Default 4 hours,
/// overridable via `TIMETRACKER_ATTENDANCE_FULL_DAY_SECONDS`.
pub const DEFAULT_FULL_DAY_SECONDS: i64 = 4 * 3600;

/// The full-day threshold (seconds), from env or the 4-hour default.
fn full_day_seconds() -> i64 {
    std::env::var("TIMETRACKER_ATTENDANCE_FULL_DAY_SECONDS")
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
        .filter(|s| *s > 0)
        .unwrap_or(DEFAULT_FULL_DAY_SECONDS)
}

/// UTC `[start, end)` bounds for a calendar day.
fn day_bounds(day: NaiveDate) -> (chrono::DateTime<Utc>, chrono::DateTime<Utc>) {
    let start = Utc.from_utc_datetime(&day.and_hms_opt(0, 0, 0).expect("valid midnight"));
    (start, start + Duration::days(1))
}

fn is_weekend(day: NaiveDate) -> bool {
    matches!(day.weekday(), Weekday::Sat | Weekday::Sun)
}

/// Derive (status, note) from tracked time + leave/holiday/weekend context.
/// `tracked_seconds` is the day's total tracked time (active + idle).
/// `full_day_seconds` is the threshold below which a *complete* day is partial.
/// `day_complete` is false for the day still in progress (today).
fn derive_status(
    tracked_seconds: i64,
    full_day_seconds: i64,
    day_complete: bool,
    leave_type: Option<&str>,
    holiday_name: Option<&str>,
    day: NaiveDate,
) -> (&'static str, String) {
    // Approved leave and holidays explain the day first (only ever supplied when
    // there was no tracked time). Weekends are never a work day: a Sat/Sun stays
    // `weekend` even when the employee tracked time — it must not count as present
    // or partial.
    if let Some(lt) = leave_type {
        ("leave", lt.to_string())
    } else if let Some(h) = holiday_name {
        ("holiday", h.to_string())
    } else if is_weekend(day) {
        ("weekend", String::new())
    } else if tracked_seconds > 0 {
        // A day still in progress is optimistically present (they may yet reach
        // a full day); once complete, under the threshold makes it a half day.
        if day_complete && tracked_seconds < full_day_seconds {
            ("partial", String::new())
        } else {
            ("present", String::new())
        }
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

    // Any tracked time — active or idle — means the user started their day, so
    // it counts as present. Only look up leave/holiday when there was none.
    let tracked_seconds = activity.worked_seconds + activity.idle_seconds;
    let (leave_type, holiday_name) = if tracked_seconds > 0 {
        (None, None)
    } else {
        (
            leave::approved_leave_type_on_day(pool, user_id, day).await?,
            leave::holiday_name_on_day(pool, day).await?,
        )
    };

    // The current day is still in progress; only a completed day can be partial.
    let day_complete = day < Utc::now().date_naive();
    let (status, note) = derive_status(
        tracked_seconds,
        full_day_seconds(),
        day_complete,
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

/// Pin a manual HR override for a user/day. Keeps the real activity numbers
/// (worked/idle/clock) but stores the HR-chosen `status` + `note`, marked so the
/// nightly rollup and the recompute-today path leave it untouched.
pub async fn override_day(
    pool: &PgPool,
    user_id: Uuid,
    day: NaiveDate,
    status: &str,
    note: &str,
    overridden_by: Uuid,
) -> Result<attendance::AttendanceDay, AppError> {
    let (start, end) = day_bounds(day);
    let activity = attendance::day_activity(pool, user_id, start, end).await?;
    attendance::upsert_override(
        pool,
        user_id,
        day,
        status,
        activity.worked_seconds as i32,
        activity.idle_seconds as i32,
        activity.first_in_utc,
        activity.last_out_utc,
        note,
        overridden_by,
    )
    .await?;
    attendance::get(pool, user_id, day).await?.ok_or_else(|| {
        AppError::Internal(anyhow::anyhow!("attendance row vanished after override"))
    })
}

/// Clear a user/day override and recompute the derived status from intervals.
pub async fn clear_override(
    pool: &PgPool,
    user_id: Uuid,
    day: NaiveDate,
) -> Result<attendance::AttendanceDay, AppError> {
    attendance::clear_override(pool, user_id, day).await?;
    // With the override cleared, the derived upsert is free to refresh the row.
    rollup_day(pool, user_id, day).await
}

/// Mark the user present for **today** the moment they start tracking — driven
/// by the presence heartbeat, so attendance appears immediately without the
/// employee opening the attendance section. Idempotent and cheap: it no-ops if
/// the day is already present or is an HR override. UTC day (matches the rest of
/// the attendance model).
pub async fn mark_present_today(pool: &PgPool, user_id: Uuid) -> Result<(), AppError> {
    let today = Utc::now().date_naive();
    // Weekends never count as a work day — don't auto-mark present on Sat/Sun.
    // (The rollup will derive `weekend`; see derive_status.)
    if is_weekend(today) {
        return Ok(());
    }
    if let Some(row) = attendance::get(pool, user_id, today).await? {
        // Leave an HR edit alone, and skip the write if it's already present.
        if row.is_override || row.status == "present" {
            return Ok(());
        }
    }
    let (start, end) = day_bounds(today);
    let activity = attendance::day_activity(pool, user_id, start, end).await?;
    // Force present regardless of how little has synced yet — the act of
    // tracking is the signal. (The derived upsert still skips HR overrides.)
    attendance::upsert(
        pool,
        user_id,
        today,
        "present",
        activity.worked_seconds as i32,
        activity.idle_seconds as i32,
        activity.first_in_utc,
        activity.last_out_utc,
        "",
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    const FULL: i64 = DEFAULT_FULL_DAY_SECONDS; // 4 hours

    // 2026-06-08 is a Monday (weekday); 2026-06-13 is a Saturday (weekend).
    fn weekday() -> NaiveDate {
        d(2026, 6, 8)
    }
    fn weekend() -> NaiveDate {
        d(2026, 6, 13)
    }

    #[test]
    fn in_progress_day_with_any_work_is_present() {
        // Today is optimistically present even below the full-day threshold —
        // the employee may still reach a full day.
        assert_eq!(
            derive_status(1, FULL, false, None, None, weekday()).0,
            "present"
        );
        assert_eq!(
            derive_status(3600, FULL, false, None, None, weekday()).0,
            "present"
        );
    }

    #[test]
    fn completed_day_under_threshold_is_partial() {
        // < 4h on a finished day is a half (partial) day; >= 4h is a full present day.
        assert_eq!(
            derive_status(3600, FULL, true, None, None, weekday()).0,
            "partial"
        );
        assert_eq!(
            derive_status(FULL - 1, FULL, true, None, None, weekday()).0,
            "partial"
        );
        assert_eq!(
            derive_status(FULL, FULL, true, None, None, weekday()).0,
            "present"
        );
        assert_eq!(
            derive_status(6 * 3600, FULL, true, None, None, weekday()).0,
            "present"
        );
    }

    #[test]
    fn no_work_is_not_present() {
        // Zero tracked time on a plain weekday is absent — never partial.
        assert_eq!(
            derive_status(0, FULL, true, None, None, weekday()).0,
            "absent"
        );
        assert_eq!(
            derive_status(0, FULL, false, None, None, weekday()).0,
            "absent"
        );
    }

    #[test]
    fn status_within_constraint() {
        // Every branch must produce a value the CHECK constraint allows.
        const ALLOWED: [&str; 6] = [
            "present", "partial", "absent", "leave", "holiday", "weekend",
        ];
        for tracked in [0i64, 1, 59, FULL - 1, FULL, 10 * 3600] {
            for complete in [true, false] {
                for (lt, hol) in [(None, None), (Some("Annual"), None), (None, Some("NY"))] {
                    for day in [weekday(), weekend()] {
                        let (s, _) = derive_status(tracked, FULL, complete, lt, hol, day);
                        assert!(ALLOWED.contains(&s), "disallowed status {s:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn weekend_work_is_never_present_or_partial() {
        // Tracking on a Saturday/Sunday must not count as a work day — the day
        // stays `weekend`, whether in progress or complete, full or partial.
        assert_eq!(
            derive_status(1, FULL, false, None, None, weekend()).0,
            "weekend"
        );
        assert_eq!(
            derive_status(3600, FULL, true, None, None, weekend()).0,
            "weekend"
        );
        assert_eq!(
            derive_status(FULL, FULL, true, None, None, weekend()).0,
            "weekend"
        );
        assert_eq!(
            derive_status(8 * 3600, FULL, false, None, None, weekend()).0,
            "weekend"
        );
    }

    #[test]
    fn no_work_prefers_leave_then_holiday_then_weekend_then_absent() {
        assert_eq!(
            derive_status(0, FULL, true, Some("Sick"), Some("NY"), weekday()).0,
            "leave"
        );
        assert_eq!(
            derive_status(0, FULL, true, None, Some("New Year"), weekday()).0,
            "holiday"
        );
        assert_eq!(
            derive_status(0, FULL, true, None, None, weekend()).0,
            "weekend"
        );
        assert_eq!(
            derive_status(0, FULL, true, None, None, weekday()).0,
            "absent"
        );
    }

    #[test]
    fn leave_note_carries_type_name() {
        let (s, note) = derive_status(0, FULL, true, Some("Annual Leave"), None, weekday());
        assert_eq!(s, "leave");
        assert_eq!(note, "Annual Leave");
    }
}

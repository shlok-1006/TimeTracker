//! Weekly hours-compliance ("40-hour rule").
//!
//! Each employee is expected to work `working_days × 8h` in a Mon–Sun week, so
//! a full 5-working-day week is 40h. Working days are business days (Mon–Fri)
//! that were not weekend, holiday, or approved leave — approved leave and
//! holidays lower the requirement (2 leave days ⇒ 24h expected).
//!
//! This module holds the pure calculation (`required_seconds` / `evaluate`) and
//! the weekly batch (`run_for_week`) that persists results and warns HR + the
//! employee's project manager about any shortfall. The Monday-morning scheduler
//! drives it for the week that just ended.

use chrono::{Datelike, Duration, NaiveDate};

use crate::db::{users, weekly_hours};
use crate::email_service;
use crate::error::AppError;
use crate::role::UserRole;
use crate::state::AppState;

/// Hours an employee is expected to work per working day.
pub const HOURS_PER_WORKING_DAY: i64 = 8;
/// Seconds expected per working day (8h). 5 working days ⇒ 40h.
pub const REQUIRED_SECONDS_PER_WORKING_DAY: i64 = HOURS_PER_WORKING_DAY * 3600;

/// Outcome of the weekly hours check for one employee.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WeeklyHours {
    pub working_days: i64,
    pub required_seconds: i64,
    pub worked_seconds: i64,
    pub shortfall_seconds: i64,
    pub compliant: bool,
}

/// Required work time for a number of working days: `working_days × 8h`.
/// (5 working days ⇒ 144_000s = 40h; 0 working days ⇒ 0.)
pub fn required_seconds(working_days: i64) -> i64 {
    working_days.max(0) * REQUIRED_SECONDS_PER_WORKING_DAY
}

/// Compare an employee's worked time against the requirement for the week.
/// Compliant when `worked_seconds >= required_seconds` — including the trivial
/// case of zero working days (e.g. a full week of approved leave or holidays).
pub fn evaluate(working_days: i64, worked_seconds: i64) -> WeeklyHours {
    let required = required_seconds(working_days);
    let worked = worked_seconds.max(0);
    let shortfall = (required - worked).max(0);
    WeeklyHours {
        working_days: working_days.max(0),
        required_seconds: required,
        worked_seconds: worked,
        shortfall_seconds: shortfall,
        compliant: worked >= required,
    }
}

/// The most recent completed Mon–Sun week relative to `today`: the Monday and
/// Sunday of the *previous* ISO week. Run on a Monday, this is the week that
/// just ended.
pub fn previous_week(today: NaiveDate) -> (NaiveDate, NaiveDate) {
    let days_since_monday = today.weekday().num_days_from_monday() as i64;
    let this_monday = today - Duration::days(days_since_monday);
    let last_monday = this_monday - Duration::days(7);
    let last_sunday = last_monday + Duration::days(6);
    (last_monday, last_sunday)
}

/// Tallies from one `run_for_week` pass.
#[derive(Debug, Default, Clone, Copy)]
pub struct RunSummary {
    pub evaluated: usize,
    pub shortfalls: usize,
    pub warned: usize,
}

/// Run the weekly compliance check for `[week_start, week_end]` across every
/// employee: ensure the attendance rollup covers the week, compute each
/// employee's hours, persist the result, and warn HR + the employee's PM about
/// any shortfall (at most once per week). Returns per-run tallies.
pub async fn run_for_week(
    state: &AppState,
    week_start: NaiveDate,
    week_end: NaiveDate,
) -> Result<RunSummary, AppError> {
    let pool = &state.db;

    // Make sure every employee has a rolled-up row for each day of the week, so
    // a no-show (no intervals at all) still produces 'absent' days and counts
    // toward the requirement rather than silently dropping out. Idempotent.
    let employee_ids = users::employee_ids(pool).await?;
    for id in &employee_ids {
        if let Err(e) =
            crate::attendance_service::ensure_range(pool, *id, week_start, week_end).await
        {
            tracing::warn!(user_id = %id, "weekly hours: attendance ensure failed: {e}");
        }
    }

    // HR recipients are the same for everyone — fetch once.
    let hr_contacts = users::contacts_with_role(pool, UserRole::Hr).await?;

    let weeks = weekly_hours::week_activity(pool, week_start, week_end).await?;
    let mut summary = RunSummary::default();

    // Compute + persist each employee's result, collecting the non-compliant
    // ones (not already warned this week) for a single consolidated mail.
    struct Pending {
        row_id: uuid::Uuid,
        row: email_service::HoursDigestRow,
        managers: Vec<(uuid::Uuid, String, String)>,
    }
    let mut pending: Vec<Pending> = Vec::new();

    for ew in weeks {
        summary.evaluated += 1;
        let h = evaluate(ew.working_days, ew.worked_seconds);

        let upserted = weekly_hours::upsert(
            pool,
            ew.user_id,
            week_start,
            week_end,
            h.working_days as i32,
            h.required_seconds,
            h.worked_seconds,
            h.shortfall_seconds,
            h.compliant,
        )
        .await?;

        if h.compliant {
            continue;
        }
        summary.shortfalls += 1;

        // Only warn once per employee per week.
        if upserted.notified_at.is_some() {
            continue;
        }

        let managers = users::managers_of(pool, ew.user_id).await?;
        pending.push(Pending {
            row_id: upserted.id,
            row: email_service::HoursDigestRow {
                name: ew.name,
                email: ew.email,
                working_days: h.working_days,
                required_seconds: h.required_seconds,
                worked_seconds: h.worked_seconds,
                shortfall_seconds: h.shortfall_seconds,
            },
            managers,
        });
    }

    if pending.is_empty() {
        return Ok(summary);
    }

    // Rows delivered in at least one mail → mark notified so we don't resend.
    let mut notified: std::collections::HashSet<uuid::Uuid> = std::collections::HashSet::new();

    // 1) A SINGLE company-wide mail to all HR listing every employee below hours.
    let hr_emails: Vec<String> = hr_contacts.iter().map(|(_, email)| email.clone()).collect();
    if hr_emails.is_empty() {
        tracing::warn!("weekly hours: shortfalls found but no HR recipients configured");
    } else {
        let rows: Vec<email_service::HoursDigestRow> =
            pending.iter().map(|p| p.row.clone()).collect();
        match email_service::send_hours_shortfall_digest(&hr_emails, week_start, week_end, &rows)
            .await
        {
            Ok(()) => {
                for p in &pending {
                    notified.insert(p.row_id);
                }
            }
            Err(e) => tracing::warn!("weekly hours HR digest email failed: {e}"),
        }
    }

    // 2) One team mail per project manager — scoped to their own reports.
    let mut by_pm: std::collections::BTreeMap<String, Vec<usize>> =
        std::collections::BTreeMap::new();
    for (i, p) in pending.iter().enumerate() {
        for (_id, _name, email) in &p.managers {
            by_pm.entry(email.clone()).or_default().push(i);
        }
    }
    for (pm_email, idxs) in &by_pm {
        let rows: Vec<email_service::HoursDigestRow> =
            idxs.iter().map(|&i| pending[i].row.clone()).collect();
        match email_service::send_hours_shortfall_digest(
            std::slice::from_ref(pm_email),
            week_start,
            week_end,
            &rows,
        )
        .await
        {
            Ok(()) => {
                for &i in idxs {
                    notified.insert(pending[i].row_id);
                }
            }
            Err(e) => tracing::warn!(pm = %pm_email, "weekly hours PM digest email failed: {e}"),
        }
    }

    // 3) Mark everyone who made it into a delivered mail.
    for id in &notified {
        weekly_hours::mark_notified(pool, *id).await?;
    }
    summary.warned = notified.len();

    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn five_working_days_requires_40h() {
        assert_eq!(required_seconds(5), 40 * 3600);
    }

    #[test]
    fn zero_working_days_requires_nothing_and_is_compliant() {
        assert_eq!(required_seconds(0), 0);
        let h = evaluate(0, 0);
        assert!(h.compliant);
        assert_eq!(h.shortfall_seconds, 0);
    }

    #[test]
    fn meeting_the_requirement_is_compliant() {
        let h = evaluate(5, 40 * 3600);
        assert!(h.compliant);
        assert_eq!(h.shortfall_seconds, 0);
        assert_eq!(h.required_seconds, 40 * 3600);
    }

    #[test]
    fn under_requirement_flags_shortfall() {
        let h = evaluate(5, 32 * 3600);
        assert!(!h.compliant);
        assert_eq!(h.required_seconds, 40 * 3600);
        assert_eq!(h.shortfall_seconds, 8 * 3600);
    }

    #[test]
    fn leave_lowers_the_requirement() {
        // 3 working days (two days on leave) ⇒ 24h expected.
        let h = evaluate(3, 24 * 3600);
        assert!(h.compliant);
        assert_eq!(h.required_seconds, 24 * 3600);
    }

    #[test]
    fn overwork_is_compliant_with_no_shortfall() {
        let h = evaluate(5, 46 * 3600);
        assert!(h.compliant);
        assert_eq!(h.shortfall_seconds, 0);
    }

    #[test]
    fn previous_week_from_monday() {
        // Mon 2026-07-06 ⇒ previous week Mon 2026-06-29 .. Sun 2026-07-05.
        assert_eq!(
            previous_week(d(2026, 7, 6)),
            (d(2026, 6, 29), d(2026, 7, 5))
        );
    }

    #[test]
    fn previous_week_from_midweek() {
        // Wed 2026-07-01 ⇒ previous week Mon 2026-06-22 .. Sun 2026-06-28.
        assert_eq!(
            previous_week(d(2026, 7, 1)),
            (d(2026, 6, 22), d(2026, 6, 28))
        );
    }
}

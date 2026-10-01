//! Leave business logic: business-day counting and request submission.

use chrono::{Datelike, NaiveDate, Weekday};
use sqlx::PgPool;
use uuid::Uuid;

use crate::db::leave;
use crate::error::AppError;

/// Count working days in the inclusive range, excluding weekends and the given
/// holidays. Returns a float to leave room for half-days later.
pub fn count_business_days(start: NaiveDate, end: NaiveDate, holidays: &[NaiveDate]) -> f64 {
    if end < start {
        return 0.0;
    }
    let mut count = 0.0;
    let mut d = start;
    loop {
        let weekend = matches!(d.weekday(), Weekday::Sat | Weekday::Sun);
        if !weekend && !holidays.contains(&d) {
            count += 1.0;
        }
        if d == end {
            break;
        }
        d = d.succ_opt().expect("date within a bounded range");
    }
    count
}

/// Every day in the inclusive range — the count for a `calendar`-basis type such as maternity, which is a
/// continuous absence (weekends and holidays inside it are still leave).
pub fn count_calendar_days(start: NaiveDate, end: NaiveDate) -> f64 {
    if end < start {
        return 0.0;
    }
    ((end - start).num_days() + 1) as f64
}

/// True when `days` is a positive multiple of a half-day (0.5, 1, 1.5, …).
/// Half-day is the finest granularity of leave we allow.
fn is_half_day_multiple(days: f64) -> bool {
    days > 0.0 && ((days * 2.0).round() - days * 2.0).abs() < 1e-9
}

/// Why a balance can't cover `days` of leave, or `None` when it can. Shared by submit and approve so both
/// apply exactly the same rule: an ineligible type (no HR override) is refused with its reason, and the
/// approved total may not exceed the allotment.
fn refusal(bal: &leave::Balance, days: f64) -> Option<String> {
    if !bal.eligible && !bal.is_override {
        return Some(format!(
            "{} isn't available: {}",
            bal.leave_type_name,
            bal.eligibility_note.as_deref().unwrap_or("not eligible")
        ));
    }
    if bal.remaining_days < days {
        return Some(format!(
            "insufficient balance: {} day(s) remaining, {} requested",
            bal.remaining_days, days
        ));
    }
    None
}

/// Validate and create a leave request.
///
/// The span of the range is counted by the type's day basis: working days (weekdays minus holidays) for
/// regular leave, every day for a `calendar` type (maternity). `requested_days` lets the employee take a
/// partial day (half-leave): when `Some`, that value is used verbatim after validation — it must be a
/// positive multiple of 0.5 and must not exceed the span (you cannot claim more time off than the range you
/// selected). When `None`, the duration is the whole span.
///
/// Eligibility (gender / tenure, migration 0050) is judged on the leave's START date, so someone who reaches
/// one year of service next month can already apply for leave that starts after that date. Then we check
/// the remaining balance and persist.
pub async fn submit_request(
    pool: &PgPool,
    user_id: Uuid,
    leave_type_id: Uuid,
    start: NaiveDate,
    end: NaiveDate,
    reason: &str,
    requested_days: Option<f64>,
) -> Result<(Uuid, f64), AppError> {
    if end < start {
        return Err(AppError::BadRequest("end_date is before start_date".into()));
    }
    let balances = leave::balances_as_of(pool, user_id, start.year(), start).await?;
    let bal = balances
        .iter()
        .find(|b| b.leave_type_id == leave_type_id)
        .ok_or_else(|| AppError::BadRequest("unknown leave type".into()))?;

    let span = if bal.day_basis == "calendar" {
        count_calendar_days(start, end)
    } else {
        let holidays = leave::holiday_dates_between(pool, start, end).await?;
        count_business_days(start, end, &holidays)
    };
    if span <= 0.0 {
        return Err(AppError::BadRequest(
            "the selected range contains no working days".into(),
        ));
    }

    let days = match requested_days {
        Some(raw) => {
            if !is_half_day_multiple(raw) {
                return Err(AppError::BadRequest(
                    "days must be a positive multiple of 0.5 (e.g. 0.5, 1, 1.5)".into(),
                ));
            }
            let requested = (raw * 2.0).round() / 2.0; // store exactly 0.5, not 0.50000000001
            if requested > span {
                return Err(AppError::BadRequest(format!(
                    "requested {requested} day(s) exceeds the {span} day(s) in the selected range"
                )));
            }
            // A half day can trim the range by at most half a day: attendance marks EVERY day of an
            // approved range as leave, so "Mon–Fri, 0.5 days" would be a week off for half a day's balance.
            if requested < span - 0.5 {
                return Err(AppError::BadRequest(format!(
                    "a {span}-day range can be booked as {span} or {} day(s) — for a shorter leave, pick fewer dates",
                    span - 0.5
                )));
            }
            if bal.day_basis == "calendar" && requested != span {
                return Err(AppError::BadRequest(format!(
                    "{} is counted in whole calendar days — leave Days blank",
                    bal.leave_type_name
                )));
            }
            requested
        }
        None => span,
    };

    if leave::has_overlapping_request(pool, user_id, start, end).await? {
        return Err(AppError::BadRequest(
            "you already have a pending or approved leave on some of these dates".into(),
        ));
    }

    if let Some(why) = refusal(bal, days) {
        return Err(AppError::BadRequest(why));
    }

    let id = leave::create_request(pool, user_id, leave_type_id, start, end, days, reason).await?;
    Ok((id, days))
}

/// Approve a pending request, re-checking it under a per-person lock: the balance may have changed since it
/// was submitted (another request approved, HR lowered the allotment) and eligibility is re-judged on its
/// start date. The transaction-scoped advisory lock serialises approvals for the same person, so two
/// approvers acting at once on two requests can't both pass the check and overdraw.
/// Returns `false` if the request was no longer pending.
pub async fn approve(pool: &PgPool, request_id: Uuid, approver_id: Uuid) -> Result<bool, AppError> {
    let (user_id, _, _, _) = leave::request_for_decision(pool, request_id)
        .await?
        .ok_or(AppError::NotFound)?;
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))")
        .bind(format!("leave-approve:{user_id}"))
        .execute(&mut *tx)
        .await?;
    // Reads/writes below run on the pool and see everything committed before we got the lock.
    check_approvable(pool, request_id).await?;
    let decided = leave::decide(pool, request_id, "approved", approver_id).await?;
    tx.commit().await?;
    Ok(decided)
}

/// The balance + eligibility check an approval must pass (see [`approve`]).
pub async fn check_approvable(pool: &PgPool, request_id: Uuid) -> Result<(), AppError> {
    let (user_id, leave_type_id, start, days) = leave::request_for_decision(pool, request_id)
        .await?
        .ok_or(AppError::NotFound)?;
    let balances = leave::balances_as_of(pool, user_id, start.year(), start).await?;
    let bal = balances
        .iter()
        .find(|b| b.leave_type_id == leave_type_id)
        .ok_or_else(|| AppError::BadRequest("the leave type no longer exists".into()))?;
    match refusal(bal, days) {
        Some(why) => Err(AppError::BadRequest(format!("can't approve: {why}"))),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn counts_weekdays_only() {
        // Mon 2026-06-08 .. Fri 2026-06-12 = 5 working days.
        assert_eq!(count_business_days(d(2026, 6, 8), d(2026, 6, 12), &[]), 5.0);
    }

    #[test]
    fn excludes_weekend() {
        // Fri .. next Mon = Fri + Mon = 2 (Sat/Sun skipped).
        assert_eq!(
            count_business_days(d(2026, 6, 12), d(2026, 6, 15), &[]),
            2.0
        );
    }

    #[test]
    fn excludes_holidays() {
        // Mon..Fri with Wed (06-10) a holiday = 4.
        let holidays = vec![d(2026, 6, 10)];
        assert_eq!(
            count_business_days(d(2026, 6, 8), d(2026, 6, 12), &holidays),
            4.0
        );
    }

    #[test]
    fn calendar_days_count_every_day() {
        // Fri 2026-06-12 .. Mon 2026-06-15 = 4 calendar days (weekend included).
        assert_eq!(count_calendar_days(d(2026, 6, 12), d(2026, 6, 15)), 4.0);
        assert_eq!(count_calendar_days(d(2026, 6, 15), d(2026, 6, 15)), 1.0);
        assert_eq!(count_calendar_days(d(2026, 6, 16), d(2026, 6, 15)), 0.0);
        // 180 days from 1 Jan 2026 ends on 29 Jun 2026.
        assert_eq!(count_calendar_days(d(2026, 1, 1), d(2026, 6, 29)), 180.0);
    }

    #[test]
    fn half_day_multiples_are_accepted() {
        for ok in [0.5, 1.0, 1.5, 2.5, 10.0] {
            assert!(
                is_half_day_multiple(ok),
                "{ok} should be a half-day multiple"
            );
        }
        for bad in [0.0, -0.5, 0.25, 1.1, 1.75] {
            assert!(
                !is_half_day_multiple(bad),
                "{bad} should not be a half-day multiple"
            );
        }
    }

    #[test]
    fn single_day_and_reversed() {
        assert_eq!(count_business_days(d(2026, 6, 8), d(2026, 6, 8), &[]), 1.0); // Monday
        assert_eq!(
            count_business_days(d(2026, 6, 13), d(2026, 6, 13), &[]),
            0.0
        ); // Saturday
        assert_eq!(count_business_days(d(2026, 6, 12), d(2026, 6, 8), &[]), 0.0);
        // reversed
    }
}

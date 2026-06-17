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

/// Validate and create a leave request: counts business days in the range,
/// checks the employee has enough remaining balance for that type, and persists.
pub async fn submit_request(
    pool: &PgPool,
    user_id: Uuid,
    leave_type_id: Uuid,
    start: NaiveDate,
    end: NaiveDate,
    reason: &str,
) -> Result<(Uuid, f64), AppError> {
    if end < start {
        return Err(AppError::BadRequest("end_date is before start_date".into()));
    }
    let holidays = leave::holiday_dates_between(pool, start, end).await?;
    let days = count_business_days(start, end, &holidays);
    if days <= 0.0 {
        return Err(AppError::BadRequest(
            "the selected range contains no working days".into(),
        ));
    }

    let year = start.year();
    let balances = leave::balances(pool, user_id, year).await?;
    let bal = balances
        .iter()
        .find(|b| b.leave_type_id == leave_type_id)
        .ok_or_else(|| AppError::BadRequest("unknown leave type".into()))?;
    if bal.remaining_days < days {
        return Err(AppError::BadRequest(format!(
            "insufficient balance: {} day(s) remaining, {} requested",
            bal.remaining_days, days
        )));
    }

    let id = leave::create_request(pool, user_id, leave_type_id, start, end, days, reason).await?;
    Ok((id, days))
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
        assert_eq!(count_business_days(d(2026, 6, 12), d(2026, 6, 15), &[]), 2.0);
    }

    #[test]
    fn excludes_holidays() {
        // Mon..Fri with Wed (06-10) a holiday = 4.
        let holidays = vec![d(2026, 6, 10)];
        assert_eq!(count_business_days(d(2026, 6, 8), d(2026, 6, 12), &holidays), 4.0);
    }

    #[test]
    fn single_day_and_reversed() {
        assert_eq!(count_business_days(d(2026, 6, 8), d(2026, 6, 8), &[]), 1.0); // Monday
        assert_eq!(count_business_days(d(2026, 6, 13), d(2026, 6, 13), &[]), 0.0); // Saturday
        assert_eq!(count_business_days(d(2026, 6, 12), d(2026, 6, 8), &[]), 0.0); // reversed
    }
}

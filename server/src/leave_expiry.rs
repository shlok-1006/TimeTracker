//! Leave requests nobody decided in time EXPIRE (migration 0056, OKF LV-13).
//!
//! A request still `pending` more than [`expiry_days`] days after its leave ended is marked `expired`: it
//! leaves the approvals queue and shows as Expired in the employee's history. Nothing is deleted, and no
//! balance changes — only approved requests count as used. The grace days are there because leave is often
//! applied for after the fact (sick leave): such a request must still reach HR before it can expire.
//!
//! Runs at start-up and then every hour. Days are judged in the org time zone (Asia/Kolkata). The UPDATE only
//! touches rows that are still `pending`, so a request HR approves or rejects at the same moment keeps HR's
//! decision (the row lock serializes the two; the loser matches nothing).

use chrono::{Duration, NaiveDate, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;
use crate::state::AppState;

/// Days after a leave's end date that a pending request may still wait for a decision.
pub const DEFAULT_EXPIRY_DAYS: i64 = 3;

/// The grace days, from `LEAVE_PENDING_EXPIRY_DAYS` (0–60) or the default.
pub fn expiry_days() -> i64 {
    std::env::var("LEAVE_PENDING_EXPIRY_DAYS")
        .ok()
        .and_then(|s| s.trim().parse::<i64>().ok())
        .filter(|d| (0..=60).contains(d))
        .unwrap_or(DEFAULT_EXPIRY_DAYS)
}

/// Today in Asia/Kolkata (UTC+5:30, no daylight saving).
pub fn today_ist() -> NaiveDate {
    (Utc::now() + Duration::minutes(330)).date_naive()
}

/// One expired request (for the log).
#[derive(Debug, Clone, PartialEq)]
pub struct Expired {
    pub id: Uuid,
    pub user_id: Uuid,
    pub end_date: NaiveDate,
}

/// Expire every request still pending whose leave ended more than `grace_days` before `today`: a leave
/// ending on the 9th with 3 grace days may wait through the 12th, and expires from the 13th.
pub async fn expire_pending(
    pool: &PgPool,
    today: NaiveDate,
    grace_days: i64,
) -> Result<Vec<Expired>, AppError> {
    let cutoff = today - Duration::days(grace_days);
    let rows = sqlx::query!(
        r#"UPDATE leave_requests
              SET status = 'expired', decided_at = now()
            WHERE status = 'pending' AND end_date < $1
        RETURNING id, user_id, end_date"#,
        cutoff
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| Expired {
            id: r.id,
            user_id: r.user_id,
            end_date: r.end_date,
        })
        .collect())
}

pub async fn run(state: AppState) {
    loop {
        run_once(&state.db).await;
        tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
    }
}

async fn run_once(pool: &PgPool) {
    let days = expiry_days();
    match expire_pending(pool, today_ist(), days).await {
        Ok(done) if done.is_empty() => {}
        Ok(done) => {
            for e in &done {
                tracing::info!(request = %e.id, user = %e.user_id, end_date = %e.end_date, "leave request expired");
            }
            tracing::info!(
                count = done.len(),
                grace_days = days,
                "expired pending leave requests"
            );
        }
        Err(e) => tracing::warn!("leave expiry failed: {e}"),
    }
}

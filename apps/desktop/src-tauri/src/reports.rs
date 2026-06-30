//! Dashboard reporting computed from the LOCAL SQLite intervals (Rule: render
//! from SQLite first). The UI reconciles these with the server's `/me/hours`.

use chrono::{DateTime, Datelike, Duration, Local};
use serde::Serialize;
use tauri::State;
use uuid::Uuid;

use crate::interval_repository::{self, Interval};
use crate::timer::DesktopState;

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct HoursSummary {
    pub total_seconds: i64,
    /// Worked (active + meeting) seconds, scoped by period.
    pub today_seconds: i64,
    pub week_seconds: i64,
    /// All-time worked / idle (kept for reconciliation; UI uses the scoped ones).
    pub active_seconds: i64,
    pub idle_seconds: i64,
    /// Idle seconds scoped to today / this week (Mon–Sun), for the donut.
    pub today_idle_seconds: i64,
    pub week_idle_seconds: i64,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct DayBucket {
    pub date: String,
    pub worked_seconds: i64,
    pub idle_seconds: i64,
}

fn secs(iv: &Interval) -> i64 {
    (iv.end_utc - iv.start_utc).num_seconds().max(0)
}

/// Summarize worked/idle time for today, this week, and all-time (local dates).
pub fn summarize(intervals: &[Interval], now: DateTime<Local>) -> HoursSummary {
    let today = now.date_naive();
    let week_start = today - Duration::days(today.weekday().num_days_from_monday() as i64);

    let mut s = HoursSummary {
        total_seconds: 0,
        today_seconds: 0,
        week_seconds: 0,
        active_seconds: 0,
        idle_seconds: 0,
        today_idle_seconds: 0,
        week_idle_seconds: 0,
    };
    for iv in intervals {
        let d = iv.start_utc.with_timezone(&Local).date_naive();
        let n = secs(iv);
        match iv.kind.as_str() {
            "idle" => {
                s.idle_seconds += n;
                if d == today {
                    s.today_idle_seconds += n;
                }
                if d >= week_start {
                    s.week_idle_seconds += n;
                }
            }
            "break" => {} // recorded for the timeline, not counted as worked
            // active + meeting count as worked.
            _ => {
                s.active_seconds += n;
                s.total_seconds += n;
                if d == today {
                    s.today_seconds += n;
                }
                if d >= week_start {
                    s.week_seconds += n;
                }
            }
        }
    }
    s
}

/// Per-day worked/idle totals for the current calendar week (Monday→Sunday,
/// oldest first). Days later in the week than today are simply zero.
pub fn weekly_timeline(intervals: &[Interval], now: DateTime<Local>) -> Vec<DayBucket> {
    let today = now.date_naive();
    let monday = today - Duration::days(today.weekday().num_days_from_monday() as i64);
    let sunday = monday + Duration::days(6);
    let mut buckets: Vec<DayBucket> = (0..7)
        .map(|i| DayBucket {
            date: (monday + Duration::days(i)).format("%Y-%m-%d").to_string(),
            worked_seconds: 0,
            idle_seconds: 0,
        })
        .collect();

    for iv in intervals {
        let d = iv.start_utc.with_timezone(&Local).date_naive();
        if d < monday || d > sunday {
            continue;
        }
        let idx = (d - monday).num_days() as usize;
        let n = secs(iv);
        match iv.kind.as_str() {
            "idle" => buckets[idx].idle_seconds += n,
            "break" => {}
            _ => buckets[idx].worked_seconds += n, // active + meeting
        }
    }
    buckets
}

#[tauri::command]
pub async fn get_hours_summary(
    state: State<'_, DesktopState>,
    user_id: String,
) -> Result<HoursSummary, String> {
    let uid = Uuid::parse_str(&user_id).map_err(|_| "invalid user id".to_string())?;
    let items = interval_repository::for_user(&state.pool, uid)
        .await
        .map_err(|e| e.to_string())?;
    Ok(summarize(&items, Local::now()))
}

#[tauri::command]
pub async fn get_daily_timeline(
    state: State<'_, DesktopState>,
    user_id: String,
) -> Result<Vec<DayBucket>, String> {
    let uid = Uuid::parse_str(&user_id).map_err(|_| "invalid user id".to_string())?;
    let items = interval_repository::for_user(&state.pool, uid)
        .await
        .map_err(|e| e.to_string())?;
    Ok(weekly_timeline(&items, Local::now()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, TimeZone};

    fn iv(start: DateTime<Local>, dur_secs: i64, kind: &str) -> Interval {
        let s = start.with_timezone(&chrono::Utc);
        Interval {
            id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            start_utc: s,
            end_utc: s + Duration::seconds(dur_secs),
            kind: kind.to_string(),
            team_id: None,
        }
    }

    fn noon_today() -> DateTime<Local> {
        Local
            .from_local_datetime(&Local::now().date_naive().and_hms_opt(12, 0, 0).unwrap())
            .unwrap()
    }

    #[test]
    fn summarize_today_week_total_idle() {
        let now = noon_today();
        let items = vec![
            iv(now - Duration::hours(2), 3600, "active"), // today, worked 1h
            iv(now - Duration::minutes(30), 900, "idle"), // today, idle 15m
            iv(now - Duration::minutes(20), 600, "break"), // today, break 10m (excluded)
            iv(now - Duration::days(20), 3600, "active"), // old worked (total only)
        ];
        let s = summarize(&items, now);
        assert_eq!(s.today_seconds, 3600);
        assert_eq!(s.week_seconds, 3600);
        assert_eq!(s.total_seconds, 7200); // 2 worked (active) intervals
        assert_eq!(s.active_seconds, 7200);
        assert_eq!(s.idle_seconds, 900);
        // Idle is scoped to today / this week as well (for the donut).
        assert_eq!(s.today_idle_seconds, 900);
        assert_eq!(s.week_idle_seconds, 900);
    }

    #[test]
    fn weekly_timeline_is_monday_to_sunday_with_today_in_place() {
        let now = noon_today();
        let items = vec![iv(now - Duration::hours(1), 1800, "active")];
        let t = weekly_timeline(&items, now);
        assert_eq!(t.len(), 7);
        // Bucket 0 is this week's Monday; today's worked time lands in its weekday slot.
        let idx = now.date_naive().weekday().num_days_from_monday() as usize;
        assert_eq!(t[idx].worked_seconds, 1800);
        let monday = now.date_naive() - Duration::days(idx as i64);
        assert_eq!(t[0].date, monday.format("%Y-%m-%d").to_string());
    }
}

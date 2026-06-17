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
    pub today_seconds: i64,
    pub week_seconds: i64,
    pub active_seconds: i64,
    pub idle_seconds: i64,
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
    };
    for iv in intervals {
        let d = iv.start_utc.with_timezone(&Local).date_naive();
        let n = secs(iv);
        match iv.kind.as_str() {
            "idle" => s.idle_seconds += n,
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

/// Per-day worked/idle totals for the last `days` days (oldest first).
pub fn daily_timeline(intervals: &[Interval], now: DateTime<Local>, days: i64) -> Vec<DayBucket> {
    let today = now.date_naive();
    let start = today - Duration::days(days - 1);
    let mut buckets: Vec<DayBucket> = (0..days)
        .map(|i| DayBucket {
            date: (start + Duration::days(i)).format("%Y-%m-%d").to_string(),
            worked_seconds: 0,
            idle_seconds: 0,
        })
        .collect();

    for iv in intervals {
        let d = iv.start_utc.with_timezone(&Local).date_naive();
        if d < start || d > today {
            continue;
        }
        let idx = (d - start).num_days() as usize;
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
    Ok(daily_timeline(&items, Local::now(), 7))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

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
    }

    #[test]
    fn daily_timeline_has_seven_buckets_with_today_last() {
        let now = noon_today();
        let items = vec![iv(now - Duration::hours(1), 1800, "active")];
        let t = daily_timeline(&items, now, 7);
        assert_eq!(t.len(), 7);
        // Today is the last bucket and holds the worked time.
        assert_eq!(t[6].worked_seconds, 1800);
        assert_eq!(t[0].worked_seconds, 0);
    }
}

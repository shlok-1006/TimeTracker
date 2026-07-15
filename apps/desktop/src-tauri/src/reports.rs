//! Dashboard reporting computed from the LOCAL SQLite intervals (Rule: render
//! from SQLite first). The UI reconciles these with the server's `/me/hours`.

use chrono::{DateTime, Datelike, Duration, Local};
use serde::Serialize;
use tauri::State;
use uuid::Uuid;

use crate::interval_repository::{self, Interval};
use crate::timer::DesktopState;

/// The work day starts at 04:00 local — late-night work counts toward the day
/// it began. Must match the server's hours-summary boundary.
const BUSINESS_DAY_START_HOUR: i64 = 4;

/// A "day's work" = active + idle + meeting (only Break is excluded). Today and
/// this week are period-scoped totals of that, each broken out so idle and
/// meeting are visible on their own. Mirrors the server's `HoursSummary`.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct HoursSummary {
    pub today_seconds: i64,
    pub today_active_seconds: i64,
    pub today_idle_seconds: i64,
    pub today_meeting_seconds: i64,
    pub week_seconds: i64,
    pub week_active_seconds: i64,
    pub week_idle_seconds: i64,
    pub week_meeting_seconds: i64,
    pub total_seconds: i64,
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

/// Summarize worked/idle time for today, this week, and all-time. Days use a
/// 04:00 LOCAL business-day boundary (BUSINESS_DAY_START_HOUR): late-night work
/// counts toward the day it began, and this matches the server's per-user
/// windowing so the desktop and admin "today" figures agree.
pub fn summarize(intervals: &[Interval], now: DateTime<Local>) -> HoursSummary {
    // Shift back 4h before taking the date, so 00:00–03:59 local belongs to the
    // previous day and a new day starts at 04:00.
    let biz_day = |t: DateTime<Local>| (t - Duration::hours(BUSINESS_DAY_START_HOUR)).date_naive();
    let today = biz_day(now);
    let week_start = today - Duration::days(today.weekday().num_days_from_monday() as i64);

    let mut s = HoursSummary {
        today_seconds: 0,
        today_active_seconds: 0,
        today_idle_seconds: 0,
        today_meeting_seconds: 0,
        week_seconds: 0,
        week_active_seconds: 0,
        week_idle_seconds: 0,
        week_meeting_seconds: 0,
        total_seconds: 0,
    };
    for iv in intervals {
        let d = biz_day(iv.start_utc.with_timezone(&Local));
        let n = secs(iv);
        // Only Break is excluded from a day's work; active/idle/meeting all count.
        let (is_active, is_idle, is_meeting) = match iv.kind.as_str() {
            "break" => continue,
            "idle" => (false, true, false),
            "meeting" => (false, false, true),
            _ => (true, false, false), // active (default)
        };
        s.total_seconds += n;
        if d == today {
            s.today_seconds += n;
            if is_active {
                s.today_active_seconds += n;
            }
            if is_idle {
                s.today_idle_seconds += n;
            }
            if is_meeting {
                s.today_meeting_seconds += n;
            }
        }
        if d >= week_start {
            s.week_seconds += n;
            if is_active {
                s.week_active_seconds += n;
            }
            if is_idle {
                s.week_idle_seconds += n;
            }
            if is_meeting {
                s.week_meeting_seconds += n;
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
    fn summarize_today_week_total_breakdown() {
        let now = noon_today();
        let items = vec![
            iv(now - Duration::hours(2), 3600, "active"), // today, active 1h
            iv(now - Duration::minutes(30), 900, "idle"), // today, idle 15m
            iv(now - Duration::minutes(45), 1800, "meeting"), // today, meeting 30m
            iv(now - Duration::minutes(20), 600, "break"), // today, break 10m (excluded)
            iv(now - Duration::days(20), 3600, "active"), // old active (total only)
        ];
        let s = summarize(&items, now);
        // A day's work = active + idle + meeting (break excluded).
        assert_eq!(s.today_seconds, 3600 + 900 + 1800);
        assert_eq!(s.today_active_seconds, 3600);
        assert_eq!(s.today_idle_seconds, 900);
        assert_eq!(s.today_meeting_seconds, 1800);
        assert_eq!(s.week_seconds, 3600 + 900 + 1800);
        assert_eq!(s.week_active_seconds, 3600);
        assert_eq!(s.week_idle_seconds, 900);
        assert_eq!(s.week_meeting_seconds, 1800);
        // All-time worked includes the 20-day-old active interval; break never counts.
        assert_eq!(s.total_seconds, 3600 + 900 + 1800 + 3600);
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

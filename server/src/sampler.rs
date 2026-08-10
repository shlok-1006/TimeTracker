//! Daily Screenshot Sampler (STEP 9).
//!
//! How many screenshots a day gets is a function of how long the person actually
//! worked: roughly one per `ANALYZER_MINUTES_PER_SHOT` of tracked time, so an
//! eight-hour day is looked at about sixteen times and a two-hour day four.
//!
//! It used to be five fixed clock buckets with one shot each, which meant a full
//! day and a two-hour day were reviewed in the same depth — five frames either
//! way — and no amount of extra work could earn a closer look. A day's score was
//! being decided by four or five frames out of hundreds.
//!
//! Tracked time comes from the same overlap-safe derivation the hours pages use
//! (`attendance::day_activity`), so a second device recording the same minute
//! cannot inflate the sample count either.
//!
//! SPREAD. The eligible shots, in capture order, are cut into `n` contiguous
//! groups of near-equal SIZE and one is drawn at random from each. Equal-size
//! rather than equal-clock-time on purpose: someone who works 09:00–12:00 and
//! 16:00–18:00 should not spend slots on the four hours in between.
//!
//! Eligibility: only *Working* screenshots count (`captured_status = 'working'`).
//! The desktop also captures during meetings (tagged `meeting`, Feature 2); those
//! are stored and viewable but are filtered out here and never sampled/analysed.
//!
//! Idempotency (Rules: "never resample same day"): the chosen set is persisted in
//! `analysis_job_samples` and `analysis_jobs` is UNIQUE per (user, day). Re-running
//! `sample_screenshots` for a day that already has samples returns the stored set
//! unchanged — it never re-rolls the random choice, and never re-sizes a day that
//! was already sampled under a different ratio.

use argon2::password_hash::rand_core::{OsRng, RngCore};
use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;

/// Tracked minutes one screenshot is meant to stand for. 30 ⇒ an 8-hour day is
/// sampled 16 times.
const DEFAULT_MINUTES_PER_SHOT: i64 = 30;
/// Floor, so a short-but-real day is still looked at properly rather than judged
/// on one frame. Matches what a sparse day used to get.
const DEFAULT_MIN_SHOTS: usize = 4;
/// Ceiling. Every sample is a vision call, so this is the cost and wall-clock
/// bound on one person-day; without it a 14-hour day would quietly cost triple.
const DEFAULT_MAX_SHOTS: usize = 20;

fn env_num<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

/// How many screenshots `worked_seconds` of tracked time earns.
///
/// Tunable without a deploy via `ANALYZER_MINUTES_PER_SHOT` / `ANALYZER_MIN_SHOTS`
/// / `ANALYZER_MAX_SHOTS` — the right ratio is a judgement about cost against
/// confidence, and that is worth being able to turn without a rebuild.
pub fn shots_for_worked_seconds(worked_seconds: i64) -> usize {
    let per_shot_minutes = env_num("ANALYZER_MINUTES_PER_SHOT", DEFAULT_MINUTES_PER_SHOT).max(1);
    let min = env_num("ANALYZER_MIN_SHOTS", DEFAULT_MIN_SHOTS).max(1);
    let max = env_num("ANALYZER_MAX_SHOTS", DEFAULT_MAX_SHOTS).max(min);

    let worked_minutes = worked_seconds.max(0) as f64 / 60.0;
    let want = (worked_minutes / per_shot_minutes as f64).round() as i64;
    (want.max(0) as usize).clamp(min, max)
}

/// A job row (one per user per day).
#[derive(Debug, Clone, Serialize)]
pub struct AnalysisJob {
    pub id: Uuid,
    pub user_id: Uuid,
    pub day: NaiveDate,
    pub status: String,
    pub created_at: DateTime<Utc>,
}

/// A screenshot chosen for a job's sample set.
#[derive(Debug, Clone, Serialize)]
pub struct SampledShot {
    pub screenshot_id: Uuid,
    pub bucket: String,
    pub taken_at: DateTime<Utc>,
    pub storage_key: String,
    /// Capture-time status (always `working` for sampled shots; carried so the
    /// analyzer can re-assert the Phase 4 guard).
    pub captured_status: String,
}

/// A screenshot in the day window, with its capture-time status.
struct CandidateShot {
    id: Uuid,
    taken_at: DateTime<Utc>,
    captured_status: String,
}

/// Sampling eligibility (Feature 2): ONLY Working screenshots may be analysed.
/// Meeting/break/idle shots remain stored and viewable but never sampled.
fn is_eligible(shot: &CandidateShot) -> bool {
    shot.captured_status == "working"
}

/// Pick one element uniformly at random, or `None` if empty.
fn pick_one<T>(items: &[T]) -> Option<&T> {
    if items.is_empty() {
        return None;
    }
    let idx = (OsRng.next_u32() as usize) % items.len();
    Some(&items[idx])
}

/// Choose up to `want` screenshots, one per contiguous slot of the day's eligible
/// shots (the pure sampling strategy).
///
/// Sorts by capture time itself rather than trusting the caller's `ORDER BY`:
/// the slots only mean "early in the day" through "late in the day" if the input
/// is ordered, and that is too quiet a thing to fail if a query is ever edited.
/// Applies `is_eligible` first (defense in depth on top of the SQL filter), so
/// non-working shots can never be chosen even if the query were loosened.
///
/// Returns `(slot_name, screenshot_id)` in time order, `min(want, eligible)`
/// entries: fewer shots than slots means one per shot, never a duplicate.
fn choose_samples(shots: &[CandidateShot], want: usize) -> Vec<(String, Uuid)> {
    let mut eligible: Vec<&CandidateShot> = shots.iter().filter(|s| is_eligible(s)).collect();
    eligible.sort_by_key(|s| s.taken_at);
    let want = want.min(eligible.len());
    if want == 0 {
        return Vec::new();
    }

    // Slot i covers [i*len/want, (i+1)*len/want) — integer maths distributes the
    // remainder across slots rather than piling it on the last one, and the bounds
    // are strictly increasing so no slot is ever empty.
    (0..want)
        .filter_map(|i| {
            let lo = i * eligible.len() / want;
            let hi = (i + 1) * eligible.len() / want;
            pick_one(&eligible[lo..hi]).map(|s| (format!("slot_{:02}", i + 1), s.id))
        })
        .collect()
}

/// UTC `[start, end)` bounds of a calendar day.
/// A `day` means the ORG-LOCAL calendar day (IST) — see crate::org_time.
fn day_bounds(day: NaiveDate) -> (DateTime<Utc>, DateTime<Utc>) {
    crate::org_time::day_bounds_utc(day)
}

/// Create (or fetch) the job for `(user, day)`. Idempotent: a second call returns
/// the existing job rather than creating a duplicate (UNIQUE(user_id, day)).
pub async fn create_daily_job(
    pool: &PgPool,
    user_id: Uuid,
    day: NaiveDate,
) -> Result<AnalysisJob, AppError> {
    sqlx::query!(
        "INSERT INTO analysis_jobs (user_id, day) VALUES ($1, $2)
         ON CONFLICT (user_id, day) DO NOTHING",
        user_id,
        day
    )
    .execute(pool)
    .await?;

    let r = sqlx::query!(
        "SELECT id, user_id, day, status, created_at
         FROM analysis_jobs WHERE user_id = $1 AND day = $2",
        user_id,
        day
    )
    .fetch_one(pool)
    .await?;

    Ok(AnalysisJob {
        id: r.id,
        user_id: r.user_id,
        day: r.day,
        status: r.status,
        created_at: r.created_at,
    })
}

/// Load an existing job and its sampled set, if one exists for `(user, day)`.
pub async fn load_existing_job(
    pool: &PgPool,
    user_id: Uuid,
    day: NaiveDate,
) -> Result<Option<(AnalysisJob, Vec<SampledShot>)>, AppError> {
    let row = sqlx::query!(
        "SELECT id, user_id, day, status, created_at
         FROM analysis_jobs WHERE user_id = $1 AND day = $2",
        user_id,
        day
    )
    .fetch_optional(pool)
    .await?;

    match row {
        None => Ok(None),
        Some(r) => {
            let job = AnalysisJob {
                id: r.id,
                user_id: r.user_id,
                day: r.day,
                status: r.status,
                created_at: r.created_at,
            };
            let samples = load_samples(pool, job.id).await?;
            Ok(Some((job, samples)))
        }
    }
}

/// The stored sampled set for a job, ordered by capture time.
async fn load_samples(pool: &PgPool, job_id: Uuid) -> Result<Vec<SampledShot>, AppError> {
    let rows = sqlx::query!(
        r#"SELECT s.bucket, sc.id, sc.taken_at, sc.storage_key, sc.captured_status
           FROM analysis_job_samples s
           JOIN screenshots sc ON sc.id = s.screenshot_id
           WHERE s.job_id = $1
           ORDER BY sc.taken_at"#,
        job_id
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| SampledShot {
            screenshot_id: r.id,
            bucket: r.bucket,
            taken_at: r.taken_at,
            storage_key: r.storage_key,
            captured_status: r.captured_status,
        })
        .collect())
}

/// Sample working screenshots for `(user, day)` in proportion to time tracked,
/// spread across the day.
///
/// Idempotent: if the day already has a sampled set, it is returned unchanged
/// (the day is never resampled). Otherwise the count is derived from the day's
/// tracked time, one screenshot is drawn at random per slot, persisted, and the
/// job marked `sampled`.
pub async fn sample_screenshots(
    pool: &PgPool,
    user_id: Uuid,
    day: NaiveDate,
) -> Result<Vec<SampledShot>, AppError> {
    let job = create_daily_job(pool, user_id, day).await?;

    // Never resample a day that already has a stored set.
    let existing = load_samples(pool, job.id).await?;
    if !existing.is_empty() {
        return Ok(existing);
    }

    // Eligible = Working screenshots only (meeting/break shots are never analysed).
    // Filtered both here (indexed) and again in `choose_samples` (defense in depth).
    let (from, to) = day_bounds(day);
    let rows = sqlx::query!(
        "SELECT id, taken_at, captured_status FROM screenshots
         WHERE user_id = $1 AND captured_status = 'working'
           AND taken_at >= $2 AND taken_at < $3
         ORDER BY taken_at",
        user_id,
        from,
        to
    )
    .fetch_all(pool)
    .await?;

    let shots: Vec<CandidateShot> = rows
        .into_iter()
        .map(|r| CandidateShot {
            id: r.id,
            taken_at: r.taken_at,
            captured_status: r.captured_status,
        })
        .collect();

    // How much of the day to review is decided by how much of it was worked. Read
    // from intervals rather than the attendance rollup so sampling does not depend
    // on the nightly job having run first.
    let worked = crate::db::attendance::day_activity(pool, user_id, from, to)
        .await
        .map(|a| a.worked_seconds)
        // A failure here must not silently collapse the day to the floor without
        // saying so — the count would look deliberate and be nothing of the kind.
        .unwrap_or_else(|e| {
            tracing::warn!(%user_id, %day, "sampler: worked-time read failed, using the floor: {e}");
            0
        });
    let want = shots_for_worked_seconds(worked);
    tracing::info!(
        %user_id, %day, worked_seconds = worked, candidates = shots.len(), want,
        "sampling screenshots"
    );

    for (bucket, screenshot_id) in choose_samples(&shots, want) {
        sqlx::query!(
            "INSERT INTO analysis_job_samples (job_id, screenshot_id, bucket)
             VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
            job.id,
            screenshot_id,
            bucket
        )
        .execute(pool)
        .await?;
    }

    sqlx::query!(
        "UPDATE analysis_jobs SET status = 'sampled' WHERE id = $1",
        job.id
    )
    .execute(pool)
    .await?;

    load_samples(pool, job.id).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn shot_with_status(id: u8, hour: u32, status: &str) -> CandidateShot {
        let d = NaiveDate::from_ymd_opt(2026, 6, 8).unwrap();
        CandidateShot {
            id: Uuid::from_u128(id as u128),
            taken_at: Utc.from_utc_datetime(&d.and_hms_opt(hour, 0, 0).unwrap()),
            captured_status: status.to_string(),
        }
    }

    /// The ratio itself. Env-tunable, so these assert the shipped defaults —
    /// 30 min/shot, floor 4, ceiling 20.
    #[test]
    fn the_count_tracks_the_hours_worked() {
        let h = |hours: f64| shots_for_worked_seconds((hours * 3600.0) as i64);
        assert_eq!(h(8.0), 16, "a full day is looked at 16 times, not 5");
        assert_eq!(h(4.0), 8);
        assert_eq!(h(2.0), 4);
        assert_eq!(h(1.0), 4, "the floor holds a short day up");
        assert_eq!(
            h(0.0),
            4,
            "and a day with no tracked time still gets the floor"
        );
        assert_eq!(h(14.0), 20, "the ceiling caps cost on a very long day");
        assert_eq!(h(100.0), 20, "and cannot be blown past");
    }

    #[test]
    fn the_count_is_monotonic_in_time_worked() {
        // Longer worked never yields fewer samples — obvious, and exactly the kind of
        // thing rounding quietly breaks.
        let mut prev = 0;
        for minutes in 0..(16 * 60) {
            let n = shots_for_worked_seconds(minutes * 60);
            assert!(n >= prev, "{minutes} min gave {n} after {prev}");
            prev = n;
        }
    }

    #[test]
    fn negative_or_absurd_input_cannot_escape_the_bounds() {
        assert_eq!(shots_for_worked_seconds(-5_000), 4);
        assert_eq!(shots_for_worked_seconds(i64::MAX), 20);
    }

    #[test]
    fn pick_one_stays_in_range_and_handles_empty() {
        let v = vec![10, 20, 30, 40, 50];
        for _ in 0..100 {
            assert!(v.contains(pick_one(&v).unwrap()));
        }
        let empty: Vec<i32> = vec![];
        assert!(pick_one(&empty).is_none());
    }

    /// `n` working shots one minute apart from 09:00, ids 1..=n — so index order
    /// and capture order are the same thing, which is what the slot assertions read.
    fn run(n: u16) -> Vec<CandidateShot> {
        let d = NaiveDate::from_ymd_opt(2026, 6, 8).unwrap();
        let base = Utc.from_utc_datetime(&d.and_hms_opt(9, 0, 0).unwrap());
        (1..=n)
            .map(|i| CandidateShot {
                id: Uuid::from_u128(i as u128),
                taken_at: base + chrono::Duration::minutes(i as i64),
                captured_status: "working".to_string(),
            })
            .collect()
    }

    #[test]
    fn a_full_day_is_sampled_sixteen_times() {
        // 120 candidate shots, 8 hours worked → 16 slots, one pick each, all distinct.
        let shots = run(120);
        let chosen = choose_samples(&shots, shots_for_worked_seconds(8 * 3600));
        assert_eq!(chosen.len(), 16);

        let ids: std::collections::HashSet<Uuid> = chosen.iter().map(|(_, id)| *id).collect();
        assert_eq!(ids.len(), 16, "no screenshot may be picked twice");

        let names: Vec<&str> = chosen.iter().map(|(b, _)| b.as_str()).collect();
        assert_eq!(names.first(), Some(&"slot_01"));
        assert_eq!(names.last(), Some(&"slot_16"));
    }

    #[test]
    fn slots_are_spread_across_the_day_in_order() {
        // Each pick must come from its own stretch of the day: slot 1 from the first
        // tenth, slot 10 from the last. Otherwise "more samples" would just mean
        // "more of the same hour".
        let shots = run(100);
        let chosen = choose_samples(&shots, 10);
        assert_eq!(chosen.len(), 10);
        for (i, (_, id)) in chosen.iter().enumerate() {
            let pos = shots.iter().position(|s| s.id == *id).expect("picked shot");
            assert!(
                pos >= i * 10 && pos < (i + 1) * 10,
                "slot {} picked index {pos}, outside its tenth",
                i + 1
            );
        }
    }

    #[test]
    fn fewer_shots_than_slots_yields_one_per_shot() {
        // A day with 3 candidates cannot produce 16 samples — and must not repeat one
        // to reach the number.
        let shots = run(3);
        let chosen = choose_samples(&shots, 16);
        assert_eq!(chosen.len(), 3);
        let ids: std::collections::HashSet<Uuid> = chosen.iter().map(|(_, id)| *id).collect();
        assert_eq!(ids.len(), 3, "no duplicates when padding out slots");
    }

    #[test]
    fn no_screenshots_yields_empty() {
        assert!(choose_samples(&[], 16).is_empty());
    }

    #[test]
    fn meeting_shots_are_never_sampled() {
        // Working at 09:00 + meeting at 11:00 and 15:00 → ONLY the working shot is
        // chosen, however many slots are asked for.
        let shots = vec![
            shot_with_status(1, 9, "working"),
            shot_with_status(2, 11, "meeting"),
            shot_with_status(3, 15, "meeting"),
        ];
        let chosen = choose_samples(&shots, 16);
        assert_eq!(chosen.len(), 1);
        assert_eq!(chosen[0].1, Uuid::from_u128(1));
    }

    #[test]
    fn only_working_status_is_eligible() {
        assert!(is_eligible(&shot_with_status(1, 9, "working")));
        for status in ["meeting", "break", "idle", "not_working"] {
            assert!(
                !is_eligible(&shot_with_status(1, 9, status)),
                "{status} must be ineligible"
            );
        }
    }

    #[test]
    fn all_meeting_day_samples_nothing() {
        let shots = vec![
            shot_with_status(1, 9, "meeting"),
            shot_with_status(2, 13, "meeting"),
        ];
        assert!(choose_samples(&shots, 16).is_empty());
    }
}

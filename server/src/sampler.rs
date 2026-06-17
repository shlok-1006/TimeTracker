//! Daily Screenshot Sampler (STEP 9).
//!
//! Picks 4–5 screenshots per employee per day for later analysis, spread across
//! the workday. The day is split into five time-of-day buckets and one random
//! screenshot is drawn from each non-empty bucket.
//!
//! Eligibility: only *Working* screenshots count (`captured_status = 'working'`).
//! The desktop also captures during meetings (tagged `meeting`, Feature 2); those
//! are stored and viewable but are filtered out here and never sampled/analysed.
//!
//! Idempotency (Rules: "never resample same day"): the chosen set is persisted in
//! `analysis_job_samples` and `analysis_jobs` is UNIQUE per (user, day). Re-running
//! `sample_screenshots` for a day that already has samples returns the stored set
//! unchanged — it never re-rolls the random choice.
//!
//! Bucket boundaries are UTC clock hours and cover the full 24h day so no shot is
//! ever ineligible. (They can later be shifted into each employee's local timezone.)

use argon2::password_hash::rand_core::{OsRng, RngCore};
use chrono::{DateTime, Duration, NaiveDate, TimeZone, Timelike, Utc};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;

/// Ordered time-of-day buckets: `(name, start_hour_inclusive, end_hour_exclusive)`.
/// They tile `[0, 24)` so every screenshot lands in exactly one bucket.
const BUCKETS: [(&str, u32, u32); 5] = [
    ("morning", 0, 10),
    ("mid_morning", 10, 12),
    ("noon", 12, 14),
    ("afternoon", 14, 17),
    ("late_afternoon", 17, 24),
];

/// The bucket name for a UTC hour-of-day (always returns one — buckets tile the day).
pub fn bucket_of(hour: u32) -> &'static str {
    for (name, start, end) in BUCKETS {
        if hour >= start && hour < end {
            return name;
        }
    }
    // Unreachable: BUCKETS tile [0, 24). Fall back to the last bucket defensively.
    BUCKETS[BUCKETS.len() - 1].0
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

/// Choose at most one screenshot per bucket (the pure sampling strategy).
/// Applies `is_eligible` first (defense in depth on top of the SQL filter), so
/// non-working shots can never be chosen even if the query were loosened.
/// Returns `(bucket, screenshot_id)` in bucket order — between 0 and 5 entries.
fn choose_samples(shots: &[CandidateShot]) -> Vec<(&'static str, Uuid)> {
    let mut chosen = Vec::with_capacity(BUCKETS.len());
    for (name, _, _) in BUCKETS {
        let in_bucket: Vec<Uuid> = shots
            .iter()
            .filter(|s| is_eligible(s))
            .filter(|s| bucket_of(s.taken_at.hour()) == name)
            .map(|s| s.id)
            .collect();
        if let Some(id) = pick_one(&in_bucket) {
            chosen.push((name, *id));
        }
    }
    chosen
}

/// UTC `[start, end)` bounds of a calendar day.
fn day_bounds(day: NaiveDate) -> (DateTime<Utc>, DateTime<Utc>) {
    let start = Utc.from_utc_datetime(&day.and_hms_opt(0, 0, 0).expect("valid midnight"));
    (start, start + Duration::days(1))
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

/// Sample 4–5 working screenshots for `(user, day)`, spread across the day.
///
/// Idempotent: if the day already has a sampled set, it is returned unchanged
/// (the day is never resampled). Otherwise one screenshot is drawn at random per
/// non-empty time bucket, persisted, and the job marked `sampled`.
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

    for (bucket, screenshot_id) in choose_samples(&shots) {
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

    fn shot_with_status(id: u8, hour: u32, status: &str) -> CandidateShot {
        let d = NaiveDate::from_ymd_opt(2026, 6, 8).unwrap();
        CandidateShot {
            id: Uuid::from_u128(id as u128),
            taken_at: Utc.from_utc_datetime(&d.and_hms_opt(hour, 0, 0).unwrap()),
            captured_status: status.to_string(),
        }
    }

    fn shot(id: u8, hour: u32) -> CandidateShot {
        shot_with_status(id, hour, "working")
    }

    #[test]
    fn buckets_tile_every_hour() {
        for h in 0..24 {
            assert!(!bucket_of(h).is_empty(), "hour {h} has no bucket");
        }
    }

    #[test]
    fn bucket_boundaries_are_correct() {
        assert_eq!(bucket_of(0), "morning");
        assert_eq!(bucket_of(9), "morning");
        assert_eq!(bucket_of(10), "mid_morning");
        assert_eq!(bucket_of(11), "mid_morning");
        assert_eq!(bucket_of(12), "noon");
        assert_eq!(bucket_of(13), "noon");
        assert_eq!(bucket_of(14), "afternoon");
        assert_eq!(bucket_of(16), "afternoon");
        assert_eq!(bucket_of(17), "late_afternoon");
        assert_eq!(bucket_of(23), "late_afternoon");
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

    #[test]
    fn full_day_yields_one_per_bucket() {
        // One shot in each of the five buckets → exactly 5 chosen, one per bucket.
        let shots = vec![shot(1, 8), shot(2, 11), shot(3, 13), shot(4, 15), shot(5, 19)];
        let chosen = choose_samples(&shots);
        assert_eq!(chosen.len(), 5);
        let buckets: Vec<&str> = chosen.iter().map(|(b, _)| *b).collect();
        assert_eq!(
            buckets,
            vec!["morning", "mid_morning", "noon", "afternoon", "late_afternoon"]
        );
    }

    #[test]
    fn sparse_day_skips_empty_buckets() {
        // Activity only in morning + afternoon → 2 chosen.
        let shots = vec![shot(1, 7), shot(2, 9), shot(3, 15)];
        let chosen = choose_samples(&shots);
        assert_eq!(chosen.len(), 2);
        assert_eq!(chosen[0].0, "morning");
        assert_eq!(chosen[1].0, "afternoon");
        // The morning pick is one of the two morning shots.
        assert!([Uuid::from_u128(1), Uuid::from_u128(2)].contains(&chosen[0].1));
    }

    #[test]
    fn no_screenshots_yields_empty() {
        assert!(choose_samples(&[]).is_empty());
    }

    #[test]
    fn meeting_shots_are_never_sampled() {
        // Working at 09:00 + meeting at 11:00 and 15:00 → ONLY the working shot
        // is chosen, even though the meeting shots sit in otherwise-empty buckets.
        let shots = vec![
            shot_with_status(1, 9, "working"),
            shot_with_status(2, 11, "meeting"),
            shot_with_status(3, 15, "meeting"),
        ];
        let chosen = choose_samples(&shots);
        assert_eq!(chosen.len(), 1);
        assert_eq!(chosen[0].0, "morning");
        assert_eq!(chosen[0].1, Uuid::from_u128(1));
    }

    #[test]
    fn only_working_status_is_eligible() {
        assert!(is_eligible(&shot_with_status(1, 9, "working")));
        for status in ["meeting", "break", "idle", "not_working"] {
            assert!(!is_eligible(&shot_with_status(1, 9, status)), "{status} must be ineligible");
        }
    }

    #[test]
    fn all_meeting_day_samples_nothing() {
        let shots = vec![
            shot_with_status(1, 9, "meeting"),
            shot_with_status(2, 13, "meeting"),
        ];
        assert!(choose_samples(&shots).is_empty());
    }
}

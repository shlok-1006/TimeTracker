//! A sampled screenshot that turns out to be a Google Meet call or Slack huddle:
//! it is re-tagged `meeting`, loses any verdict, and is replaced by another shot
//! from the same part of the day. Hits a live DB via DATABASE_URL; skips if unset.
//!
//! Uses year-2020 dates for a fresh user so it never collides with real data.

use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc, Weekday};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use uuid::Uuid;

use server::db::{analysis_results, screenshots, users};
use server::role::UserRole;
use server::sampler::{self, SampledShot};
use server::vision_analyzer::AnalysisResult;

async fn pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .ok()
}

fn a_monday() -> NaiveDate {
    let mut day = NaiveDate::from_ymd_opt(2020, 7, 1).unwrap();
    while day.weekday() != Weekday::Mon {
        day += Duration::days(1);
    }
    day
}

struct Day {
    pool: PgPool,
    emp: Uuid,
    day: NaiveDate,
    start: DateTime<Utc>,
    job: Uuid,
}

impl Day {
    async fn new(pool: PgPool) -> Self {
        let emp = users::create(
            &pool,
            "Meeting Detection",
            &format!("meet-{}@t.local", Uuid::new_v4()),
            "h",
            UserRole::Employee,
            None,
        )
        .await
        .unwrap()
        .id;
        let day = a_monday();
        let (start, _) = server::org_time::day_bounds_utc(day);
        let job = sampler::create_daily_job(&pool, emp, day).await.unwrap().id;
        Self {
            pool,
            emp,
            day,
            start,
            job,
        }
    }

    /// A working screenshot at `h:m` org-local time.
    async fn shot(&self, h: i64, m: i64) -> Uuid {
        screenshots::insert(
            &self.pool,
            self.emp,
            &format!("{}/20200706/{}.jpg", self.emp, Uuid::new_v4()),
            self.start + Duration::hours(h) + Duration::minutes(m),
            None,
            "working",
        )
        .await
        .unwrap()
    }

    async fn sample(&self, id: Uuid, bucket: &str) {
        sqlx::query(
            "INSERT INTO analysis_job_samples (job_id, screenshot_id, bucket) VALUES ($1, $2, $3)",
        )
        .bind(self.job)
        .bind(id)
        .bind(bucket)
        .execute(&self.pool)
        .await
        .unwrap();
    }

    async fn sampled(&self, id: Uuid) -> SampledShot {
        sampler::load_existing_job(&self.pool, self.emp, self.day)
            .await
            .unwrap()
            .unwrap()
            .1
            .into_iter()
            .find(|s| s.screenshot_id == id)
            .expect("in the sampled set")
    }

    /// Tag `id` as a call and draw its replacement, as the day analysis does.
    async fn meeting_found(&self, id: Uuid) -> Option<Uuid> {
        let s = self.sampled(id).await;
        assert!(screenshots::mark_meeting_detected(&self.pool, id)
            .await
            .unwrap());
        sampler::add_replacement(&self.pool, self.job, self.emp, self.day, &s)
            .await
            .unwrap()
            .map(|r| {
                assert_eq!(r.bucket, s.bucket, "the replacement keeps the slot");
                r.screenshot_id
            })
    }
}

#[tokio::test]
async fn a_meeting_shot_is_tagged_and_replaced_from_the_same_stretch() {
    let Some(pool) = pool().await else {
        eprintln!("skipping meeting-detection test: DATABASE_URL not set");
        return;
    };
    let d = Day::new(pool.clone()).await;
    let nine = d.shot(9, 0).await;
    let ten = d.shot(10, 0).await;
    let ten_05 = d.shot(10, 5).await;
    let ten_40 = d.shot(10, 40).await;
    let one = d.shot(13, 0).await;
    let three = d.shot(15, 0).await; // after the last sample: never a stand-in for 10:00
    d.sample(nine, "slot_01").await;
    d.sample(ten, "slot_02").await;
    d.sample(one, "slot_03").await;

    // A verdict stored before the call was recognised must not survive it.
    analysis_results::upsert(
        &pool,
        d.job,
        ten,
        &AnalysisResult {
            verdict: "aligned".into(),
            matched_ticket_id: None,
            confidence: 0.9,
            observed: "Meet call".into(),
            rationale: "x".into(),
            inconclusive_reason: None,
            model: "m".into(),
        },
    )
    .await
    .unwrap();

    // 10:00 is a call. 10:05 is the same call; 10:40 is clear of it.
    assert_eq!(d.meeting_found(ten).await, Some(ten_40));
    let (status, detected): (String, Option<DateTime<Utc>>) = sqlx::query_as(
        "SELECT captured_status, meeting_detected_at FROM screenshots WHERE id = $1",
    )
    .bind(ten)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(status, "meeting");
    assert!(detected.is_some(), "the AI's tag is stamped");
    let verdicts: i64 =
        sqlx::query_scalar("SELECT count(*) FROM analysis_results WHERE screenshot_id = $1")
            .bind(ten)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(verdicts, 0, "a meeting shot keeps no score");
    assert!(
        !screenshots::mark_meeting_detected(&pool, ten)
            .await
            .unwrap(),
        "tagging twice changes nothing"
    );

    // 10:40 was the call too. Only 10:05 is left between 09:00 and 13:00 — close
    // to both meetings, but better than nothing.
    assert_eq!(d.meeting_found(ten_40).await, Some(ten_05));
    // And 10:05 as well: nothing left in that stretch, and 15:00 is not borrowed.
    assert_eq!(d.meeting_found(ten_05).await, None);
    let _ = three;

    // The meeting rows stay in the set, so the day still counts as sampled and
    // the nightly catch-up does not pick it up again.
    let late = screenshots::users_with_unsampled_working_shots(&pool, d.day)
        .await
        .unwrap();
    assert!(!late.contains(&d.emp));
    let samples = sampler::sample_screenshots(&pool, d.emp, d.day)
        .await
        .unwrap();
    assert_eq!(
        samples.len(),
        5,
        "never resampled: 3 drawn + 2 replacements"
    );
    assert_eq!(
        samples
            .iter()
            .filter(|s| s.captured_status == "working")
            .count(),
        2,
        "09:00 and 13:00 are still the day's scorable samples"
    );

    users::delete(&pool, d.emp).await.unwrap();
}

//! Analysis-reports repository (Feature 1, Rule 7): the daily per-employee
//! aggregate of AI work-verification verdicts.

use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;

/// A stored daily report row.
#[derive(Debug, Clone, Serialize)]
pub struct AnalysisReport {
    pub id: Uuid,
    pub user_id: Uuid,
    pub day: NaiveDate,
    pub job_id: Uuid,
    pub total_analyzed: i32,
    pub aligned_count: i32,
    pub partially_count: i32,
    pub not_aligned_count: i32,
    pub inconclusive_count: i32,
    /// The day score (0–100): the screenshot alignment scaled down by idle beyond the allowance
    /// (report_service::idle_adjusted). Screenshot-only on reports built before migration 0061.
    pub alignment_score: f64,
    /// The verdicts alone, before idle (None on reports built before migration 0061).
    pub screen_alignment_score: Option<f64>,
    /// Active + meeting seconds that day, as the idle adjustment saw them (None before 0061).
    pub worked_seconds: Option<i32>,
    /// Idle seconds that day, as the idle adjustment saw them (None before 0061).
    pub idle_seconds: Option<i32>,
    pub summary_text: String,
    pub model: String,
    pub created_at: DateTime<Utc>,
}

/// Fields needed to create/refresh a report (everything except the generated
/// id and created_at).
#[derive(Debug, Clone)]
pub struct ReportInput {
    pub user_id: Uuid,
    pub day: NaiveDate,
    pub job_id: Uuid,
    pub total_analyzed: i32,
    pub aligned_count: i32,
    pub partially_count: i32,
    pub not_aligned_count: i32,
    pub inconclusive_count: i32,
    pub alignment_score: f64,
    pub screen_alignment_score: f64,
    /// None = the day's activity couldn't be read (stored NULL, not a real zero).
    pub worked_seconds: Option<i32>,
    pub idle_seconds: Option<i32>,
    pub summary_text: String,
    pub model: String,
}

/// Insert or refresh the report for a `(user, day)` (idempotent via the unique
/// constraint). Re-running an analysis updates the existing row in place.
pub async fn upsert(pool: &PgPool, r: &ReportInput) -> Result<AnalysisReport, AppError> {
    let row = sqlx::query!(
        r#"
        INSERT INTO analysis_reports
            (user_id, day, job_id, total_analyzed, aligned_count, partially_count,
             not_aligned_count, inconclusive_count, alignment_score, summary_text, model,
             screen_alignment_score, worked_seconds, idle_seconds)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
        ON CONFLICT (user_id, day) DO UPDATE SET
            job_id             = EXCLUDED.job_id,
            total_analyzed     = EXCLUDED.total_analyzed,
            aligned_count      = EXCLUDED.aligned_count,
            partially_count    = EXCLUDED.partially_count,
            not_aligned_count  = EXCLUDED.not_aligned_count,
            inconclusive_count = EXCLUDED.inconclusive_count,
            alignment_score    = EXCLUDED.alignment_score,
            screen_alignment_score = EXCLUDED.screen_alignment_score,
            worked_seconds     = EXCLUDED.worked_seconds,
            idle_seconds       = EXCLUDED.idle_seconds,
            summary_text       = EXCLUDED.summary_text,
            model              = EXCLUDED.model,
            created_at         = now()
        RETURNING id, user_id, day, job_id, total_analyzed, aligned_count, partially_count,
                  not_aligned_count, inconclusive_count, alignment_score, screen_alignment_score,
                  worked_seconds, idle_seconds, summary_text, model, created_at
        "#,
        r.user_id,
        r.day,
        r.job_id,
        r.total_analyzed,
        r.aligned_count,
        r.partially_count,
        r.not_aligned_count,
        r.inconclusive_count,
        r.alignment_score,
        r.summary_text,
        r.model,
        r.screen_alignment_score,
        r.worked_seconds,
        r.idle_seconds,
    )
    .fetch_one(pool)
    .await?;

    Ok(map_row(
        row.id,
        row.user_id,
        row.day,
        row.job_id,
        row.total_analyzed,
        row.aligned_count,
        row.partially_count,
        row.not_aligned_count,
        row.inconclusive_count,
        row.alignment_score,
        IdleParts {
            screen_alignment_score: row.screen_alignment_score,
            worked_seconds: row.worked_seconds,
            idle_seconds: row.idle_seconds,
        },
        row.summary_text,
        row.model,
        row.created_at,
    ))
}

/// Update only the idle-score columns of an existing report (report_service::rescore_idle), stamping `created_at`
/// so data that arrived before now isn't treated as late again.
pub async fn set_idle_score(
    pool: &PgPool,
    user_id: Uuid,
    day: NaiveDate,
    alignment_score: f64,
    screen_alignment_score: f64,
    worked_seconds: i32,
    idle_seconds: i32,
) -> Result<AnalysisReport, AppError> {
    let r = sqlx::query!(
        r#"UPDATE analysis_reports
              SET alignment_score = $3, screen_alignment_score = $4, worked_seconds = $5,
                  idle_seconds = $6, created_at = now()
            WHERE user_id = $1 AND day = $2
        RETURNING id, user_id, day, job_id, total_analyzed, aligned_count, partially_count,
                  not_aligned_count, inconclusive_count, alignment_score, screen_alignment_score,
                  worked_seconds, idle_seconds, summary_text, model, created_at"#,
        user_id,
        day,
        alignment_score,
        screen_alignment_score,
        worked_seconds,
        idle_seconds
    )
    .fetch_one(pool)
    .await?;
    Ok(map_row(
        r.id,
        r.user_id,
        r.day,
        r.job_id,
        r.total_analyzed,
        r.aligned_count,
        r.partially_count,
        r.not_aligned_count,
        r.inconclusive_count,
        r.alignment_score,
        IdleParts {
            screen_alignment_score: r.screen_alignment_score,
            worked_seconds: r.worked_seconds,
            idle_seconds: r.idle_seconds,
        },
        r.summary_text,
        r.model,
        r.created_at,
    ))
}

/// Reports in `[from, to]` (with real verdicts) whose user has activity that reached the server AFTER the report was
/// built and falls inside that report's org day — they were scored on incomplete idle (a late / VPN sync).
/// `offset_minutes` is the org timezone offset (org_time::offset_minutes).
pub async fn stale_for_late_activity(
    pool: &PgPool,
    from: NaiveDate,
    to: NaiveDate,
    offset_minutes: i32,
) -> Result<Vec<AnalysisReport>, AppError> {
    let rows = sqlx::query!(
        r#"SELECT r.id, r.user_id, r.day, r.job_id, r.total_analyzed, r.aligned_count,
                  r.partially_count, r.not_aligned_count, r.inconclusive_count, r.alignment_score,
                  r.screen_alignment_score, r.worked_seconds, r.idle_seconds, r.summary_text,
                  r.model, r.created_at
           FROM analysis_reports r
           WHERE r.day >= $1 AND r.day <= $2
             AND r.aligned_count + r.partially_count + r.not_aligned_count > 0
             AND EXISTS (
               SELECT 1 FROM intervals i
                WHERE i.user_id = r.user_id
                  AND i.received_at > r.created_at
                  AND i.end_utc > ((r.day::timestamp - make_interval(mins => $3)) AT TIME ZONE 'UTC')
                  AND i.start_utc < ((r.day::timestamp + interval '1 day' - make_interval(mins => $3)) AT TIME ZONE 'UTC'))"#,
        from,
        to,
        offset_minutes
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            map_row(
                r.id,
                r.user_id,
                r.day,
                r.job_id,
                r.total_analyzed,
                r.aligned_count,
                r.partially_count,
                r.not_aligned_count,
                r.inconclusive_count,
                r.alignment_score,
                IdleParts {
                    screen_alignment_score: r.screen_alignment_score,
                    worked_seconds: r.worked_seconds,
                    idle_seconds: r.idle_seconds,
                },
                r.summary_text,
                r.model,
                r.created_at,
            )
        })
        .collect())
}

/// The report for a specific `(user, day)`, if one exists.
pub async fn get(
    pool: &PgPool,
    user_id: Uuid,
    day: NaiveDate,
) -> Result<Option<AnalysisReport>, AppError> {
    let row = sqlx::query!(
        r#"SELECT id, user_id, day, job_id, total_analyzed, aligned_count, partially_count,
                  not_aligned_count, inconclusive_count, alignment_score, screen_alignment_score,
                  worked_seconds, idle_seconds, summary_text, model, created_at
           FROM analysis_reports WHERE user_id = $1 AND day = $2"#,
        user_id,
        day
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| {
        map_row(
            r.id,
            r.user_id,
            r.day,
            r.job_id,
            r.total_analyzed,
            r.aligned_count,
            r.partially_count,
            r.not_aligned_count,
            r.inconclusive_count,
            r.alignment_score,
            IdleParts {
                screen_alignment_score: r.screen_alignment_score,
                worked_seconds: r.worked_seconds,
                idle_seconds: r.idle_seconds,
            },
            r.summary_text,
            r.model,
            r.created_at,
        )
    }))
}

/// Whether HR has already been alerted about this `(user, day)` report's low
/// score (so the nightly job never re-notifies on a re-run/restart).
pub async fn low_score_notified(
    pool: &PgPool,
    user_id: Uuid,
    day: NaiveDate,
) -> Result<bool, AppError> {
    let row = sqlx::query!(
        "SELECT low_score_notified_at FROM analysis_reports WHERE user_id = $1 AND day = $2",
        user_id,
        day
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.and_then(|r| r.low_score_notified_at).is_some())
}

/// Stamp `low_score_notified_at = now()` after HR has been emailed.
pub async fn mark_low_score_notified(
    pool: &PgPool,
    user_id: Uuid,
    day: NaiveDate,
) -> Result<(), AppError> {
    sqlx::query!(
        "UPDATE analysis_reports SET low_score_notified_at = now() WHERE user_id = $1 AND day = $2",
        user_id,
        day
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// A report joined with the employee's identity, for HR/PM roster views.
#[derive(Debug, Clone, Serialize)]
pub struct ReportRow {
    pub user_id: Uuid,
    pub employee_name: String,
    pub employee_email: String,
    pub day: NaiveDate,
    pub total_analyzed: i32,
    pub aligned_count: i32,
    pub partially_count: i32,
    pub not_aligned_count: i32,
    pub inconclusive_count: i32,
    /// The day score (idle-adjusted from migration 0061 on).
    pub alignment_score: f64,
    pub screen_alignment_score: Option<f64>,
    pub worked_seconds: Option<i32>,
    pub idle_seconds: Option<i32>,
    pub summary_text: String,
    pub model: String,
    pub created_at: DateTime<Utc>,
}

/// All reports for a given day. `manager_id = Some(pm)` scopes to that manager's
/// team; `None` (HR) returns everyone's.
pub async fn list_for_day(
    pool: &PgPool,
    manager_id: Option<Uuid>,
    day: NaiveDate,
) -> Result<Vec<ReportRow>, AppError> {
    let rows = sqlx::query!(
        r#"SELECT r.user_id, u.name AS employee_name, u.email AS employee_email, r.day,
                  r.total_analyzed, r.aligned_count, r.partially_count, r.not_aligned_count,
                  r.inconclusive_count, r.alignment_score, r.screen_alignment_score, r.worked_seconds,
                  r.idle_seconds, r.summary_text, r.model, r.created_at
           FROM analysis_reports r
           JOIN users u ON u.id = r.user_id
           WHERE r.day = $1
             AND ($2::uuid IS NULL
                  OR EXISTS (SELECT 1 FROM user_managers um
                             WHERE um.user_id = u.id AND um.manager_id = $2))
           ORDER BY u.name"#,
        day,
        manager_id
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| ReportRow {
            user_id: r.user_id,
            employee_name: r.employee_name,
            employee_email: r.employee_email,
            day: r.day,
            total_analyzed: r.total_analyzed,
            aligned_count: r.aligned_count,
            partially_count: r.partially_count,
            not_aligned_count: r.not_aligned_count,
            inconclusive_count: r.inconclusive_count,
            alignment_score: r.alignment_score,
            screen_alignment_score: r.screen_alignment_score,
            worked_seconds: r.worked_seconds,
            idle_seconds: r.idle_seconds,
            summary_text: r.summary_text,
            model: r.model,
            created_at: r.created_at,
        })
        .collect())
}

/// All of a user's reports, most recent day first.
pub async fn list_for_user(pool: &PgPool, user_id: Uuid) -> Result<Vec<AnalysisReport>, AppError> {
    let rows = sqlx::query!(
        r#"SELECT id, user_id, day, job_id, total_analyzed, aligned_count, partially_count,
                  not_aligned_count, inconclusive_count, alignment_score, screen_alignment_score,
                  worked_seconds, idle_seconds, summary_text, model, created_at
           FROM analysis_reports WHERE user_id = $1 ORDER BY day DESC"#,
        user_id
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| {
            map_row(
                r.id,
                r.user_id,
                r.day,
                r.job_id,
                r.total_analyzed,
                r.aligned_count,
                r.partially_count,
                r.not_aligned_count,
                r.inconclusive_count,
                r.alignment_score,
                IdleParts {
                    screen_alignment_score: r.screen_alignment_score,
                    worked_seconds: r.worked_seconds,
                    idle_seconds: r.idle_seconds,
                },
                r.summary_text,
                r.model,
                r.created_at,
            )
        })
        .collect())
}

#[allow(clippy::too_many_arguments)]
fn map_row(
    id: Uuid,
    user_id: Uuid,
    day: NaiveDate,
    job_id: Uuid,
    total_analyzed: i32,
    aligned_count: i32,
    partially_count: i32,
    not_aligned_count: i32,
    inconclusive_count: i32,
    alignment_score: f64,
    idle: IdleParts,
    summary_text: String,
    model: String,
    created_at: DateTime<Utc>,
) -> AnalysisReport {
    AnalysisReport {
        id,
        user_id,
        day,
        job_id,
        total_analyzed,
        aligned_count,
        partially_count,
        not_aligned_count,
        inconclusive_count,
        alignment_score,
        screen_alignment_score: idle.screen_alignment_score,
        worked_seconds: idle.worked_seconds,
        idle_seconds: idle.idle_seconds,
        summary_text,
        model,
        created_at,
    }
}

/// The idle-score columns of a row (migration 0061), all NULL on older reports.
struct IdleParts {
    screen_alignment_score: Option<f64>,
    worked_seconds: Option<i32>,
    idle_seconds: Option<i32>,
}

/// One user's daily reports across an inclusive day range — the source for the
/// monthly rollup's "how many days scored above the threshold" figure.
pub async fn list_for_range(
    pool: &PgPool,
    user_id: Uuid,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<AnalysisReport>, AppError> {
    let rows = sqlx::query!(
        r#"SELECT id, user_id, day, job_id, total_analyzed, aligned_count, partially_count,
                  not_aligned_count, inconclusive_count, alignment_score, screen_alignment_score,
                  worked_seconds, idle_seconds, summary_text, model, created_at
           FROM analysis_reports
           WHERE user_id = $1 AND day >= $2 AND day <= $3
           ORDER BY day"#,
        user_id,
        from,
        to
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| AnalysisReport {
            id: r.id,
            user_id: r.user_id,
            day: r.day,
            job_id: r.job_id,
            total_analyzed: r.total_analyzed,
            aligned_count: r.aligned_count,
            partially_count: r.partially_count,
            not_aligned_count: r.not_aligned_count,
            inconclusive_count: r.inconclusive_count,
            alignment_score: r.alignment_score,
            screen_alignment_score: r.screen_alignment_score,
            worked_seconds: r.worked_seconds,
            idle_seconds: r.idle_seconds,
            summary_text: r.summary_text,
            model: r.model,
            created_at: r.created_at,
        })
        .collect())
}

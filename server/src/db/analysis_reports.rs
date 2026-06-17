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
    pub alignment_score: f64,
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
             not_aligned_count, inconclusive_count, alignment_score, summary_text, model)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        ON CONFLICT (user_id, day) DO UPDATE SET
            job_id             = EXCLUDED.job_id,
            total_analyzed     = EXCLUDED.total_analyzed,
            aligned_count      = EXCLUDED.aligned_count,
            partially_count    = EXCLUDED.partially_count,
            not_aligned_count  = EXCLUDED.not_aligned_count,
            inconclusive_count = EXCLUDED.inconclusive_count,
            alignment_score    = EXCLUDED.alignment_score,
            summary_text       = EXCLUDED.summary_text,
            model              = EXCLUDED.model,
            created_at         = now()
        RETURNING id, user_id, day, job_id, total_analyzed, aligned_count, partially_count,
                  not_aligned_count, inconclusive_count, alignment_score, summary_text, model, created_at
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
        row.summary_text,
        row.model,
        row.created_at,
    ))
}

/// The report for a specific `(user, day)`, if one exists.
pub async fn get(
    pool: &PgPool,
    user_id: Uuid,
    day: NaiveDate,
) -> Result<Option<AnalysisReport>, AppError> {
    let row = sqlx::query!(
        r#"SELECT id, user_id, day, job_id, total_analyzed, aligned_count, partially_count,
                  not_aligned_count, inconclusive_count, alignment_score, summary_text, model, created_at
           FROM analysis_reports WHERE user_id = $1 AND day = $2"#,
        user_id,
        day
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| {
        map_row(
            r.id, r.user_id, r.day, r.job_id, r.total_analyzed, r.aligned_count, r.partially_count,
            r.not_aligned_count, r.inconclusive_count, r.alignment_score, r.summary_text, r.model,
            r.created_at,
        )
    }))
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
    pub alignment_score: f64,
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
                  r.inconclusive_count, r.alignment_score, r.summary_text, r.model, r.created_at
           FROM analysis_reports r
           JOIN users u ON u.id = r.user_id
           WHERE r.day = $1 AND ($2::uuid IS NULL OR u.manager_id = $2)
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
                  not_aligned_count, inconclusive_count, alignment_score, summary_text, model, created_at
           FROM analysis_reports WHERE user_id = $1 ORDER BY day DESC"#,
        user_id
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| {
            map_row(
                r.id, r.user_id, r.day, r.job_id, r.total_analyzed, r.aligned_count,
                r.partially_count, r.not_aligned_count, r.inconclusive_count, r.alignment_score,
                r.summary_text, r.model, r.created_at,
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
        summary_text,
        model,
        created_at,
    }
}

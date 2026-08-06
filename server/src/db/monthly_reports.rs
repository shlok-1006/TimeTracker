//! Monthly summary reports: one frozen row per (employee, month).
//!
//! The month key is the FIRST DAY of the org-local (IST) month, matching the
//! day-key basis used by analysis_reports (see `crate::org_time`). Writes are
//! upserts so an on-demand regeneration and the month-end scheduler converge on
//! the same row instead of racing to insert duplicates.

use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;

/// One day inside a monthly report's frozen series.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct MonthDay {
    pub day: NaiveDate,
    pub worked_seconds: i32,
    pub status: String,
    /// None when that day was never analysed (no daily report).
    pub alignment_score: Option<f64>,
    pub total_analyzed: i32,
}

#[derive(Debug, Clone, Serialize)]
pub struct MonthlyReport {
    pub id: Uuid,
    pub user_id: Uuid,
    /// First day of the org-local month.
    pub month: NaiveDate,
    pub worked_seconds: i64,
    pub grace_seconds: i64,
    pub days_present: i32,
    pub days_partial: i32,
    pub days_absent: i32,
    pub days_leave: i32,
    pub days_holiday: i32,
    pub days_weekend: i32,
    pub days_analyzed: i32,
    pub days_above_threshold: i32,
    pub alignment_threshold: f64,
    pub avg_alignment_score: Option<f64>,
    pub screenshots_analyzed: i32,
    pub days: Vec<MonthDay>,
    /// None = generated automatically by the month-end scheduler.
    pub generated_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Everything the service computes; the row is derived from this.
pub struct MonthlyReportInput {
    pub user_id: Uuid,
    pub month: NaiveDate,
    pub worked_seconds: i64,
    pub grace_seconds: i64,
    pub days_present: i32,
    pub days_partial: i32,
    pub days_absent: i32,
    pub days_leave: i32,
    pub days_holiday: i32,
    pub days_weekend: i32,
    pub days_analyzed: i32,
    pub days_above_threshold: i32,
    pub alignment_threshold: f64,
    pub avg_alignment_score: Option<f64>,
    pub screenshots_analyzed: i32,
    pub days: Vec<MonthDay>,
    pub generated_by: Option<Uuid>,
}

/// Insert or refresh a month's report (idempotent per (user, month)).
pub async fn upsert(pool: &PgPool, input: MonthlyReportInput) -> Result<MonthlyReport, AppError> {
    let days_json = serde_json::to_value(&input.days).unwrap_or_else(|_| serde_json::json!([]));
    let row = sqlx::query!(
        r#"
        INSERT INTO monthly_reports (
            user_id, month, worked_seconds, grace_seconds,
            days_present, days_partial, days_absent, days_leave, days_holiday, days_weekend,
            days_analyzed, days_above_threshold, alignment_threshold, avg_alignment_score,
            screenshots_analyzed, days, generated_by
        )
        VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)
        ON CONFLICT (user_id, month) DO UPDATE SET
            worked_seconds = EXCLUDED.worked_seconds,
            grace_seconds = EXCLUDED.grace_seconds,
            days_present = EXCLUDED.days_present,
            days_partial = EXCLUDED.days_partial,
            days_absent = EXCLUDED.days_absent,
            days_leave = EXCLUDED.days_leave,
            days_holiday = EXCLUDED.days_holiday,
            days_weekend = EXCLUDED.days_weekend,
            days_analyzed = EXCLUDED.days_analyzed,
            days_above_threshold = EXCLUDED.days_above_threshold,
            alignment_threshold = EXCLUDED.alignment_threshold,
            avg_alignment_score = EXCLUDED.avg_alignment_score,
            screenshots_analyzed = EXCLUDED.screenshots_analyzed,
            days = EXCLUDED.days,
            generated_by = EXCLUDED.generated_by,
            updated_at = now()
        RETURNING id, user_id, month, worked_seconds, grace_seconds,
                  days_present, days_partial, days_absent, days_leave, days_holiday, days_weekend,
                  days_analyzed, days_above_threshold, alignment_threshold, avg_alignment_score,
                  screenshots_analyzed, days, generated_by, created_at, updated_at
        "#,
        input.user_id,
        input.month,
        input.worked_seconds,
        input.grace_seconds,
        input.days_present,
        input.days_partial,
        input.days_absent,
        input.days_leave,
        input.days_holiday,
        input.days_weekend,
        input.days_analyzed,
        input.days_above_threshold,
        input.alignment_threshold,
        input.avg_alignment_score,
        input.screenshots_analyzed,
        days_json,
        input.generated_by,
    )
    .fetch_one(pool)
    .await?;

    Ok(MonthlyReport {
        id: row.id,
        user_id: row.user_id,
        month: row.month,
        worked_seconds: row.worked_seconds,
        grace_seconds: row.grace_seconds,
        days_present: row.days_present,
        days_partial: row.days_partial,
        days_absent: row.days_absent,
        days_leave: row.days_leave,
        days_holiday: row.days_holiday,
        days_weekend: row.days_weekend,
        days_analyzed: row.days_analyzed,
        days_above_threshold: row.days_above_threshold,
        alignment_threshold: row.alignment_threshold,
        avg_alignment_score: row.avg_alignment_score,
        screenshots_analyzed: row.screenshots_analyzed,
        days: serde_json::from_value(row.days).unwrap_or_default(),
        generated_by: row.generated_by,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

/// One employee's stored report for a month (None when never generated).
pub async fn get(
    pool: &PgPool,
    user_id: Uuid,
    month: NaiveDate,
) -> Result<Option<MonthlyReport>, AppError> {
    let row = sqlx::query!(
        r#"SELECT id, user_id, month, worked_seconds, grace_seconds,
                  days_present, days_partial, days_absent, days_leave, days_holiday, days_weekend,
                  days_analyzed, days_above_threshold, alignment_threshold, avg_alignment_score,
                  screenshots_analyzed, days, generated_by, created_at, updated_at
           FROM monthly_reports WHERE user_id = $1 AND month = $2"#,
        user_id,
        month
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| MonthlyReport {
        id: r.id,
        user_id: r.user_id,
        month: r.month,
        worked_seconds: r.worked_seconds,
        grace_seconds: r.grace_seconds,
        days_present: r.days_present,
        days_partial: r.days_partial,
        days_absent: r.days_absent,
        days_leave: r.days_leave,
        days_holiday: r.days_holiday,
        days_weekend: r.days_weekend,
        days_analyzed: r.days_analyzed,
        days_above_threshold: r.days_above_threshold,
        alignment_threshold: r.alignment_threshold,
        avg_alignment_score: r.avg_alignment_score,
        screenshots_analyzed: r.screenshots_analyzed,
        days: serde_json::from_value(r.days).unwrap_or_default(),
        generated_by: r.generated_by,
        created_at: r.created_at,
        updated_at: r.updated_at,
    }))
}

/// A roster row: one employee's monthly headline figures (no daily series — the
/// list view doesn't need it and the payload stays small).
#[derive(Debug, Serialize)]
pub struct MonthlyRosterRow {
    pub user_id: Uuid,
    pub name: String,
    pub email: String,
    pub worked_seconds: i64,
    pub days_present: i32,
    pub days_partial: i32,
    pub days_absent: i32,
    pub days_analyzed: i32,
    pub days_above_threshold: i32,
    pub avg_alignment_score: Option<f64>,
    pub updated_at: DateTime<Utc>,
}

/// Everyone's report for a month. `manager_id` = None for HR (all employees);
/// Some(pm) restricts to that PM's reports — same scoping shape as
/// `analysis_reports::list_for_day`.
pub async fn list_for_month(
    pool: &PgPool,
    manager_id: Option<Uuid>,
    month: NaiveDate,
) -> Result<Vec<MonthlyRosterRow>, AppError> {
    let rows = sqlx::query!(
        r#"SELECT m.user_id, u.name, u.email, m.worked_seconds,
                  m.days_present, m.days_partial, m.days_absent,
                  m.days_analyzed, m.days_above_threshold, m.avg_alignment_score, m.updated_at
           FROM monthly_reports m
           JOIN users u ON u.id = m.user_id
           WHERE m.month = $1
             AND ($2::uuid IS NULL
                  OR EXISTS (SELECT 1 FROM user_managers um
                             WHERE um.user_id = u.id AND um.manager_id = $2))
           ORDER BY u.name"#,
        month,
        manager_id
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| MonthlyRosterRow {
            user_id: r.user_id,
            name: r.name,
            email: r.email,
            worked_seconds: r.worked_seconds,
            days_present: r.days_present,
            days_partial: r.days_partial,
            days_absent: r.days_absent,
            days_analyzed: r.days_analyzed,
            days_above_threshold: r.days_above_threshold,
            avg_alignment_score: r.avg_alignment_score,
            updated_at: r.updated_at,
        })
        .collect())
}

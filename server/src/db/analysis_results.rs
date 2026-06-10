//! Analysis-results repository (STEP 10, Rule 7).

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;
use crate::vision_analyzer::AnalysisResult;

/// A stored analysis row (for listing in the dashboard).
#[derive(Debug, Clone, Serialize)]
pub struct StoredResult {
    pub id: Uuid,
    pub screenshot_id: Uuid,
    pub verdict: String,
    pub matched_ticket: Option<String>,
    pub confidence: f64,
    pub observed: String,
    pub rationale: String,
    pub inconclusive_reason: Option<String>,
    pub model: String,
    pub created_at: DateTime<Utc>,
}

/// Insert (or replace) the analysis result for a `(job, screenshot)` pair.
pub async fn upsert(
    pool: &PgPool,
    job_id: Uuid,
    screenshot_id: Uuid,
    r: &AnalysisResult,
) -> Result<Uuid, AppError> {
    let row = sqlx::query!(
        r#"
        INSERT INTO analysis_results
            (job_id, screenshot_id, verdict, matched_ticket, confidence,
             observed, rationale, inconclusive_reason, model)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        ON CONFLICT (job_id, screenshot_id) DO UPDATE SET
            verdict             = EXCLUDED.verdict,
            matched_ticket      = EXCLUDED.matched_ticket,
            confidence          = EXCLUDED.confidence,
            observed            = EXCLUDED.observed,
            rationale           = EXCLUDED.rationale,
            inconclusive_reason = EXCLUDED.inconclusive_reason,
            model               = EXCLUDED.model,
            created_at          = now()
        RETURNING id
        "#,
        job_id,
        screenshot_id,
        r.verdict,
        r.matched_ticket_id,
        r.confidence,
        r.observed,
        r.rationale,
        r.inconclusive_reason,
        r.model,
    )
    .fetch_one(pool)
    .await?;
    Ok(row.id)
}

/// All analysis results for a job, oldest first.
pub async fn list_for_job(pool: &PgPool, job_id: Uuid) -> Result<Vec<StoredResult>, AppError> {
    let rows = sqlx::query!(
        r#"
        SELECT id, screenshot_id, verdict, matched_ticket, confidence,
               observed, rationale, inconclusive_reason, model, created_at
        FROM analysis_results
        WHERE job_id = $1
        ORDER BY created_at
        "#,
        job_id
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| StoredResult {
            id: r.id,
            screenshot_id: r.screenshot_id,
            verdict: r.verdict,
            matched_ticket: r.matched_ticket,
            confidence: r.confidence,
            observed: r.observed,
            rationale: r.rationale,
            inconclusive_reason: r.inconclusive_reason,
            model: r.model,
            created_at: r.created_at,
        })
        .collect())
}

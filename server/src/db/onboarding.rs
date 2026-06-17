//! Onboarding repository (Feature 6A, Rule 7): stages, candidates, checklist
//! tasks, and document metadata.

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;

#[derive(Debug, Clone, Serialize)]
pub struct Stage {
    pub id: Uuid,
    pub name: String,
    pub sequence: i32,
}

#[derive(Debug, Clone, Serialize)]
pub struct Candidate {
    pub id: Uuid,
    pub name: String,
    pub email: String,
    pub position: String,
    pub stage_id: Uuid,
    pub stage_name: String,
    pub status: String,
    pub converted_user_id: Option<Uuid>,
    pub hired_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CandidateTask {
    pub id: Uuid,
    pub candidate_id: Uuid,
    pub title: String,
    pub done: bool,
    pub done_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CandidateDocument {
    pub id: Uuid,
    pub candidate_id: Uuid,
    pub doc_type: String,
    pub storage_key: String,
    pub created_at: DateTime<Utc>,
}

/// Build a `Candidate` from a `query!` row that selected the candidate columns
/// plus the joined `stage_name`. Shared by `list` and `get`.
macro_rules! candidate_from {
    ($r:expr) => {
        Candidate {
            id: $r.id,
            name: $r.name,
            email: $r.email,
            position: $r.position,
            stage_id: $r.stage_id,
            stage_name: $r.stage_name,
            status: $r.status,
            converted_user_id: $r.converted_user_id,
            hired_at: $r.hired_at,
            created_at: $r.created_at,
            updated_at: $r.updated_at,
        }
    };
}

// ---- Stages ----

pub async fn list_stages(pool: &PgPool) -> Result<Vec<Stage>, AppError> {
    let rows = sqlx::query!("SELECT id, name, sequence FROM onboarding_stages ORDER BY sequence")
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|r| Stage { id: r.id, name: r.name, sequence: r.sequence }).collect())
}

/// The first pipeline stage (lowest sequence) — the default for new candidates.
pub async fn first_stage_id(pool: &PgPool) -> Result<Uuid, AppError> {
    let r = sqlx::query!("SELECT id FROM onboarding_stages ORDER BY sequence LIMIT 1")
        .fetch_one(pool)
        .await?;
    Ok(r.id)
}

pub async fn stage_exists(pool: &PgPool, id: Uuid) -> Result<bool, AppError> {
    let r = sqlx::query!(r#"SELECT EXISTS(SELECT 1 FROM onboarding_stages WHERE id=$1) AS "e!""#, id)
        .fetch_one(pool)
        .await?;
    Ok(r.e)
}

// ---- Candidates ----

pub async fn create(
    pool: &PgPool,
    name: &str,
    email: &str,
    position: &str,
    stage_id: Uuid,
    created_by: Uuid,
) -> Result<Candidate, AppError> {
    let r = sqlx::query!(
        "INSERT INTO candidates (name, email, position, stage_id, created_by)
         VALUES ($1, $2, $3, $4, $5) RETURNING id",
        name,
        email,
        position,
        stage_id,
        created_by
    )
    .fetch_one(pool)
    .await?;
    get(pool, r.id).await?.ok_or_else(|| AppError::Internal(anyhow::anyhow!("created candidate vanished")))
}

pub async fn list(pool: &PgPool) -> Result<Vec<Candidate>, AppError> {
    let rows = sqlx::query!(
        r#"SELECT c.id, c.name, c.email, c.position, c.stage_id, s.name AS stage_name,
                  c.status, c.converted_user_id, c.hired_at, c.created_at, c.updated_at
           FROM candidates c JOIN onboarding_stages s ON s.id = c.stage_id
           ORDER BY s.sequence, c.created_at"#
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|r| candidate_from!(r)).collect())
}

pub async fn get(pool: &PgPool, id: Uuid) -> Result<Option<Candidate>, AppError> {
    let row = sqlx::query!(
        r#"SELECT c.id, c.name, c.email, c.position, c.stage_id, s.name AS stage_name,
                  c.status, c.converted_user_id, c.hired_at, c.created_at, c.updated_at
           FROM candidates c JOIN onboarding_stages s ON s.id = c.stage_id
           WHERE c.id = $1"#,
        id
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| candidate_from!(r)))
}

pub async fn update(
    pool: &PgPool,
    id: Uuid,
    name: Option<&str>,
    email: Option<&str>,
    position: Option<&str>,
) -> Result<bool, AppError> {
    let res = sqlx::query!(
        r#"UPDATE candidates SET
             name = COALESCE($2, name),
             email = COALESCE($3, email),
             position = COALESCE($4, position),
             updated_at = now()
           WHERE id = $1"#,
        id,
        name,
        email,
        position
    )
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// Move a candidate to another stage (Kanban transition).
pub async fn set_stage(pool: &PgPool, id: Uuid, stage_id: Uuid) -> Result<bool, AppError> {
    let res = sqlx::query!(
        "UPDATE candidates SET stage_id = $2, updated_at = now() WHERE id = $1",
        id,
        stage_id
    )
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn set_status(pool: &PgPool, id: Uuid, status: &str) -> Result<bool, AppError> {
    let res = sqlx::query!(
        "UPDATE candidates SET status = $2, updated_at = now() WHERE id = $1",
        id,
        status
    )
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// Mark a candidate converted to an employee: record the user, set hired, and
/// move to the final stage.
pub async fn mark_converted(
    pool: &PgPool,
    id: Uuid,
    user_id: Uuid,
    final_stage_id: Uuid,
) -> Result<(), AppError> {
    sqlx::query!(
        "UPDATE candidates
         SET converted_user_id = $2, status = 'hired', hired_at = now(),
             stage_id = $3, updated_at = now()
         WHERE id = $1",
        id,
        user_id,
        final_stage_id
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete(pool: &PgPool, id: Uuid) -> Result<bool, AppError> {
    let res = sqlx::query!("DELETE FROM candidates WHERE id = $1", id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

// ---- Tasks ----

pub async fn list_tasks(pool: &PgPool, candidate_id: Uuid) -> Result<Vec<CandidateTask>, AppError> {
    let rows = sqlx::query!(
        "SELECT id, candidate_id, title, done, done_at, created_at
         FROM candidate_tasks WHERE candidate_id = $1 ORDER BY created_at",
        candidate_id
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| CandidateTask {
            id: r.id,
            candidate_id: r.candidate_id,
            title: r.title,
            done: r.done,
            done_at: r.done_at,
            created_at: r.created_at,
        })
        .collect())
}

pub async fn create_task(
    pool: &PgPool,
    candidate_id: Uuid,
    title: &str,
) -> Result<CandidateTask, AppError> {
    let r = sqlx::query!(
        "INSERT INTO candidate_tasks (candidate_id, title) VALUES ($1, $2)
         RETURNING id, candidate_id, title, done, done_at, created_at",
        candidate_id,
        title
    )
    .fetch_one(pool)
    .await?;
    Ok(CandidateTask {
        id: r.id,
        candidate_id: r.candidate_id,
        title: r.title,
        done: r.done,
        done_at: r.done_at,
        created_at: r.created_at,
    })
}

pub async fn set_task_done(pool: &PgPool, task_id: Uuid, done: bool) -> Result<bool, AppError> {
    let res = sqlx::query!(
        "UPDATE candidate_tasks
         SET done = $2, done_at = CASE WHEN $2 THEN now() ELSE NULL END
         WHERE id = $1",
        task_id,
        done
    )
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn delete_task(pool: &PgPool, task_id: Uuid) -> Result<bool, AppError> {
    let res = sqlx::query!("DELETE FROM candidate_tasks WHERE id = $1", task_id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

// ---- Documents (metadata only) ----

pub async fn list_documents(
    pool: &PgPool,
    candidate_id: Uuid,
) -> Result<Vec<CandidateDocument>, AppError> {
    let rows = sqlx::query!(
        "SELECT id, candidate_id, doc_type, storage_key, created_at
         FROM candidate_documents WHERE candidate_id = $1 ORDER BY created_at",
        candidate_id
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| CandidateDocument {
            id: r.id,
            candidate_id: r.candidate_id,
            doc_type: r.doc_type,
            storage_key: r.storage_key,
            created_at: r.created_at,
        })
        .collect())
}

pub async fn add_document(
    pool: &PgPool,
    candidate_id: Uuid,
    doc_type: &str,
    storage_key: &str,
) -> Result<CandidateDocument, AppError> {
    let r = sqlx::query!(
        "INSERT INTO candidate_documents (candidate_id, doc_type, storage_key)
         VALUES ($1, $2, $3)
         RETURNING id, candidate_id, doc_type, storage_key, created_at",
        candidate_id,
        doc_type,
        storage_key
    )
    .fetch_one(pool)
    .await?;
    Ok(CandidateDocument {
        id: r.id,
        candidate_id: r.candidate_id,
        doc_type: r.doc_type,
        storage_key: r.storage_key,
        created_at: r.created_at,
    })
}

//! Manual-tasks repository (Feature 5, Rule 7): HR/PM-assigned work items.

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;

/// Allowed task statuses (must match the DB CHECK).
pub const STATUSES: [&str; 2] = ["open", "done"];

pub fn is_valid_status(s: &str) -> bool {
    STATUSES.contains(&s)
}

#[derive(Debug, Clone, Serialize)]
pub struct ManualTask {
    pub id: Uuid,
    pub user_id: Uuid,
    pub created_by: Option<Uuid>,
    pub title: String,
    pub description: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

fn map(
    id: Uuid,
    user_id: Uuid,
    created_by: Option<Uuid>,
    title: String,
    description: String,
    status: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
) -> ManualTask {
    ManualTask {
        id,
        user_id,
        created_by,
        title,
        description,
        status,
        created_at,
        updated_at,
    }
}

/// Create a task for `user_id`, attributed to `created_by` (HR/PM).
pub async fn create(
    pool: &PgPool,
    user_id: Uuid,
    created_by: Uuid,
    title: &str,
    description: &str,
) -> Result<ManualTask, AppError> {
    let r = sqlx::query!(
        r#"INSERT INTO manual_tasks (user_id, created_by, title, description)
           VALUES ($1, $2, $3, $4)
           RETURNING id, user_id, created_by, title, description, status, created_at, updated_at"#,
        user_id,
        created_by,
        title,
        description
    )
    .fetch_one(pool)
    .await?;
    Ok(map(r.id, r.user_id, r.created_by, r.title, r.description, r.status, r.created_at, r.updated_at))
}

/// All of an employee's manual tasks, newest first.
pub async fn list_for_user(pool: &PgPool, user_id: Uuid) -> Result<Vec<ManualTask>, AppError> {
    let rows = sqlx::query!(
        r#"SELECT id, user_id, created_by, title, description, status, created_at, updated_at
           FROM manual_tasks WHERE user_id = $1 ORDER BY created_at DESC"#,
        user_id
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| map(r.id, r.user_id, r.created_by, r.title, r.description, r.status, r.created_at, r.updated_at))
        .collect())
}

/// A single task by id.
pub async fn get(pool: &PgPool, id: Uuid) -> Result<Option<ManualTask>, AppError> {
    let row = sqlx::query!(
        r#"SELECT id, user_id, created_by, title, description, status, created_at, updated_at
           FROM manual_tasks WHERE id = $1"#,
        id
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| map(r.id, r.user_id, r.created_by, r.title, r.description, r.status, r.created_at, r.updated_at)))
}

/// Update title and/or description (PATCH semantics; `None` leaves a field).
/// Returns whether a row was updated.
pub async fn update(
    pool: &PgPool,
    id: Uuid,
    title: Option<&str>,
    description: Option<&str>,
) -> Result<bool, AppError> {
    let res = sqlx::query!(
        r#"UPDATE manual_tasks
           SET title = COALESCE($2, title), description = COALESCE($3, description), updated_at = now()
           WHERE id = $1"#,
        id,
        title,
        description
    )
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// Set the task status (open / done). Returns whether a row was updated.
pub async fn set_status(pool: &PgPool, id: Uuid, status: &str) -> Result<bool, AppError> {
    let res = sqlx::query!(
        "UPDATE manual_tasks SET status = $2, updated_at = now() WHERE id = $1",
        id,
        status
    )
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// Delete a task. Returns whether a row was removed.
pub async fn delete(pool: &PgPool, id: Uuid) -> Result<bool, AppError> {
    let res = sqlx::query!("DELETE FROM manual_tasks WHERE id = $1", id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_status() {
        assert!(is_valid_status("open"));
        assert!(is_valid_status("done"));
        assert!(!is_valid_status("closed"));
        assert!(!is_valid_status(""));
    }
}

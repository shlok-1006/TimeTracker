//! Manual-tasks repository (Feature 5, Rule 7): HR/PM-assigned work items.

use chrono::{DateTime, NaiveDate, Utc};
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
    /// Importance / effort weight, 1–10 (set by the assigner).
    pub weight: i32,
    /// Expected due date (calendar day, no time); `None` if open-ended.
    pub due_date: Option<NaiveDate>,
    /// GitHub PR URLs linked to this task — the HRMS engine reviews these to score the person.
    /// A task can carry several (one feature, many PRs); empty means "not PR-scored".
    pub pr_links: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[allow(clippy::too_many_arguments)]
fn map(
    id: Uuid,
    user_id: Uuid,
    created_by: Option<Uuid>,
    title: String,
    description: String,
    status: String,
    weight: i32,
    due_date: Option<NaiveDate>,
    pr_links: Vec<String>,
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
        weight,
        due_date,
        pr_links,
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
    weight: i32,
    due_date: Option<NaiveDate>,
    pr_links: &[String],
) -> Result<ManualTask, AppError> {
    let r = sqlx::query!(
        r#"INSERT INTO manual_tasks (user_id, created_by, title, description, weight, due_date, pr_links)
           VALUES ($1, $2, $3, $4, $5, $6, $7)
           RETURNING id, user_id, created_by, title, description, status, weight, due_date, pr_links, created_at, updated_at"#,
        user_id,
        created_by,
        title,
        description,
        weight,
        due_date,
        pr_links
    )
    .fetch_one(pool)
    .await?;
    Ok(map(
        r.id,
        r.user_id,
        r.created_by,
        r.title,
        r.description,
        r.status,
        r.weight,
        r.due_date,
        r.pr_links,
        r.created_at,
        r.updated_at,
    ))
}

/// All of an employee's manual tasks, newest first.
pub async fn list_for_user(pool: &PgPool, user_id: Uuid) -> Result<Vec<ManualTask>, AppError> {
    let rows = sqlx::query!(
        r#"SELECT id, user_id, created_by, title, description, status, weight, due_date, pr_links, created_at, updated_at
           FROM manual_tasks WHERE user_id = $1 ORDER BY created_at DESC"#,
        user_id
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            map(
                r.id,
                r.user_id,
                r.created_by,
                r.title,
                r.description,
                r.status,
                r.weight,
                r.due_date,
                r.pr_links,
                r.created_at,
                r.updated_at,
            )
        })
        .collect())
}

/// A single task by id.
pub async fn get(pool: &PgPool, id: Uuid) -> Result<Option<ManualTask>, AppError> {
    let row = sqlx::query!(
        r#"SELECT id, user_id, created_by, title, description, status, weight, due_date, pr_links, created_at, updated_at
           FROM manual_tasks WHERE id = $1"#,
        id
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| {
        map(
            r.id,
            r.user_id,
            r.created_by,
            r.title,
            r.description,
            r.status,
            r.weight,
            r.due_date,
            r.pr_links,
            r.created_at,
            r.updated_at,
        )
    }))
}

/// Update title, description, weight and/or due date (PATCH semantics; `None`
/// leaves a field unchanged). Returns whether a row was updated.
///
/// Note: because unset fields are `None`, a due date can be set or changed but
/// not cleared back to open-ended through this path — matching the existing
/// COALESCE semantics for the other fields.
#[allow(clippy::too_many_arguments)]
pub async fn update(
    pool: &PgPool,
    id: Uuid,
    title: Option<&str>,
    description: Option<&str>,
    weight: Option<i32>,
    due_date: Option<NaiveDate>,
    pr_links: Option<&[String]>,
) -> Result<bool, AppError> {
    let res = sqlx::query!(
        r#"UPDATE manual_tasks
           SET title = COALESCE($2, title),
               description = COALESCE($3, description),
               weight = COALESCE($4, weight),
               due_date = COALESCE($5, due_date),
               pr_links = COALESCE($6, pr_links),
               updated_at = now()
           WHERE id = $1"#,
        id,
        title,
        description,
        weight,
        due_date,
        pr_links.map(|s| s.to_vec()) as Option<Vec<String>>
    )
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// One PR-bearing task for the HRMS performance engine — the task plus the owner's email, scoped
/// to a team. `only_with_pr` restricts to tasks that actually carry a PR (the engine's default).
#[derive(Debug, Clone, Serialize)]
pub struct TeamTask {
    pub task_id: Uuid,
    pub user_id: Uuid,
    pub user_email: String,
    pub title: String,
    pub weight: i32,
    pub due_date: Option<NaiveDate>,
    pub status: String,
    pub pr_links: Vec<String>,
    pub updated_at: DateTime<Utc>,
}

/// Tasks for every member of `team_id` (via `user_teams`). With `only_with_pr`, only tasks that
/// have at least one PR link — exactly what the engine pulls to score. Newest-updated first.
pub async fn list_for_team(
    pool: &PgPool,
    team_id: Uuid,
    only_with_pr: bool,
) -> Result<Vec<TeamTask>, AppError> {
    let rows = sqlx::query!(
        r#"SELECT mt.id, mt.user_id, u.email AS user_email, mt.title, mt.weight,
                  mt.due_date, mt.status, mt.pr_links, mt.updated_at
           FROM manual_tasks mt
           JOIN users u       ON u.id = mt.user_id
           JOIN user_teams ut ON ut.user_id = mt.user_id AND ut.team_id = $1
           WHERE u.deactivated_at IS NULL
             AND (NOT $2 OR cardinality(mt.pr_links) > 0)
           ORDER BY mt.updated_at DESC"#,
        team_id,
        only_with_pr
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| TeamTask {
            task_id: r.id,
            user_id: r.user_id,
            user_email: r.user_email,
            title: r.title,
            weight: r.weight,
            due_date: r.due_date,
            status: r.status,
            pr_links: r.pr_links,
            updated_at: r.updated_at,
        })
        .collect())
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

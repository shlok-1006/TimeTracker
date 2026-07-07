//! Alumni repository (Rule 7): a snapshot of removed employees, written just
//! before a user is hard-deleted so their identity is retained for the admin
//! "Alumni" view.

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;

#[derive(Debug, Clone, Serialize)]
pub struct Alumnus {
    pub id: Uuid,
    pub user_id: Option<Uuid>,
    pub name: String,
    pub email: String,
    pub role: String,
    pub team_id: Option<Uuid>,
    pub joined_at: Option<DateTime<Utc>>,
    pub removed_at: DateTime<Utc>,
    pub removed_by: Option<Uuid>,
}

/// Record a removed employee. Called from `delete_user` with the identity
/// captured before the cascade delete runs.
#[allow(clippy::too_many_arguments)]
pub async fn record(
    pool: &PgPool,
    user_id: Uuid,
    name: &str,
    email: &str,
    role: &str,
    team_id: Option<Uuid>,
    joined_at: DateTime<Utc>,
    removed_by: Uuid,
) -> Result<(), AppError> {
    sqlx::query!(
        r#"
        INSERT INTO alumni (user_id, name, email, role, team_id, joined_at, removed_by)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        "#,
        user_id,
        name,
        email,
        role,
        team_id,
        joined_at,
        removed_by
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// All alumni, most recently removed first.
pub async fn list(pool: &PgPool) -> Result<Vec<Alumnus>, AppError> {
    let rows = sqlx::query!(
        r#"SELECT id, user_id, name, email, role, team_id, joined_at, removed_at, removed_by
           FROM alumni ORDER BY removed_at DESC"#
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| Alumnus {
            id: r.id,
            user_id: r.user_id,
            name: r.name,
            email: r.email,
            role: r.role,
            team_id: r.team_id,
            joined_at: r.joined_at,
            removed_at: r.removed_at,
            removed_by: r.removed_by,
        })
        .collect())
}

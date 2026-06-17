//! Teams repository (Feature 4, Rule 7): team catalogue + multi-team membership.

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;

#[derive(Debug, Clone, Serialize)]
pub struct Team {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub created_at: DateTime<Utc>,
}

/// A team member (an employee belonging to a team).
#[derive(Debug, Clone, Serialize)]
pub struct TeamMember {
    pub id: Uuid,
    pub name: String,
    pub email: String,
}

// ---- Teams ----

pub async fn create(pool: &PgPool, name: &str, description: &str) -> Result<Team, AppError> {
    let result = sqlx::query!(
        "INSERT INTO teams (name, description) VALUES ($1, $2)
         RETURNING id, name, description, created_at",
        name,
        description
    )
    .fetch_one(pool)
    .await;

    match result {
        Ok(r) => Ok(Team {
            id: r.id,
            name: r.name,
            description: r.description,
            created_at: r.created_at,
        }),
        Err(sqlx::Error::Database(db)) if db.is_unique_violation() => {
            Err(AppError::BadRequest("a team with that name already exists".into()))
        }
        Err(e) => Err(e.into()),
    }
}

pub async fn list(pool: &PgPool) -> Result<Vec<Team>, AppError> {
    let rows = sqlx::query!("SELECT id, name, description, created_at FROM teams ORDER BY name")
        .fetch_all(pool)
        .await?;
    Ok(rows
        .into_iter()
        .map(|r| Team { id: r.id, name: r.name, description: r.description, created_at: r.created_at })
        .collect())
}

pub async fn get(pool: &PgPool, id: Uuid) -> Result<Option<Team>, AppError> {
    let row = sqlx::query!(
        "SELECT id, name, description, created_at FROM teams WHERE id = $1",
        id
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| Team {
        id: r.id,
        name: r.name,
        description: r.description,
        created_at: r.created_at,
    }))
}

/// Update a team's name and/or description (PATCH semantics: `None` fields are
/// left unchanged). Returns `None` if no team has that id.
pub async fn update(
    pool: &PgPool,
    id: Uuid,
    name: Option<&str>,
    description: Option<&str>,
) -> Result<Option<Team>, AppError> {
    let result = sqlx::query!(
        r#"UPDATE teams
           SET name = COALESCE($2, name), description = COALESCE($3, description)
           WHERE id = $1
           RETURNING id, name, description, created_at"#,
        id,
        name,
        description
    )
    .fetch_optional(pool)
    .await;

    match result {
        Ok(row) => Ok(row.map(|r| Team {
            id: r.id,
            name: r.name,
            description: r.description,
            created_at: r.created_at,
        })),
        Err(sqlx::Error::Database(db)) if db.is_unique_violation() => {
            Err(AppError::BadRequest("a team with that name already exists".into()))
        }
        Err(e) => Err(e.into()),
    }
}

/// Delete a team. Membership rows and interval `team_id`s are cleaned up by the
/// FK rules (cascade / set null). Returns whether a row was removed.
pub async fn delete(pool: &PgPool, id: Uuid) -> Result<bool, AppError> {
    let res = sqlx::query!("DELETE FROM teams WHERE id = $1", id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

// ---- Membership ----

/// Add an employee to a team (idempotent). Returns Err if the team doesn't exist.
pub async fn add_member(pool: &PgPool, user_id: Uuid, team_id: Uuid) -> Result<(), AppError> {
    let res = sqlx::query!(
        "INSERT INTO user_teams (user_id, team_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        user_id,
        team_id
    )
    .execute(pool)
    .await;

    match res {
        Ok(_) => Ok(()),
        Err(sqlx::Error::Database(db)) if db.is_foreign_key_violation() => {
            Err(AppError::BadRequest("unknown user or team".into()))
        }
        Err(e) => Err(e.into()),
    }
}

/// Remove an employee from a team. Returns whether a membership was removed.
pub async fn remove_member(pool: &PgPool, user_id: Uuid, team_id: Uuid) -> Result<bool, AppError> {
    let res = sqlx::query!(
        "DELETE FROM user_teams WHERE user_id = $1 AND team_id = $2",
        user_id,
        team_id
    )
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// The teams an employee belongs to (used by the desktop's pre-timer dropdown).
pub async fn teams_for_user(pool: &PgPool, user_id: Uuid) -> Result<Vec<Team>, AppError> {
    let rows = sqlx::query!(
        r#"SELECT t.id, t.name, t.description, t.created_at
           FROM teams t
           JOIN user_teams ut ON ut.team_id = t.id
           WHERE ut.user_id = $1
           ORDER BY t.name"#,
        user_id
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| Team { id: r.id, name: r.name, description: r.description, created_at: r.created_at })
        .collect())
}

/// The employees in a team.
pub async fn members_of(pool: &PgPool, team_id: Uuid) -> Result<Vec<TeamMember>, AppError> {
    let rows = sqlx::query!(
        r#"SELECT u.id, u.name, u.email
           FROM users u
           JOIN user_teams ut ON ut.user_id = u.id
           WHERE ut.team_id = $1
           ORDER BY u.name"#,
        team_id
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| TeamMember { id: r.id, name: r.name, email: r.email })
        .collect())
}

// ---- Summary metrics (Feature 4 Phase 4) ----

/// A team plus its member count, for the team index.
#[derive(Debug, Clone, Serialize)]
pub struct TeamWithCount {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub created_at: DateTime<Utc>,
    pub member_count: i64,
}

/// Worked-time status breakdown for a team (seconds). `total` = active + meeting.
#[derive(Debug, Clone, Serialize)]
pub struct StatusBreakdown {
    pub total: i64,
    pub active: i64,
    pub idle: i64,
    pub meeting: i64,
    pub break_: i64,
}

/// One member's worked total within a team (seconds).
#[derive(Debug, Clone, Serialize)]
pub struct MemberTotal {
    pub user_id: Uuid,
    pub name: String,
    pub email: String,
    pub worked_seconds: i64,
}

/// All teams with their member counts.
pub async fn list_with_counts(pool: &PgPool) -> Result<Vec<TeamWithCount>, AppError> {
    let rows = sqlx::query!(
        r#"SELECT t.id, t.name, t.description, t.created_at,
                  CAST(COUNT(ut.user_id) AS BIGINT) AS "member_count!"
           FROM teams t
           LEFT JOIN user_teams ut ON ut.team_id = t.id
           GROUP BY t.id, t.name, t.description, t.created_at
           ORDER BY t.name"#
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| TeamWithCount {
            id: r.id,
            name: r.name,
            description: r.description,
            created_at: r.created_at,
            member_count: r.member_count,
        })
        .collect())
}

/// Team-wide worked-time status breakdown (over `intervals.team_id`).
pub async fn status_breakdown(pool: &PgPool, team_id: Uuid) -> Result<StatusBreakdown, AppError> {
    let r = sqlx::query!(
        r#"SELECT
             CAST(COALESCE(SUM(EXTRACT(EPOCH FROM (end_utc-start_utc))) FILTER (WHERE kind IN ('active','meeting')),0) AS BIGINT) AS "total!",
             CAST(COALESCE(SUM(EXTRACT(EPOCH FROM (end_utc-start_utc))) FILTER (WHERE kind='active'),0) AS BIGINT) AS "active!",
             CAST(COALESCE(SUM(EXTRACT(EPOCH FROM (end_utc-start_utc))) FILTER (WHERE kind='idle'),0) AS BIGINT) AS "idle!",
             CAST(COALESCE(SUM(EXTRACT(EPOCH FROM (end_utc-start_utc))) FILTER (WHERE kind='meeting'),0) AS BIGINT) AS "meeting!",
             CAST(COALESCE(SUM(EXTRACT(EPOCH FROM (end_utc-start_utc))) FILTER (WHERE kind='break'),0) AS BIGINT) AS "brk!"
           FROM intervals WHERE team_id = $1"#,
        team_id
    )
    .fetch_one(pool)
    .await?;
    Ok(StatusBreakdown {
        total: r.total,
        active: r.active,
        idle: r.idle,
        meeting: r.meeting,
        break_: r.brk,
    })
}

/// Per-member worked totals within a team (all members, even those with 0).
pub async fn member_totals(pool: &PgPool, team_id: Uuid) -> Result<Vec<MemberTotal>, AppError> {
    let rows = sqlx::query!(
        r#"SELECT u.id, u.name, u.email,
                  CAST(COALESCE(SUM(EXTRACT(EPOCH FROM (i.end_utc-i.start_utc))) FILTER (WHERE i.kind IN ('active','meeting')),0) AS BIGINT) AS "worked!"
           FROM users u
           JOIN user_teams ut ON ut.user_id = u.id AND ut.team_id = $1
           LEFT JOIN intervals i ON i.user_id = u.id AND i.team_id = $1
           GROUP BY u.id, u.name, u.email
           ORDER BY 4 DESC, u.name"#,
        team_id
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| MemberTotal {
            user_id: r.id,
            name: r.name,
            email: r.email,
            worked_seconds: r.worked,
        })
        .collect())
}

/// Whether an employee belongs to a team (validates timer team selection).
pub async fn is_member(pool: &PgPool, user_id: Uuid, team_id: Uuid) -> Result<bool, AppError> {
    let row = sqlx::query!(
        r#"SELECT EXISTS(
              SELECT 1 FROM user_teams WHERE user_id = $1 AND team_id = $2
           ) AS "member!""#,
        user_id,
        team_id
    )
    .fetch_one(pool)
    .await?;
    Ok(row.member)
}

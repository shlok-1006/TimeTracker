//! Users repository (Rule 7: SQLx, compile-time checked queries, repository pattern).
//!
//! The Postgres `user_role` enum is crossed as `text` at the query boundary
//! (`role::text` on read, `$n::text::user_role` on write) so the macros resolve
//! to `String`, then we convert to the strongly-typed `UserRole` in Rust. This
//! keeps the queries compile-time checked without a bespoke enum type mapping.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;
use crate::role::UserRole;

#[derive(Debug, Clone)]
pub struct User {
    pub id: Uuid,
    pub name: String,
    pub email: String,
    pub password_hash: String,
    pub role: UserRole,
    pub manager_id: Option<Uuid>,
    pub team_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// The manager assigned to `user_id`, if any (used for PM scope checks).
pub async fn manager_id_of(pool: &PgPool, user_id: Uuid) -> Result<Option<Uuid>, AppError> {
    let row = sqlx::query!("SELECT manager_id FROM users WHERE id = $1", user_id)
        .fetch_optional(pool)
        .await?;
    Ok(row.and_then(|r| r.manager_id))
}

fn parse_role(s: &str) -> Result<UserRole, AppError> {
    s.parse::<UserRole>()
        .map_err(|e| AppError::Internal(anyhow::anyhow!("invalid role stored in db: {e}")))
}

/// Look up a user by email. Returns `None` if no such user exists.
pub async fn find_by_email(pool: &PgPool, email: &str) -> Result<Option<User>, AppError> {
    let row = sqlx::query!(
        r#"
        SELECT id, name, email, password_hash, role::text AS "role!",
               manager_id, team_id, created_at, updated_at
        FROM users
        WHERE email = $1
        "#,
        email
    )
    .fetch_optional(pool)
    .await?;

    match row {
        None => Ok(None),
        Some(r) => Ok(Some(User {
            id: r.id,
            name: r.name,
            email: r.email,
            password_hash: r.password_hash,
            role: parse_role(&r.role)?,
            manager_id: r.manager_id,
            team_id: r.team_id,
            created_at: r.created_at,
            updated_at: r.updated_at,
        })),
    }
}

/// Insert a user, or update the existing one with the same email (idempotent).
/// Used by the dev seed.
pub async fn upsert(
    pool: &PgPool,
    name: &str,
    email: &str,
    password_hash: &str,
    role: UserRole,
    team_id: Option<Uuid>,
) -> Result<User, AppError> {
    let role_str = role.as_str();
    let r = sqlx::query!(
        r#"
        INSERT INTO users (name, email, password_hash, role, team_id)
        VALUES ($1, $2, $3, $4::text::user_role, $5)
        ON CONFLICT (email) DO UPDATE SET
            name = EXCLUDED.name,
            password_hash = EXCLUDED.password_hash,
            role = EXCLUDED.role,
            team_id = EXCLUDED.team_id,
            updated_at = now()
        RETURNING id, name, email, password_hash, role::text AS "role!",
                  manager_id, team_id, created_at, updated_at
        "#,
        name,
        email,
        password_hash,
        role_str,
        team_id
    )
    .fetch_one(pool)
    .await?;

    Ok(User {
        id: r.id,
        name: r.name,
        email: r.email,
        password_hash: r.password_hash,
        role: parse_role(&r.role)?,
        manager_id: r.manager_id,
        team_id: r.team_id,
        created_at: r.created_at,
        updated_at: r.updated_at,
    })
}

//! Refresh-token repository (Rule 6/7). Only SHA-256 hashes are stored.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;

/// Persist a refresh token's hash.
pub async fn insert(
    pool: &PgPool,
    user_id: Uuid,
    token_hash: &str,
    expires_at: DateTime<Utc>,
) -> Result<(), AppError> {
    sqlx::query!(
        "INSERT INTO refresh_tokens (user_id, token_hash, expires_at) VALUES ($1, $2, $3)",
        user_id,
        token_hash,
        expires_at
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Find a non-revoked, unexpired token by its hash. Returns `(id, user_id)`.
pub async fn find_valid(pool: &PgPool, token_hash: &str) -> Result<Option<(Uuid, Uuid)>, AppError> {
    let row = sqlx::query!(
        r#"
        SELECT id, user_id
        FROM refresh_tokens
        WHERE token_hash = $1 AND revoked_at IS NULL AND expires_at > now()
        "#,
        token_hash
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| (r.id, r.user_id)))
}

/// Atomically consume a valid token: revoke it and return `(id, user_id)` in a
/// single statement, so two concurrent refreshes can't both succeed on the same
/// token (SEC-17). Returns `None` if the token is missing, revoked, or expired.
pub async fn consume(pool: &PgPool, token_hash: &str) -> Result<Option<(Uuid, Uuid)>, AppError> {
    let row = sqlx::query!(
        r#"
        UPDATE refresh_tokens
        SET revoked_at = now()
        WHERE token_hash = $1 AND revoked_at IS NULL AND expires_at > now()
        RETURNING id, user_id
        "#,
        token_hash
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| (r.id, r.user_id)))
}

/// State of a presented-but-not-consumable token, used to classify a replay
/// (SEC-17). `revoked_at` tells us how long ago it was invalidated (to tell a
/// benign concurrent-refresh race from a genuine stolen-token replay), and
/// `family_live` says whether the owner still has any live session (if not, the
/// family was already revoked and this replay is just stale — not a new
/// incident, so we must not re-revoke or re-audit it on every retry).
pub struct ReplayInfo {
    pub user_id: Uuid,
    pub revoked_at: Option<DateTime<Utc>>,
    pub family_live: bool,
}

/// Look up a token hash regardless of its state, with the context needed to
/// decide whether a replay is a real theft signal. Returns `None` if the hash
/// was never issued.
pub async fn replay_info(pool: &PgPool, token_hash: &str) -> Result<Option<ReplayInfo>, AppError> {
    let row = sqlx::query!(
        r#"
        SELECT
          t.user_id AS user_id,
          t.revoked_at AS revoked_at,
          EXISTS (
            SELECT 1 FROM refresh_tokens f
            WHERE f.user_id = t.user_id
              AND f.revoked_at IS NULL
              AND f.expires_at > now()
          ) AS "family_live!"
        FROM refresh_tokens t
        WHERE t.token_hash = $1
        "#,
        token_hash
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| ReplayInfo {
        user_id: r.user_id,
        revoked_at: r.revoked_at,
        family_live: r.family_live,
    }))
}

/// Revoke a token by id (used on rotation and logout).
pub async fn revoke(pool: &PgPool, id: Uuid) -> Result<(), AppError> {
    sqlx::query!(
        "UPDATE refresh_tokens SET revoked_at = now() WHERE id = $1 AND revoked_at IS NULL",
        id
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Revoke ALL of a user's refresh tokens (e.g. after a password reset).
pub async fn revoke_all_for_user(pool: &PgPool, user_id: Uuid) -> Result<(), AppError> {
    sqlx::query!(
        "UPDATE refresh_tokens SET revoked_at = now() WHERE user_id = $1 AND revoked_at IS NULL",
        user_id
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Revoke a token by its hash (logout when we only have the token string).
/// Returns the owning `user_id` if a live token was revoked (for audit logging).
pub async fn revoke_by_hash(pool: &PgPool, token_hash: &str) -> Result<Option<Uuid>, AppError> {
    let row = sqlx::query!(
        "UPDATE refresh_tokens SET revoked_at = now() WHERE token_hash = $1 AND revoked_at IS NULL RETURNING user_id",
        token_hash
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| r.user_id))
}

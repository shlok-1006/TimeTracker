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
pub async fn revoke_by_hash(pool: &PgPool, token_hash: &str) -> Result<(), AppError> {
    sqlx::query!(
        "UPDATE refresh_tokens SET revoked_at = now() WHERE token_hash = $1 AND revoked_at IS NULL",
        token_hash
    )
    .execute(pool)
    .await?;
    Ok(())
}

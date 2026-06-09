//! Linear account links repository (Rule 7). Stores which Linear user an
//! employee maps to — never the API token.

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;

/// Link (or re-link) an internal user to a Linear user id.
pub async fn upsert(pool: &PgPool, user_id: Uuid, linear_user_id: &str) -> Result<(), AppError> {
    sqlx::query!(
        r#"
        INSERT INTO linear_links (user_id, linear_user_id)
        VALUES ($1, $2)
        ON CONFLICT (user_id) DO UPDATE SET
            linear_user_id = EXCLUDED.linear_user_id,
            linked_at = now()
        "#,
        user_id,
        linear_user_id
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// The Linear user id linked to `user_id`, if any.
pub async fn get_linear_user_id(pool: &PgPool, user_id: Uuid) -> Result<Option<String>, AppError> {
    let row = sqlx::query!(
        "SELECT linear_user_id FROM linear_links WHERE user_id = $1",
        user_id
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| r.linear_user_id))
}

//! Screenshots repository — metadata only (Rule 5; Rule 7 compile-time queries).

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;

/// Insert screenshot metadata, returning the new row id.
pub async fn insert(
    pool: &PgPool,
    user_id: Uuid,
    storage_key: &str,
    taken_at: DateTime<Utc>,
    interval_id: Option<Uuid>,
) -> Result<Uuid, AppError> {
    let row = sqlx::query!(
        r#"
        INSERT INTO screenshots (user_id, storage_key, taken_at, interval_id)
        VALUES ($1, $2, $3, $4)
        RETURNING id
        "#,
        user_id,
        storage_key,
        taken_at,
        interval_id
    )
    .fetch_one(pool)
    .await?;
    Ok(row.id)
}

/// A screenshot's stored metadata.
pub struct ScreenshotRow {
    pub id: Uuid,
    pub storage_key: String,
    pub taken_at: DateTime<Utc>,
    pub interval_id: Option<Uuid>,
}

/// List a user's most recent screenshots (metadata only).
pub async fn list_for_user(
    pool: &PgPool,
    user_id: Uuid,
    limit: i64,
) -> Result<Vec<ScreenshotRow>, AppError> {
    let rows = sqlx::query!(
        r#"
        SELECT id, storage_key, taken_at, interval_id
        FROM screenshots
        WHERE user_id = $1
        ORDER BY taken_at DESC
        LIMIT $2
        "#,
        user_id,
        limit
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| ScreenshotRow {
            id: r.id,
            storage_key: r.storage_key,
            taken_at: r.taken_at,
            interval_id: r.interval_id,
        })
        .collect())
}

/// Count a user's screenshots (used by tests / future reporting).
pub async fn count_for_user(pool: &PgPool, user_id: Uuid) -> Result<i64, AppError> {
    let row = sqlx::query!(
        r#"SELECT COUNT(*) AS "count!" FROM screenshots WHERE user_id = $1"#,
        user_id
    )
    .fetch_one(pool)
    .await?;
    Ok(row.count)
}

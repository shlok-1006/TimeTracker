//! OKF — the company rulebook document (single HR-editable source of truth).
//! One canonical row (`id = 1`, seeded by migration 0037). HR reads and edits it
//! from admin-web via `/admin/okf`; every save records the editor and timestamp.

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;

#[derive(Debug, Clone, Serialize)]
pub struct OkfDocument {
    pub content: String,
    pub updated_by: Option<Uuid>,
    pub updated_by_name: Option<String>,
    pub updated_at: DateTime<Utc>,
}

/// The current rulebook (with the last editor's name, if any).
pub async fn get(pool: &PgPool) -> Result<OkfDocument, AppError> {
    let r = sqlx::query!(
        r#"SELECT d.content AS "content!", d.updated_by,
                  u.name AS "updated_by_name?", d.updated_at AS "updated_at!"
           FROM okf_document d
           LEFT JOIN users u ON u.id = d.updated_by
           WHERE d.id = 1"#
    )
    .fetch_optional(pool)
    .await?;
    r.map(|r| OkfDocument {
        content: r.content,
        updated_by: r.updated_by,
        updated_by_name: r.updated_by_name,
        updated_at: r.updated_at,
    })
    .ok_or(AppError::NotFound)
}

/// Replace the rulebook content, stamping the editor. Returns the fresh document.
pub async fn update(pool: &PgPool, content: &str, actor: Uuid) -> Result<OkfDocument, AppError> {
    sqlx::query!(
        r#"UPDATE okf_document
           SET content = $1, updated_by = $2, updated_at = now()
           WHERE id = 1"#,
        content,
        actor
    )
    .execute(pool)
    .await?;
    get(pool).await
}

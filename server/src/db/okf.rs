//! OKF policy library — many HR-editable documents (the company handbook),
//! readable by every authenticated user. Markdown documents carry `content`;
//! file documents carry a GCS `storage_key` + file metadata instead. One row
//! (`slug = 'system-rulebook'`) is the system-config rulebook carried over from
//! the single-document era (migration 0038). See `routes::okf`.

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;

/// Slug that must never be deleted (the system config rulebook).
pub const SYSTEM_SLUG: &str = "system-rulebook";

#[derive(Debug, Clone, Serialize)]
pub struct OkfSummary {
    pub id: Uuid,
    pub slug: String,
    pub title: String,
    pub category: String,
    pub kind: String,
    pub file_name: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OkfDocument {
    pub id: Uuid,
    pub slug: String,
    pub title: String,
    pub category: String,
    pub kind: String,
    pub content: String,
    pub storage_key: Option<String>,
    pub file_name: Option<String>,
    pub content_type: Option<String>,
    pub size_bytes: Option<i64>,
    pub sort_order: i32,
    pub updated_by: Option<Uuid>,
    pub updated_by_name: Option<String>,
    pub updated_at: DateTime<Utc>,
}

/// A stable, unique slug from a title (plus a short random suffix so titles can
/// repeat). Used only as a human-readable handle; the UI addresses docs by id.
fn slugify(title: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for c in title.trim().to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    let base = out.trim_matches('-');
    let base = if base.is_empty() { "doc" } else { base };
    format!("{base}-{}", &Uuid::new_v4().simple().to_string()[..6])
}

/// All documents, ordered for a grouped-by-category list.
pub async fn list(pool: &PgPool) -> Result<Vec<OkfSummary>, AppError> {
    let rows = sqlx::query!(
        r#"SELECT id, slug, title, category, kind, file_name, updated_at
           FROM okf_documents
           ORDER BY sort_order, category, title"#
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| OkfSummary {
            id: r.id,
            slug: r.slug,
            title: r.title,
            category: r.category,
            kind: r.kind,
            file_name: r.file_name,
            updated_at: r.updated_at,
        })
        .collect())
}

/// One document with the last editor's name.
pub async fn get(pool: &PgPool, id: Uuid) -> Result<OkfDocument, AppError> {
    let r = sqlx::query!(
        r#"SELECT d.id AS "id!", d.slug AS "slug!", d.title AS "title!",
                  d.category AS "category!", d.kind AS "kind!", d.content AS "content!",
                  d.storage_key, d.file_name, d.content_type, d.size_bytes,
                  d.sort_order AS "sort_order!", d.updated_by,
                  u.name AS "updated_by_name?", d.updated_at AS "updated_at!"
           FROM okf_documents d
           LEFT JOIN users u ON u.id = d.updated_by
           WHERE d.id = $1"#,
        id
    )
    .fetch_optional(pool)
    .await?;
    r.map(|r| OkfDocument {
        id: r.id,
        slug: r.slug,
        title: r.title,
        category: r.category,
        kind: r.kind,
        content: r.content,
        storage_key: r.storage_key,
        file_name: r.file_name,
        content_type: r.content_type,
        size_bytes: r.size_bytes,
        sort_order: r.sort_order,
        updated_by: r.updated_by,
        updated_by_name: r.updated_by_name,
        updated_at: r.updated_at,
    })
    .ok_or(AppError::NotFound)
}

/// Create a new markdown document.
pub async fn create(
    pool: &PgPool,
    title: &str,
    category: &str,
    content: &str,
    actor: Uuid,
) -> Result<OkfDocument, AppError> {
    let slug = slugify(title);
    let r = sqlx::query!(
        r#"INSERT INTO okf_documents (slug, title, category, kind, content, updated_by)
           VALUES ($1, $2, $3, 'markdown', $4, $5) RETURNING id"#,
        slug,
        title,
        category,
        content,
        actor
    )
    .fetch_one(pool)
    .await?;
    get(pool, r.id).await
}

/// Create a file document (an uploaded attachment already in object storage).
pub async fn create_file(
    pool: &PgPool,
    title: &str,
    category: &str,
    storage_key: &str,
    file_name: &str,
    content_type: &str,
    size_bytes: i64,
    actor: Uuid,
) -> Result<OkfDocument, AppError> {
    let slug = slugify(title);
    let r = sqlx::query!(
        r#"INSERT INTO okf_documents
             (slug, title, category, kind, storage_key, file_name, content_type, size_bytes, updated_by)
           VALUES ($1, $2, $3, 'file', $4, $5, $6, $7, $8) RETURNING id"#,
        slug,
        title,
        category,
        storage_key,
        file_name,
        content_type,
        size_bytes,
        actor
    )
    .fetch_one(pool)
    .await?;
    get(pool, r.id).await
}

/// Update a document's title/category/content (content is ignored for files).
/// `sort_order` is left unchanged when `None`.
pub async fn update(
    pool: &PgPool,
    id: Uuid,
    title: &str,
    category: &str,
    content: &str,
    sort_order: Option<i32>,
    actor: Uuid,
) -> Result<OkfDocument, AppError> {
    let res = sqlx::query!(
        r#"UPDATE okf_documents
           SET title = $2, category = $3, content = $4,
               sort_order = COALESCE($5, sort_order),
               updated_by = $6, updated_at = now()
           WHERE id = $1"#,
        id,
        title,
        category,
        content,
        sort_order,
        actor
    )
    .execute(pool)
    .await?;
    if res.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    get(pool, id).await
}

/// Delete a document. Returns its `storage_key` (so a file's object can be
/// removed too). The system rulebook is protected by the route, not here.
pub async fn delete(pool: &PgPool, id: Uuid) -> Result<Option<String>, AppError> {
    let r = sqlx::query!(
        "DELETE FROM okf_documents WHERE id = $1 RETURNING storage_key",
        id
    )
    .fetch_optional(pool)
    .await?;
    match r {
        Some(row) => Ok(row.storage_key),
        None => Err(AppError::NotFound),
    }
}

//! Company policy library (OKF). A set of HR-editable documents (the handbook)
//! that every authenticated user can read.
//!
//!   GET    /policies            list all documents (any signed-in user)
//!   GET    /policies/:id        one document (any signed-in user)
//!   POST   /admin/policies      create a markdown document        (HR)
//!   PUT    /admin/policies/:id  edit title/category/content/order (HR)
//!   DELETE /admin/policies/:id  remove a document                 (HR)
//!
//! File attachments (kind = 'file') are added in a follow-up (upload/download via
//! object storage). Editing is HR-only (`RequireHr`) and audited; reading only
//! requires authentication so employees can see the handbook.

use axum::{
    extract::{Path, State},
    routing::{get, post, put},
    Json, Router,
};
use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::db::{audit, okf};
use crate::error::AppError;
use crate::middleware::{AuthUser, RequireHr};
use crate::state::AppState;

/// Guard against an accidental huge paste.
const MAX_CONTENT_BYTES: usize = 512 * 1024;
/// Presigned PUT lifetime for a file upload (bigger than a screenshot capture).
const UPLOAD_URL_EXPIRES_SECS: u64 = 900;
/// Presigned GET lifetime for a file download.
const DOWNLOAD_URL_EXPIRES_SECS: u64 = 300;

/// Keep an uploaded file name safe for a storage key (no path traversal).
fn safe_file_name(name: &str) -> String {
    let n: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let n = n.trim_matches('.').to_string();
    if n.is_empty() {
        "file".into()
    } else {
        n
    }
}

fn norm_category(c: &str) -> &str {
    let c = c.trim();
    if c.is_empty() {
        "General"
    } else {
        c
    }
}

async fn list_policies(
    State(state): State<AppState>,
    _user: AuthUser,
) -> Result<Json<Value>, AppError> {
    Ok(Json(json!(okf::list(&state.db).await?)))
}

async fn get_policy(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(json!(okf::get(&state.db, id).await?)))
}

#[derive(Deserialize)]
struct NewPolicy {
    title: String,
    #[serde(default)]
    category: String,
    #[serde(default)]
    content: String,
}

async fn create_policy(
    State(state): State<AppState>,
    RequireHr(actor): RequireHr,
    Json(body): Json<NewPolicy>,
) -> Result<Json<Value>, AppError> {
    let title = body.title.trim();
    if title.is_empty() {
        return Err(AppError::BadRequest("a title is required".into()));
    }
    if body.content.len() > MAX_CONTENT_BYTES {
        return Err(AppError::BadRequest("the document is too large".into()));
    }
    let doc = okf::create(&state.db, title, norm_category(&body.category), &body.content, actor.id).await?;
    audit::log(&state.db, actor.id, "okf.create", "okf_document", Some(doc.id)).await;
    Ok(Json(json!(doc)))
}

#[derive(Deserialize)]
struct UpdatePolicy {
    title: String,
    #[serde(default)]
    category: String,
    #[serde(default)]
    content: String,
    #[serde(default)]
    sort_order: Option<i32>,
}

async fn update_policy(
    State(state): State<AppState>,
    RequireHr(actor): RequireHr,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdatePolicy>,
) -> Result<Json<Value>, AppError> {
    let title = body.title.trim();
    if title.is_empty() {
        return Err(AppError::BadRequest("a title is required".into()));
    }
    if body.content.len() > MAX_CONTENT_BYTES {
        return Err(AppError::BadRequest("the document is too large".into()));
    }
    let doc = okf::update(
        &state.db,
        id,
        title,
        norm_category(&body.category),
        &body.content,
        body.sort_order,
        actor.id,
    )
    .await?;
    audit::log(&state.db, actor.id, "okf.update", "okf_document", Some(id)).await;
    Ok(Json(json!(doc)))
}

async fn delete_policy(
    State(state): State<AppState>,
    RequireHr(actor): RequireHr,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    // The system-config rulebook is not deletable.
    let doc = okf::get(&state.db, id).await?;
    if doc.slug == okf::SYSTEM_SLUG {
        return Err(AppError::BadRequest(
            "the system rulebook can't be deleted".into(),
        ));
    }
    okf::delete(&state.db, id).await?;
    audit::log(&state.db, actor.id, "okf.delete", "okf_document", Some(id)).await;
    Ok(Json(json!({ "deleted": true })))
}

// ---- File attachments (kind = 'file') ----

#[derive(Deserialize)]
struct UploadReq {
    file_name: String,
}

/// Presign a PUT so HR can upload a file straight to object storage, then call
/// `POST /admin/policies/file` with the returned `storage_key` to register it.
async fn upload_url(
    State(state): State<AppState>,
    _hr: RequireHr,
    Json(body): Json<UploadReq>,
) -> Result<Json<Value>, AppError> {
    let name = safe_file_name(&body.file_name);
    let key = format!("policies/{}/{}", Uuid::new_v4(), name);
    let url = state
        .storage
        .presign_put(&key, UPLOAD_URL_EXPIRES_SECS, Utc::now());
    Ok(Json(json!({
        "url": url,
        "method": "PUT",
        "storage_key": key,
        "expires_in": UPLOAD_URL_EXPIRES_SECS,
    })))
}

#[derive(Deserialize)]
struct NewFile {
    title: String,
    #[serde(default)]
    category: String,
    storage_key: String,
    file_name: String,
    #[serde(default)]
    content_type: String,
    #[serde(default)]
    size_bytes: i64,
}

async fn create_file_policy(
    State(state): State<AppState>,
    RequireHr(actor): RequireHr,
    Json(body): Json<NewFile>,
) -> Result<Json<Value>, AppError> {
    let title = body.title.trim();
    if title.is_empty() {
        return Err(AppError::BadRequest("a title is required".into()));
    }
    // Only accept a key our own presign step handed out.
    if !body.storage_key.starts_with("policies/") {
        return Err(AppError::BadRequest("invalid storage key".into()));
    }
    let doc = okf::create_file(
        &state.db,
        title,
        norm_category(&body.category),
        &body.storage_key,
        &body.file_name,
        &body.content_type,
        body.size_bytes,
        actor.id,
    )
    .await?;
    audit::log(&state.db, actor.id, "okf.create_file", "okf_document", Some(doc.id)).await;
    Ok(Json(json!(doc)))
}

/// A short-lived presigned download URL for a file document (any signed-in user).
async fn download_policy(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    let doc = okf::get(&state.db, id).await?;
    let key = match doc.storage_key {
        Some(k) if doc.kind == "file" => k,
        _ => return Err(AppError::BadRequest("this document has no attached file".into())),
    };
    let url = state
        .storage
        .presign_get(&key, DOWNLOAD_URL_EXPIRES_SECS, Utc::now());
    Ok(Json(json!({
        "url": url,
        "file_name": doc.file_name,
        "expires_in": DOWNLOAD_URL_EXPIRES_SECS,
    })))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/policies", get(list_policies))
        .route("/policies/:id", get(get_policy))
        .route("/policies/:id/download", get(download_policy))
        .route("/admin/policies", post(create_policy))
        .route(
            "/admin/policies/:id",
            put(update_policy).delete(delete_policy),
        )
        .route("/admin/policies/upload-url", post(upload_url))
        .route("/admin/policies/file", post(create_file_policy))
}

//! Candidate onboarding (Feature 6A). HR only; every mutating action audited.
//!
//!   GET    /admin/onboarding/stages              list pipeline stages
//!   GET    /admin/candidates                     list candidates (Kanban feed)
//!   POST   /admin/candidates                     create { name, email, position?, stage_id? }
//!   GET    /admin/candidates/:id                 detail + tasks + documents
//!   PATCH  /admin/candidates/:id                 update fields / move stage / set status
//!   DELETE /admin/candidates/:id                 delete
//!   POST   /admin/candidates/:id/tasks           add checklist task { title }
//!   PATCH  /admin/candidate-tasks/:tid           toggle done { done }
//!   DELETE /admin/candidate-tasks/:tid           remove task
//!   POST   /admin/candidates/:id/documents/presign  mint upload URL { doc_type?, filename? }
//!   POST   /admin/candidates/:id/documents       save metadata { doc_type?, storage_key }
//!   POST   /admin/candidates/:id/convert         convert candidate -> employee user
//!
//! Documents follow Rule 5: only metadata is stored; bytes go straight to object
//! storage via a short-lived presigned PUT, viewed via a short-lived presigned GET.

use axum::{
    extract::{Path, State},
    routing::{get, patch, post},
    Json, Router,
};
use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::db::{audit, onboarding, users};
use crate::error::AppError;
use crate::middleware::RequireHr;
use crate::auth;
use crate::role::UserRole;
use crate::state::AppState;

/// Presigned URL lifetimes for candidate documents. View URLs are short-lived
/// bearer capabilities over sensitive docs, so kept brief (SEC-15); the upload
/// (PUT) window stays longer to allow the client time to transfer.
const UPLOAD_URL_EXPIRES_SECS: u64 = 900;
const VIEW_URL_EXPIRES_SECS: u64 = 120;


/// Storage key prefix for a candidate's documents.
fn doc_prefix(candidate_id: Uuid) -> String {
    format!("candidates/{candidate_id}/")
}

// ---- Stages ----

async fn list_stages(
    State(state): State<AppState>,
    RequireHr(_hr): RequireHr,
) -> Result<Json<Value>, AppError> {
    Ok(Json(json!(onboarding::list_stages(&state.db).await?)))
}

// ---- Candidates ----

async fn list_candidates(
    State(state): State<AppState>,
    RequireHr(_hr): RequireHr,
) -> Result<Json<Value>, AppError> {
    Ok(Json(json!(onboarding::list(&state.db).await?)))
}

#[derive(Deserialize)]
struct CreateCandidate {
    name: String,
    email: String,
    #[serde(default)]
    position: String,
    #[serde(default)]
    stage_id: Option<Uuid>,
}

async fn create_candidate(
    State(state): State<AppState>,
    RequireHr(hr): RequireHr,
    Json(body): Json<CreateCandidate>,
) -> Result<Json<Value>, AppError> {
    // SEC-27: length caps, control-char rejection, and a real email check.
    let name = crate::validate::text(&body.name, "name", 200, true)?;
    let email = crate::validate::email(&body.email)?;
    let position = crate::validate::text(&body.position, "position", 200, false)?;
    // Default to the first pipeline stage; validate an explicit one.
    let stage_id = match body.stage_id {
        Some(s) => {
            if !onboarding::stage_exists(&state.db, s).await? {
                return Err(AppError::BadRequest("unknown stage_id".into()));
            }
            s
        }
        None => onboarding::first_stage_id(&state.db).await?,
    };

    let candidate =
        onboarding::create(&state.db, &name, &email, &position, stage_id, hr.id).await?;
    audit::log(&state.db, hr.id, "candidate.create", "candidate", Some(candidate.id)).await;
    Ok(Json(json!(candidate)))
}

/// Candidate detail: the candidate, its checklist, and its documents (each with
/// a short-lived presigned view URL).
async fn get_candidate(
    State(state): State<AppState>,
    RequireHr(_hr): RequireHr,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    let candidate = onboarding::get(&state.db, id).await?.ok_or(AppError::NotFound)?;
    let tasks = onboarding::list_tasks(&state.db, id).await?;
    let docs = onboarding::list_documents(&state.db, id).await?;
    let now = Utc::now();
    let documents: Vec<Value> = docs
        .iter()
        .map(|d| {
            json!({
                "id": d.id,
                "doc_type": d.doc_type,
                // RA-14: don't expose the internal storage key (Rule 5); the
                // presigned URL is all the client needs.
                "created_at": d.created_at,
                "url": state.storage.presign_get(&d.storage_key, VIEW_URL_EXPIRES_SECS, now),
            })
        })
        .collect();
    Ok(Json(json!({
        "candidate": candidate,
        "tasks": tasks,
        "documents": documents,
    })))
}

#[derive(Deserialize)]
struct UpdateCandidate {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    position: Option<String>,
    /// Move the candidate to another stage (Kanban transition).
    #[serde(default)]
    stage_id: Option<Uuid>,
    /// 'active' | 'hired' | 'rejected'.
    #[serde(default)]
    status: Option<String>,
}

const STATUSES: [&str; 3] = ["active", "hired", "rejected"];

async fn update_candidate(
    State(state): State<AppState>,
    RequireHr(hr): RequireHr,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateCandidate>,
) -> Result<Json<Value>, AppError> {
    if onboarding::get(&state.db, id).await?.is_none() {
        return Err(AppError::NotFound);
    }

    // SEC-27: validate each provided field (caps, control chars, email shape).
    let name = match body.name.as_deref() {
        Some(v) => Some(crate::validate::text(v, "name", 200, true)?),
        None => None,
    };
    let email = match body.email.as_deref() {
        Some(v) => Some(crate::validate::email(v)?),
        None => None,
    };
    let position = match body.position.as_deref() {
        Some(v) => Some(crate::validate::text(v, "position", 200, false)?),
        None => None,
    };

    if name.is_some() || email.is_some() || position.is_some() {
        onboarding::update(
            &state.db,
            id,
            name.as_deref(),
            email.as_deref(),
            position.as_deref(),
        )
        .await?;
    }
    if let Some(stage_id) = body.stage_id {
        if !onboarding::stage_exists(&state.db, stage_id).await? {
            return Err(AppError::BadRequest("unknown stage_id".into()));
        }
        onboarding::set_stage(&state.db, id, stage_id).await?;
    }
    if let Some(status) = body.status.as_deref() {
        if !STATUSES.contains(&status) {
            return Err(AppError::BadRequest(
                "status must be 'active', 'hired' or 'rejected'".into(),
            ));
        }
        onboarding::set_status(&state.db, id, status).await?;
    }
    audit::log(&state.db, hr.id, "candidate.update", "candidate", Some(id)).await;

    let updated = onboarding::get(&state.db, id).await?.ok_or(AppError::NotFound)?;
    Ok(Json(json!(updated)))
}

async fn delete_candidate(
    State(state): State<AppState>,
    RequireHr(hr): RequireHr,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    if !onboarding::delete(&state.db, id).await? {
        return Err(AppError::NotFound);
    }
    audit::log(&state.db, hr.id, "candidate.delete", "candidate", Some(id)).await;
    Ok(Json(json!({ "deleted": true })))
}

// ---- Checklist tasks ----

#[derive(Deserialize)]
struct CreateCandidateTask {
    title: String,
}

async fn add_task(
    State(state): State<AppState>,
    RequireHr(hr): RequireHr,
    Path(id): Path<Uuid>,
    Json(body): Json<CreateCandidateTask>,
) -> Result<Json<Value>, AppError> {
    let title = crate::validate::text(&body.title, "title", 500, true)?;
    if onboarding::get(&state.db, id).await?.is_none() {
        return Err(AppError::NotFound);
    }
    let task = onboarding::create_task(&state.db, id, &title).await?;
    audit::log(&state.db, hr.id, "candidate.task.create", "candidate_task", Some(task.id)).await;
    Ok(Json(json!(task)))
}

#[derive(Deserialize)]
struct ToggleTask {
    done: bool,
}

async fn toggle_task(
    State(state): State<AppState>,
    RequireHr(hr): RequireHr,
    Path(tid): Path<Uuid>,
    Json(body): Json<ToggleTask>,
) -> Result<Json<Value>, AppError> {
    if !onboarding::set_task_done(&state.db, tid, body.done).await? {
        return Err(AppError::NotFound);
    }
    audit::log(&state.db, hr.id, "candidate.task.update", "candidate_task", Some(tid)).await;
    Ok(Json(json!({ "ok": true })))
}

async fn delete_task(
    State(state): State<AppState>,
    RequireHr(hr): RequireHr,
    Path(tid): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    if !onboarding::delete_task(&state.db, tid).await? {
        return Err(AppError::NotFound);
    }
    audit::log(&state.db, hr.id, "candidate.task.delete", "candidate_task", Some(tid)).await;
    Ok(Json(json!({ "deleted": true })))
}

// ---- Documents (Rule 5: metadata only) ----

#[derive(Deserialize)]
struct PresignDoc {
    #[serde(default)]
    doc_type: String,
    /// Original filename — used only to give the stored object a friendly suffix.
    #[serde(default)]
    filename: Option<String>,
}

/// Mint a presigned PUT for a candidate document. The server picks the storage
/// key (namespaced under the candidate) so a client can't write elsewhere.
async fn presign_document(
    State(state): State<AppState>,
    RequireHr(_hr): RequireHr,
    Path(id): Path<Uuid>,
    Json(body): Json<PresignDoc>,
) -> Result<Json<Value>, AppError> {
    if onboarding::get(&state.db, id).await?.is_none() {
        return Err(AppError::NotFound);
    }
    // RA-19: validate doc_type like the other onboarding fields.
    let doc_type = crate::validate::text(&body.doc_type, "doc_type", 100, false)?;
    let suffix = sanitize_filename(body.filename.as_deref().unwrap_or(""));
    let storage_key = format!("{}{}-{}", doc_prefix(id), Uuid::new_v4(), suffix);
    let url = state.storage.presign_put(&storage_key, UPLOAD_URL_EXPIRES_SECS, Utc::now());
    Ok(Json(json!({
        "url": url,
        "method": "PUT",
        // storage_key is returned here because the client must echo it back to
        // POST /documents after uploading (round-trip), unlike read responses.
        "storage_key": storage_key,
        "doc_type": doc_type,
        "expires_in": UPLOAD_URL_EXPIRES_SECS,
    })))
}

#[derive(Deserialize)]
struct SaveDoc {
    #[serde(default)]
    doc_type: String,
    storage_key: String,
}

/// Persist document metadata after a successful direct upload.
async fn save_document(
    State(state): State<AppState>,
    RequireHr(hr): RequireHr,
    Path(id): Path<Uuid>,
    Json(body): Json<SaveDoc>,
) -> Result<Json<Value>, AppError> {
    if onboarding::get(&state.db, id).await?.is_none() {
        return Err(AppError::NotFound);
    }
    // SEC-11: the key must be exactly `candidates/<id>/<single-segment>` — a
    // prefix check alone allows `candidates/<id>/../<other>/...` traversal, so
    // require the remainder to be a single path segment (no further `/`).
    let within_namespace = body
        .storage_key
        .strip_prefix(&doc_prefix(id))
        .is_some_and(|rest| !rest.is_empty() && !rest.contains('/'));
    if !within_namespace {
        return Err(AppError::BadRequest(
            "storage_key is not a valid key within the candidate namespace".into(),
        ));
    }
    // RA-19: validate doc_type.
    let doc_type = crate::validate::text(&body.doc_type, "doc_type", 100, false)?;
    let doc = onboarding::add_document(&state.db, id, &doc_type, &body.storage_key).await?;
    audit::log(&state.db, hr.id, "candidate.document.add", "candidate_document", Some(doc.id)).await;
    let now = Utc::now();
    Ok(Json(json!({
        "id": doc.id,
        "doc_type": doc.doc_type,
        // RA-14: internal storage key omitted (Rule 5).
        "created_at": doc.created_at,
        "url": state.storage.presign_get(&doc.storage_key, VIEW_URL_EXPIRES_SECS, now),
    })))
}

// ---- Convert to employee ----

/// `POST /admin/candidates/:id/convert` — create an employee user from the
/// candidate, mark the candidate hired, and return the temporary password once.
async fn convert_candidate(
    State(state): State<AppState>,
    RequireHr(hr): RequireHr,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    let candidate = onboarding::get(&state.db, id).await?.ok_or(AppError::NotFound)?;
    if candidate.converted_user_id.is_some() {
        return Err(AppError::BadRequest("candidate is already converted".into()));
    }

    let password = auth::generate_temp_password();
    let hash = auth::hash_password(&password).map_err(AppError::Internal)?;
    // `users::create` maps a duplicate email to a clean BadRequest.
    let user = users::create(
        &state.db,
        &candidate.name,
        &candidate.email,
        &hash,
        UserRole::Employee,
        None,
    )
    .await?;

    // Move to the final (highest-sequence) stage and flag as hired.
    let stages = onboarding::list_stages(&state.db).await?;
    let final_stage = stages.last().map(|s| s.id).unwrap_or(candidate.stage_id);
    onboarding::mark_converted(&state.db, id, user.id, final_stage).await?;

    audit::log(&state.db, hr.id, "candidate.convert", "candidate", Some(id)).await;
    audit::log(&state.db, hr.id, "user.create", "user", Some(user.id)).await;

    Ok(Json(json!({
        "user": user,
        "password": password,
    })))
}

/// Keep only safe filename characters; cap length. Empty -> "file".
fn sanitize_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' { c } else { '-' })
        .collect();
    let trimmed = cleaned.trim_matches('-');
    let out = if trimmed.is_empty() { "file" } else { trimmed };
    out.chars().take(64).collect()
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/onboarding/stages", get(list_stages))
        .route("/admin/candidates", get(list_candidates).post(create_candidate))
        .route(
            "/admin/candidates/:id",
            get(get_candidate).patch(update_candidate).delete(delete_candidate),
        )
        .route("/admin/candidates/:id/tasks", post(add_task))
        .route(
            "/admin/candidate-tasks/:tid",
            patch(toggle_task).delete(delete_task),
        )
        .route("/admin/candidates/:id/documents/presign", post(presign_document))
        .route("/admin/candidates/:id/documents", post(save_document))
        .route("/admin/candidates/:id/convert", post(convert_candidate))
}

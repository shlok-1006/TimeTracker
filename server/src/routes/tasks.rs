//! Manual-task management (Feature 5 Phase 2). HR only; every action audited.
//!
//!   POST   /admin/users/:id/tasks   create a task for an employee  { title, description }
//!   GET    /admin/users/:id/tasks   list an employee's tasks
//!   PATCH  /admin/tasks/:id          update title / description / status
//!   DELETE /admin/tasks/:id          delete a task
//!
//! These tasks are internal only — they never touch Linear.

use axum::{
    extract::{Path, State},
    routing::get,
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::db::{audit, manual_tasks, users};
use crate::error::AppError;
use crate::middleware::{AuthUser, RequireHr};
use crate::state::AppState;

/// `GET /me/tasks` — the authenticated employee's own manual tasks.
async fn my_tasks(State(state): State<AppState>, user: AuthUser) -> Result<Json<Value>, AppError> {
    Ok(Json(json!(manual_tasks::list_for_user(&state.db, user.id).await?)))
}

#[derive(Deserialize)]
struct CreateTask {
    title: String,
    #[serde(default)]
    description: String,
}

async fn create_task(
    State(state): State<AppState>,
    RequireHr(hr): RequireHr,
    Path(target): Path<Uuid>,
    Json(body): Json<CreateTask>,
) -> Result<Json<Value>, AppError> {
    let title = body.title.trim();
    if title.is_empty() {
        return Err(AppError::BadRequest("title is required".into()));
    }
    // Assignee must exist (gives a clean 404 instead of an FK error).
    if users::find_by_id(&state.db, target).await?.is_none() {
        return Err(AppError::NotFound);
    }
    let task = manual_tasks::create(&state.db, target, hr.id, title, body.description.trim()).await?;
    audit::log(&state.db, hr.id, "task.create", "manual_task", Some(task.id)).await;
    Ok(Json(json!(task)))
}

async fn list_tasks(
    State(state): State<AppState>,
    RequireHr(_hr): RequireHr,
    Path(target): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(json!(manual_tasks::list_for_user(&state.db, target).await?)))
}

#[derive(Deserialize)]
struct UpdateTask {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    status: Option<String>,
}

async fn update_task(
    State(state): State<AppState>,
    RequireHr(hr): RequireHr,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateTask>,
) -> Result<Json<Value>, AppError> {
    if manual_tasks::get(&state.db, id).await?.is_none() {
        return Err(AppError::NotFound);
    }
    if let Some(s) = body.status.as_deref() {
        if !manual_tasks::is_valid_status(s) {
            return Err(AppError::BadRequest("status must be 'open' or 'done'".into()));
        }
    }
    let title = body.title.as_deref().map(str::trim);
    if matches!(title, Some("")) {
        return Err(AppError::BadRequest("title cannot be empty".into()));
    }
    let description = body.description.as_deref().map(str::trim);

    if title.is_some() || description.is_some() {
        manual_tasks::update(&state.db, id, title, description).await?;
    }
    if let Some(s) = body.status.as_deref() {
        manual_tasks::set_status(&state.db, id, s).await?;
    }
    audit::log(&state.db, hr.id, "task.update", "manual_task", Some(id)).await;

    let updated = manual_tasks::get(&state.db, id).await?.ok_or(AppError::NotFound)?;
    Ok(Json(json!(updated)))
}

async fn delete_task(
    State(state): State<AppState>,
    RequireHr(hr): RequireHr,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    if !manual_tasks::delete(&state.db, id).await? {
        return Err(AppError::NotFound);
    }
    audit::log(&state.db, hr.id, "task.delete", "manual_task", Some(id)).await;
    Ok(Json(json!({ "deleted": true })))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/me/tasks", get(my_tasks))
        .route("/admin/users/:id/tasks", get(list_tasks).post(create_task))
        .route(
            "/admin/tasks/:id",
            axum::routing::patch(update_task).delete(delete_task),
        )
}

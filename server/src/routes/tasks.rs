//! Manual-task management (Feature 5 Phase 2). HR or project manager; every
//! action is audited.
//!
//!   POST   /admin/users/:id/tasks   assign a task  { title, description, weight, due_date }
//!   GET    /admin/users/:id/tasks   list an employee's tasks
//!   PATCH  /admin/tasks/:id          update title / description / weight / due_date / status
//!   DELETE /admin/tasks/:id          delete a task
//!
//! Scope (CLAUDE.md Rule 11): HR may assign to anyone; a project manager only to
//! employees they manage (enforced via `authorize_view`). These tasks are
//! internal only — they never touch Linear.

use axum::{
    extract::{Path, State},
    routing::get,
    Json, Router,
};
use chrono::NaiveDate;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::db::{audit, manual_tasks, users};
use crate::error::AppError;
use crate::middleware::{AuthUser, RequireAdmin};
use crate::routes::admin::authorize_view;
use crate::state::AppState;

/// Default weight when the assigner doesn't specify one (neutral middle of 1–10).
fn default_weight() -> i32 {
    5
}

/// Weights are an importance/effort scale out of 10.
fn validate_weight(weight: i32) -> Result<(), AppError> {
    if (1..=10).contains(&weight) {
        Ok(())
    } else {
        Err(AppError::BadRequest(
            "weight must be between 1 and 10".into(),
        ))
    }
}

/// `GET /me/tasks` — the authenticated employee's own manual tasks.
async fn my_tasks(State(state): State<AppState>, user: AuthUser) -> Result<Json<Value>, AppError> {
    Ok(Json(json!(
        manual_tasks::list_for_user(&state.db, user.id).await?
    )))
}

#[derive(Deserialize)]
struct CreateTask {
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default = "default_weight")]
    weight: i32,
    /// Optional expected due date ("YYYY-MM-DD"); `None` = open-ended.
    #[serde(default)]
    due_date: Option<NaiveDate>,
}

async fn create_task(
    State(state): State<AppState>,
    RequireAdmin(actor): RequireAdmin,
    Path(target): Path<Uuid>,
    Json(body): Json<CreateTask>,
) -> Result<Json<Value>, AppError> {
    // A PM may only assign to employees they manage; HR to anyone.
    authorize_view(&state, &actor, target).await?;

    let title = body.title.trim();
    if title.is_empty() {
        return Err(AppError::BadRequest("title is required".into()));
    }
    validate_weight(body.weight)?;
    // Assignee must exist (gives a clean 404 instead of an FK error).
    if users::find_by_id(&state.db, target).await?.is_none() {
        return Err(AppError::NotFound);
    }
    let task = manual_tasks::create(
        &state.db,
        target,
        actor.id,
        title,
        body.description.trim(),
        body.weight,
        body.due_date,
    )
    .await?;
    audit::log(
        &state.db,
        actor.id,
        "task.create",
        "manual_task",
        Some(task.id),
    )
    .await;
    Ok(Json(json!(task)))
}

async fn list_tasks(
    State(state): State<AppState>,
    RequireAdmin(actor): RequireAdmin,
    Path(target): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    authorize_view(&state, &actor, target).await?;
    Ok(Json(json!(
        manual_tasks::list_for_user(&state.db, target).await?
    )))
}

#[derive(Deserialize)]
struct UpdateTask {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    weight: Option<i32>,
    #[serde(default)]
    due_date: Option<NaiveDate>,
}

async fn update_task(
    State(state): State<AppState>,
    RequireAdmin(actor): RequireAdmin,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateTask>,
) -> Result<Json<Value>, AppError> {
    // Load first so we can authorize against the task's owner (PM team scope).
    let task = manual_tasks::get(&state.db, id)
        .await?
        .ok_or(AppError::NotFound)?;
    authorize_view(&state, &actor, task.user_id).await?;

    if let Some(s) = body.status.as_deref() {
        if !manual_tasks::is_valid_status(s) {
            return Err(AppError::BadRequest(
                "status must be 'open' or 'done'".into(),
            ));
        }
    }
    if let Some(w) = body.weight {
        validate_weight(w)?;
    }
    let title = body.title.as_deref().map(str::trim);
    if matches!(title, Some("")) {
        return Err(AppError::BadRequest("title cannot be empty".into()));
    }
    let description = body.description.as_deref().map(str::trim);

    if title.is_some() || description.is_some() || body.weight.is_some() || body.due_date.is_some()
    {
        manual_tasks::update(
            &state.db,
            id,
            title,
            description,
            body.weight,
            body.due_date,
        )
        .await?;
    }
    if let Some(s) = body.status.as_deref() {
        manual_tasks::set_status(&state.db, id, s).await?;
    }
    audit::log(&state.db, actor.id, "task.update", "manual_task", Some(id)).await;

    let updated = manual_tasks::get(&state.db, id)
        .await?
        .ok_or(AppError::NotFound)?;
    Ok(Json(json!(updated)))
}

async fn delete_task(
    State(state): State<AppState>,
    RequireAdmin(actor): RequireAdmin,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    let task = manual_tasks::get(&state.db, id)
        .await?
        .ok_or(AppError::NotFound)?;
    authorize_view(&state, &actor, task.user_id).await?;

    manual_tasks::delete(&state.db, id).await?;
    audit::log(&state.db, actor.id, "task.delete", "manual_task", Some(id)).await;
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

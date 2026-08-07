//! Manual tasks — both the assigned kind and the ones people set themselves.
//!
//! ASSIGNED (HR / project manager; every action audited):
//!   POST   /admin/users/:id/tasks   assign a task  { title, description, weight, due_date }
//!   GET    /admin/users/:id/tasks   list an employee's tasks
//!   PATCH  /admin/tasks/:id         update title / description / weight / due_date / status
//!   DELETE /admin/tasks/:id         delete a task
//!
//! OWN (any signed-in employee, for themselves):
//!   GET    /me/tasks                everything on your list, assigned or self-set
//!   POST   /me/tasks                add one   { title, description, weight, due_date? }
//!   PATCH  /me/tasks/:id            tick off any of yours; edit only your own
//!   DELETE /me/tasks/:id            remove one you added yourself
//!
//! Both kinds live in one table and one list, because a person's work does not
//! divide into "what I was given" and "what I chose" — but `created_by` keeps
//! the distinction where it matters: you may complete assigned work, never
//! reword or delete it.
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

// ─────────────────────────── employee self-serve ───────────────────────────
//
// Everything above is the assigner's view. What follows lets people keep their
// OWN list: the work you plan for yourself is most of the work, and until now
// the only way onto this list was for a manager to put you there.
//
// The ownership rule is deliberately asymmetric, and `created_by` is what makes
// it expressible:
//
//   * anyone may flip the status of a task assigned to THEM — ticking off work
//     your manager gave you is the normal case, not an edit;
//   * only the AUTHOR may change a task's wording, weight or date, or delete
//     it. An employee silently rewriting or removing a task their PM assigned
//     would make the list untrustworthy for exactly the person relying on it.

/// Fetch a task and confirm it belongs to `user`; 404 otherwise (never leak the
/// existence of someone else's task through a distinguishable error).
async fn own_task(
    state: &AppState,
    user_id: Uuid,
    id: Uuid,
) -> Result<manual_tasks::ManualTask, AppError> {
    let task = manual_tasks::get(&state.db, id).await?.ok_or(AppError::NotFound)?;
    if task.user_id != user_id {
        return Err(AppError::NotFound);
    }
    Ok(task)
}

#[derive(Deserialize)]
struct CreateMyTask {
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default = "default_weight")]
    weight: i32,
    /// Optional — a task with no date is open-ended, not overdue.
    #[serde(default)]
    due_date: Option<NaiveDate>,
}

/// `POST /me/tasks` — add a task to your own list.
async fn create_my_task(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<CreateMyTask>,
) -> Result<Json<Value>, AppError> {
    let title = body.title.trim();
    if title.is_empty() {
        return Err(AppError::BadRequest("title is required".into()));
    }
    validate_weight(body.weight)?;
    // Assignee and author are the same person — that IS the self-serve case,
    // and it is what later distinguishes these from assigned work.
    let task = manual_tasks::create(
        &state.db,
        user.id,
        user.id,
        title,
        body.description.trim(),
        body.weight,
        body.due_date,
    )
    .await?;
    audit::log(
        &state.db,
        user.id,
        "task.create_own",
        "manual_task",
        Some(task.id),
    )
    .await;
    Ok(Json(json!(task)))
}

#[derive(Deserialize)]
struct UpdateMyTask {
    title: Option<String>,
    description: Option<String>,
    weight: Option<i32>,
    due_date: Option<NaiveDate>,
    /// true = make it open-ended. Distinguishes "no change" from "remove it",
    /// which `due_date: None` alone cannot say.
    #[serde(default)]
    clear_due_date: bool,
    status: Option<String>,
}

/// `PATCH /me/tasks/:id` — tick off any task of yours; edit only your own.
async fn update_my_task(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateMyTask>,
) -> Result<Json<Value>, AppError> {
    let task = own_task(&state, user.id, id).await?;

    if let Some(status) = body.status.as_deref() {
        if !manual_tasks::is_valid_status(status) {
            return Err(AppError::BadRequest("status must be open or done".into()));
        }
        manual_tasks::set_status(&state.db, id, status).await?;
    }

    let edits_requested = body.title.is_some()
        || body.description.is_some()
        || body.weight.is_some()
        || body.due_date.is_some()
        || body.clear_due_date;

    if edits_requested {
        // Assigned work is the assigner's to word. You may complete it, not restate it.
        if task.created_by != Some(user.id) {
            return Err(AppError::Forbidden);
        }
        let title = body.title.as_deref().map(str::trim);
        if title.is_some_and(str::is_empty) {
            return Err(AppError::BadRequest("title cannot be empty".into()));
        }
        if let Some(w) = body.weight {
            validate_weight(w)?;
        }
        manual_tasks::update_fields(
            &state.db,
            id,
            title,
            body.description.as_deref().map(str::trim),
            body.weight,
            body.due_date,
            body.clear_due_date,
        )
        .await?;
    }

    let updated = manual_tasks::get(&state.db, id).await?.ok_or(AppError::NotFound)?;
    Ok(Json(json!(updated)))
}

/// `DELETE /me/tasks/:id` — remove a task you added yourself.
async fn delete_my_task(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    let task = own_task(&state, user.id, id).await?;
    // Deleting assigned work would erase the record a manager is relying on.
    if task.created_by != Some(user.id) {
        return Err(AppError::Forbidden);
    }
    manual_tasks::delete(&state.db, id).await?;
    audit::log(
        &state.db,
        user.id,
        "task.delete_own",
        "manual_task",
        Some(id),
    )
    .await;
    Ok(Json(json!({ "deleted": true })))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/me/tasks", get(my_tasks).post(create_my_task))
        .route(
            "/me/tasks/:id",
            axum::routing::patch(update_my_task).delete(delete_my_task),
        )
        .route("/admin/users/:id/tasks", get(list_tasks).post(create_task))
        .route(
            "/admin/tasks/:id",
            axum::routing::patch(update_task).delete(delete_task),
        )
}

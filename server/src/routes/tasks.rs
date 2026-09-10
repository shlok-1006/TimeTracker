//! Manual-task management (Feature 5 Phase 2). HR or project manager; every
//! action is audited.
//!
//!   POST   /admin/users/:id/tasks   assign a task  { title, description, weight, due_date, pr_links }
//!   GET    /admin/users/:id/tasks   list an employee's tasks
//!   PATCH  /admin/tasks/:id          update title / description / weight / due_date / status / pr_links
//!   DELETE /admin/tasks/:id          delete a task
//!
//! Self-serve (the employee's own AddTask board):
//!   GET    /me/tasks                 own tasks
//!   POST   /me/tasks                 add a task to yourself  { title, ..., pr_links }
//!   PATCH  /me/tasks/:id             edit your own task (full on self-created; status+pr on assigned)
//!   DELETE /me/tasks/:id             remove a self-created task
//!
//! `pr_links` are full GitHub PR URLs (multi-value) the HRMS performance engine reviews to score
//! the person; the engine reads them per team via GET /admin/teams/:id/tasks?has_pr=1.
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
use crate::middleware::{AuthUser, RequireStaff};
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

/// PR links are full GitHub PR URLs (the URL carries the repo, so nothing needs a naming
/// convention). Trim, drop blanks, and require each to look like an http(s) URL so the engine
/// never receives junk. Caps the count so one task can't carry an unbounded list.
fn clean_pr_links(links: &[String]) -> Result<Vec<String>, AppError> {
    let mut out = Vec::new();
    for raw in links {
        let s = raw.trim();
        if s.is_empty() {
            continue;
        }
        if !(s.starts_with("https://") || s.starts_with("http://")) {
            return Err(AppError::BadRequest(format!(
                "pr_links must be full URLs (got {s:?})"
            )));
        }
        out.push(s.to_string());
    }
    if out.len() > 50 {
        return Err(AppError::BadRequest("too many pr_links (max 50)".into()));
    }
    out.dedup();
    Ok(out)
}

/// `GET /me/tasks` — the authenticated employee's own manual tasks. Each task is
/// flagged `self_created` (created by the employee vs assigned by HR/PM) so the
/// desktop only offers Delete on the employee's own.
async fn my_tasks(State(state): State<AppState>, user: AuthUser) -> Result<Json<Value>, AppError> {
    let tasks = manual_tasks::list_for_user(&state.db, user.id).await?;
    let out: Vec<Value> = tasks
        .into_iter()
        .map(|t| {
            let self_created = t.created_by == Some(user.id);
            let mut v = serde_json::to_value(&t).unwrap_or_else(|_| json!({}));
            if let Some(obj) = v.as_object_mut() {
                obj.insert("self_created".to_string(), json!(self_created));
            }
            v
        })
        .collect();
    Ok(Json(json!(out)))
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
    /// Full GitHub PR URLs linked to the task (multi-value). Optional.
    #[serde(default)]
    pr_links: Vec<String>,
}

async fn create_task(
    State(state): State<AppState>,
    RequireStaff(actor): RequireStaff,
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
    let pr_links = clean_pr_links(&body.pr_links)?;
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
        &pr_links,
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
    RequireStaff(actor): RequireStaff,
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
    /// Replace the PR link set (send the full list). Omit to leave it unchanged.
    #[serde(default)]
    pr_links: Option<Vec<String>>,
}

async fn update_task(
    State(state): State<AppState>,
    RequireStaff(actor): RequireStaff,
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
    let pr_links = body.pr_links.as_deref().map(clean_pr_links).transpose()?;

    if title.is_some()
        || description.is_some()
        || body.weight.is_some()
        || body.due_date.is_some()
        || pr_links.is_some()
    {
        manual_tasks::update(
            &state.db,
            id,
            title,
            description,
            body.weight,
            body.due_date,
            pr_links.as_deref(),
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
    RequireStaff(actor): RequireStaff,
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

// ---- Employee self-service: assign a task to yourself + mark it done ----

/// `POST /me/tasks` — an employee assigns a task to themselves. Same shape as the
/// HR/PM assign flow, but the owner and creator are the caller. Fed to the AI
/// analysis as work context, like an HR-assigned task.
async fn create_my_task(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<CreateTask>,
) -> Result<Json<Value>, AppError> {
    let title = body.title.trim();
    if title.is_empty() {
        return Err(AppError::BadRequest("title is required".into()));
    }
    validate_weight(body.weight)?;
    let pr_links = clean_pr_links(&body.pr_links)?;
    let task = manual_tasks::create(
        &state.db,
        user.id,
        user.id,
        title,
        body.description.trim(),
        body.weight,
        body.due_date,
        &pr_links,
    )
    .await?;
    audit::log(
        &state.db,
        user.id,
        "task.create.self",
        "manual_task",
        Some(task.id),
    )
    .await;
    Ok(Json(json!(task)))
}

#[derive(Deserialize)]
struct MyTaskUpdate {
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
    /// Replace the PR link set (send the full list). Omit to leave unchanged.
    #[serde(default)]
    pr_links: Option<Vec<String>>,
}

/// `PATCH /me/tasks/:id` — the employee edits one of their OWN tasks.
///
/// Asymmetric on purpose: on a task they CREATED, they may edit everything (title, description,
/// weight, due date, PR links, status) — that is their AddTask board. On a task HR/PM ASSIGNED
/// them, the assigner's title/weight/due stand; the employee may still set `status` and attach
/// `pr_links` (report the PRs that delivered the work, mark it done) but not rewrite the rest.
async fn update_my_task(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<MyTaskUpdate>,
) -> Result<Json<Value>, AppError> {
    let task = manual_tasks::get(&state.db, id)
        .await?
        .ok_or(AppError::NotFound)?;
    if task.user_id != user.id {
        return Err(AppError::Forbidden);
    }
    let self_created = task.created_by == Some(user.id);

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
    let pr_links = body.pr_links.as_deref().map(clean_pr_links).transpose()?;

    // Assigned task: the protected fields belong to the assigner. Refuse rather than silently drop.
    let touches_protected = title.is_some()
        || description.is_some()
        || body.weight.is_some()
        || body.due_date.is_some();
    if !self_created && touches_protected {
        return Err(AppError::Forbidden);
    }

    // Only apply the protected fields on a self-created task; status + pr_links apply either way.
    let (t, d, w, dd) = if self_created {
        (title, description, body.weight, body.due_date)
    } else {
        (None, None, None, None)
    };
    if t.is_some() || d.is_some() || w.is_some() || dd.is_some() || pr_links.is_some() {
        manual_tasks::update(&state.db, id, t, d, w, dd, pr_links.as_deref()).await?;
    }
    if let Some(s) = body.status.as_deref() {
        manual_tasks::set_status(&state.db, id, s).await?;
    }
    audit::log(
        &state.db,
        user.id,
        "task.update.self",
        "manual_task",
        Some(id),
    )
    .await;
    let updated = manual_tasks::get(&state.db, id)
        .await?
        .ok_or(AppError::NotFound)?;
    Ok(Json(json!(updated)))
}

/// `DELETE /me/tasks/:id` — remove one of your OWN self-created tasks. A task
/// HR/PM assigned to you can't be deleted here (only marked done), so an employee
/// can't hide assigned work.
async fn delete_my_task(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    let task = manual_tasks::get(&state.db, id)
        .await?
        .ok_or(AppError::NotFound)?;
    if task.created_by != Some(user.id) {
        return Err(AppError::Forbidden);
    }
    manual_tasks::delete(&state.db, id).await?;
    audit::log(
        &state.db,
        user.id,
        "task.delete.self",
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

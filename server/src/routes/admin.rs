//! Admin/dashboard routes (require HR or project-manager — `RequireAdmin`).
//!
//! Scope (CLAUDE.md): HR sees everyone; a project manager sees only their own
//! team (`users.manager_id = <pm>`). Enforced on the team list (query filter)
//! and on every drill-down (explicit `authorize_view`).

use axum::{
    extract::{Path, Query, State},
    routing::{delete, get},
    Json, Router,
};
use chrono::{DateTime, NaiveDate, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use std::str::FromStr;
use uuid::Uuid;

use crate::auth;
use crate::db::{analysis_results, audit, intervals, presence, refresh_tokens, screenshots, users};
use crate::error::AppError;
use crate::middleware::{AuthUser, RequireAdmin, RequireHr};
use crate::role::UserRole;
use crate::state::AppState;
use crate::{analysis_service, sampler};

const VIEW_URL_EXPIRES_SECS: u64 = 900;

/// Which employees the caller may see in the team list (`None` = all).
pub(crate) fn team_scope(user: &AuthUser) -> Option<Uuid> {
    match user.role {
        UserRole::Hr => None,
        _ => Some(user.id), // project manager: own team
    }
}

/// Authorize a drill-down on `target`. HR: anyone. PM: only their team.
pub(crate) async fn authorize_view(
    state: &AppState,
    viewer: &AuthUser,
    target: Uuid,
) -> Result<(), AppError> {
    if viewer.role == UserRole::Hr {
        return Ok(());
    }
    match users::manager_id_of(&state.db, target).await? {
        Some(mgr) if mgr == viewer.id => Ok(()),
        _ => Err(AppError::Forbidden),
    }
}

/// `GET /admin/team` — live team statuses + today's hours.
async fn team(
    State(state): State<AppState>,
    RequireAdmin(user): RequireAdmin,
) -> Result<Json<Value>, AppError> {
    let members = presence::team(&state.db, team_scope(&user)).await?;
    let body: Vec<Value> = members
        .into_iter()
        .map(|m| {
            json!({
                "user": { "id": m.id, "name": m.name, "email": m.email, "role": m.role },
                "status": m.status,
                "last_seen_at": m.last_seen_at,
                "today_seconds": m.today_seconds,
            })
        })
        .collect();
    Ok(Json(Value::Array(body)))
}

/// `GET /admin/users/:id/hours` — drill-down hours for one employee.
async fn user_hours(
    State(state): State<AppState>,
    RequireAdmin(user): RequireAdmin,
    Path(target): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    authorize_view(&state, &user, target).await?;
    let s = intervals::hours_summary(&state.db, target).await?;
    Ok(Json(json!({
        "total_seconds": s.total_seconds,
        "today_seconds": s.today_seconds,
        "week_seconds": s.week_seconds,
        "active_seconds": s.active_seconds,
        "idle_seconds": s.idle_seconds,
    })))
}

/// `GET /admin/users/:id/screenshots?day=` — drill-down screenshots for a day,
/// each with verdict, meeting flag, and a presigned view URL. `day` defaults to
/// today (UTC). PM is team-scoped; HR sees anyone.
async fn user_screenshots(
    State(state): State<AppState>,
    RequireAdmin(user): RequireAdmin,
    Path(target): Path<Uuid>,
    Query(q): Query<SampleQuery>,
) -> Result<Json<Value>, AppError> {
    authorize_view(&state, &user, target).await?;
    let day = q.day.unwrap_or_else(|| Utc::now().date_naive());
    let now = Utc::now();
    let rows = screenshots::list_for_day(&state.db, target, day).await?;
    let items: Vec<Value> = rows
        .iter()
        .map(|r| crate::routes::uploads::day_item(&state.storage, r, now))
        .collect();
    Ok(Json(Value::Array(items)))
}

#[derive(Deserialize)]
struct TimelineQuery {
    from: DateTime<Utc>,
    to: DateTime<Utc>,
}

/// `GET /admin/users/:id/timeline?from=&to=` — activity segments for the day
/// window, for the colored timeline bar.
async fn user_timeline(
    State(state): State<AppState>,
    RequireAdmin(user): RequireAdmin,
    Path(target): Path<Uuid>,
    Query(q): Query<TimelineQuery>,
) -> Result<Json<Value>, AppError> {
    authorize_view(&state, &user, target).await?;
    let segments = intervals::day_segments(&state.db, target, q.from, q.to).await?;
    let items: Vec<Value> = segments
        .into_iter()
        .map(|s| {
            json!({
                "start_utc": s.start_utc,
                "end_utc": s.end_utc,
                "kind": s.kind,
            })
        })
        .collect();
    Ok(Json(json!({ "from": q.from, "to": q.to, "segments": items })))
}

#[derive(Deserialize)]
struct SampleQuery {
    /// Calendar day to sample (defaults to today, UTC).
    #[serde(default)]
    day: Option<NaiveDate>,
}

/// `POST /admin/users/:id/sample?day=YYYY-MM-DD` — run the daily sampler for an
/// employee's day and return the chosen 4–5 screenshots (presigned URLs).
/// Idempotent: re-running returns the same stored set (the day is never resampled).
async fn sample_day(
    State(state): State<AppState>,
    RequireAdmin(user): RequireAdmin,
    Path(target): Path<Uuid>,
    Query(q): Query<SampleQuery>,
) -> Result<Json<Value>, AppError> {
    authorize_view(&state, &user, target).await?;
    let day = q.day.unwrap_or_else(|| Utc::now().date_naive());

    let shots = sampler::sample_screenshots(&state.db, target, day).await?;
    let now = Utc::now();
    let samples: Vec<Value> = shots
        .into_iter()
        .map(|s| {
            json!({
                "id": s.screenshot_id,
                "bucket": s.bucket,
                "taken_at": s.taken_at,
                "url": state.storage.presign_get(&s.storage_key, VIEW_URL_EXPIRES_SECS, now),
            })
        })
        .collect();

    audit::log(&state.db, user.id, "screenshot.sample", "user", Some(target)).await;
    Ok(Json(json!({ "day": day, "count": samples.len(), "samples": samples })))
}

/// `POST /admin/users/:id/analyze?day=YYYY-MM-DD` — run Vision AI over the day's
/// sampled screenshots, comparing each against the employee's assigned Linear
/// tickets, and store the validated verdicts. Idempotent at the storage layer
/// (re-running upserts each `(job, screenshot)` result).
async fn analyze_day(
    State(state): State<AppState>,
    RequireAdmin(user): RequireAdmin,
    Path(target): Path<Uuid>,
    Query(q): Query<SampleQuery>,
) -> Result<Json<Value>, AppError> {
    authorize_view(&state, &user, target).await?;
    if !state.claude.is_configured() {
        return Err(AppError::BadRequest(
            "Vision AI is not configured (set ANTHROPIC_API_KEY)".into(),
        ));
    }
    let day = q.day.unwrap_or_else(|| Utc::now().date_naive());

    // Shared with the nightly scheduler: sample → analyze → persist → report.
    let out = analysis_service::analyze_user_day(
        &state.db,
        &state.storage,
        &state.claude,
        &state.linear,
        target,
        day,
    )
    .await?;

    audit::log(&state.db, user.id, "screenshot.analyze", "user", Some(target)).await;
    Ok(Json(json!({
        "day": day,
        "analyzed": out.analyzed,
        "skipped": out.skipped,
        "model": state.claude.model(),
        "report": out.report,
    })))
}

/// `GET /admin/users/:id/analysis?day=YYYY-MM-DD` — stored analysis results.
async fn analysis_for_day(
    State(state): State<AppState>,
    RequireAdmin(user): RequireAdmin,
    Path(target): Path<Uuid>,
    Query(q): Query<SampleQuery>,
) -> Result<Json<Value>, AppError> {
    authorize_view(&state, &user, target).await?;
    let day = q.day.unwrap_or_else(|| Utc::now().date_naive());
    match sampler::load_existing_job(&state.db, target, day).await? {
        None => Ok(Json(json!({ "day": day, "results": [] }))),
        Some((job, _)) => {
            let results = analysis_results::list_for_job(&state.db, job.id).await?;
            Ok(Json(json!({ "day": day, "results": results })))
        }
    }
}

// ---- User management (HR only) ----

/// `GET /admin/users` — list all users.
async fn list_users(
    State(state): State<AppState>,
    RequireHr(_hr): RequireHr,
) -> Result<Json<Value>, AppError> {
    Ok(Json(json!(users::list_all(&state.db).await?)))
}

#[derive(Deserialize)]
struct CreateUser {
    name: String,
    email: String,
    password: String,
    role: String,
    #[serde(default)]
    manager_id: Option<Uuid>,
}

/// `POST /admin/users` — create a user (HR only). Logged to audit_logs.
async fn create_user(
    State(state): State<AppState>,
    RequireHr(hr): RequireHr,
    Json(body): Json<CreateUser>,
) -> Result<Json<Value>, AppError> {
    let role = UserRole::from_str(&body.role)
        .map_err(|_| AppError::BadRequest("role must be employee, project_manager or hr".into()))?;
    if body.password.len() < 8 {
        return Err(AppError::BadRequest("password must be at least 8 characters".into()));
    }
    if !body.email.contains('@') {
        return Err(AppError::BadRequest("invalid email".into()));
    }

    let password_hash = auth::hash_password(&body.password).map_err(AppError::Internal)?;
    let user = users::create(
        &state.db,
        body.name.trim(),
        body.email.trim(),
        &password_hash,
        role,
        body.manager_id,
    )
    .await?;

    audit::log(&state.db, hr.id, "user.create", "user", Some(user.id)).await;
    Ok(Json(json!(user)))
}

/// `DELETE /admin/users/:id` — delete a user (HR only). Logged to audit_logs.
async fn delete_user(
    State(state): State<AppState>,
    RequireHr(hr): RequireHr,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    if id == hr.id {
        return Err(AppError::BadRequest("you cannot delete your own account".into()));
    }
    if !users::delete(&state.db, id).await? {
        return Err(AppError::NotFound);
    }
    audit::log(&state.db, hr.id, "user.delete", "user", Some(id)).await;
    Ok(Json(json!({ "deleted": true })))
}

#[derive(Deserialize)]
struct ResetPassword {
    /// Optional explicit password; if omitted a temporary one is generated.
    #[serde(default)]
    password: Option<String>,
}

/// A readable temporary password (upper/lower/digit/symbol, > 8 chars).
fn temp_password() -> String {
    format!("Tt-{}!", &Uuid::new_v4().simple().to_string()[..8])
}

/// `POST /admin/users/:id/reset-password` (HR only). Sets a new password,
/// invalidates the user's existing sessions, logs it, and returns the new
/// password ONCE so HR can hand it over. (Existing passwords are never readable.)
async fn reset_password(
    State(state): State<AppState>,
    RequireHr(hr): RequireHr,
    Path(id): Path<Uuid>,
    Json(body): Json<ResetPassword>,
) -> Result<Json<Value>, AppError> {
    let password = match body.password {
        Some(p) if p.len() >= 8 => p,
        Some(_) => return Err(AppError::BadRequest("password must be at least 8 characters".into())),
        None => temp_password(),
    };

    let hash = auth::hash_password(&password).map_err(AppError::Internal)?;
    if !users::set_password(&state.db, id, &hash).await? {
        return Err(AppError::NotFound);
    }
    // Force re-login everywhere with the old credentials.
    refresh_tokens::revoke_all_for_user(&state.db, id).await?;
    audit::log(&state.db, hr.id, "user.reset_password", "user", Some(id)).await;

    Ok(Json(json!({ "password": password })))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/team", get(team))
        .route("/admin/users", get(list_users).post(create_user))
        .route("/admin/users/:id", delete(delete_user))
        .route("/admin/users/:id/reset-password", axum::routing::post(reset_password))
        .route("/admin/users/:id/hours", get(user_hours))
        .route("/admin/users/:id/screenshots", get(user_screenshots))
        .route("/admin/users/:id/timeline", get(user_timeline))
        .route("/admin/users/:id/sample", axum::routing::post(sample_day))
        .route("/admin/users/:id/analyze", axum::routing::post(analyze_day))
        .route("/admin/users/:id/analysis", get(analysis_for_day))
}

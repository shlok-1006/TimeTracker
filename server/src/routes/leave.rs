//! Leave routes: employee self-service (`/me/leave/*`), approver actions and HR
//! configuration (`/admin/leave/*`, `/admin/holidays`).
//!
//! RBAC: employees manage their own requests; project managers approve their own
//! team's requests (HR approves anyone's); only HR configures leave types,
//! allocations, and holidays.

use axum::{
    extract::{Path, Query, State},
    routing::{get, post},
    Json, Router,
};
use chrono::{Datelike, NaiveDate, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::db::{audit, leave, users};
use crate::employment_type::EmploymentType;
use crate::error::AppError;
use crate::leave_service;
use crate::middleware::{AuthUser, RequireHr, RequireStaff, RequireTeamViewer};
use crate::role::UserRole;
use crate::state::AppState;

#[derive(Deserialize)]
struct YearQuery {
    year: Option<i32>,
}

// ---- Employee self-service ----

async fn my_types(State(state): State<AppState>, _u: AuthUser) -> Result<Json<Value>, AppError> {
    Ok(Json(json!(leave::list_types(&state.db).await?)))
}

async fn my_balance(
    State(state): State<AppState>,
    user: AuthUser,
    Query(q): Query<YearQuery>,
) -> Result<Json<Value>, AppError> {
    let year = q.year.unwrap_or_else(|| Utc::now().year());
    Ok(Json(json!({
        "year": year,
        "balances": leave::balances(&state.db, user.id, year).await?,
    })))
}

async fn my_requests(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<Value>, AppError> {
    Ok(Json(json!(
        leave::list_requests_for_user(&state.db, user.id).await?
    )))
}

#[derive(Deserialize)]
struct NewLeave {
    leave_type_id: Uuid,
    start_date: NaiveDate,
    end_date: NaiveDate,
    #[serde(default)]
    reason: String,
    /// Optional explicit duration for a partial (half-day) leave, e.g. 0.5, 1.5.
    /// When omitted, the server counts whole business days in the range.
    #[serde(default)]
    days: Option<f64>,
    /// Which half a half-day covers: `"first"` | `"second"` (migration 0051). Only valid on a
    /// single-day request of exactly 0.5 days. Omitted by clients that predate the field.
    #[serde(default)]
    half_period: Option<String>,
}

async fn request_leave(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<NewLeave>,
) -> Result<Json<Value>, AppError> {
    let (id, days) = leave_service::submit_request(
        &state.db,
        user.id,
        body.leave_type_id,
        body.start_date,
        body.end_date,
        &body.reason,
        body.days,
        body.half_period.as_deref(),
    )
    .await?;
    audit::log(
        &state.db,
        user.id,
        "leave.request",
        "leave_request",
        Some(id),
    )
    .await;
    // FYI to any employee who manages this person (they see their people but don't approve).
    crate::leave_notify::spawn_notify(
        state.db.clone(),
        id,
        crate::leave_notify::LeaveEvent::Requested,
        None,
    );
    Ok(Json(json!({ "id": id, "days": days, "status": "pending" })))
}

async fn cancel_leave(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    if !leave::cancel(&state.db, id, user.id).await? {
        return Err(AppError::NotFound);
    }
    Ok(Json(json!({ "id": id, "status": "cancelled" })))
}

// ---- Approver (HR / project manager) ----

/// Ensure the caller may act on `target`'s request: HR anyone; PM only users
/// they manage (user_managers — an employee may have several managers).
async fn authorize_approver(
    state: &AppState,
    approver: &AuthUser,
    target: Uuid,
) -> Result<(), AppError> {
    if approver.role.at_least(UserRole::Hr) {
        return Ok(());
    }
    if users::is_manager_of(&state.db, approver.id, target).await? {
        Ok(())
    } else {
        Err(AppError::Forbidden)
    }
}

async fn pending_requests(
    State(state): State<AppState>,
    RequireTeamViewer(user): RequireTeamViewer,
) -> Result<Json<Value>, AppError> {
    // HR sees all; a project manager sees only their team's.
    let scope = if user.role.at_least(UserRole::Hr) {
        None
    } else {
        Some(user.id)
    };
    Ok(Json(json!(leave::list_pending(&state.db, scope).await?)))
}

#[derive(Deserialize)]
struct CalendarQuery {
    from: NaiveDate,
    to: NaiveDate,
}

/// `GET /admin/leave/calendar?from=YYYY-MM-DD&to=YYYY-MM-DD` — every approved/pending leave that
/// overlaps the window, for the month-register grid. HR sees everyone; a PM sees only their team
/// (same scope as the pending queue).
async fn leave_calendar(
    State(state): State<AppState>,
    RequireTeamViewer(user): RequireTeamViewer,
    Query(q): Query<CalendarQuery>,
) -> Result<Json<Value>, AppError> {
    if q.to < q.from {
        return Err(AppError::BadRequest("`to` is before `from`".into()));
    }
    let scope = if user.role.at_least(UserRole::Hr) {
        None
    } else {
        Some(user.id)
    };
    Ok(Json(json!(
        leave::list_in_range(&state.db, q.from, q.to, scope).await?
    )))
}

/// The widest window `/me/leave/out` serves: the dashboard asks for a few weeks; nobody needs a year of it.
const OUT_MAX_DAYS: i64 = 62;

/// `GET /me/leave/out?from=YYYY-MM-DD&to=YYYY-MM-DD` — who is off work in the window, for ANY signed-in user
/// (HRMS integration: the "On Leave" card on every home dashboard). Approved and pending leave, company-wide,
/// active people only. Same reasoning as `/me/holidays`: a team needs to know who is away on which day, so this
/// is name + days + status only — never the leave type or the reason, which stay with HR and the approver
/// (`/admin/leave/calendar`).
async fn who_is_out(
    State(state): State<AppState>,
    _user: AuthUser,
    Query(q): Query<CalendarQuery>,
) -> Result<Json<Value>, AppError> {
    if q.to < q.from {
        return Err(AppError::BadRequest("`to` is before `from`".into()));
    }
    if (q.to - q.from).num_days() > OUT_MAX_DAYS {
        return Err(AppError::BadRequest(format!(
            "the window is at most {OUT_MAX_DAYS} days"
        )));
    }
    Ok(Json(json!(
        leave::list_out_in_range(&state.db, q.from, q.to).await?
    )))
}

async fn decide_request(
    state: &AppState,
    approver: &AuthUser,
    id: Uuid,
    status: &str,
) -> Result<Json<Value>, AppError> {
    let (owner, current) = leave::owner_and_status(&state.db, id)
        .await?
        .ok_or(AppError::NotFound)?;
    authorize_approver(state, approver, owner).await?;
    if current != "pending" {
        return Err(AppError::BadRequest(format!(
            "request is already {current}"
        )));
    }
    let decided = if status == "approved" {
        match leave_service::approve(&state.db, id, approver.id).await {
            Ok(d) => d,
            Err(e) => {
                // A refused approval is a decision too: keep it on the audit trail.
                audit::log(
                    &state.db,
                    approver.id,
                    "leave.approve_refused",
                    "leave_request",
                    Some(id),
                )
                .await;
                return Err(e);
            }
        }
    } else {
        leave::decide(&state.db, id, status, approver.id).await?
    };
    if !decided {
        return Err(AppError::BadRequest("request is no longer pending".into()));
    }
    audit::log(
        &state.db,
        approver.id,
        &format!("leave.{status}"),
        "leave_request",
        Some(id),
    )
    .await;
    let event = if status == "approved" {
        crate::leave_notify::LeaveEvent::Approved
    } else {
        crate::leave_notify::LeaveEvent::Rejected
    };
    crate::leave_notify::spawn_notify(state.db.clone(), id, event, Some(approver.id));
    Ok(Json(json!({ "id": id, "status": status })))
}

async fn approve_request(
    State(state): State<AppState>,
    RequireTeamViewer(user): RequireTeamViewer,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    decide_request(&state, &user, id, "approved").await
}

async fn reject_request(
    State(state): State<AppState>,
    RequireTeamViewer(user): RequireTeamViewer,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    decide_request(&state, &user, id, "rejected").await
}

// ---- HR configuration ----

#[derive(Deserialize)]
struct NewType {
    name: String,
    #[serde(default = "default_true")]
    paid: bool,
    /// Default days for employees (also PMs/HR).
    #[serde(default)]
    default_days: f64,
    #[serde(default)]
    default_days_contractor: f64,
    #[serde(default)]
    default_days_intern: f64,
    /// Eligibility (migration 0050). Omitted = no rule (everyone, from day one, working days).
    #[serde(default)]
    eligible_gender: Option<String>,
    #[serde(default)]
    min_tenure_months: Option<i32>,
    #[serde(default)]
    day_basis: Option<String>,
    /// Who may take it (migration 0055). Omitted = everyone.
    #[serde(default)]
    employment_types: Option<Vec<EmploymentType>>,
}
fn default_true() -> bool {
    true
}

/// Deserialize a field so that an explicit JSON `null` is `Some(None)` and an omitted field (via
/// `#[serde(default)]`) is `None` — "set to everyone" vs "leave unchanged".
fn explicit_option<'de, D>(de: D) -> Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Some(Option::<String>::deserialize(de)?))
}

/// Who may take a type: at least one employment type, each once.
fn parse_employment_types(given: &[EmploymentType]) -> Result<Vec<EmploymentType>, AppError> {
    let mut out: Vec<EmploymentType> = Vec::new();
    for t in given {
        if !out.contains(t) {
            out.push(*t);
        }
    }
    if out.is_empty() {
        return Err(AppError::BadRequest(
            "a leave type must be open to at least one employment type".into(),
        ));
    }
    Ok(out)
}

/// Validate the eligibility fields of a create/update body into [`leave::TypeRules`].
/// `eligible_gender`: "male" | "female" | "" / "any" (= everyone).
fn parse_rules(
    gender: Option<&str>,
    tenure: Option<i32>,
    basis: Option<&str>,
) -> Result<leave::TypeRules, AppError> {
    let eligible_gender = match gender.map(|g| g.trim().to_ascii_lowercase()) {
        None => None,
        Some(g) if g.is_empty() || g == "any" => None,
        Some(g) if g == "male" || g == "female" => Some(g),
        Some(_) => {
            return Err(AppError::BadRequest(
                "eligible_gender must be male, female or any".into(),
            ))
        }
    };
    let min_tenure_months = tenure.unwrap_or(0);
    if !(0..=600).contains(&min_tenure_months) {
        return Err(AppError::BadRequest(
            "min_tenure_months must be between 0 and 600".into(),
        ));
    }
    let day_basis = basis.unwrap_or("working").trim().to_ascii_lowercase();
    if day_basis != "working" && day_basis != "calendar" {
        return Err(AppError::BadRequest(
            "day_basis must be working or calendar".into(),
        ));
    }
    Ok(leave::TypeRules {
        eligible_gender,
        min_tenure_months,
        day_basis,
    })
}

async fn create_type(
    State(state): State<AppState>,
    RequireHr(hr): RequireHr,
    Json(body): Json<NewType>,
) -> Result<Json<Value>, AppError> {
    if body.name.trim().is_empty() {
        return Err(AppError::BadRequest("name is required".into()));
    }
    let rules = parse_rules(
        body.eligible_gender.as_deref(),
        body.min_tenure_months,
        body.day_basis.as_deref(),
    )?;
    let types = match &body.employment_types {
        Some(given) => parse_employment_types(given)?,
        None => leave::all_employment_types(),
    };
    let t = leave::create_type_for(
        &state.db,
        body.name.trim(),
        body.paid,
        body.default_days,
        body.default_days_contractor,
        body.default_days_intern,
        &rules,
        &types,
    )
    .await?;
    audit::log(
        &state.db,
        hr.id,
        "leave.type.create",
        "leave_type",
        Some(t.id),
    )
    .await;
    Ok(Json(json!(t)))
}

#[derive(Deserialize)]
struct UpdateType {
    #[serde(default = "default_true")]
    paid: bool,
    #[serde(default)]
    default_days: f64,
    #[serde(default)]
    default_days_contractor: f64,
    #[serde(default)]
    default_days_intern: f64,
    /// Eligibility (migration 0050). Send ALL THREE to change the rule; omit all three to keep it — an
    /// older client that only edits days must not wipe a paternity/maternity rule. `eligible_gender` may be
    /// `null`, `""` or `"any"` for everyone (an explicit null counts as "sent", unlike an omitted field).
    #[serde(default, deserialize_with = "explicit_option")]
    eligible_gender: Option<Option<String>>,
    #[serde(default)]
    min_tenure_months: Option<i32>,
    #[serde(default)]
    day_basis: Option<String>,
    /// Who may take it (migration 0055). Omitted = unchanged.
    #[serde(default)]
    employment_types: Option<Vec<EmploymentType>>,
}

/// `PATCH /admin/leave/types/:id` (HR) — update a type's paid flag and its
/// per-category default allotments (and, optionally, its eligibility rule).
async fn update_type(
    State(state): State<AppState>,
    RequireHr(hr): RequireHr,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateType>,
) -> Result<Json<Value>, AppError> {
    let given = [
        body.eligible_gender.is_some(), // Some(None) = explicit null
        body.min_tenure_months.is_some(),
        body.day_basis.is_some(),
    ];
    let any_rule = given.iter().any(|g| *g);
    if any_rule && !given.iter().all(|g| *g) {
        // A partial rule would silently reset the missing fields to "everyone / day one / working".
        return Err(AppError::BadRequest(
            "send eligible_gender, min_tenure_months and day_basis together (or none of them)"
                .into(),
        ));
    }
    let rules = if any_rule {
        Some(parse_rules(
            Some(
                body.eligible_gender
                    .as_ref()
                    .and_then(|g| g.as_deref())
                    .unwrap_or(""),
            ),
            body.min_tenure_months,
            body.day_basis.as_deref(),
        )?)
    } else {
        None
    };
    let types = match &body.employment_types {
        Some(given) => Some(parse_employment_types(given)?),
        None => None,
    };
    let t = leave::update_type_for(
        &state.db,
        id,
        body.paid,
        body.default_days,
        body.default_days_contractor,
        body.default_days_intern,
        rules.as_ref(),
        types.as_deref(),
    )
    .await?
    .ok_or(AppError::NotFound)?;
    audit::log(
        &state.db,
        hr.id,
        "leave.type.update",
        "leave_type",
        Some(t.id),
    )
    .await;
    Ok(Json(json!(t)))
}

#[derive(Deserialize)]
struct NewAllocation {
    user_id: Uuid,
    leave_type_id: Uuid,
    year: Option<i32>,
    allotted_days: f64,
}

/// Changing your own leave balance — allotment or used/remaining — is a conflict of interest (and could undo
/// what another HR person recorded against you); only an admin, the tier above HR, may.
fn refuse_own_balance(hr: &crate::middleware::AuthUser, user_id: Uuid) -> Result<(), AppError> {
    if user_id == hr.id && hr.role != UserRole::Admin {
        return Err(AppError::BadRequest(
            "you can't change your own leave balance — ask another HR person".into(),
        ));
    }
    Ok(())
}

/// HR can't give someone days of a type that isn't for their category (migration 0055): no Earned Leave
/// allocated to an intern. 0 days is always allowed (it changes nothing they can take).
async fn refuse_type_not_for(
    state: &AppState,
    user_id: Uuid,
    leave_type_id: Uuid,
    days: f64,
) -> Result<(), AppError> {
    if days <= 0.0 {
        return Ok(());
    }
    match leave::type_allowed_for(&state.db, user_id, leave_type_id).await? {
        None => Err(AppError::NotFound),
        Some((true, _)) => Ok(()),
        Some((false, category)) => Err(AppError::BadRequest(format!(
            "this leave type isn't available to {}",
            leave::category_plural(&category)
        ))),
    }
}

/// An allotment is a day count: finite, 0–366.
fn check_allotment(days: f64) -> Result<(), AppError> {
    if !days.is_finite() || !(0.0..=366.0).contains(&days) {
        return Err(AppError::BadRequest(
            "allotted days must be between 0 and 366".into(),
        ));
    }
    Ok(())
}

async fn allocate(
    State(state): State<AppState>,
    RequireHr(hr): RequireHr,
    Json(body): Json<NewAllocation>,
) -> Result<Json<Value>, AppError> {
    refuse_own_balance(&hr, body.user_id)?;
    check_allotment(body.allotted_days)?;
    refuse_type_not_for(&state, body.user_id, body.leave_type_id, body.allotted_days).await?;
    let year = body.year.unwrap_or_else(|| Utc::now().year());
    leave::upsert_allocation(
        &state.db,
        body.user_id,
        body.leave_type_id,
        year,
        body.allotted_days,
    )
    .await?;
    audit::log(
        &state.db,
        hr.id,
        "leave.allocate",
        "user",
        Some(body.user_id),
    )
    .await;
    Ok(Json(json!({
        "user_id": body.user_id, "leave_type_id": body.leave_type_id,
        "year": year, "allotted_days": body.allotted_days
    })))
}

#[derive(Deserialize)]
struct AdjustAllocation {
    user_id: Uuid,
    leave_type_id: Uuid,
    year: Option<i32>,
    /// Positive to increase, negative to decrease (result clamped at 0).
    delta: f64,
}

/// `POST /admin/leave/allocations/adjust` (HR) — increase or decrease a user's
/// allotment for a type by `delta` days, writing an explicit override.
async fn adjust_allocation(
    State(state): State<AppState>,
    RequireHr(hr): RequireHr,
    Json(body): Json<AdjustAllocation>,
) -> Result<Json<Value>, AppError> {
    refuse_own_balance(&hr, body.user_id)?;
    if !body.delta.is_finite() || body.delta.abs() > 366.0 {
        return Err(AppError::BadRequest(
            "delta must be between −366 and 366".into(),
        ));
    }
    refuse_type_not_for(&state, body.user_id, body.leave_type_id, body.delta).await?;
    let year = body.year.unwrap_or_else(|| Utc::now().year());
    let allotted = leave::adjust_allocation(
        &state.db,
        body.user_id,
        body.leave_type_id,
        year,
        body.delta,
    )
    .await?
    .ok_or(AppError::NotFound)?;
    audit::log(
        &state.db,
        hr.id,
        "leave.allocate.adjust",
        "user",
        Some(body.user_id),
    )
    .await;
    Ok(Json(json!({
        "user_id": body.user_id, "leave_type_id": body.leave_type_id,
        "year": year, "allotted_days": allotted
    })))
}

#[derive(Deserialize)]
struct SetUsed {
    user_id: Uuid,
    leave_type_id: Uuid,
    year: Option<i32>,
    /// The total days of this type the person has used in `year` (approved requests included).
    used_days: Option<f64>,
    /// …or the days they have LEFT of it in `year`…
    remaining_days: Option<f64>,
    /// …or days to ADD to what they have used (negative gives days back), applied to the balance as it is when
    /// the server holds the person's lock (migration 0057). Exactly one of the three.
    add_used_days: Option<f64>,
    /// Optional: makes the call safe to retry. A key already used for the same request replays (nothing changes,
    /// `"replayed": true`); one used for a different person, type, year or request is a 409.
    idempotency_key: Option<String>,
}

/// An idempotency key: 1–200 characters of `A–Z a–z 0–9 : _ -`.
fn check_idempotency_key(key: &str) -> Result<(), AppError> {
    let ok = (1..=200).contains(&key.len())
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b':' | b'_' | b'-'));
    if !ok {
        return Err(AppError::BadRequest(
            "idempotency_key must be 1–200 characters of letters, digits, ':', '_' or '-'".into(),
        ));
    }
    Ok(())
}

/// Which of `used_days`, `remaining_days` and `add_used_days` was sent, checked: exactly one, a whole or half
/// day, 0–366 for a total (−366–366 and not 0 for an add).
fn used_target(body: &SetUsed) -> Result<leave::UsedTarget, AppError> {
    let target = match (body.used_days, body.remaining_days, body.add_used_days) {
        (Some(d), None, None) => leave::UsedTarget::Used(d),
        (None, Some(d), None) => leave::UsedTarget::Remaining(d),
        (None, None, Some(d)) => leave::UsedTarget::Add(d),
        _ => {
            return Err(AppError::BadRequest(
                "send exactly one of used_days, remaining_days or add_used_days".into(),
            ))
        }
    };
    // Leave is counted in half days; a year has at most 366 of them.
    let half_days = |d: f64| d.is_finite() && (d * 2.0).fract() == 0.0;
    match target {
        leave::UsedTarget::Used(d) | leave::UsedTarget::Remaining(d)
            if !half_days(d) || !(0.0..=366.0).contains(&d) =>
        {
            let what = if matches!(target, leave::UsedTarget::Used(_)) {
                "used"
            } else {
                "remaining"
            };
            Err(AppError::BadRequest(format!(
                "{what} days must be between 0 and 366, in steps of 0.5"
            )))
        }
        leave::UsedTarget::Add(d)
            if !half_days(d) || d == 0.0 || !(-366.0..=366.0).contains(&d) =>
        {
            Err(AppError::BadRequest(
                "add_used_days must be between −366 and 366, not 0, in steps of 0.5".into(),
            ))
        }
        _ => Ok(target),
    }
}

/// `POST /admin/leave/used` (HR) — set how many days of a leave type someone has used, or has remaining, in a
/// year, or add to what they have used (migrations 0053, 0054, 0057). Either way the server records the days used
/// outside TimeTracker on top of the approved requests, so leave approved afterwards keeps lowering what's
/// remaining. Remaining can't be more than the allotment minus approved requests; used can't be more than the
/// allotment. An add is applied under the same per-person lock as leave approval, so concurrent adds and
/// approvals don't lose each other; an `idempotency_key` makes a retry replay instead of applying twice.
async fn set_used(
    State(state): State<AppState>,
    RequireHr(hr): RequireHr,
    Json(body): Json<SetUsed>,
) -> Result<Json<Value>, AppError> {
    refuse_own_balance(&hr, body.user_id)?;
    let year = body.year.unwrap_or_else(|| Utc::now().year());
    if !(2000..=2100).contains(&year) {
        return Err(AppError::BadRequest("year is out of range".into()));
    }
    let target = used_target(&body)?;
    if let Some(key) = &body.idempotency_key {
        check_idempotency_key(key)?;
    }
    let outcome = leave::set_used_keyed(
        &state.db,
        body.user_id,
        body.leave_type_id,
        year,
        target,
        hr.id,
        body.idempotency_key.as_deref(),
    )
    .await?;
    let (allotted, approved, adjustment, used, remaining, replayed) = match outcome {
        leave::SetUsed::NotFound => return Err(AppError::NotFound),
        leave::SetUsed::KeyConflict => {
            return Err(AppError::Conflict(
                "idempotency_key was already used for a different person, leave type, year or request".into(),
            ))
        }
        leave::SetUsed::BelowApproved { allotted, approved } => {
            return Err(AppError::BadRequest(match target {
                leave::UsedTarget::Remaining(_) if approved > allotted => format!(
                    "approved leave requests already use {approved} day(s), more than the {allotted} allotted, so \
                     nothing can remain — raise the allotment first (0 clears days recorded by HR)"
                ),
                leave::UsedTarget::Remaining(_) => format!(
                    "approved leave requests already use {approved} of the {allotted} day(s) allotted, so at most \
                     {} can remain — raise the allotment to give more",
                    allotted - approved
                ),
                leave::UsedTarget::Used(_) => format!(
                    "{approved} day(s) already come from approved leave requests this year, so used can't be \
                     lower — cancel or reject those requests instead"
                ),
                leave::UsedTarget::Add(_) => format!(
                    "{approved} day(s) already come from approved leave requests this year, so used can't go \
                     below that — only days recorded by HR can be given back; cancel or reject requests instead"
                ),
            }))
        }
        leave::SetUsed::AboveAllotment { allotted } => {
            return Err(AppError::BadRequest(format!(
                "used can't be more than the {allotted} day(s) allotted for this type — raise the allotment first"
            )))
        }
        leave::SetUsed::Replayed { allotted, approved, adjustment, used, remaining } => {
            (allotted, approved, adjustment, used, remaining, true)
        }
        leave::SetUsed::Saved { allotted, approved, adjustment, used, remaining, changed } => {
            if changed {
                audit::log(&state.db, hr.id, "leave.used.set", "user", Some(body.user_id)).await;
            }
            (allotted, approved, adjustment, used, remaining, false)
        }
    };
    Ok(Json(json!({
        "user_id": body.user_id, "leave_type_id": body.leave_type_id, "year": year,
        "allotted_days": allotted, "used_days": used, "remaining_days": remaining,
        "approved_days": approved, "used_adjustment": adjustment, "replayed": replayed
    })))
}

#[derive(Deserialize)]
struct DeleteAllocation {
    user_id: Uuid,
    leave_type_id: Uuid,
    year: Option<i32>,
}

/// `DELETE /admin/leave/allocations` (HR) — remove a user's explicit override,
/// reverting the balance to their category default.
async fn delete_allocation(
    State(state): State<AppState>,
    RequireHr(hr): RequireHr,
    Json(body): Json<DeleteAllocation>,
) -> Result<Json<Value>, AppError> {
    refuse_own_balance(&hr, body.user_id)?;
    let year = body.year.unwrap_or_else(|| Utc::now().year());
    if !leave::delete_allocation(&state.db, body.user_id, body.leave_type_id, year).await? {
        return Err(AppError::NotFound);
    }
    audit::log(
        &state.db,
        hr.id,
        "leave.allocate.delete",
        "user",
        Some(body.user_id),
    )
    .await;
    Ok(Json(json!({ "deleted": true })))
}

/// `GET /admin/users/:id/leave/balance?year=` (HR) — a specific user's per-type
/// balances (effective allotments with override flags), for the allocation UI.
async fn user_balance(
    State(state): State<AppState>,
    RequireHr(_hr): RequireHr,
    Path(target): Path<Uuid>,
    Query(q): Query<YearQuery>,
) -> Result<Json<Value>, AppError> {
    if users::find_by_id(&state.db, target).await?.is_none() {
        return Err(AppError::NotFound);
    }
    let year = q.year.unwrap_or_else(|| Utc::now().year());
    Ok(Json(json!({
        "year": year,
        "balances": leave::balances(&state.db, target, year).await?,
    })))
}

async fn list_holidays(
    State(state): State<AppState>,
    RequireStaff(_u): RequireStaff,
    Query(q): Query<YearQuery>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(json!(leave::list_holidays(&state.db, q.year).await?)))
}

/// `GET /me/holidays?year=` — the company holiday calendar for ANY signed-in
/// user (HRMS integration: the employee home dashboard shows upcoming holidays).
///
/// Same `[{id, day, name}]` payload as the admin route. Holidays are company-wide
/// and carry no personal data, so `AuthUser` is the right guard — this is a read
/// of a published calendar, not a view onto anyone's record. Writes stay HR-only
/// on `POST /admin/holidays`; the admin GET is left in place so existing HR
/// surfaces keep working unchanged.
async fn my_holidays(
    State(state): State<AppState>,
    _user: AuthUser,
    Query(q): Query<YearQuery>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(json!(leave::list_holidays(&state.db, q.year).await?)))
}

#[derive(Deserialize)]
struct NewHoliday {
    day: NaiveDate,
    name: String,
}

async fn create_holiday(
    State(state): State<AppState>,
    RequireHr(hr): RequireHr,
    Json(body): Json<NewHoliday>,
) -> Result<Json<Value>, AppError> {
    let h = leave::create_holiday(&state.db, body.day, body.name.trim()).await?;
    audit::log(&state.db, hr.id, "holiday.create", "holiday", Some(h.id)).await;
    // Holidays are never a work day: re-derive a day (and an already-computed week) that was rolled up
    // before HR entered it. In the background, so the HR request isn't held up by a week's recompute.
    let refresh = state.clone();
    let day = body.day;
    tokio::spawn(async move {
        crate::weekly_hours_service::refresh_for_new_holiday(&refresh, day).await;
    });
    Ok(Json(json!(h)))
}

pub fn router() -> Router<AppState> {
    Router::new()
        // Employee self-service
        .route("/me/leave/types", get(my_types))
        .route("/me/leave/balance", get(my_balance))
        .route("/me/leave/requests", get(my_requests).post(request_leave))
        .route("/me/leave/requests/:id/cancel", post(cancel_leave))
        .route("/me/leave/out", get(who_is_out))
        // Approver
        .route("/admin/leave/requests", get(pending_requests))
        .route("/admin/leave/calendar", get(leave_calendar))
        .route("/admin/leave/requests/:id/approve", post(approve_request))
        .route("/admin/leave/requests/:id/reject", post(reject_request))
        // HR configuration
        .route("/admin/leave/types", post(create_type))
        .route("/admin/leave/types/:id", axum::routing::patch(update_type))
        .route(
            "/admin/leave/allocations",
            post(allocate).delete(delete_allocation),
        )
        .route("/admin/leave/allocations/adjust", post(adjust_allocation))
        .route("/admin/leave/used", post(set_used))
        .route("/admin/users/:id/leave/balance", get(user_balance))
        .route("/admin/holidays", get(list_holidays).post(create_holiday))
        .route("/me/holidays", get(my_holidays))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(used: Option<f64>, remaining: Option<f64>, add: Option<f64>) -> SetUsed {
        SetUsed {
            user_id: Uuid::nil(),
            leave_type_id: Uuid::nil(),
            year: None,
            used_days: used,
            remaining_days: remaining,
            add_used_days: add,
            idempotency_key: None,
        }
    }

    #[test]
    fn exactly_one_target_is_accepted() {
        assert!(matches!(
            used_target(&body(Some(3.5), None, None)),
            Ok(leave::UsedTarget::Used(d)) if d == 3.5
        ));
        assert!(matches!(
            used_target(&body(None, Some(4.0), None)),
            Ok(leave::UsedTarget::Remaining(d)) if d == 4.0
        ));
        assert!(matches!(
            used_target(&body(None, None, Some(-0.5))),
            Ok(leave::UsedTarget::Add(d)) if d == -0.5
        ));
        for b in [
            body(None, None, None),
            body(Some(1.0), Some(1.0), None),
            body(Some(1.0), None, Some(0.5)),
            body(None, Some(1.0), Some(0.5)),
            body(Some(1.0), Some(1.0), Some(0.5)),
        ] {
            assert!(used_target(&b).is_err());
        }
    }

    #[test]
    fn add_must_be_a_nonzero_half_day_step_within_a_year() {
        for ok in [0.5, -0.5, 1.0, 366.0, -366.0] {
            assert!(used_target(&body(None, None, Some(ok))).is_ok(), "{ok}");
        }
        for bad in [
            0.0,
            -0.0,
            0.25,
            1.3,
            366.5,
            -366.5,
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
        ] {
            assert!(used_target(&body(None, None, Some(bad))).is_err(), "{bad}");
        }
        // Totals keep their 0–366 range: negative is still refused.
        assert!(used_target(&body(Some(-0.5), None, None)).is_err());
        assert!(used_target(&body(None, Some(0.0), None)).is_ok());
    }

    #[test]
    fn idempotency_keys_are_short_and_plain() {
        for ok in [
            "a",
            "hrms:short-day:2033-03-07:42",
            "A_b-C:9",
            &"k".repeat(200),
        ] {
            assert!(check_idempotency_key(ok).is_ok(), "{ok}");
        }
        for bad in ["", " ", "a b", "a/b", "ü", "a\n", &"k".repeat(201)] {
            assert!(check_idempotency_key(bad).is_err(), "{bad:?}");
        }
    }
}

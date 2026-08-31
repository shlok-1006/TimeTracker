//! Employee directory routes — the onboarding form's data, read back.
//!
//! RBAC is the access matrix from the RUH HRMS "Employee & Teams" proposal,
//! enforced here rather than in the UI:
//!
//!   own personal details      employee (`GET /me/directory/profile`)
//!   team members' details     PM, only where they are the manager
//!   everyone's details        HR
//!   edit / verify             HR only — the form is the employee's claim until
//!                             HR checks it, after which it is the single truth
//!   bank details              HR only, separate routes (see below)
//!
//! The sealed tier is deliberately its own endpoint rather than a field on the
//! profile response. Nothing that merely lists people can leak an account
//! number, and a PM cannot reach it at all — `RequireHr`, not `RequireStaff`.

use axum::{
    extract::{Path, Query, State},
    routing::{get, post},
    Json, Router,
};
use chrono::{Datelike, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use std::str::FromStr;

use crate::auth;
use crate::db::employee_directory::CelebrationSource;
use crate::db::{audit, employee_directory as repo, users};
use crate::employment_type::EmploymentType;
use crate::error::AppError;
use crate::middleware::{AuthUser, RequireHr, RequireStaff};
use crate::role::UserRole;
use crate::routes::admin::{authorize_view, team_scope};
use crate::state::AppState;

/// India is UTC+5:30; the team's "today" for a celebrations reminder is the IST day.
const IST_OFFSET: Duration = Duration::minutes(330);
/// Default reminder window (the Claude design's "7-day reminder").
const DEFAULT_CELEBRATION_DAYS: i64 = 7;
/// A sane upper bound so `?days=` can't ask us to walk a decade.
const MAX_CELEBRATION_DAYS: i64 = 62;

/// One upcoming celebration, shaped for the HRMS "Upcoming events" card.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Celebration {
    /// "birthday" or "anniversary".
    pub kind: String,
    pub name: String,
    /// The upcoming occurrence as `YYYY-MM-DD` (IST calendar).
    pub date: String,
    /// Anniversaries only: completed years this occurrence marks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub years: Option<i32>,
}

/// Birthdays and work anniversaries falling within `[today, today + days]`, matched on
/// MONTH-DAY so a 1994-07-28 birthday recurs every year. Sorted by date. Pure (takes
/// `today`) so the window logic is testable without a clock.
///
/// The birth YEAR is never emitted — only the upcoming month-day — so the feed can be
/// company-wide without broadcasting anyone's age. A joining year IS used, but only to
/// count completed years, and the join date itself (year 0) is not an anniversary.
fn upcoming_celebrations(
    sources: &[CelebrationSource],
    today: NaiveDate,
    days: i64,
) -> Vec<Celebration> {
    let mut out = Vec::new();
    for i in 0..=days {
        let Some(d) = today.checked_add_signed(Duration::days(i)) else {
            break;
        };
        let (mm, dd) = (d.month(), d.day());
        for s in sources {
            if let Some(dob) = s.date_of_birth {
                if dob.month() == mm && dob.day() == dd {
                    out.push(Celebration {
                        kind: "birthday".into(),
                        name: s.name.clone(),
                        date: d.format("%Y-%m-%d").to_string(),
                        years: None,
                    });
                }
            }
            if let Some(joined) = s.joined_on {
                if joined.month() == mm && joined.day() == dd {
                    let years = d.year() - joined.year();
                    if years > 0 {
                        out.push(Celebration {
                            kind: "anniversary".into(),
                            name: s.name.clone(),
                            date: d.format("%Y-%m-%d").to_string(),
                            years: Some(years),
                        });
                    }
                }
            }
        }
    }
    out
}

#[derive(Deserialize)]
struct CelebrationQuery {
    days: Option<i64>,
}

fn clamp_days(days: Option<i64>) -> i64 {
    days.unwrap_or(DEFAULT_CELEBRATION_DAYS)
        .clamp(0, MAX_CELEBRATION_DAYS)
}

/// `GET /me/celebrations?days=7` — the same upcoming birthdays and work anniversaries, for
/// EVERY authenticated user (employees included), so the celebrations card can live on their
/// dashboard too. Company-wide on purpose: celebrations are a shared, social feature, and the
/// response carries only names and the month-day of the occurrence — never a birth year, never
/// any other personal field — so it exposes nothing the org roster wouldn't.
async fn my_celebrations(
    State(state): State<AppState>,
    _user: AuthUser,
    Query(q): Query<CelebrationQuery>,
) -> Result<Json<Value>, AppError> {
    let days = clamp_days(q.days);
    let today = (Utc::now() + IST_OFFSET).date_naive();
    let sources = repo::celebration_sources(&state.db, None).await?;
    let events = upcoming_celebrations(&sources, today, days);
    Ok(Json(
        json!({ "days": days, "from": today, "celebrations": events }),
    ))
}

/// `GET /admin/directory/celebrations?days=7` — upcoming birthdays and work anniversaries
/// from the onboarding form's dates, for the celebrations card. HR sees everyone; a PM sees
/// only the people they manage. `days` defaults to 7 and is clamped to a sane range.
async fn celebrations(
    State(state): State<AppState>,
    RequireStaff(user): RequireStaff,
    Query(q): Query<CelebrationQuery>,
) -> Result<Json<Value>, AppError> {
    let days = clamp_days(q.days);
    let today = (Utc::now() + IST_OFFSET).date_naive();
    let sources = repo::celebration_sources(&state.db, team_scope(&user)).await?;
    let events = upcoming_celebrations(&sources, today, days);
    Ok(Json(
        json!({ "days": days, "from": today, "celebrations": events }),
    ))
}

/// `GET /me/directory/profile` — the caller's own record. Employees have no
/// other way to see what the onboarding form recorded about them.
async fn my_profile(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<Value>, AppError> {
    let bundle = repo::get_bundle(&state.db, user.id).await?;
    Ok(Json(json!({ "profile": bundle })))
}

/// `GET /admin/directory` — the roster. HR sees everyone; a PM sees only the
/// people they manage.
async fn directory(
    State(state): State<AppState>,
    RequireStaff(user): RequireStaff,
) -> Result<Json<Value>, AppError> {
    let people = repo::list_directory(&state.db, team_scope(&user)).await?;
    Ok(Json(json!({ "people": people })))
}

/// `GET /admin/directory/:id` — one person's full tier-2 record.
async fn user_profile(
    State(state): State<AppState>,
    RequireStaff(user): RequireStaff,
    Path(target): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    authorize_view(&state, &user, target).await?;
    let bundle = repo::get_bundle(&state.db, target).await?;
    Ok(Json(json!({ "profile": bundle })))
}

#[derive(Deserialize)]
struct ProfileUpdate {
    // Tier-1 employment facts (live on the core row).
    employee_code: Option<String>,
    department: Option<String>,
    designation: Option<String>,
    joined_on: Option<NaiveDate>,
    // Tier-2 personal details.
    #[serde(default)]
    profile: Option<repo::EmployeeProfile>,
    #[serde(default)]
    education: Option<Vec<repo::Education>>,
    #[serde(default)]
    prev_employment: Option<Vec<repo::PrevEmployment>>,
}

/// `PUT /admin/directory/:id` — HR corrects or completes a record.
///
/// HR-only by design: a PM can read their team's details but must not be able
/// to rewrite someone's date of birth or address. Each list field is optional —
/// omitting `education` leaves it alone, sending `[]` clears it.
async fn update_profile(
    State(state): State<AppState>,
    RequireHr(user): RequireHr,
    Path(target): Path<Uuid>,
    Json(body): Json<ProfileUpdate>,
) -> Result<Json<Value>, AppError> {
    repo::set_employment_facts(
        &state.db,
        target,
        body.employee_code.as_deref(),
        body.department.as_deref(),
        body.designation.as_deref(),
        body.joined_on,
    )
    .await?;
    if let Some(p) = &body.profile {
        repo::upsert_profile(&state.db, target, p).await?;
    }
    if let Some(rows) = &body.education {
        repo::replace_education(&state.db, target, rows).await?;
    }
    if let Some(rows) = &body.prev_employment {
        repo::replace_prev_employment(&state.db, target, rows).await?;
    }
    audit::log(
        &state.db,
        user.id,
        "employee_profile.update",
        "user",
        Some(target),
    )
    .await;
    let bundle = repo::get_bundle(&state.db, target).await?;
    Ok(Json(json!({ "profile": bundle })))
}

/// The onboarding "create employee" payload. Identity is required; everything else mirrors the
/// `PUT` shape so the HRMS can send the whole onboarding form in one call. Sealed bank details are
/// accepted here too (routed to the audited tier-3 path), but are optional.
#[derive(Deserialize)]
struct CreateEmployee {
    // ── Identity (required) ──
    name: String,
    email: String,
    /// The HRMS Razorpay ID — the upsert key. Stored as `employee_code` (globally unique).
    #[serde(default)]
    employee_code: Option<String>,
    // ── Account (optional) ──
    /// Defaults to "employee". Only an admin may create an admin.
    #[serde(default)]
    role: Option<String>,
    /// Defaults to "employee" (vs contractor / intern).
    #[serde(default)]
    employment_type: Option<String>,
    /// If omitted on a NEW hire, a temp password is generated and emailed; returned once here too.
    #[serde(default)]
    password: Option<String>,
    #[serde(default)]
    manager_id: Option<Uuid>,
    // ── Employment facts (COALESCE — only what's sent overwrites) ──
    #[serde(default)]
    department: Option<String>,
    #[serde(default)]
    designation: Option<String>,
    #[serde(default)]
    joined_on: Option<NaiveDate>,
    // ── Tier-2 + lists (same as PUT; each replaces the stored value when present) ──
    #[serde(default)]
    profile: Option<repo::EmployeeProfile>,
    #[serde(default)]
    education: Option<Vec<repo::Education>>,
    #[serde(default)]
    prev_employment: Option<Vec<repo::PrevEmployment>>,
    // ── Sealed tier (optional; audited, never echoed in the roster) ──
    #[serde(default)]
    bank: Option<repo::BankDetails>,
}

/// `POST /admin/directory` — create a new employee, or upsert if one already matches. HR/admin.
///
/// This is the onboarding hand-off: a brand-new hire lands in the directory and immediately flows
/// into attendance, leave, live status and (if code-tracked) performance like everyone else.
///
/// Keying (idempotent): resolves an existing person by `employee_code` (Razorpay ID) first, then
/// work email. A match UPDATES that row (and reactivates it if the person had left — a re-hire);
/// no match CREATES the account, provisions sign-in (welcome email + temp password) and applies
/// the profile. Re-sending is therefore safe — it never duplicates.
async fn create_employee(
    State(state): State<AppState>,
    RequireHr(hr): RequireHr,
    Json(body): Json<CreateEmployee>,
) -> Result<Json<Value>, AppError> {
    if body.name.trim().is_empty() {
        return Err(AppError::BadRequest("name is required".into()));
    }
    if !body.email.contains('@') {
        return Err(AppError::BadRequest("a valid work email is required".into()));
    }
    let role = match body.role.as_deref() {
        None | Some("") => UserRole::Employee,
        Some(r) => UserRole::from_str(r).map_err(|_| {
            AppError::BadRequest("role must be employee, project_manager, hr or admin".into())
        })?,
    };
    // Only an admin may mint another admin (same rule as POST /admin/users).
    if role == UserRole::Admin && hr.role != UserRole::Admin {
        return Err(AppError::Forbidden);
    }
    let employment_type = match body.employment_type.as_deref() {
        None | Some("") => EmploymentType::Employee,
        Some(e) => EmploymentType::from_str(e).map_err(|_| {
            AppError::BadRequest("employment type must be employee, contractor or intern".into())
        })?,
    };

    let existing =
        users::find_for_directory_upsert(&state.db, body.employee_code.as_deref(), &body.email)
            .await?;

    let (user_id, created, reactivated, temp_password) = match existing {
        Some((id, was_deactivated)) => {
            // Existing person: never silently rewrite their name/email/role/password — only the
            // employment facts + profile below. A re-hire (they had left) is brought back.
            let re = if was_deactivated {
                users::reactivate(&state.db, id).await?
            } else {
                false
            };
            (id, false, re, None)
        }
        None => {
            let (password, generated) = match body.password {
                Some(ref p) if p.len() >= 8 => (p.clone(), false),
                Some(_) => {
                    return Err(AppError::BadRequest(
                        "password must be at least 8 characters".into(),
                    ))
                }
                None => (auth::generate_temp_password(), true),
            };
            let hash = auth::hash_password(&password).map_err(AppError::Internal)?;
            let user = users::create(
                &state.db,
                body.name.trim(),
                body.email.trim(),
                &hash,
                role,
                body.manager_id,
            )
            .await?;
            if employment_type != EmploymentType::Employee {
                users::set_employment_type(&state.db, user.id, employment_type).await?;
            }
            // Best-effort welcome email (credentials + desktop download) — a mail failure must
            // never block onboarding, so it is only logged (identical to POST /admin/users).
            let download_url = std::env::var("DESKTOP_DOWNLOAD_URL").unwrap_or_else(|_| {
                "https://github.com/shlok-1006/TimeTracker/releases/latest".to_string()
            });
            let setup_guide_url = std::env::var("SETUP_GUIDE_URL").ok().filter(|s| !s.is_empty());
            let server_url = std::env::var("DESKTOP_SERVER_URL").unwrap_or_default();
            if let Err(e) = crate::email_service::send_welcome(crate::email_service::WelcomeEmail {
                email: &user.email,
                name: &user.name,
                temp_password: &password,
                download_url: &download_url,
                setup_guide_url: setup_guide_url.as_deref(),
                server_url: &server_url,
            })
            .await
            {
                tracing::warn!(email = %user.email, "welcome email failed: {e}");
            }
            (user.id, true, false, generated.then_some(password))
        }
    };

    // Apply employment facts + the profile bundle (create and update alike). COALESCE on the facts
    // so an omitted field never blanks an existing value.
    repo::merge_employment_facts(
        &state.db,
        user_id,
        body.employee_code.as_deref(),
        body.department.as_deref(),
        body.designation.as_deref(),
        body.joined_on,
    )
    .await?;
    if let Some(p) = &body.profile {
        repo::upsert_profile(&state.db, user_id, p).await?;
    }
    if let Some(rows) = &body.education {
        repo::replace_education(&state.db, user_id, rows).await?;
    }
    if let Some(rows) = &body.prev_employment {
        repo::replace_prev_employment(&state.db, user_id, rows).await?;
    }
    if let Some(b) = &body.bank {
        repo::upsert_bank(&state.db, user_id, b).await?;
        audit::log(&state.db, hr.id, "employee_bank.update", "user", Some(user_id)).await;
    }

    audit::log(
        &state.db,
        hr.id,
        if created {
            "employee.create"
        } else {
            "employee.upsert"
        },
        "user",
        Some(user_id),
    )
    .await;

    let bundle = repo::get_bundle(&state.db, user_id).await?;
    Ok(Json(json!({
        "user_id": user_id,
        "created": created,
        "reactivated": reactivated,
        // Present only when the server generated a password for a NEW hire — show it once.
        "temp_password": temp_password,
        "profile": bundle,
    })))
}

/// `POST /admin/directory/:id/verify` — HR confirms the form's answers are
/// checked. Audited: "who said this data is true" is exactly the kind of claim
/// that needs a name against it.
async fn verify_profile(
    State(state): State<AppState>,
    RequireHr(user): RequireHr,
    Path(target): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    repo::mark_verified(&state.db, target, user.id).await?;
    audit::log(
        &state.db,
        user.id,
        "employee_profile.verify",
        "user",
        Some(target),
    )
    .await;
    let bundle = repo::get_bundle(&state.db, target).await?;
    Ok(Json(json!({ "profile": bundle })))
}

/// `GET /admin/directory/:id/bank` — sealed tier. HR only, and every read is
/// audited: unlike the rest of the record, merely *looking* at bank details is
/// an event worth being able to reconstruct later.
async fn bank(
    State(state): State<AppState>,
    RequireHr(user): RequireHr,
    Path(target): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    let details = repo::get_bank(&state.db, target).await?;
    audit::log(
        &state.db,
        user.id,
        "employee_bank.view",
        "user",
        Some(target),
    )
    .await;
    Ok(Json(json!({ "bank": details })))
}

/// `PUT /admin/directory/:id/bank` — sealed tier, HR only, audited.
async fn set_bank(
    State(state): State<AppState>,
    RequireHr(user): RequireHr,
    Path(target): Path<Uuid>,
    Json(body): Json<repo::BankDetails>,
) -> Result<Json<Value>, AppError> {
    repo::upsert_bank(&state.db, target, &body).await?;
    audit::log(
        &state.db,
        user.id,
        "employee_bank.update",
        "user",
        Some(target),
    )
    .await;
    Ok(Json(json!({ "ok": true })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn src(name: &str, dob: Option<NaiveDate>, joined: Option<NaiveDate>) -> CelebrationSource {
        CelebrationSource {
            name: name.into(),
            date_of_birth: dob,
            joined_on: joined,
        }
    }

    #[test]
    fn a_birthday_inside_the_window_shows_with_this_years_date_and_no_year() {
        let today = ymd(2026, 7, 25);
        let people = [src("Asha", Some(ymd(1994, 7, 28)), None)];
        let got = upcoming_celebrations(&people, today, 7);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].kind, "birthday");
        assert_eq!(got[0].date, "2026-07-28", "recurs on this year's month-day");
        assert!(got[0].years.is_none(), "a birthday must not leak the birth year");
    }

    #[test]
    fn an_anniversary_reports_completed_years() {
        let today = ymd(2026, 8, 14);
        let people = [src("Ben", None, Some(ymd(2022, 8, 16)))];
        let got = upcoming_celebrations(&people, today, 7);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].kind, "anniversary");
        assert_eq!(got[0].date, "2026-08-16");
        assert_eq!(got[0].years, Some(4));
    }

    #[test]
    fn the_joining_day_itself_is_not_an_anniversary() {
        // Someone who joins today has completed zero years — no celebration.
        let today = ymd(2026, 8, 16);
        let people = [src("Newbie", None, Some(ymd(2026, 8, 16)))];
        assert!(upcoming_celebrations(&people, today, 7).is_empty());
    }

    #[test]
    fn dates_outside_the_window_are_excluded_and_today_is_included() {
        let today = ymd(2026, 7, 25);
        let people = [
            src("EdgeIn", Some(ymd(1990, 8, 1)), None),  // +7 days, inside
            src("EdgeOut", Some(ymd(1990, 8, 2)), None), // +8 days, outside a 7-day window
            src("Today", Some(ymd(1990, 7, 25)), None),  // day 0, inside
        ];
        let got = upcoming_celebrations(&people, today, 7);
        let names: Vec<&str> = got.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"EdgeIn"));
        assert!(names.contains(&"Today"));
        assert!(!names.contains(&"EdgeOut"), "the 8th day is past a 7-day reminder");
    }

    #[test]
    fn the_window_wraps_across_a_year_boundary() {
        // Late December looking forward into January must still match.
        let today = ymd(2026, 12, 30);
        let people = [src("NewYear", Some(ymd(1988, 1, 2)), Some(ymd(2020, 1, 2)))];
        let got = upcoming_celebrations(&people, today, 7);
        assert_eq!(got.len(), 2, "both a birthday and an anniversary on 2 Jan");
        assert!(got.iter().all(|c| c.date == "2027-01-02"), "dated in the next year");
    }

    #[test]
    fn results_are_sorted_by_date() {
        let today = ymd(2026, 7, 25);
        let people = [
            src("Later", Some(ymd(1990, 7, 30)), None),
            src("Sooner", Some(ymd(1990, 7, 26)), None),
        ];
        let got = upcoming_celebrations(&people, today, 7);
        assert_eq!(got[0].name, "Sooner");
        assert_eq!(got[1].name, "Later");
    }
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/me/directory/profile", get(my_profile))
        .route("/me/celebrations", get(my_celebrations))
        .route("/admin/directory", get(directory).post(create_employee))
        // Static segment registered before `/:id` so "celebrations" is never parsed as a UUID.
        .route("/admin/directory/celebrations", get(celebrations))
        .route(
            "/admin/directory/:id",
            get(user_profile).put(update_profile),
        )
        .route("/admin/directory/:id/verify", post(verify_profile))
        .route("/admin/directory/:id/bank", get(bank).put(set_bank))
}

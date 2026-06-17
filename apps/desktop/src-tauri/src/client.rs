//! Dashboard data proxies. The webview never sees tokens — these fetch via the
//! authenticated `http` helper (which transparently refreshes on expiry).

use serde_json::Value;

use crate::http;

/// Append `?day=YYYY-MM-DD` when a day is supplied (server defaults to today).
fn with_day(base: &str, day: Option<String>) -> String {
    match day {
        Some(d) if !d.is_empty() => format!("{base}?day={d}"),
        _ => base.to_string(),
    }
}

/// `GET /me/hours` — server-side summary (for reconciliation).
#[tauri::command]
pub async fn me_hours() -> Result<Value, String> {
    http::get_json("/me/hours").await
}

/// `GET /me/screenshots?day=` — own screenshots for a day (verdict + presigned URL).
#[tauri::command]
pub async fn me_screenshots(day: Option<String>) -> Result<Value, String> {
    http::get_json(&with_day("/me/screenshots", day)).await
}

/// `GET /me/report?day=` — own daily AI work-verification report.
#[tauri::command]
pub async fn me_report(day: Option<String>) -> Result<Value, String> {
    http::get_json(&with_day("/me/report", day)).await
}

/// `GET /me/teams` — the employee's teams (for the pre-timer dropdown).
#[tauri::command]
pub async fn me_teams() -> Result<Value, String> {
    http::get_json("/me/teams").await
}

/// `GET /me/team-options` — all teams the employee can choose from.
#[tauri::command]
pub async fn me_team_options() -> Result<Value, String> {
    http::get_json("/me/team-options").await
}

/// `POST /me/teams/:id/join` — join a team (self-service).
#[tauri::command]
pub async fn join_team(team_id: String) -> Result<Value, String> {
    http::post_json(&format!("/me/teams/{team_id}/join"), serde_json::json!({})).await
}

/// `POST /me/teams/:id/leave` — leave a team (self-service).
#[tauri::command]
pub async fn leave_team(team_id: String) -> Result<Value, String> {
    http::post_json(&format!("/me/teams/{team_id}/leave"), serde_json::json!({})).await
}

/// `GET /me/tickets` — assigned Linear tickets.
#[tauri::command]
pub async fn me_tickets() -> Result<Value, String> {
    http::get_json("/me/tickets").await
}

/// `GET /me/tasks` — HR/PM-assigned manual tasks for the employee.
#[tauri::command]
pub async fn me_tasks() -> Result<Value, String> {
    http::get_json("/me/tasks").await
}

/// `GET /me/attendance?from=&to=` — own derived attendance calendar (Feature 6C).
#[tauri::command]
pub async fn me_attendance(from: String, to: String) -> Result<Value, String> {
    http::get_json(&format!("/me/attendance?from={from}&to={to}")).await
}

// ---- Leave self-service (Feature 6B) ----

/// `GET /me/leave/types` — leave types the employee can request.
#[tauri::command]
pub async fn me_leave_types() -> Result<Value, String> {
    http::get_json("/me/leave/types").await
}

/// `GET /me/leave/balance` — per-type allotted/used/remaining for the year.
#[tauri::command]
pub async fn me_leave_balance() -> Result<Value, String> {
    http::get_json("/me/leave/balance").await
}

/// `GET /me/leave/requests` — the employee's own leave requests + statuses.
#[tauri::command]
pub async fn me_leave_requests() -> Result<Value, String> {
    http::get_json("/me/leave/requests").await
}

/// `POST /me/leave/requests` — apply for leave. Dates are `YYYY-MM-DD`.
#[tauri::command]
pub async fn request_leave(
    leave_type_id: String,
    start_date: String,
    end_date: String,
    reason: String,
) -> Result<Value, String> {
    http::post_json(
        "/me/leave/requests",
        serde_json::json!({
            "leave_type_id": leave_type_id,
            "start_date": start_date,
            "end_date": end_date,
            "reason": reason,
        }),
    )
    .await
}

/// `POST /me/leave/requests/:id/cancel` — cancel a still-pending request.
#[tauri::command]
pub async fn cancel_leave(id: String) -> Result<Value, String> {
    http::post_json(&format!("/me/leave/requests/{id}/cancel"), serde_json::json!({})).await
}

/// `GET /me/tickets/requests` — manual ticket access requests + statuses.
#[tauri::command]
pub async fn my_ticket_requests() -> Result<Value, String> {
    http::get_json("/me/tickets/requests").await
}

/// `POST /me/tickets/request` — request access to a ticket by id/identifier.
#[tauri::command]
pub async fn request_ticket(ticket: String) -> Result<Value, String> {
    http::post_json("/me/tickets/request", serde_json::json!({ "ticket": ticket })).await
}

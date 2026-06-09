//! Dashboard data proxies. The webview never sees tokens — these fetch via the
//! authenticated `http` helper (which transparently refreshes on expiry).

use serde_json::Value;

use crate::http;

/// `GET /me/hours` — server-side summary (for reconciliation).
#[tauri::command]
pub async fn me_hours() -> Result<Value, String> {
    http::get_json("/me/hours").await
}

/// `GET /me/screenshots` — own screenshots with presigned view URLs.
#[tauri::command]
pub async fn me_screenshots() -> Result<Value, String> {
    http::get_json("/me/screenshots").await
}

/// `GET /me/tickets` — assigned Linear tickets.
#[tauri::command]
pub async fn me_tickets() -> Result<Value, String> {
    http::get_json("/me/tickets").await
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

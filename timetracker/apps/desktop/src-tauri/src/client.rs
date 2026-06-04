//! Authenticated proxy to the API for the dashboard. The JWT lives in the OS
//! keychain (Rust side) and is never exposed to the webview; these commands
//! fetch on the frontend's behalf and return parsed JSON.

use serde_json::Value;

use crate::auth;

fn api_base() -> String {
    std::env::var("TIMETRACKER_API_BASE_URL").unwrap_or_else(|_| "http://localhost:8090".to_string())
}

async fn get_json(path: &str) -> Result<Value, String> {
    let token = auth::stored_token().ok_or_else(|| "not authenticated".to_string())?;
    let resp = reqwest::Client::new()
        .get(format!("{}{}", api_base(), path))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| format!("could not reach the server: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("server returned {}", resp.status()));
    }
    resp.json::<Value>().await.map_err(|e| e.to_string())
}

/// `GET /me/hours` — server-side summary (for reconciliation).
#[tauri::command]
pub async fn me_hours() -> Result<Value, String> {
    get_json("/me/hours").await
}

/// `GET /me/screenshots` — own screenshots with presigned view URLs.
#[tauri::command]
pub async fn me_screenshots() -> Result<Value, String> {
    get_json("/me/screenshots").await
}

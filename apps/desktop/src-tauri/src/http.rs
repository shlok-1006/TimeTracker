//! Authenticated HTTP to the API with transparent token refresh.
//!
//! Adds the bearer access token; on `401` it rotates the refresh token once
//! (`auth::do_refresh`) and retries. Keeps tokens out of the webview entirely.

use std::sync::OnceLock;

use reqwest::StatusCode;
use serde_json::Value;
use tokio::sync::Mutex;

use crate::auth;

/// Serializes refresh-token rotation across concurrent requests. Without it,
/// several requests hitting 401 at the same time each rotate the *same* refresh
/// token; the server's reuse-detection then treats the consumed token as a
/// stolen-token replay and revokes ALL sessions, forcing the user to log in
/// again. Single-flight: exactly one task refreshes; the others wait and reuse
/// the freshly-rotated token.
fn refresh_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

async fn send(
    method: reqwest::Method,
    path: &str,
    token: &str,
    body: Option<&Value>,
) -> Result<reqwest::Response, String> {
    let url = format!("{}{}", auth::api_base(), path);
    let mut req = reqwest::Client::new()
        .request(method, url)
        .bearer_auth(token);
    if let Some(b) = body {
        req = req.json(b);
    }
    req.send().await.map_err(|e| format!("request failed: {e}"))
}

/// Perform an authenticated request, refreshing once on 401.
async fn authed(
    method: reqwest::Method,
    path: &str,
    body: Option<Value>,
) -> Result<reqwest::Response, String> {
    let token = auth::stored_access().ok_or_else(|| "not authenticated".to_string())?;
    let resp = send(method.clone(), path, &token, body.as_ref()).await?;

    if resp.status() == StatusCode::UNAUTHORIZED {
        // Access token likely expired — refresh once (single-flight) and retry.
        {
            let _guard = refresh_lock().lock().await;
            // Another task may have rotated the token while we waited for the
            // lock. Only refresh if the stored token is still the one we just
            // used with a 401 — otherwise reuse the token that task obtained.
            if auth::stored_access().as_deref() == Some(token.as_str()) {
                auth::do_refresh().await?;
            }
        }
        let token = auth::stored_access().ok_or_else(|| "not authenticated".to_string())?;
        return send(method, path, &token, body.as_ref()).await;
    }
    Ok(resp)
}

async fn parse_json(resp: reqwest::Response) -> Result<Value, String> {
    if !resp.status().is_success() {
        return Err(format!("server returned {}", resp.status()));
    }
    resp.json::<Value>().await.map_err(|e| e.to_string())
}

pub async fn get_json(path: &str) -> Result<Value, String> {
    parse_json(authed(reqwest::Method::GET, path, None).await?).await
}

pub async fn post_json(path: &str, body: Value) -> Result<Value, String> {
    parse_json(authed(reqwest::Method::POST, path, Some(body)).await?).await
}

//! Authenticated HTTP to the API with transparent token refresh.
//!
//! Adds the bearer access token; on `401` it rotates the refresh token once
//! (`auth::do_refresh`) and retries. Keeps tokens out of the webview entirely.

use reqwest::StatusCode;
use serde_json::Value;

use crate::auth;

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
        // Access token likely expired — rotate and retry once.
        auth::do_refresh().await?;
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

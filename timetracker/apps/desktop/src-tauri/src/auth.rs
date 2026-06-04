//! Desktop authentication: log in against the API, enforce employee-only access,
//! and persist the JWT in the OS keychain (Rule 6 — never plaintext on disk).
//!
//! The keychain stores ONLY the access token. The user id is provided by the
//! frontend (from the login response) or restored from `GET /me`, so the timer
//! never has to decode the JWT or juggle multiple keychain entries.

use keyring::Entry;
use serde::{Deserialize, Serialize};

const KEYRING_SERVICE: &str = "com.timetracker.desktop";
const KEYRING_ACCOUNT: &str = "access_token";

fn api_base() -> String {
    std::env::var("TIMETRACKER_API_BASE_URL").unwrap_or_else(|_| "http://localhost:8090".to_string())
}

fn entry() -> Result<Entry, String> {
    Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT).map_err(|e| format!("keychain error: {e}"))
}

// ---- Wire types (mirror the server's auth DTOs) ----

#[derive(Serialize)]
struct LoginBody<'a> {
    email: &'a str,
    password: &'a str,
}

#[derive(Deserialize)]
struct ApiUser {
    id: String,
    name: String,
    email: String,
    role: String,
    #[allow(dead_code)]
    team: Option<String>,
}

#[derive(Deserialize)]
struct ApiLoginResponse {
    access_token: String,
    user: ApiUser,
}

/// Session info handed back to the UI.
#[derive(Serialize)]
pub struct EmployeeSession {
    pub id: String,
    pub name: String,
    pub email: String,
    pub role: String,
}

/// Log in. Only `employee` accounts are accepted; any other role is rejected
/// (and no token is stored).
#[tauri::command]
pub async fn login(email: String, password: String) -> Result<EmployeeSession, String> {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/auth/login", api_base()))
        .json(&LoginBody {
            email: &email,
            password: &password,
        })
        .send()
        .await
        .map_err(|e| format!("could not reach the server: {e}"))?;

    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err("Invalid email or password.".to_string());
    }
    if !resp.status().is_success() {
        return Err(format!("login failed (status {})", resp.status()));
    }

    let body: ApiLoginResponse = resp
        .json()
        .await
        .map_err(|e| format!("unexpected server response: {e}"))?;

    if body.user.role != "employee" {
        return Err("This application is for employees only.".to_string());
    }

    entry()?
        .set_password(&body.access_token)
        .map_err(|e| format!("failed to store token: {e}"))?;

    Ok(EmployeeSession {
        id: body.user.id,
        name: body.user.name,
        email: body.user.email,
        role: body.user.role,
    })
}

#[derive(Deserialize)]
struct MeResponse {
    id: String,
    role: String,
}

/// Restore a session on launch: if a token is stored, ask the server who we are.
/// Returns `None` if there is no token or it is no longer valid.
#[tauri::command]
pub async fn restore_session() -> Result<Option<EmployeeSession>, String> {
    let token = match stored_token() {
        Some(t) => t,
        None => return Ok(None),
    };

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{}/me", api_base()))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| format!("could not reach the server: {e}"))?;

    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        // Token expired/invalid — clear it.
        let _ = logout();
        return Ok(None);
    }
    if !resp.status().is_success() {
        return Err(format!("status {}", resp.status()));
    }

    let me: MeResponse = resp
        .json()
        .await
        .map_err(|e| format!("unexpected server response: {e}"))?;

    Ok(Some(EmployeeSession {
        id: me.id,
        name: "Employee".to_string(),
        email: String::new(),
        role: me.role,
    }))
}

/// Clear the stored token (logout).
#[tauri::command]
pub fn logout() -> Result<(), String> {
    match entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(format!("keychain error: {e}")),
    }
}

/// Read the stored access token (used by the sync worker).
pub fn stored_token() -> Option<String> {
    match entry().ok()?.get_password() {
        Ok(t) => Some(t),
        Err(_) => None,
    }
}

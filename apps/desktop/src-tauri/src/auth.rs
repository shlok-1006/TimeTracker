//! Desktop authentication: log in against the API, enforce employee-only access,
//! and persist tokens in the OS keychain (Rule 6 — never plaintext on disk).
//!
//! Stores BOTH the short-lived access token and the long-lived refresh token.
//! `http.rs` transparently rotates them when the access token expires.

use keyring::Entry;
use serde::{Deserialize, Serialize};

const KEYRING_SERVICE: &str = "com.timetracker.desktop";
const ACCOUNT_ACCESS: &str = "access_token";
const ACCOUNT_REFRESH: &str = "refresh_token";

pub fn api_base() -> String {
    // Resolution order:
    //   1. runtime env var  -> lets a dev point a local build at any server
    //   2. compile-time env  -> the production URL baked in by CI at build time
    //                           (`option_env!` reads the value present during `cargo build`)
    //   3. localhost default -> plain `cargo run` / `tauri dev`
    std::env::var("TIMETRACKER_API_BASE_URL")
        .ok()
        .or_else(|| option_env!("TIMETRACKER_API_BASE_URL").map(str::to_string))
        .unwrap_or_else(|| "http://localhost:9000".to_string())
}

fn entry(account: &str) -> Result<Entry, String> {
    Entry::new(KEYRING_SERVICE, account).map_err(|e| format!("keychain error: {e}"))
}

fn read(account: &str) -> Option<String> {
    entry(account).ok()?.get_password().ok()
}

fn write(account: &str, value: &str) -> Result<(), String> {
    entry(account)?
        .set_password(value)
        .map_err(|e| format!("keychain error: {e}"))
}

fn delete(account: &str) {
    if let Ok(e) = entry(account) {
        let _ = e.delete_credential();
    }
}

pub fn stored_access() -> Option<String> {
    read(ACCOUNT_ACCESS)
}
pub fn stored_refresh() -> Option<String> {
    read(ACCOUNT_REFRESH)
}

fn store_tokens(access: &str, refresh: &str) -> Result<(), String> {
    write(ACCOUNT_ACCESS, access)?;
    write(ACCOUNT_REFRESH, refresh)?;
    Ok(())
}

// ---- Wire types ----

#[derive(Serialize)]
struct LoginBody<'a> {
    email: &'a str,
    password: &'a str,
}

#[derive(Serialize)]
struct RefreshBody<'a> {
    refresh_token: &'a str,
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
    refresh_token: String,
    user: ApiUser,
}

#[derive(Deserialize)]
struct ApiTokenPair {
    access_token: String,
    refresh_token: String,
}

#[derive(Serialize)]
pub struct EmployeeSession {
    pub id: String,
    pub name: String,
    pub email: String,
    pub role: String,
}

/// Log in. Employees only; stores both tokens on success.
#[tauri::command]
pub async fn login(email: String, password: String) -> Result<EmployeeSession, String> {
    let resp = reqwest::Client::new()
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

    store_tokens(&body.access_token, &body.refresh_token)?;
    Ok(EmployeeSession {
        id: body.user.id,
        name: body.user.name,
        email: body.user.email,
        role: body.user.role,
    })
}

/// Rotate the refresh token for a fresh access token. Returns Err if there is
/// no refresh token or the server rejects it (user must log in again).
pub async fn do_refresh() -> Result<(), String> {
    let refresh = stored_refresh().ok_or_else(|| "no refresh token".to_string())?;
    let resp = reqwest::Client::new()
        .post(format!("{}/auth/refresh", api_base()))
        .json(&RefreshBody {
            refresh_token: &refresh,
        })
        .send()
        .await
        .map_err(|e| format!("refresh request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("refresh rejected ({})", resp.status()));
    }
    let pair: ApiTokenPair = resp.json().await.map_err(|e| e.to_string())?;
    store_tokens(&pair.access_token, &pair.refresh_token)
}

/// Restore a session on launch (verifies/refreshes the token via `/me`).
#[tauri::command]
pub async fn restore_session() -> Result<Option<EmployeeSession>, String> {
    if stored_access().is_none() {
        return Ok(None);
    }
    match crate::http::get_json("/me").await {
        Ok(v) => {
            let id = v
                .get("id")
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string();
            let role = v
                .get("role")
                .and_then(|x| x.as_str())
                .unwrap_or("employee")
                .to_string();
            if id.is_empty() {
                return Ok(None);
            }
            Ok(Some(EmployeeSession {
                id,
                name: "Employee".to_string(),
                email: String::new(),
                role,
            }))
        }
        Err(_) => Ok(None), // token invalid and refresh failed → treat as logged out
    }
}

/// Log out: revoke the refresh token server-side (best effort) and clear the keychain.
#[tauri::command]
pub async fn logout() -> Result<(), String> {
    if let Some(refresh) = stored_refresh() {
        let _ = reqwest::Client::new()
            .post(format!("{}/auth/logout", api_base()))
            .json(&RefreshBody {
                refresh_token: &refresh,
            })
            .send()
            .await;
    }
    delete(ACCOUNT_ACCESS);
    delete(ACCOUNT_REFRESH);
    Ok(())
}

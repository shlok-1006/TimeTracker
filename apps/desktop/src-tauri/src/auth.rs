//! Desktop authentication: log in against the API and persist tokens in the OS
//! keychain (Rule 6 — never plaintext on disk). All roles may log in and track
//! their time here (employee, project_manager, hr) — role only affects who can
//! use the admin dashboard, not the tracker.
//!
//! Stores BOTH the short-lived access token and the long-lived refresh token.
//! `http.rs` transparently rotates them when the access token expires.

use keyring::Entry;
use serde::{Deserialize, Serialize};

/// Keychain service name. Debug builds use a separate entry so that running the
/// app locally cannot read, refresh or overwrite the session belonging to the
/// installed release on the same machine — signing in during development would
/// otherwise rotate the real user's refresh token out from under them.
const KEYRING_SERVICE: &str = if cfg!(debug_assertions) {
    "com.timetracker.desktop.dev"
} else {
    "com.timetracker.desktop"
};
const ACCOUNT_ACCESS: &str = "access_token";
const ACCOUNT_REFRESH: &str = "refresh_token";
/// Cached user profile (id/name/email/role) so a session can be restored on
/// launch even when the server is momentarily unreachable (offline / VPN not up).
const ACCOUNT_PROFILE: &str = "profile";
/// Last-used credentials (email + password), kept ONLY in the OS keychain
/// (Rule 6 — never plaintext on disk) so the login screen can pre-fill them.
/// This makes re-signing in a single click once the long-lived (90-day) refresh
/// token finally expires or is revoked. Cleared on a deliberate "Sign out", but
/// kept when a session merely expires so the user isn't asked to retype anything.
const ACCOUNT_CREDENTIALS: &str = "credentials";

/// Default API base URL, resolved at compile time. **Release** builds (the
/// installers employees run) point at the hosted backend so a fresh install
/// works with no configuration; **debug** builds (`tauri dev`) point at a local
/// server. Either is overridable at runtime with `TIMETRACKER_API_BASE_URL`
/// (e.g. `http://localhost:9000` for local dev against a release build).
///
/// The base includes the `/api` prefix: Nginx routes `/api/*` to the server and
/// strips the prefix, so `{base}/auth/login` reaches the server's `/auth/login`.
const DEFAULT_API_BASE: &str = if cfg!(debug_assertions) {
    "http://localhost:9000"
} else {
    "https://time-tracker.rapidinnovation.dev/api"
};

pub fn api_base() -> String {
    let base = std::env::var("TIMETRACKER_API_BASE_URL")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_API_BASE.to_string());
    // Trim any trailing slash so `{base}/path` never doubles up.
    base.trim_end_matches('/').to_string()
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

/// Cache the signed-in user's profile so `restore_session` can bring them back
/// even when `/me` can't be reached at launch. Best-effort.
fn store_profile(s: &EmployeeSession) {
    if let Ok(json) = serde_json::to_string(s) {
        let _ = write(ACCOUNT_PROFILE, &json);
    }
}

fn stored_profile() -> Option<EmployeeSession> {
    serde_json::from_str(&read(ACCOUNT_PROFILE)?).ok()
}

/// Remember the last-used credentials so the login form can pre-fill them.
/// Best-effort; stored in the OS keychain, never a plaintext file (Rule 6).
fn store_credentials(email: &str, password: &str) {
    if let Ok(json) = serde_json::to_string(&SavedCredentials {
        email: email.to_string(),
        password: password.to_string(),
    }) {
        let _ = write(ACCOUNT_CREDENTIALS, &json);
    }
}

/// The remembered credentials for pre-filling the login form, if any. Returns
/// `None` on a fresh install or after a deliberate sign-out.
#[tauri::command]
pub fn saved_credentials() -> Option<SavedCredentials> {
    serde_json::from_str(&read(ACCOUNT_CREDENTIALS)?).ok()
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

#[derive(Serialize)]
struct ChangePasswordBody<'a> {
    email: &'a str,
    current_password: &'a str,
    new_password: &'a str,
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

#[derive(Serialize, Deserialize, Clone)]
pub struct EmployeeSession {
    pub id: String,
    pub name: String,
    pub email: String,
    pub role: String,
}

/// Credentials remembered for one-click re-login (kept in the OS keychain only).
#[derive(Serialize, Deserialize, Clone)]
pub struct SavedCredentials {
    pub email: String,
    pub password: String,
}

/// Log in (any role — HR and project managers can track their time too);
/// stores both tokens on success.
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

    store_tokens(&body.access_token, &body.refresh_token)?;
    let session = EmployeeSession {
        id: body.user.id,
        name: body.user.name,
        email: body.user.email,
        role: body.user.role,
    };
    store_profile(&session);
    // Remember for one-click re-login after the refresh token eventually dies.
    store_credentials(&session.email, &password);
    Ok(session)
}

/// Change the password (verifying the current one) and log in with the new one.
/// Any role; stores both tokens on success. Used from the login screen.
#[tauri::command]
pub async fn change_password(
    email: String,
    current_password: String,
    new_password: String,
) -> Result<EmployeeSession, String> {
    let resp = reqwest::Client::new()
        .post(format!("{}/auth/change-password", api_base()))
        .json(&ChangePasswordBody {
            email: &email,
            current_password: &current_password,
            new_password: &new_password,
        })
        .send()
        .await
        .map_err(|e| format!("could not reach the server: {e}"))?;

    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err("Current email or password is incorrect.".to_string());
    }
    if resp.status() == reqwest::StatusCode::BAD_REQUEST {
        // Surface the server's validation message (e.g. "at least 8 characters").
        let body = resp.text().await.unwrap_or_default();
        let detail = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
            .unwrap_or_else(|| "Invalid new password.".to_string());
        return Err(detail);
    }
    if !resp.status().is_success() {
        return Err(format!("password change failed (status {})", resp.status()));
    }

    let body: ApiLoginResponse = resp
        .json()
        .await
        .map_err(|e| format!("unexpected server response: {e}"))?;

    store_tokens(&body.access_token, &body.refresh_token)?;
    let session = EmployeeSession {
        id: body.user.id,
        name: body.user.name,
        email: body.user.email,
        role: body.user.role,
    };
    store_profile(&session);
    // The password just changed — remember the new one for one-click re-login.
    store_credentials(&session.email, &new_password);
    Ok(session)
}

/// Rotate the refresh token for a fresh access token. Returns Err if there is
/// no refresh token or the server rejects it (user must log in again).
///
/// On a definitive rejection (HTTP 401) the stored tokens are DELETED so we stop
/// re-presenting a dead token on every poll — otherwise the app's timers (hours,
/// activity, heartbeat, sync) hammer `/auth/refresh` with the same revoked token
/// forever, which the server logs as repeated "reuse detected" and which never
/// prompts the user to log back in. Transient failures (network, 5xx) keep the
/// tokens so a blip doesn't force a re-login.
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
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            // The refresh token is dead (rotated, revoked, or expired). Clear it
            // so we don't loop; the UI's session check will send the user to log
            // in again.
            delete(ACCOUNT_ACCESS);
            delete(ACCOUNT_REFRESH);
            delete(ACCOUNT_PROFILE);
        }
        return Err(format!("refresh rejected ({})", resp.status()));
    }
    let pair: ApiTokenPair = resp.json().await.map_err(|e| e.to_string())?;
    store_tokens(&pair.access_token, &pair.refresh_token)
}

/// Restore a session on launch. Keeps the user signed in across restarts for as
/// long as the (long-lived) refresh token survives — the short-lived access token
/// having expired overnight is fine, `http` refreshes it.
///
/// Crucially, a *transient* failure to reach `/me` at launch (offline, VPN not
/// connected yet, or the auth endpoint briefly rate-limited) must NOT force a
/// re-login — that was the "log in every morning" bug. We restore from the cached
/// profile and let the 60s `session_alive` check re-verify once the network is
/// back. Only a genuine token rejection (which clears the tokens) logs out.
#[tauri::command]
pub async fn restore_session() -> Result<Option<EmployeeSession>, String> {
    // The refresh token — not the 5-minute access token — is what determines
    // whether there is a session to restore.
    if stored_refresh().is_none() {
        return Ok(None);
    }
    match crate::http::get_json("/me").await {
        Ok(v) => {
            // Freshen id/role from the server; name/email come from the cached
            // profile (`/me` doesn't return them). Re-cache for next launch.
            let mut session = stored_profile().unwrap_or_else(|| EmployeeSession {
                id: String::new(),
                name: "Employee".to_string(),
                email: String::new(),
                role: "employee".to_string(),
            });
            if let Some(id) = v.get("id").and_then(|x| x.as_str()).filter(|s| !s.is_empty()) {
                session.id = id.to_string();
            }
            if let Some(role) = v.get("role").and_then(|x| x.as_str()) {
                session.role = role.to_string();
            }
            if session.id.is_empty() {
                return Ok(None);
            }
            store_profile(&session);
            Ok(Some(session))
        }
        Err(_) => {
            // If the refresh token was rejected, `do_refresh` deleted the tokens →
            // genuinely logged out. Otherwise the failure was transient: stay
            // signed in from the cached profile.
            if stored_refresh().is_none() {
                Ok(None)
            } else {
                Ok(stored_profile())
            }
        }
    }
}

/// Definitive session-health check for the UI's periodic re-verification.
///
/// `Ok(true)`  — authenticated (token valid, or refreshed successfully).
/// `Ok(false)` — signed out: the refresh token was rejected and the stored
///               tokens have been cleared. The UI must return to the login
///               screen so the user re-signs in and tracking resumes syncing.
/// `Err(_)`    — transient (network / server unreachable). The UI must NOT log
///               the user out on this — the session is presumed still valid.
///
/// This is what lets us catch a user who is "working" locally but whose session
/// has quietly died: instead of silently failing to sync, the app surfaces it.
#[tauri::command]
pub async fn session_alive() -> Result<bool, String> {
    if stored_refresh().is_none() {
        return Ok(false); // already signed out (no credentials at all)
    }
    match crate::http::get_json("/me").await {
        Ok(_) => Ok(true),
        // `http` refreshes on 401; a rejected refresh token makes `do_refresh`
        // delete the stored tokens. Tokens gone now => definitively signed out;
        // still present => the failure was transient, keep the session.
        Err(e) => {
            if stored_refresh().is_none() {
                Ok(false)
            } else {
                Err(e)
            }
        }
    }
}

/// Log out: revoke the refresh token server-side (best effort) and clear the
/// session from the keychain.
///
/// `forget` controls the remembered credentials:
///   * `false` — the session merely expired/was revoked. Keep the saved
///     credentials so the login screen pre-fills them and re-signing in is one
///     click (this is the "stay effortlessly logged in" path).
///   * `true`  — a deliberate "Sign out". Forget the credentials too, so the
///     next person sees an empty form.
#[tauri::command]
pub async fn logout(forget: bool) -> Result<(), String> {
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
    delete(ACCOUNT_PROFILE);
    if forget {
        delete(ACCOUNT_CREDENTIALS);
    }
    Ok(())
}

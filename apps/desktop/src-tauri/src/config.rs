//! Local app configuration, persisted to a small JSON file in the app data dir.
//!
//! Currently just the server URL — this lets a user point the app at any server
//! (entered once on the login screen) without rebuilding/redistributing. The
//! value takes precedence over the compile-time baked default (see
//! `auth::api_base`), so a shipped installer can be retargeted in the field.

use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};

use serde::{Deserialize, Serialize};

static CONFIG_PATH: OnceLock<PathBuf> = OnceLock::new();
static SERVER_URL: RwLock<Option<String>> = RwLock::new(None);

#[derive(Default, Serialize, Deserialize)]
struct ConfigFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    server_url: Option<String>,
}

/// Resolve the config path (creating the data dir if needed) and load any saved
/// server URL into memory. Call once at startup before workers spin up.
pub fn init(data_dir: &Path) {
    let _ = std::fs::create_dir_all(data_dir);
    let path = data_dir.join("config.json");
    if let Ok(contents) = std::fs::read_to_string(&path) {
        if let Ok(cfg) = serde_json::from_str::<ConfigFile>(&contents) {
            if let Some(url) = cfg.server_url.filter(|s| !s.trim().is_empty()) {
                *SERVER_URL.write().unwrap() = Some(url);
            }
        }
    }
    let _ = CONFIG_PATH.set(path);
}

/// The user-configured server URL, if one has been saved.
pub fn server_url() -> Option<String> {
    SERVER_URL.read().unwrap().clone()
}

/// Normalize a user-entered URL: trim, default to http:// when no scheme is
/// given, and drop a trailing slash so it concatenates cleanly with paths.
fn normalize(raw: &str) -> String {
    let trimmed = raw.trim();
    let with_scheme = if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else {
        format!("http://{trimmed}")
    };
    with_scheme.trim_end_matches('/').to_string()
}

/// Persist the server URL to disk and update the in-memory value. Returns the
/// normalized URL that was stored.
pub fn set_server_url_value(raw: &str) -> Result<String, String> {
    let normalized = normalize(raw);
    *SERVER_URL.write().unwrap() = Some(normalized.clone());
    if let Some(path) = CONFIG_PATH.get() {
        let cfg = ConfigFile {
            server_url: Some(normalized.clone()),
        };
        let json = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
        std::fs::write(path, json).map_err(|e| format!("failed to save config: {e}"))?;
    }
    Ok(normalized)
}

/// Return the saved server URL (for pre-filling the login screen).
#[tauri::command]
pub fn get_server_url() -> Option<String> {
    server_url()
}

/// Save the server URL the user entered; returns the normalized value.
#[tauri::command]
pub fn set_server_url(url: String) -> Result<String, String> {
    set_server_url_value(&url)
}

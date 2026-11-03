//! In-app updates.
//!
//! On launch and periodically the UI asks whether a newer release exists and,
//! if so, shows a banner. Its "Update now" runs [`install_update`]: download
//! the release's signed update package, install it over this copy, and restart
//! into the new version — no browser, no installer to click through. A running
//! timer is carried across the restart (`timer::suspend_for_update`).
//!
//! Releases (0.1.19 onwards) publish a `latest.json` next to the installers on
//! the public downloads repo; the Tauri updater reads it and refuses any package
//! not signed with the key whose public half is in tauri.conf.json. Where that
//! manifest can't be read (older releases, offline) the check falls back to the
//! downloads repo's GitHub API, and the banner then offers the manual download.
//!
//! Network / rate-limit failures return `Err`, which the UI treats as
//! "couldn't check" (no banner) rather than a false prompt.
//!
//! Windows: the MSI installs for all users, so Windows shows its own "allow
//! changes" prompt once; the installer then runs with a progress bar only
//! (`installMode: passive`) and relaunches the app.

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_updater::{Updater, UpdaterExt};

use crate::timer::DesktopState;

/// Latest published release of the public downloads repo (installers are
/// mirrored here from every release).
const LATEST_RELEASE_API: &str =
    "https://api.github.com/repos/shlok-1006/TimeTracker-Download/releases/latest";
/// Download progress, sent to the banner while an update downloads.
const PROGRESS_EVENT: &str = "update-progress";
/// The download finished and the install has begun.
const INSTALLING_EVENT: &str = "update-installing";

#[derive(Clone, Serialize)]
struct Progress {
    downloaded: u64,
    total: Option<u64>,
}

/// The updater, reading `TIMETRACKER_UPDATE_ENDPOINT` instead of the configured
/// manifest when set (for testing an update against a local server; packages
/// must still carry a valid signature).
fn updater(app: &AppHandle) -> Result<Updater, String> {
    let mut builder = app.updater_builder();
    if let Ok(url) = std::env::var("TIMETRACKER_UPDATE_ENDPOINT") {
        let url = url
            .parse()
            .map_err(|e| format!("bad update endpoint: {e}"))?;
        builder = builder.endpoints(vec![url]).map_err(|e| e.to_string())?;
    }
    builder.build().map_err(|e| e.to_string())
}

/// Human-facing downloads page — the fallback when an update can't install itself.
const DOWNLOADS_PAGE: &str = "https://github.com/shlok-1006/TimeTracker-Download/releases/latest";

/// Returns `Some(latest_version)` when a newer release is available, or `None`
/// when the running build is current. Returns `Err` on a network / parse
/// failure so the UI can silently ignore a check it couldn't complete (an
/// offline or rate-limited check must never turn into a false "update" prompt).
#[tauri::command]
pub async fn check_for_update(app: tauri::AppHandle) -> Result<Option<String>, String> {
    // The release manifest first: it is what "Update now" will install.
    match updater(&app)?.check().await {
        Ok(update) => return Ok(update.map(|u| u.version)),
        Err(e) => tracing::debug!("update manifest unavailable, asking the releases API: {e}"),
    }

    // The real installed version comes from tauri.conf.json, not the workspace
    // crate version (which `env!("CARGO_PKG_VERSION")` would give).
    let current = app.package_info().version.to_string();

    // The shared client sends a User-Agent, which GitHub's API requires.
    let resp = crate::http::client()
        .get(LATEST_RELEASE_API)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("update check failed: HTTP {}", resp.status()));
    }

    let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    let tag = body
        .get("tag_name")
        .and_then(|t| t.as_str())
        .ok_or("latest release has no tag_name")?;
    let latest = tag.trim_start_matches('v');

    Ok(if is_newer(latest, &current) {
        Some(latest.to_string())
    } else {
        None
    })
}

/// "Update now": download the newest release, install it over this copy and
/// restart into it. Only returns on failure (the UI then offers the manual
/// download); on success the process is replaced by the new version.
#[tauri::command]
pub async fn install_update(app: AppHandle, state: State<'_, DesktopState>) -> Result<(), String> {
    let update = updater(&app)?
        .check()
        .await
        .map_err(|e| format!("couldn't reach the update server: {e}"))?
        .ok_or("no automatic update is published for this computer yet")?;
    tracing::info!(from = %update.current_version, to = %update.version, "update: downloading");

    let mut downloaded = 0u64;
    let bytes = update
        .download(
            |chunk, total| {
                downloaded += chunk as u64;
                let _ = app.emit(PROGRESS_EVENT, Progress { downloaded, total });
            },
            || {},
        )
        .await
        .map_err(|e| format!("download failed: {e}"))?;
    let _ = app.emit(INSTALLING_EVENT, ());

    // From here the process may end at any moment (Windows exits inside
    // `install` to let the installer run), so save the session first.
    crate::timer::suspend_for_update(&state)
        .await
        .map_err(|e| format!("{e:#}"))?;
    if let Err(e) = update.install(&bytes) {
        tracing::warn!("update: install failed: {e}");
        crate::timer::resume_after_update(&state).await;
        return Err(format!("install failed: {e}"));
    }
    tracing::info!(version = %update.version, "update: installed, restarting");
    app.restart()
}

/// Open the public downloads page in the user's default browser.
#[tauri::command]
pub fn open_downloads_page() -> Result<(), String> {
    open_url(DOWNLOADS_PAGE)
}

/// Compare dotted numeric versions ("0.1.15" vs "0.1.14") component-by-component
/// so 14 sorts after 2 (a plain string compare would get that wrong). Avoids
/// pulling in a semver dependency. Non-numeric or missing components count as 0,
/// so a "-beta" suffix on the running build never reads as newer.
fn is_newer(latest: &str, current: &str) -> bool {
    fn parts(v: &str) -> Vec<u64> {
        v.split('.')
            .map(|p| {
                p.chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect::<String>()
                    .parse::<u64>()
                    .unwrap_or(0)
            })
            .collect()
    }
    parts(latest) > parts(current)
}

/// Open a URL with the platform's default handler.
fn open_url(url: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = std::process::Command::new("open");
        c.arg(url);
        c
    };
    #[cfg(target_os = "windows")]
    let mut cmd = {
        // `cmd /C start "" <url>` — the empty title argument stops a URL with
        // special characters from being consumed as the window title.
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", "start", "", url]);
        c
    };
    #[cfg(target_os = "linux")]
    let mut cmd = {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(url);
        c
    };
    cmd.spawn().map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::is_newer;

    #[test]
    fn detects_newer_patch_minor_major() {
        assert!(is_newer("0.1.15", "0.1.14"));
        assert!(is_newer("0.2.0", "0.1.14"));
        assert!(is_newer("1.0.0", "0.9.99"));
    }

    #[test]
    fn same_or_older_is_not_newer() {
        assert!(!is_newer("0.1.14", "0.1.14"));
        assert!(!is_newer("0.1.13", "0.1.14"));
        // 2 < 14 numerically — a string compare would wrongly call this newer.
        assert!(!is_newer("0.1.2", "0.1.14"));
    }

    #[test]
    fn tolerates_prerelease_suffix() {
        assert!(!is_newer("0.1.14-beta", "0.1.14"));
    }
}

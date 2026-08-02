//! In-app update check.
//!
//! The desktop app ships as manually-installed installers (there is no bundled
//! auto-updater — that would need its own updater signing key). Instead, on
//! launch and periodically, the UI compares the running version against the
//! latest published release and, when we're behind, shows a banner prompting
//! the user to download the new build.
//!
//! The latest version is read straight from the public downloads repo's GitHub
//! API. Network / rate-limit failures return `Err`, which the UI treats as
//! "couldn't check" (no banner) rather than a false prompt.

/// Latest published release of the public downloads repo (installers are
/// mirrored here from every release).
const LATEST_RELEASE_API: &str =
    "https://api.github.com/repos/shlok-1006/TimeTracker-Download/releases/latest";
/// Human-facing downloads page the "Update now" button opens.
const DOWNLOADS_PAGE: &str = "https://github.com/shlok-1006/TimeTracker-Download/releases/latest";

/// Returns `Some(latest_version)` when a newer release is available, or `None`
/// when the running build is current. Returns `Err` on a network / parse
/// failure so the UI can silently ignore a check it couldn't complete (an
/// offline or rate-limited check must never turn into a false "update" prompt).
#[tauri::command]
pub async fn check_for_update(app: tauri::AppHandle) -> Result<Option<String>, String> {
    // The real installed version comes from tauri.conf.json, not the workspace
    // crate version (which `env!("CARGO_PKG_VERSION")` would give).
    let current = app.package_info().version.to_string();

    let client = reqwest::Client::builder()
        // GitHub's API rejects requests without a User-Agent.
        .user_agent("TimeTracker-Desktop")
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
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

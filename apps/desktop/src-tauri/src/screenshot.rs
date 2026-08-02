//! Screenshot capture + upload (STEP 4) with token-refreshing API calls.
//!
//! Captures the primary monitor while **Working or in a Meeting** (never during
//! a break, idle, or when not tracking). Meeting shots are uploaded too, tagged
//! `meeting`, so they appear in the gallery — but the server never samples or
//! analyses them, so meeting time still can't skew the report. Each upload is
//! tagged with the capture-time status. Flow per Rule 5:
//!   1. POST /uploads/presign  -> presigned PUT URL + storage key
//!   2. PUT the JPEG bytes directly to storage (MinIO/R2)
//!   3. POST /screenshots      -> store metadata only
//!
//! Capture runs on a blocking thread; failures are logged and retried.

use std::sync::atomic::Ordering;
use std::time::Duration;

use chrono::Utc;
use image::{codecs::jpeg::JpegEncoder, DynamicImage, ExtendedColorType, RgbaImage};
use xcap::Monitor;

use crate::auth;
use crate::http;
use crate::presence::derive_status;
use crate::timer::DesktopState;

/// Upper bound (secs) of the randomized capture window: the next screenshot is
/// taken at a uniformly random point within the `[min, max]` window *after* the
/// last one. Defaults to a window bracketing 5 min ([150, 450]) so the cadence
/// is unpredictable while the *average* gap stays ~5 min (matching the old
/// fixed interval). Overridable via `TIMETRACKER_SCREENSHOT_INTERVAL_SECS`.
const DEFAULT_MAX_INTERVAL_SECS: u64 = 450;
/// Lower bound (secs) of the window — a floor so captures can't land
/// back-to-back. Overridable via `TIMETRACKER_SCREENSHOT_MIN_INTERVAL_SECS`.
const DEFAULT_MIN_INTERVAL_SECS: u64 = 150;
const JPEG_QUALITY: u8 = 70;

/// The `[min, max]` delay window (secs), read from env and clamped so
/// `min <= max`. `max` comes from `TIMETRACKER_SCREENSHOT_INTERVAL_SECS`.
fn interval_window() -> (u64, u64) {
    let max = std::env::var("TIMETRACKER_SCREENSHOT_INTERVAL_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|s| *s > 0)
        .unwrap_or(DEFAULT_MAX_INTERVAL_SECS);
    let min = std::env::var("TIMETRACKER_SCREENSHOT_MIN_INTERVAL_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(DEFAULT_MIN_INTERVAL_SECS)
        .min(max);
    (min, max)
}

/// Map a raw random value into an inclusive `[min, max]` seconds delay.
/// Pure (RNG is injected as `r`) so the windowing is unit-testable.
fn pick_delay_secs(min: u64, max: u64, r: u64) -> u64 {
    if max <= min {
        return min;
    }
    min + r % (max - min + 1)
}

/// A fresh randomized delay until the next capture attempt: uniform within the
/// `[min, max]` window. Because each interval is drawn independently, the
/// cadence never settles on a fixed clock an employee could game.
fn next_delay() -> Duration {
    let (min, max) = interval_window();
    Duration::from_secs(pick_delay_secs(min, max, rand::random::<u64>()))
}

/// Capture is allowed while actively **working** or **in a meeting**. Meeting
/// shots are uploaded (tagged `meeting`) so they show in the gallery, but the
/// server never samples or analyses them. Idle, break, and not-tracking are
/// excluded.
pub fn should_capture(status: &str) -> bool {
    status == "working" || status == "meeting"
}

/// Encode an RGBA frame as JPEG bytes.
pub fn encode_jpeg(img: &RgbaImage, quality: u8) -> anyhow::Result<Vec<u8>> {
    let rgb = DynamicImage::ImageRgba8(img.clone()).to_rgb8();
    let mut buf = Vec::new();
    JpegEncoder::new_with_quality(&mut buf, quality).encode(
        rgb.as_raw(),
        rgb.width(),
        rgb.height(),
        ExtendedColorType::Rgb8,
    )?;
    Ok(buf)
}

/// Capture the primary monitor as JPEG bytes (blocking).
pub fn capture_primary_jpeg(quality: u8) -> anyhow::Result<Vec<u8>> {
    let monitors = Monitor::all().map_err(|e| anyhow::anyhow!("enumerate monitors: {e}"))?;
    let monitor = monitors
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("no monitor found"))?;
    let frame = monitor
        .capture_image()
        .map_err(|e| anyhow::anyhow!("capture failed: {e}"))?;
    encode_jpeg(&frame, quality)
}

/// macOS Screen Recording (TCC) permission, asked the honest way. Without it,
/// capture still "succeeds" but silently returns only the wallpaper + our own
/// windows — so probing by capturing cannot detect the problem. CoreGraphics
/// exposes the real answer.
#[cfg(target_os = "macos")]
mod macos_permission {
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        // Boolean (unsigned char) in C — map as u8, compare against 0.
        fn CGPreflightScreenCaptureAccess() -> u8;
        fn CGRequestScreenCaptureAccess() -> u8;
    }

    /// Is Screen Recording permission currently granted?
    pub fn granted() -> bool {
        unsafe { CGPreflightScreenCaptureAccess() != 0 }
    }

    /// Ask macOS for the permission. Shows the system prompt the first time
    /// (afterwards the user must enable it in System Settings by hand).
    pub fn request() -> bool {
        unsafe { CGRequestScreenCaptureAccess() != 0 }
    }
}

/// Probe whether screen capture will produce a USEFUL image.
/// macOS: ask TCC directly — a capture "succeeding" proves nothing there (see
/// `macos_permission`). Elsewhere: probe by capturing (covers Wayland etc.).
#[tauri::command]
pub async fn check_capture() -> Result<bool, String> {
    #[cfg(target_os = "macos")]
    {
        if !macos_permission::granted() {
            return Ok(false);
        }
    }
    let res = tokio::task::spawn_blocking(|| capture_primary_jpeg(JPEG_QUALITY)).await;
    Ok(matches!(res, Ok(Ok(_))))
}

/// Trigger the OS permission flow (macOS: system prompt / Settings listing).
/// Returns the resulting permission state. No-op elsewhere.
#[tauri::command]
pub async fn request_capture_permission() -> Result<bool, String> {
    #[cfg(target_os = "macos")]
    {
        // `CGRequestScreenCaptureAccess` shows the native prompt only the FIRST
        // time; once the app has been decided it silently no-ops. So when we're
        // still not granted, also deep-link straight to the Screen Recording
        // pane so the user can toggle TimeTracker on.
        let granted = macos_permission::request();
        if !granted {
            let _ = std::process::Command::new("open")
                .arg(
                    "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture",
                )
                .spawn();
        }
        Ok(granted)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok(true)
    }
}

/// Relaunch the app. Needed after granting Screen Recording: macOS caches the
/// permission decision for the life of the process, so the running app keeps
/// seeing "denied" until it restarts. This lets the user apply the grant from
/// inside the app instead of manually quitting and reopening.
#[tauri::command]
pub fn relaunch_app(app: tauri::AppHandle) {
    app.restart();
}

/// One-time macOS repair for the unsigned→signed transition.
///
/// macOS ties a Screen Recording grant to the app's code signature. Machines
/// that ran an earlier UNSIGNED build kept a grant that doesn't apply to this
/// Developer ID–signed build — it can even still look "on" in the list while
/// capture stays blocked, so the user is stuck no matter how many times they
/// re-grant. This clears that stale entry once and re-registers the app with
/// TCC (so it reappears in the Screen Recording list under the new signature)
/// and the user's next grant sticks.
///
/// Safe by construction: runs only when permission is currently MISSING (a
/// fresh process reads the true TCC state, so there is no valid grant to lose)
/// and only ONCE per machine (marker file) — it never resets a working grant
/// or re-prompts on later launches. No-op off macOS and for fresh installs
/// (nothing to reset). Signed→signed updates from here never need this.
pub fn repair_stale_grant_if_needed(data_dir: &std::path::Path) {
    #[cfg(target_os = "macos")]
    {
        let marker = data_dir.join(".tcc_repaired");
        if marker.exists() || macos_permission::granted() {
            return;
        }
        let _ = std::process::Command::new("tccutil")
            .args(["reset", "ScreenCapture", "com.timetracker.desktop"])
            .status();
        let _ = std::fs::write(&marker, b"1");
        // Re-register with TCC so the app reappears in the Screen Recording list
        // (tied to the new signature) and the user gets a fresh prompt.
        let _ = macos_permission::request();
        tracing::info!("reset a stale Screen Recording grant from a prior build (one-time)");
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = data_dir; // no-op off macOS
    }
}

/// Background worker: after each randomized delay, capture + upload if Working
/// or in a Meeting.
pub async fn run(state: DesktopState) {
    let client = reqwest::Client::new();
    loop {
        tokio::time::sleep(next_delay()).await;

        let status = {
            let on_break = state.on_break.load(Ordering::Relaxed);
            let in_meeting = state.in_meeting.load(Ordering::Relaxed);
            let tracking = state.tracker.lock().await.is_some();
            derive_status(on_break, in_meeting, tracking, state.idle.is_idle())
        };

        if !should_capture(status) {
            continue;
        }
        // Without the TCC grant, capturing only yields wallpaper + our own
        // windows AND makes macOS re-show its permission prompt every cycle.
        // Skip entirely; the UI banner is the one allowed to ask.
        #[cfg(target_os = "macos")]
        {
            if !macos_permission::granted() {
                tracing::warn!(
                    "skipping screenshot: macOS Screen Recording permission not granted"
                );
                continue;
            }
        }
        // Tag the upload with the status that authorized the capture (Feature 2).
        if let Err(e) = capture_and_upload(&client, status).await {
            tracing::warn!("screenshot capture/upload failed (will retry): {e}");
        }
    }
}

async fn capture_and_upload(client: &reqwest::Client, status: &str) -> anyhow::Result<()> {
    if auth::stored_access().is_none() {
        return Ok(());
    }

    // 1. Presigned URL (server picks the namespaced key). Token auto-refreshes.
    let presign = http::post_json("/uploads/presign", serde_json::json!({}))
        .await
        .map_err(|e| anyhow::anyhow!(e))?;
    let url = presign
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing presign url"))?
        .to_string();
    let storage_key = presign
        .get("storage_key")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing storage_key"))?
        .to_string();

    // 2. Capture on a blocking thread (xcap is synchronous).
    let bytes = tokio::task::spawn_blocking(|| capture_primary_jpeg(JPEG_QUALITY)).await??;

    // 3. Upload bytes directly to storage (presigned — no auth header).
    let put = client
        .put(&url)
        .header("content-type", "image/jpeg")
        .body(bytes)
        .send()
        .await?;
    if !put.status().is_success() {
        anyhow::bail!("storage upload returned {}", put.status());
    }

    // 4. Notify the API (metadata only) with the capture-time status.
    http::post_json(
        "/screenshots",
        serde_json::json!({
            "storage_key": storage_key,
            "taken_at": Utc::now().to_rfc3339(),
            "captured_status": status,
        }),
    )
    .await
    .map_err(|e| anyhow::anyhow!(e))?;

    tracing::info!("screenshot uploaded: {}", storage_key);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_while_working_or_meeting() {
        assert!(should_capture("working"));
        assert!(should_capture("meeting")); // meeting shots are captured (tagged, never analysed)
        assert!(!should_capture("idle"));
        assert!(!should_capture("break"));
        assert!(!should_capture("not_working"));
    }

    #[test]
    fn encodes_jpeg_bytes() {
        let img = RgbaImage::from_pixel(8, 8, image::Rgba([10, 20, 30, 255]));
        let bytes = encode_jpeg(&img, 70).unwrap();
        assert!(bytes.len() > 2 && bytes[0] == 0xFF && bytes[1] == 0xD8);
    }

    #[test]
    fn delay_stays_within_window() {
        // Whatever the RNG yields, the delay is clamped to [min, max] inclusive.
        for r in [0u64, 1, 42, 299, 300, 301, 1_000_000, u64::MAX] {
            let d = pick_delay_secs(0, 300, r);
            assert!(d <= 300, "r={r} -> {d} exceeded max");
        }
    }

    #[test]
    fn delay_respects_a_nonzero_floor() {
        for r in [0u64, 5, 120, u64::MAX] {
            let d = pick_delay_secs(60, 300, r);
            assert!((60..=300).contains(&d), "r={r} -> {d} outside [60,300]");
        }
    }

    #[test]
    fn delay_endpoints_are_reachable() {
        // r == 0 gives the floor; the value just below the span width gives max.
        assert_eq!(pick_delay_secs(0, 300, 0), 0);
        assert_eq!(pick_delay_secs(0, 300, 300), 300);
        assert_eq!(pick_delay_secs(60, 300, 0), 60);
        assert_eq!(pick_delay_secs(60, 300, 240), 300);
    }

    #[test]
    fn degenerate_window_is_fixed() {
        // min == max (and the guard for max < min) collapses to a fixed delay.
        assert_eq!(pick_delay_secs(300, 300, 12345), 300);
        assert_eq!(pick_delay_secs(300, 100, 999), 300);
    }
}

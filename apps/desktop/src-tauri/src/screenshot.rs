//! Screenshot capture + upload (STEP 4) with token-refreshing API calls.
//!
//! Captures the primary monitor while **Working or in a Meeting** (never on
//! break, idle, or when not tracking). Each upload is tagged with the
//! capture-time status (Feature 2): meeting screenshots are stored and
//! viewable but excluded from AI sampling/analysis. Flow per Rule 5:
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

/// Capture is allowed while actively working or in a meeting. Meeting shots
/// are tagged `meeting` and excluded from AI analysis server-side (Feature 2).
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

/// Probe whether screen capture works (screen-recording permission granted).
/// Used by the UI to warn on macOS/Wayland when permission is missing.
#[tauri::command]
pub async fn check_capture() -> Result<bool, String> {
    let res = tokio::task::spawn_blocking(|| capture_primary_jpeg(JPEG_QUALITY)).await;
    Ok(matches!(res, Ok(Ok(_))))
}

/// Background worker: after each randomized delay, capture + upload if Working.
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
        assert!(should_capture("meeting"));
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

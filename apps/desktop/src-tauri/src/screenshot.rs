//! Screenshot capture + upload (STEP 4) with token-refreshing API calls.
//!
//! Captures the primary monitor **only while Working** (never on break, idle,
//! meeting, or when not tracking). Flow per Rule 5:
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

const DEFAULT_INTERVAL_SECS: u64 = 300; // 5 minutes
const JPEG_QUALITY: u8 = 70;

fn interval() -> Duration {
    let secs = std::env::var("TIMETRACKER_SCREENSHOT_INTERVAL_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|s| *s > 0)
        .unwrap_or(DEFAULT_INTERVAL_SECS);
    Duration::from_secs(secs)
}

/// Capture is allowed only while actively working.
pub fn should_capture(status: &str) -> bool {
    status == "working"
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

/// Background worker: every interval, capture + upload if Working.
pub async fn run(state: DesktopState) {
    let client = reqwest::Client::new();
    loop {
        tokio::time::sleep(interval()).await;

        let status = {
            let on_break = state.on_break.load(Ordering::Relaxed);
            let in_meeting = state.in_meeting.load(Ordering::Relaxed);
            let tracking = state.tracker.lock().await.is_some();
            derive_status(on_break, in_meeting, tracking, state.idle.is_idle())
        };

        if !should_capture(status) {
            continue;
        }
        if let Err(e) = capture_and_upload(&client).await {
            tracing::warn!("screenshot capture/upload failed (will retry): {e}");
        }
    }
}

async fn capture_and_upload(client: &reqwest::Client) -> anyhow::Result<()> {
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

    // 4. Notify the API (metadata only).
    http::post_json(
        "/screenshots",
        serde_json::json!({ "storage_key": storage_key, "taken_at": Utc::now().to_rfc3339() }),
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
    fn captures_only_while_working() {
        assert!(should_capture("working"));
        assert!(!should_capture("idle"));
        assert!(!should_capture("break"));
        assert!(!should_capture("meeting"));
        assert!(!should_capture("not_working"));
    }

    #[test]
    fn encodes_jpeg_bytes() {
        let img = RgbaImage::from_pixel(8, 8, image::Rgba([10, 20, 30, 255]));
        let bytes = encode_jpeg(&img, 70).unwrap();
        assert!(bytes.len() > 2 && bytes[0] == 0xFF && bytes[1] == 0xD8);
    }
}

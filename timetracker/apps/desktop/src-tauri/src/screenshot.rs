//! Screenshot capture + upload (STEP 4).
//!
//! Captures the primary monitor **only while Working** (never on break, idle,
//! meeting, or when not tracking). Flow per Rule 5:
//!   1. POST /uploads/presign  -> presigned PUT URL + storage key
//!   2. PUT the JPEG bytes directly to storage (MinIO/R2)
//!   3. POST /screenshots      -> store metadata only
//!
//! Captures never block the UI: capture runs on a blocking thread and failures
//! are logged and retried next tick.

use std::time::Duration;

use chrono::Utc;
use image::{codecs::jpeg::JpegEncoder, DynamicImage, ExtendedColorType, RgbaImage};
use serde::Deserialize;
use std::sync::atomic::Ordering;
use xcap::Monitor;

use crate::auth;
use crate::presence::derive_status;
use crate::timer::DesktopState;

const DEFAULT_INTERVAL_SECS: u64 = 300; // 5 minutes
const JPEG_QUALITY: u8 = 70;

fn api_base() -> String {
    std::env::var("TIMETRACKER_API_BASE_URL").unwrap_or_else(|_| "http://localhost:8090".to_string())
}

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

#[derive(Deserialize)]
struct PresignResponse {
    url: String,
    storage_key: String,
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
    let token = match auth::stored_token() {
        Some(t) => t,
        None => return Ok(()),
    };

    // 1. Presigned URL (server picks the namespaced key).
    let presign: PresignResponse = client
        .post(format!("{}/uploads/presign", api_base()))
        .bearer_auth(&token)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    // 2. Capture on a blocking thread (xcap is synchronous).
    let bytes = tokio::task::spawn_blocking(|| capture_primary_jpeg(JPEG_QUALITY)).await??;

    // 3. Upload bytes directly to storage.
    let put = client
        .put(&presign.url)
        .header("content-type", "image/jpeg")
        .body(bytes)
        .send()
        .await?;
    if !put.status().is_success() {
        anyhow::bail!("storage upload returned {}", put.status());
    }

    // 4. Notify the API (metadata only).
    client
        .post(format!("{}/screenshots", api_base()))
        .bearer_auth(&token)
        .json(&serde_json::json!({
            "storage_key": presign.storage_key,
            "taken_at": Utc::now().to_rfc3339(),
        }))
        .send()
        .await?
        .error_for_status()?;

    tracing::info!("screenshot uploaded: {}", presign.storage_key);
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
        assert!(!should_capture("not_logged_in"));
    }

    #[test]
    fn encodes_jpeg_bytes() {
        let img = RgbaImage::from_pixel(8, 8, image::Rgba([10, 20, 30, 255]));
        let bytes = encode_jpeg(&img, 70).unwrap();
        // JPEG SOI marker.
        assert!(bytes.len() > 2 && bytes[0] == 0xFF && bytes[1] == 0xD8);
    }
}

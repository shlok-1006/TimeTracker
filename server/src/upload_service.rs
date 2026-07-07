//! Upload service: mints presigned URLs for direct-to-storage screenshot
//! uploads (Rule 5). The server picks the storage key (namespaced by user) so a
//! client can't write outside its own prefix.

use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::storage::StorageClient;

/// How long a presigned upload URL is valid.
const PRESIGN_EXPIRES_SECS: u64 = 900; // 15 min

#[derive(Debug, Serialize)]
pub struct PresignedUpload {
    pub url: String,
    pub method: &'static str,
    pub storage_key: String,
    pub expires_in: u64,
}

/// Storage key for a user's screenshot: `<user_id>/<yyyymmdd>/<uuid>.jpg`.
pub fn screenshot_key(user_id: Uuid, now: DateTime<Utc>) -> String {
    format!(
        "{}/{}/{}.jpg",
        user_id,
        now.format("%Y%m%d"),
        Uuid::new_v4()
    )
}

/// Validate a client-submitted screenshot key against the exact shape this
/// service mints: `<user_id>/<yyyymmdd>/<uuid>.jpg`. This rejects path traversal
/// (`..`, `//`, leading `/`) and any key outside the caller's namespace, because
/// none of those can satisfy the strict 3-segment structure (SEC-11).
pub fn is_valid_screenshot_key(key: &str, user_id: Uuid) -> bool {
    let parts: Vec<&str> = key.split('/').collect();
    if parts.len() != 3 {
        return false;
    }
    // Segment 0: exactly the caller's own user id.
    if parts[0] != user_id.to_string() {
        return false;
    }
    // Segment 1: an 8-digit yyyymmdd date.
    if parts[1].len() != 8 || !parts[1].bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    // Segment 2: `<uuid>.jpg`.
    match parts[2].strip_suffix(".jpg") {
        Some(stem) => Uuid::parse_str(stem).is_ok(),
        None => false,
    }
}

/// Generate a presigned PUT for a new screenshot.
pub fn presign_screenshot(
    storage: &StorageClient,
    user_id: Uuid,
    now: DateTime<Utc>,
) -> PresignedUpload {
    let storage_key = screenshot_key(user_id, now);
    let url = storage.presign_put(&storage_key, PRESIGN_EXPIRES_SECS, now);
    PresignedUpload {
        url,
        method: "PUT",
        storage_key,
        expires_in: PRESIGN_EXPIRES_SECS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_is_namespaced_by_user() {
        let user = Uuid::new_v4();
        let key = screenshot_key(user, Utc::now());
        assert!(key.starts_with(&format!("{user}/")));
        assert!(key.ends_with(".jpg"));
    }

    #[test]
    fn minted_key_validates() {
        let user = Uuid::new_v4();
        let key = screenshot_key(user, Utc::now());
        assert!(is_valid_screenshot_key(&key, user));
    }

    #[test]
    fn rejects_traversal_and_cross_user() {
        let user = Uuid::new_v4();
        let victim = Uuid::new_v4();
        let uuid = Uuid::new_v4();
        // Traversal that satisfies a naive `starts_with("<user>/")` check.
        let sneaky = format!("{user}/../{victim}/20260101/{uuid}.jpg");
        assert!(!is_valid_screenshot_key(&sneaky, user));
        // Leading slash, double slash, wrong owner, bad date, non-jpg, non-uuid.
        assert!(!is_valid_screenshot_key(&format!("/{user}/20260101/{uuid}.jpg"), user));
        assert!(!is_valid_screenshot_key(&format!("{user}//{uuid}.jpg"), user));
        assert!(!is_valid_screenshot_key(&format!("{victim}/20260101/{uuid}.jpg"), user));
        assert!(!is_valid_screenshot_key(&format!("{user}/2026/{uuid}.jpg"), user));
        assert!(!is_valid_screenshot_key(&format!("{user}/20260101/{uuid}.png"), user));
        assert!(!is_valid_screenshot_key(&format!("{user}/20260101/not-a-uuid.jpg"), user));
    }
}

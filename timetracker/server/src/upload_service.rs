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
}

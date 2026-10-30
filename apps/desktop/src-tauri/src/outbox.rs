//! Screenshot outbox (Rule 1: local first).
//!
//! Every screenshot is written here before any network call, then uploaded by
//! [`ScreenshotOutbox::drain`] whenever the server answers — with its ORIGINAL
//! capture time. Before this, capture asked the server for an upload URL first,
//! so an employee on a VPN that blocks us had no screenshots at all for the day.
//!
//! Upload per row, following Rule 5:
//!   1. POST /uploads/presign -> URL + storage key   } skipped on a retry once
//!   2. PUT the bytes to storage                     } the key is recorded
//!   3. POST /screenshots     -> metadata
//!
//! Files are encrypted (XChaCha20-Poly1305) with a device key held in the OS
//! keychain, and bound to their row id so one file can't be passed off as
//! another. The queue is bounded: rows older than [`MAX_AGE_DAYS`], rows the
//! server refused [`MAX_REJECTIONS`] times, and anything past [`MAX_QUEUED`]
//! (oldest first) are dropped.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Context};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use chrono::{DateTime, Duration as ChronoDuration, SecondsFormat, Utc};
use rand::RngCore;
use serde::Serialize;
use sqlx::{Row, SqlitePool};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::auth;
use crate::http;

/// A screenshot not uploaded within a week is dropped.
const MAX_AGE_DAYS: i64 = 7;
/// Hard cap on queued screenshots (~a week of full days at the default
/// cadence); past it the oldest go first.
const MAX_QUEUED: i64 = 1000;
/// A row the server has refused this many times is dropped.
const MAX_REJECTIONS: i64 = 20;
const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 24;

#[derive(Clone)]
pub struct ScreenshotOutbox {
    dir: PathBuf,
    /// One drain at a time: the background uploader and "Sync now" must never
    /// upload the same row twice.
    drain_lock: Arc<Mutex<()>>,
    /// The device key, once loaded from (or created in) the keychain.
    key: Arc<std::sync::Mutex<Option<[u8; KEY_LEN]>>>,
}

#[derive(Debug, Clone)]
struct QueuedShot {
    id: Uuid,
    taken_at: String,
    captured_status: String,
    storage_key: Option<String>,
}

/// What one drain pass did.
#[derive(Debug, Default, Clone, Copy, Serialize)]
pub struct DrainReport {
    pub uploaded: usize,
    pub refused: usize,
}

enum UploadError {
    /// Server or storage unreachable, down, or the session needs a refresh —
    /// try again later; the rest of the queue would fail the same way.
    Unreachable(String),
    /// The server answered and refused this row.
    Refused(String),
    /// The file is gone or no longer decrypts (keychain reset).
    Unreadable(String),
}

impl ScreenshotOutbox {
    pub fn new(data_dir: &Path) -> Self {
        let dir = data_dir.join("screenshot-outbox");
        if let Err(e) = std::fs::create_dir_all(&dir) {
            tracing::warn!("could not create screenshot outbox dir: {e}");
        }
        Self {
            dir,
            drain_lock: Arc::new(Mutex::new(())),
            key: Arc::new(std::sync::Mutex::new(None)),
        }
    }

    fn file(&self, id: Uuid) -> PathBuf {
        self.dir.join(format!("{id}.bin"))
    }

    /// The device key: cached, else read from the keychain, else created there.
    fn key(&self) -> anyhow::Result<[u8; KEY_LEN]> {
        let mut cached = self
            .key
            .lock()
            .map_err(|_| anyhow!("outbox key lock poisoned"))?;
        if let Some(k) = *cached {
            return Ok(k);
        }
        let key = match auth::stored_outbox_key().and_then(|h| decode_key(&h)) {
            Some(k) => k,
            None => {
                let mut k = [0u8; KEY_LEN];
                rand::rngs::OsRng.fill_bytes(&mut k);
                auth::store_outbox_key(&encode_hex(&k)).map_err(|e| anyhow!(e))?;
                k
            }
        };
        *cached = Some(key);
        Ok(key)
    }

    /// Queue a captured JPEG. Never touches the network.
    pub async fn enqueue(
        &self,
        pool: &SqlitePool,
        user_id: Uuid,
        taken_at: DateTime<Utc>,
        captured_status: &str,
        jpeg: Vec<u8>,
    ) -> anyhow::Result<Uuid> {
        let id = Uuid::new_v4();
        let sealed = seal(&self.key()?, id, &jpeg)?;
        let path = self.file(id);
        tokio::fs::write(&path, sealed)
            .await
            .with_context(|| format!("write {}", path.display()))?;

        let inserted = sqlx::query(
            "INSERT INTO screenshot_outbox (id, user_id, taken_at, captured_status)
             VALUES (?, ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(user_id.to_string())
        .bind(ts(taken_at))
        .bind(captured_status)
        .execute(pool)
        .await;
        if let Err(e) = inserted {
            let _ = tokio::fs::remove_file(&path).await;
            return Err(e).context("queue screenshot");
        }
        self.prune(pool).await?;
        Ok(id)
    }

    /// Screenshots still waiting to upload for `user_id`.
    pub async fn pending_count(&self, pool: &SqlitePool, user_id: Uuid) -> anyhow::Result<i64> {
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM screenshot_outbox WHERE user_id = ?")
            .bind(user_id.to_string())
            .fetch_one(pool)
            .await?;
        Ok(n)
    }

    /// Upload everything queued for the signed-in user. Returns `Err` when the
    /// server or storage can't be reached (the remainder waits for next time);
    /// rows the server refuses are skipped and counted.
    pub async fn drain(&self, pool: &SqlitePool) -> anyhow::Result<DrainReport> {
        let mut report = DrainReport::default();
        let Ok(_guard) = self.drain_lock.try_lock() else {
            return Ok(report); // another drain is already running
        };
        self.prune(pool).await?;
        let Some(user_id) = auth::signed_in_user_id() else {
            return Ok(report);
        };

        // Least-refused first, then oldest: a row the server keeps refusing
        // sinks to the back instead of blocking the ones behind it.
        let rows = sqlx::query(
            "SELECT id, taken_at, captured_status, storage_key FROM screenshot_outbox
             WHERE user_id = ? ORDER BY attempts, taken_at",
        )
        .bind(user_id.to_string())
        .fetch_all(pool)
        .await?;
        let shots = rows
            .iter()
            .map(|r| {
                let id: String = r.try_get("id")?;
                Ok(QueuedShot {
                    id: Uuid::parse_str(&id).context("invalid outbox id")?,
                    taken_at: r.try_get("taken_at")?,
                    captured_status: r.try_get("captured_status")?,
                    storage_key: r.try_get("storage_key")?,
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;

        for shot in shots {
            match self.upload_one(pool, &shot).await {
                Ok(()) => {
                    self.remove(pool, shot.id).await?;
                    report.uploaded += 1;
                }
                Err(UploadError::Refused(e)) => {
                    tracing::warn!(id = %shot.id, "queued screenshot refused: {e}");
                    sqlx::query(
                        "UPDATE screenshot_outbox SET attempts = attempts + 1, last_error = ?
                         WHERE id = ?",
                    )
                    .bind(&e)
                    .bind(shot.id.to_string())
                    .execute(pool)
                    .await?;
                    report.refused += 1;
                }
                Err(UploadError::Unreadable(e)) => {
                    tracing::warn!(id = %shot.id, "dropping unreadable queued screenshot: {e}");
                    self.remove(pool, shot.id).await?;
                }
                Err(UploadError::Unreachable(e)) => {
                    if report.uploaded > 0 {
                        tracing::info!("uploaded {} queued screenshot(s)", report.uploaded);
                    }
                    return Err(anyhow!(e));
                }
            }
        }
        if report.uploaded > 0 {
            tracing::info!("uploaded {} queued screenshot(s)", report.uploaded);
        }
        Ok(report)
    }

    async fn upload_one(&self, pool: &SqlitePool, shot: &QueuedShot) -> Result<(), UploadError> {
        let storage_key = match &shot.storage_key {
            Some(k) => k.clone(),
            None => {
                let k = self.upload_bytes(shot).await?;
                sqlx::query("UPDATE screenshot_outbox SET storage_key = ? WHERE id = ?")
                    .bind(&k)
                    .bind(shot.id.to_string())
                    .execute(pool)
                    .await
                    .map_err(|e| UploadError::Unreachable(format!("record storage key: {e}")))?;
                k
            }
        };
        http::post_json(
            "/screenshots",
            serde_json::json!({
                "storage_key": storage_key,
                "taken_at": shot.taken_at,
                "captured_status": shot.captured_status,
            }),
        )
        .await
        .map_err(classify)?;
        tracing::info!(id = %shot.id, taken_at = %shot.taken_at, "screenshot uploaded: {storage_key}");
        Ok(())
    }

    /// Presign + PUT. Returns the storage key the bytes now live under.
    async fn upload_bytes(&self, shot: &QueuedShot) -> Result<String, UploadError> {
        let sealed = tokio::fs::read(self.file(shot.id))
            .await
            .map_err(|e| UploadError::Unreadable(format!("read: {e}")))?;
        let key = self
            .key()
            .map_err(|e| UploadError::Unreachable(e.to_string()))?;
        let jpeg =
            open(&key, shot.id, &sealed).map_err(|e| UploadError::Unreadable(e.to_string()))?;

        let presign = http::post_json("/uploads/presign", serde_json::json!({}))
            .await
            .map_err(classify)?;
        let field = |name: &str| {
            presign
                .get(name)
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .ok_or_else(|| UploadError::Unreachable(format!("presign response missing {name}")))
        };
        let (url, storage_key) = (field("url")?, field("storage_key")?);

        let put = http::client()
            .put(&url)
            .timeout(http::UPLOAD_TIMEOUT)
            .header("content-type", "image/jpeg")
            .body(jpeg)
            .send()
            .await
            .map_err(|e| {
                UploadError::Unreachable(format!("storage upload: {}", http::describe(&e)))
            })?;
        let status = put.status();
        if !status.is_success() {
            // A presigned URL is single-use from our side; a refused PUT (say,
            // it expired) is retried with a fresh one, so it is never the row's
            // fault — treat it as transient.
            return Err(UploadError::Unreachable(format!(
                "storage upload returned {status}"
            )));
        }
        Ok(storage_key)
    }

    async fn remove(&self, pool: &SqlitePool, id: Uuid) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM screenshot_outbox WHERE id = ?")
            .bind(id.to_string())
            .execute(pool)
            .await?;
        match tokio::fs::remove_file(self.file(id)).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e).context("remove queued screenshot file"),
        }
    }

    /// Enforce the age, refusal and size bounds.
    pub async fn prune(&self, pool: &SqlitePool) -> anyhow::Result<()> {
        let cutoff = ts(Utc::now() - ChronoDuration::days(MAX_AGE_DAYS));
        let doomed: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM screenshot_outbox
             WHERE taken_at < ? OR attempts >= ?
                OR id IN (SELECT id FROM screenshot_outbox
                          ORDER BY taken_at DESC LIMIT -1 OFFSET ?)",
        )
        .bind(&cutoff)
        .bind(MAX_REJECTIONS)
        .bind(MAX_QUEUED)
        .fetch_all(pool)
        .await?;
        if !doomed.is_empty() {
            tracing::warn!(
                "dropping {} queued screenshot(s) past the outbox limits",
                doomed.len()
            );
        }
        for id in doomed {
            if let Ok(id) = Uuid::parse_str(&id) {
                self.remove(pool, id).await?;
            }
        }
        Ok(())
    }

    /// Startup cleanup: delete files with no row (a crash between writing the
    /// file and inserting the row) and rows with no file.
    pub async fn sweep(&self, pool: &SqlitePool) -> anyhow::Result<()> {
        let ids: std::collections::HashSet<String> =
            sqlx::query_scalar::<_, String>("SELECT id FROM screenshot_outbox")
                .fetch_all(pool)
                .await?
                .into_iter()
                .collect();
        let mut on_disk = std::collections::HashSet::new();
        let mut entries = tokio::fs::read_dir(&self.dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string();
            if ids.contains(&stem) {
                on_disk.insert(stem);
            } else {
                let _ = tokio::fs::remove_file(&path).await;
            }
        }
        for id in ids.difference(&on_disk) {
            sqlx::query("DELETE FROM screenshot_outbox WHERE id = ?")
                .bind(id)
                .execute(pool)
                .await?;
        }
        Ok(())
    }
}

fn classify(err: String) -> UploadError {
    if http::is_rejection(&err) {
        UploadError::Refused(err)
    } else {
        UploadError::Unreachable(err)
    }
}

/// Fixed-width RFC3339 (millis, `Z`) so stored times compare as strings.
fn ts(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// `nonce || ciphertext`, with the row id as associated data.
fn seal(key: &[u8; KEY_LEN], id: Uuid, plaintext: &[u8]) -> anyhow::Result<Vec<u8>> {
    let cipher = XChaCha20Poly1305::new(Key::from_slice(key));
    let mut nonce = [0u8; NONCE_LEN];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    let ciphertext = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad: id.as_bytes(),
            },
        )
        .map_err(|_| anyhow!("encrypt screenshot"))?;
    let mut out = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

fn open(key: &[u8; KEY_LEN], id: Uuid, sealed: &[u8]) -> anyhow::Result<Vec<u8>> {
    if sealed.len() < NONCE_LEN {
        return Err(anyhow!("queued screenshot is truncated"));
    }
    let (nonce, ciphertext) = sealed.split_at(NONCE_LEN);
    XChaCha20Poly1305::new(Key::from_slice(key))
        .decrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: ciphertext,
                aad: id.as_bytes(),
            },
        )
        .map_err(|_| anyhow!("queued screenshot does not decrypt (device key changed?)"))
}

fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn decode_key(hex: &str) -> Option<[u8; KEY_LEN]> {
    let hex = hex.trim();
    if hex.len() != KEY_LEN * 2 {
        return None;
    }
    let mut out = [0u8; KEY_LEN];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(hex.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    fn outbox_with_key() -> ScreenshotOutbox {
        let dir = std::env::temp_dir().join(format!("tt-outbox-test-{}", Uuid::new_v4()));
        let outbox = ScreenshotOutbox::new(&dir);
        *outbox.key.lock().unwrap() = Some([7u8; KEY_LEN]);
        outbox
    }

    async fn pool() -> SqlitePool {
        let pool = db::connect_in_memory().await.unwrap();
        db::migrate(&pool).await.unwrap();
        pool
    }

    async fn count(pool: &SqlitePool) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM screenshot_outbox")
            .fetch_one(pool)
            .await
            .unwrap()
    }

    #[test]
    fn sealed_bytes_round_trip_and_are_bound_to_their_id() {
        let key = [3u8; KEY_LEN];
        let id = Uuid::new_v4();
        let sealed = seal(&key, id, b"jpeg bytes").unwrap();
        assert_ne!(&sealed[NONCE_LEN..], b"jpeg bytes");
        assert_eq!(open(&key, id, &sealed).unwrap(), b"jpeg bytes");
        // Another row's id, another key, or a flipped bit must not decrypt.
        assert!(open(&key, Uuid::new_v4(), &sealed).is_err());
        assert!(open(&[4u8; KEY_LEN], id, &sealed).is_err());
        let mut tampered = sealed.clone();
        *tampered.last_mut().unwrap() ^= 1;
        assert!(open(&key, id, &tampered).is_err());
        assert!(open(&key, id, &sealed[..5]).is_err());
    }

    #[test]
    fn key_hex_round_trips() {
        let key: [u8; KEY_LEN] = std::array::from_fn(|i| i as u8 * 7);
        assert_eq!(decode_key(&encode_hex(&key)), Some(key));
        assert_eq!(decode_key("abc"), None);
        assert_eq!(decode_key(&"zz".repeat(KEY_LEN)), None);
    }

    #[tokio::test]
    async fn enqueue_writes_an_encrypted_file_and_a_row() {
        let pool = pool().await;
        let outbox = outbox_with_key();
        let user = Uuid::new_v4();
        let id = outbox
            .enqueue(&pool, user, Utc::now(), "working", b"\xFF\xD8jpeg".to_vec())
            .await
            .unwrap();
        let on_disk = std::fs::read(outbox.file(id)).unwrap();
        assert_eq!(
            open(&[7u8; KEY_LEN], id, &on_disk).unwrap(),
            b"\xFF\xD8jpeg"
        );
        assert_eq!(outbox.pending_count(&pool, user).await.unwrap(), 1);
        assert_eq!(
            outbox.pending_count(&pool, Uuid::new_v4()).await.unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn prune_drops_old_refused_and_overflow_rows() {
        let pool = pool().await;
        let outbox = outbox_with_key();
        let user = Uuid::new_v4();
        let old = outbox
            .enqueue(
                &pool,
                user,
                Utc::now() - ChronoDuration::days(8),
                "working",
                vec![1],
            )
            .await
            .unwrap();
        assert!(
            !outbox.file(old).exists(),
            "a week-old shot is dropped with its file"
        );
        let refused = outbox
            .enqueue(&pool, user, Utc::now(), "working", vec![2])
            .await
            .unwrap();
        sqlx::query("UPDATE screenshot_outbox SET attempts = ? WHERE id = ?")
            .bind(MAX_REJECTIONS)
            .bind(refused.to_string())
            .execute(&pool)
            .await
            .unwrap();
        outbox.prune(&pool).await.unwrap();
        assert_eq!(count(&pool).await, 0);

        // Overflow keeps the newest MAX_QUEUED.
        let base = Utc::now() - ChronoDuration::hours(1);
        for i in 0..(MAX_QUEUED + 3) {
            sqlx::query(
                "INSERT INTO screenshot_outbox (id, user_id, taken_at, captured_status)
                 VALUES (?, ?, ?, 'working')",
            )
            .bind(Uuid::new_v4().to_string())
            .bind(user.to_string())
            .bind(ts(base + ChronoDuration::seconds(i)))
            .execute(&pool)
            .await
            .unwrap();
        }
        outbox.prune(&pool).await.unwrap();
        assert_eq!(count(&pool).await, MAX_QUEUED);
        let oldest: String = sqlx::query_scalar("SELECT MIN(taken_at) FROM screenshot_outbox")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(oldest, ts(base + ChronoDuration::seconds(3)));
    }

    #[tokio::test]
    async fn sweep_removes_orphan_files_and_rows() {
        let pool = pool().await;
        let outbox = outbox_with_key();
        let user = Uuid::new_v4();
        let kept = outbox
            .enqueue(&pool, user, Utc::now(), "working", vec![1])
            .await
            .unwrap();
        let fileless = outbox
            .enqueue(&pool, user, Utc::now(), "working", vec![2])
            .await
            .unwrap();
        std::fs::remove_file(outbox.file(fileless)).unwrap();
        let stray = outbox.file(Uuid::new_v4());
        std::fs::write(&stray, b"x").unwrap();

        outbox.sweep(&pool).await.unwrap();
        assert!(!stray.exists());
        assert!(outbox.file(kept).exists());
        assert_eq!(count(&pool).await, 1);
    }
}

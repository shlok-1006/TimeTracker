//! Object storage: S3-compatible (MinIO local / Cloudflare R2) **or** Google
//! Cloud Storage signed with a service-account key.
//!
//! The server never stores screenshot bytes (Rule 5). It hands the desktop a
//! short-lived **presigned PUT URL**; the desktop uploads bytes directly to the
//! bucket, then notifies the API with metadata only.
//!
//! Two signing modes, chosen by config:
//!   * **S3 SigV4** (default) — local HMAC over `S3_ACCESS_KEY_ID/SECRET`.
//!     Verified against AWS's published test vector in the tests below.
//!   * **GCS V4 (`GOOG4-RSA-SHA256`)** — when `GCS_SA_KEY_BASE64`/`_JSON` is set,
//!     sign with the service account's RSA private key (no HMAC key needed).
//!
//! Both are local computations — no network, no cloud SDK.

use std::sync::Arc;

use anyhow::Context;
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use rsa::pkcs1v15::SigningKey;
use rsa::pkcs8::DecodePrivateKey;
use rsa::signature::{SignatureEncoding, Signer};
use rsa::RsaPrivateKey;
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone)]
pub struct S3Config {
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub access_key: String,
    pub secret_key: String,
    pub force_path_style: bool,
    /// When set, presigned URLs are signed with GCS V4 (GOOG4-RSA-SHA256) using
    /// this service account instead of S3 HMAC SigV4; `access_key`/`secret_key`
    /// are then unused.
    pub gcs: Option<GcsSigner>,
}

impl S3Config {
    /// Load from env. Object-storage credentials are **required** — the server
    /// refuses to start without real `S3_ACCESS_KEY_ID` / `S3_SECRET_ACCESS_KEY`,
    /// and rejects the well-known `minioadmin` default (SEC-03). Local MinIO dev
    /// can opt back into the defaults with `S3_ALLOW_INSECURE_DEFAULTS=true`.
    pub fn from_env() -> anyhow::Result<Self> {
        fn var(key: &str, default: &str) -> String {
            std::env::var(key).unwrap_or_else(|_| default.to_string())
        }

        // GCS service-account signing (Option B): if a key is provided we sign
        // with GCS V4 and the HMAC keys are irrelevant. Defaults switch to GCS.
        let gcs = load_gcs_signer()?;

        let allow_insecure = std::env::var("S3_ALLOW_INSECURE_DEFAULTS")
            .map(|v| v == "true")
            .unwrap_or(false);

        let env_access = std::env::var("S3_ACCESS_KEY_ID").unwrap_or_default();
        let env_secret = std::env::var("S3_SECRET_ACCESS_KEY").unwrap_or_default();

        let (access_key, secret_key) = if gcs.is_some() {
            // Signing uses the RSA key; HMAC creds are unused.
            (String::new(), String::new())
        } else if !env_access.is_empty() && !env_secret.is_empty() {
            (env_access, env_secret)
        } else if allow_insecure {
            ("minioadmin".to_string(), "minioadmin".to_string())
        } else {
            anyhow::bail!(
                "no object-storage credentials: set GCS_SA_KEY_BASE64 (GCS), or \
                 S3_ACCESS_KEY_ID + S3_SECRET_ACCESS_KEY (S3/MinIO/R2), or \
                 S3_ALLOW_INSECURE_DEFAULTS=true for local MinIO dev"
            );
        };

        if gcs.is_none()
            && !allow_insecure
            && (access_key == "minioadmin" || secret_key == "minioadmin")
        {
            anyhow::bail!(
                "refusing to start with the default 'minioadmin' S3 credentials — \
                 set real S3_ACCESS_KEY_ID / S3_SECRET_ACCESS_KEY, or \
                 S3_ALLOW_INSECURE_DEFAULTS=true for local dev"
            );
        }

        // GCS uses storage.googleapis.com + region "auto" (Google's V4 scope).
        let (default_endpoint, default_region) = if gcs.is_some() {
            ("https://storage.googleapis.com", "auto")
        } else {
            ("http://localhost:9000", "us-east-1")
        };

        Ok(Self {
            endpoint: var("S3_ENDPOINT", default_endpoint),
            region: var("S3_REGION", default_region),
            bucket: var("S3_BUCKET", "screenshots"),
            access_key,
            secret_key,
            force_path_style: var("S3_FORCE_PATH_STYLE", "true") == "true",
            gcs,
        })
    }

    /// Fixed local-MinIO config for tests and local tooling only. Production
    /// always goes through `from_env`, which enforces real credentials (SEC-03).
    pub fn insecure_local() -> Self {
        Self {
            endpoint: "http://localhost:9100".into(),
            region: "us-east-1".into(),
            bucket: "screenshots".into(),
            access_key: "minioadmin".into(),
            secret_key: "minioadmin".into(),
            force_path_style: true,
            gcs: None,
        }
    }
}

/// A Google Cloud Storage V4 signer backed by a service-account RSA private key.
/// Signs presigned-URL string-to-sign values with `GOOG4-RSA-SHA256`.
#[derive(Clone)]
pub struct GcsSigner {
    client_email: String,
    signing_key: Arc<SigningKey<Sha256>>,
}

impl std::fmt::Debug for GcsSigner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print the private key.
        f.debug_struct("GcsSigner")
            .field("client_email", &self.client_email)
            .finish_non_exhaustive()
    }
}

impl GcsSigner {
    /// Build from a GCS service-account JSON (needs `client_email` + PEM
    /// `private_key`).
    pub fn from_json(json: &str) -> anyhow::Result<Self> {
        #[derive(serde::Deserialize)]
        struct Sa {
            client_email: String,
            private_key: String,
        }
        let sa: Sa = serde_json::from_str(json).context("parse service-account JSON")?;
        let key = RsaPrivateKey::from_pkcs8_pem(&sa.private_key)
            .map_err(|e| anyhow::anyhow!("parse service-account private key: {e}"))?;
        Ok(Self {
            client_email: sa.client_email,
            signing_key: Arc::new(SigningKey::<Sha256>::new(key)),
        })
    }

    /// RSA-SHA256 sign the string-to-sign, hex-encoded (GCS V4 signature form).
    fn sign_hex(&self, string_to_sign: &str) -> String {
        let sig = self.signing_key.sign(string_to_sign.as_bytes());
        hex::encode(sig.to_bytes())
    }
}

/// Load a GCS signer from `GCS_SA_KEY_BASE64` (preferred) or `GCS_SA_KEY_JSON`.
/// Returns `None` when neither is set (S3 HMAC path).
fn load_gcs_signer() -> anyhow::Result<Option<GcsSigner>> {
    use base64::Engine;
    if let Ok(b64) = std::env::var("GCS_SA_KEY_BASE64") {
        if !b64.trim().is_empty() {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(b64.trim())
                .context("GCS_SA_KEY_BASE64 is not valid base64")?;
            let json = String::from_utf8(bytes).context("GCS service-account key is not UTF-8")?;
            return Ok(Some(GcsSigner::from_json(&json)?));
        }
    }
    if let Ok(json) = std::env::var("GCS_SA_KEY_JSON") {
        if !json.trim().is_empty() {
            return Ok(Some(GcsSigner::from_json(&json)?));
        }
    }
    Ok(None)
}

#[derive(Clone)]
pub struct StorageClient {
    cfg: S3Config,
}

impl StorageClient {
    pub fn new(cfg: S3Config) -> Self {
        Self { cfg }
    }

    pub fn bucket(&self) -> &str {
        &self.cfg.bucket
    }

    /// Build a presigned `PUT` URL for `key`, valid for `expires_secs`.
    pub fn presign_put(&self, key: &str, expires_secs: u64, now: DateTime<Utc>) -> String {
        self.presign("PUT", key, expires_secs, now)
    }

    /// Build a presigned `GET` URL for `key` (short-lived viewing — Rule 5).
    pub fn presign_get(&self, key: &str, expires_secs: u64, now: DateTime<Utc>) -> String {
        self.presign("GET", key, expires_secs, now)
    }

    /// Download a screenshot's bytes for AI analysis via a short-lived presigned
    /// GET. Because the presigned PUT can't bind content-type/size (SigV4 query
    /// signing), we validate here (SEC-16): reject anything over the size cap or
    /// that isn't a JPEG, so an attacker-uploaded oversized/non-image blob never
    /// reaches the vision model. (A bucket-side max-object-size policy should
    /// also be configured to bound storage-cost abuse at the source.)
    pub async fn fetch_object(&self, key: &str) -> Result<Vec<u8>, String> {
        const MAX_SCREENSHOT_BYTES: u64 = 15 * 1024 * 1024;

        let url = self.presign_get(key, 300, Utc::now());
        let mut resp = reqwest::get(&url)
            .await
            .map_err(|e| format!("storage GET failed: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("storage GET {key} -> HTTP {}", resp.status()));
        }
        // Reject early if the advertised length is over the cap.
        if let Some(len) = resp.content_length() {
            if len > MAX_SCREENSHOT_BYTES {
                return Err(format!("object {key} too large: {len} bytes"));
            }
        }
        // Stream with a running cap (RA-16) so a spoofed/absent Content-Length
        // can't force us to buffer an arbitrarily large body before rejecting.
        let mut buf: Vec<u8> = Vec::new();
        while let Some(chunk) = resp
            .chunk()
            .await
            .map_err(|e| format!("reading object bytes failed: {e}"))?
        {
            if buf.len() as u64 + chunk.len() as u64 > MAX_SCREENSHOT_BYTES {
                return Err(format!(
                    "object {key} too large: exceeds {MAX_SCREENSHOT_BYTES} bytes"
                ));
            }
            buf.extend_from_slice(&chunk);
        }
        // JPEG SOI marker (0xFF 0xD8 0xFF) — the desktop only ever uploads JPEG.
        if buf.len() < 3 || buf[0] != 0xFF || buf[1] != 0xD8 || buf[2] != 0xFF {
            return Err(format!("object {key} is not a JPEG image"));
        }
        Ok(buf)
    }

    fn presign(&self, method: &str, key: &str, expires_secs: u64, now: DateTime<Utc>) -> String {
        let (scheme, endpoint_host) = split_scheme(&self.cfg.endpoint);

        let (host, canonical_uri) = if self.cfg.force_path_style {
            // MinIO: http://host:9000/<bucket>/<key>
            (
                endpoint_host.to_string(),
                format!("/{}/{}", self.cfg.bucket, key),
            )
        } else {
            // Virtual-hosted: https://<bucket>.<host>/<key>
            (
                format!("{}.{}", self.cfg.bucket, endpoint_host),
                format!("/{key}"),
            )
        };

        match &self.cfg.gcs {
            Some(signer) => presigned_url_gcs(
                signer,
                &self.cfg.region,
                method,
                scheme,
                &host,
                &canonical_uri,
                expires_secs,
                now,
            ),
            None => presigned_url(
                &self.cfg.access_key,
                &self.cfg.secret_key,
                &self.cfg.region,
                method,
                scheme,
                &host,
                &canonical_uri,
                expires_secs,
                now,
            ),
        }
    }
}

fn split_scheme(endpoint: &str) -> (&str, &str) {
    match endpoint.split_once("://") {
        Some((scheme, host)) => (scheme, host.trim_end_matches('/')),
        None => ("https", endpoint.trim_end_matches('/')),
    }
}

fn hmac(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

fn sha256_hex(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    hex::encode(h.finalize())
}

/// RFC 3986 encoding (AWS-style). Unreserved chars pass through; `/` is kept
/// when `encode_slash` is false (used for path components).
fn uri_encode(s: &str, encode_slash: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b'/' if !encode_slash => out.push('/'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Core AWS SigV4 query presigner. Returns the full signed URL.
#[allow(clippy::too_many_arguments)]
fn presigned_url(
    access_key: &str,
    secret_key: &str,
    region: &str,
    method: &str,
    scheme: &str,
    host: &str,
    canonical_uri: &str,
    expires_secs: u64,
    now: DateTime<Utc>,
) -> String {
    let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
    let date_stamp = now.format("%Y%m%d").to_string();
    let scope = format!("{date_stamp}/{region}/s3/aws4_request");
    let credential = format!("{access_key}/{scope}");

    // Canonical query string (keys are already in sorted order).
    let canonical_querystring = format!(
        "X-Amz-Algorithm=AWS4-HMAC-SHA256\
         &X-Amz-Credential={}\
         &X-Amz-Date={}\
         &X-Amz-Expires={}\
         &X-Amz-SignedHeaders=host",
        uri_encode(&credential, true),
        amz_date,
        expires_secs,
    );

    let encoded_uri = uri_encode(canonical_uri, false);
    let canonical_headers = format!("host:{host}\n");
    let payload_hash = "UNSIGNED-PAYLOAD";
    let canonical_request = format!(
        "{method}\n{encoded_uri}\n{canonical_querystring}\n{canonical_headers}\nhost\n{payload_hash}"
    );

    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
        sha256_hex(canonical_request.as_bytes())
    );

    // Derive the signing key.
    let k_date = hmac(
        format!("AWS4{secret_key}").as_bytes(),
        date_stamp.as_bytes(),
    );
    let k_region = hmac(&k_date, region.as_bytes());
    let k_service = hmac(&k_region, b"s3");
    let k_signing = hmac(&k_service, b"aws4_request");
    let signature = hex::encode(hmac(&k_signing, string_to_sign.as_bytes()));

    format!("{scheme}://{host}{encoded_uri}?{canonical_querystring}&X-Amz-Signature={signature}")
}

/// GCS V4 query presigner (`GOOG4-RSA-SHA256`). Same canonical-request shape as
/// SigV4, but the credential scope is `.../auto/storage/goog4_request` and the
/// signature is an RSA-SHA256 signature (hex) over the string-to-sign, produced
/// with the service account's private key.
#[allow(clippy::too_many_arguments)]
fn presigned_url_gcs(
    signer: &GcsSigner,
    region: &str,
    method: &str,
    scheme: &str,
    host: &str,
    canonical_uri: &str,
    expires_secs: u64,
    now: DateTime<Utc>,
) -> String {
    let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
    let date_stamp = now.format("%Y%m%d").to_string();
    let scope = format!("{date_stamp}/{region}/storage/goog4_request");
    let credential = format!("{}/{}", signer.client_email, scope);

    // Keys must be in sorted order for the canonical query string.
    let canonical_querystring = format!(
        "X-Goog-Algorithm=GOOG4-RSA-SHA256\
         &X-Goog-Credential={}\
         &X-Goog-Date={}\
         &X-Goog-Expires={}\
         &X-Goog-SignedHeaders=host",
        uri_encode(&credential, true),
        amz_date,
        expires_secs,
    );

    let encoded_uri = uri_encode(canonical_uri, false);
    let canonical_headers = format!("host:{host}\n");
    let payload_hash = "UNSIGNED-PAYLOAD";
    let canonical_request = format!(
        "{method}\n{encoded_uri}\n{canonical_querystring}\n{canonical_headers}\nhost\n{payload_hash}"
    );

    let string_to_sign = format!(
        "GOOG4-RSA-SHA256\n{amz_date}\n{scope}\n{}",
        sha256_hex(canonical_request.as_bytes())
    );

    let signature = signer.sign_hex(&string_to_sign);

    format!("{scheme}://{host}{encoded_uri}?{canonical_querystring}&X-Goog-Signature={signature}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    /// AWS's official SigV4 "GET Object" presign example. If our signature
    /// matches the documented value, the implementation is correct.
    /// https://docs.aws.amazon.com/AmazonS3/latest/API/sigv4-query-string-auth.html
    #[test]
    fn matches_aws_documented_vector() {
        let now = Utc.with_ymd_and_hms(2013, 5, 24, 0, 0, 0).unwrap();
        let url = presigned_url(
            "AKIAIOSFODNN7EXAMPLE",
            "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            "us-east-1",
            "GET",
            "https",
            "examplebucket.s3.amazonaws.com",
            "/test.txt",
            86400,
            now,
        );
        assert!(
            url.contains(
                "X-Amz-Signature=aeeed9bbccd4d02ee5c0109b86d86835f995330da4c265957d157751f604d404"
            ),
            "unexpected signature in {url}"
        );
    }

    #[test]
    fn presign_put_uses_path_style_for_minio() {
        let cfg = S3Config {
            endpoint: "http://localhost:9000".into(),
            region: "us-east-1".into(),
            bucket: "screenshots".into(),
            access_key: "minioadmin".into(),
            secret_key: "minioadmin".into(),
            force_path_style: true,
            gcs: None,
        };
        let url = StorageClient::new(cfg).presign_put("user/abc.jpg", 900, Utc::now());
        assert!(url.starts_with("http://localhost:9000/screenshots/user/abc.jpg?"));
        assert!(url.contains("X-Amz-Algorithm=AWS4-HMAC-SHA256"));
        assert!(url.contains("X-Amz-Signature="));
    }

    // Throwaway 2048-bit RSA key (PKCS#8) — for tests ONLY, never a real credential.
    const TEST_PEM: &str = "-----BEGIN PRIVATE KEY-----\n\
        MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQC61AOB+SCVZ3+7\n\
        k/ZHYIOWTZNjPHOjF9gQj9ZtJHIvvBWzkl7bIad5cIFUzQ3nzwF0aVVDR+WurRNw\n\
        dcFwNWgPJydnGFdq/3KseYDGrVUFDmB8SCnzv6c6PcWMLwGf2g1B58m31hwVCrMu\n\
        L5ykH+IhRly1DHsuvWyRuTFZZyrIYeOzIexPB2bjbOhGJfzETHn/OQ2Fcyabwm0D\n\
        lMkdjvg1b3JJ4cqkq/yfGjZPP1JnVGxPDSxlRpzCGOVFi0uKX8fjyfhVT9bJHt/E\n\
        xBm+bcx0srUa/CBmOndAOXvc99PvtE030vkdH3AuXk+E9Jxvgnl81dvNB5MPUDWf\n\
        BXrSKd6VAgMBAAECggEAFhRg3r8z0dxjvOYpbLmUC5Ma+FcoXm2+uARbxdXRFeGf\n\
        WfPSOIkTaxd3/W7ndg4hoKGjNTqdwyVKvxd3lzyEkgfhUP6QNEHAym/on3JUMi8H\n\
        CaEYrikoCQrMWjsi8MKbHv8W+J45/uWfE/YGB+KJvb98TNxPAZDa8CToF2YJKRmu\n\
        a5BL0K+cApsk12hcyiB9xs7EF06+ikcfTMnGsFnELeDfp3j9qYbVt0YCra8jSA5J\n\
        kIwnz1DqHRzyrvV0lHyamX9Tf3160gHYeCX4ci3TL3M83k7hXZmEjtZEw7aw53A+\n\
        ZoSC1Sjdgy/HW/O+tBtaR4w9c1Dmzwhj8B2pYUvz2QKBgQD4cjq1UNBW8On13PRF\n\
        GA0QCu2bVhE4twoEdLW5/PGZcABXu/b4Urq9k+GABi0b368AQMH2vtemLRyxdNIX\n\
        crUo50vcCRO8pI74uhnPxNnYsP58urWbGd1db8D6tu1hEHeO3OebPmQX8Qkg3mAz\n\
        6VqHAFeBVFnI7h7ItBddkZ8uqQKBgQDAgi7aFT82Jh4wJJ659krnH152fNzyj7ZQ\n\
        trJ0mkpwwVmrCDC05xvEwRPHB0LqwkM1+Im3csPT7wsACItobtpaskrL8ecp1b4v\n\
        YJZSdud0oqmKv+vV2y/8uYNiOGUKsckB2zUYY1T7zncI5agxfxsRmauHKjE6HzyD\n\
        +tmSf8CADQKBgDF6rm6F1bg66p1oj81i6NcVFhUlovBko74XcEnGMmeYgrj2Wk1C\n\
        TKaM2RAiKsGuXq/yNa7qexBBU8GTvnOlCKdIyTbdJ+0d6MtoZNOYJLqfJU+574kg\n\
        MZH2O7yyybvXB7iQDiBA4LZT7rl5EDfGdZ0FWGgNYIQ/yCm4lB6ybb1hAoGBAKnP\n\
        rChvaY0IXsS06Na9LSFnCFqYlgXZQ0c7gXtdxqBjzgcSeHR7EIGklx+PhjWzGrp2\n\
        /HQ35goC6L8kymRatH4gm93/CpxakSMVnkQr4st92PQti4jfihryQbTYbjjazqa2\n\
        iMV0ibh9TX8ppg3TQztpRGc8jqPofecl1dpmmpXZAoGAc5z8AXxFma7toBdzrlb0\n\
        Ee4hIf+urMaA1hiD8jg1fIcINis4U/upv/rRRbgvhf0+GnbKHfIGlCr6B/imQHfO\n\
        P1Yt5xOnP7Z7z3JnPalLhBUcInBjF4V0obpkyLCxOYzr1kAdZQuOS5Un0Zo27TiV\n\
        QVgUpdc+37SzHcxF2h7NtXw=\n\
        -----END PRIVATE KEY-----\n";

    fn test_gcs_config() -> S3Config {
        let json = format!(
            r#"{{"client_email":"timetracker-storage@ruh-ai-dev.iam.gserviceaccount.com","private_key":{}}}"#,
            serde_json::to_string(TEST_PEM).unwrap()
        );
        S3Config {
            endpoint: "https://storage.googleapis.com".into(),
            region: "auto".into(),
            bucket: "screenshots".into(),
            access_key: String::new(),
            secret_key: String::new(),
            force_path_style: true,
            gcs: Some(GcsSigner::from_json(&json).unwrap()),
        }
    }

    #[test]
    fn gcs_signer_parses_service_account_json() {
        let cfg = test_gcs_config();
        assert_eq!(
            cfg.gcs.as_ref().unwrap().client_email,
            "timetracker-storage@ruh-ai-dev.iam.gserviceaccount.com"
        );
    }

    #[test]
    fn gcs_presign_produces_goog4_signed_url() {
        let url =
            StorageClient::new(test_gcs_config()).presign_put("user/abc.jpg", 900, Utc::now());
        // Path-style GCS host + key.
        assert!(
            url.starts_with("https://storage.googleapis.com/screenshots/user/abc.jpg?"),
            "unexpected url: {url}"
        );
        assert!(url.contains("X-Goog-Algorithm=GOOG4-RSA-SHA256"));
        // Credential carries the SA email (with '/' + '@' percent-encoded).
        assert!(url.contains(
            "X-Goog-Credential=timetracker-storage%40ruh-ai-dev.iam.gserviceaccount.com%2F"
        ));
        assert!(url.contains("%2Fauto%2Fstorage%2Fgoog4_request"));
        assert!(url.contains("X-Goog-SignedHeaders=host"));
        // A signature is present and looks like hex (RSA-2048 => 512 hex chars).
        let sig = url.split("X-Goog-Signature=").nth(1).unwrap();
        assert_eq!(sig.len(), 512, "expected 2048-bit RSA signature hex");
        assert!(sig.bytes().all(|b| b.is_ascii_hexdigit()));
    }

    #[test]
    fn gcs_signatures_are_deterministic_for_fixed_inputs() {
        // PKCS#1 v1.5 is deterministic, so the same inputs must yield the same URL.
        let now = chrono::TimeZone::with_ymd_and_hms(&Utc, 2026, 7, 8, 12, 0, 0).unwrap();
        let a = StorageClient::new(test_gcs_config()).presign_put("k.jpg", 300, now);
        let b = StorageClient::new(test_gcs_config()).presign_put("k.jpg", 300, now);
        assert_eq!(a, b);
    }
}

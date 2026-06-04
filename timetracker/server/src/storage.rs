//! S3-compatible object storage (MinIO local / Cloudflare R2 production).
//!
//! The server never stores screenshot bytes (Rule 5). It hands the desktop a
//! short-lived **presigned PUT URL**; the desktop uploads bytes directly to the
//! bucket, then notifies the API with metadata only.
//!
//! Presigning is AWS Signature V4 query signing — a local HMAC computation, no
//! network and no AWS SDK (keeps the build free of C toolchains). Verified
//! against AWS's published test vector in the tests below.

use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
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
}

impl S3Config {
    pub fn from_env() -> Self {
        fn var(key: &str, default: &str) -> String {
            std::env::var(key).unwrap_or_else(|_| default.to_string())
        }
        Self {
            endpoint: var("S3_ENDPOINT", "http://localhost:9000"),
            region: var("S3_REGION", "us-east-1"),
            bucket: var("S3_BUCKET", "screenshots"),
            access_key: var("S3_ACCESS_KEY_ID", "minioadmin"),
            secret_key: var("S3_SECRET_ACCESS_KEY", "minioadmin"),
            force_path_style: var("S3_FORCE_PATH_STYLE", "true") == "true",
        }
    }
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

        presigned_url(
            &self.cfg.access_key,
            &self.cfg.secret_key,
            &self.cfg.region,
            method,
            scheme,
            &host,
            &canonical_uri,
            expires_secs,
            now,
        )
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
    let k_date = hmac(format!("AWS4{secret_key}").as_bytes(), date_stamp.as_bytes());
    let k_region = hmac(&k_date, region.as_bytes());
    let k_service = hmac(&k_region, b"s3");
    let k_signing = hmac(&k_service, b"aws4_request");
    let signature = hex::encode(hmac(&k_signing, string_to_sign.as_bytes()));

    format!("{scheme}://{host}{encoded_uri}?{canonical_querystring}&X-Amz-Signature={signature}")
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
        };
        let url = StorageClient::new(cfg).presign_put("user/abc.jpg", 900, Utc::now());
        assert!(url.starts_with("http://localhost:9000/screenshots/user/abc.jpg?"));
        assert!(url.contains("X-Amz-Algorithm=AWS4-HMAC-SHA256"));
        assert!(url.contains("X-Amz-Signature="));
    }
}

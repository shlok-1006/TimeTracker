//! Google Cloud Storage (GCS) object storage.
//!
//! The server never stores screenshot bytes (Rule 5). It hands the desktop a
//! short-lived **V4 signed PUT URL**; the desktop uploads bytes directly to the
//! bucket, then notifies the API with metadata only.
//!
//! Signing is GCS V4 (`GOOG4-RSA-SHA256`) query signing — a local RSA-SHA256
//! computation using the service account's private key. No network and no GCS
//! SDK (keeps the build free of C toolchains; pure-Rust `rsa`). Implements the
//! algorithm documented at:
//! <https://cloud.google.com/storage/docs/access-control/signing-urls-manually>

use anyhow::{anyhow, Context, Result};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use chrono::{DateTime, Utc};
use rsa::pkcs1v15::SigningKey;
use rsa::pkcs8::DecodePrivateKey;
use rsa::signature::{SignatureEncoding, Signer};
use rsa::RsaPrivateKey;
use sha2::{Digest, Sha256};

/// Default GCS endpoint host. Path-style URLs are
/// `https://storage.googleapis.com/<bucket>/<object>`.
const GCS_HOST: &str = "storage.googleapis.com";

#[derive(Debug, Clone)]
pub struct GcsConfig {
    pub bucket: String,
    /// Storage host (path-style). Defaults to `storage.googleapis.com`.
    pub host: String,
    /// Credential-scope location. GCS echoes this back from the signed URL and
    /// recomputes the signature with it, so any consistent value verifies;
    /// `auto` is the conventional default.
    pub location: String,
    /// Service-account email used as the signing principal.
    pub client_email: String,
    /// Service-account private key (PKCS#8 PEM). Empty when unconfigured.
    pub private_key_pem: String,
}

impl GcsConfig {
    /// Build config from the environment. Credentials come from a GCS service
    /// account key, sourced (in order) from: `GCS_SA_KEY_BASE64` (base64-encoded
    /// JSON — the primary, single-line-safe form), `GCS_SA_KEY_JSON` (raw inline
    /// JSON), or `GOOGLE_APPLICATION_CREDENTIALS` (key file path). Missing or
    /// unreadable credentials yield an unconfigured client (signing fails only
    /// when a URL is actually requested), so non-storage code paths and tests
    /// still run without a key.
    pub fn from_env() -> Self {
        fn var(key: &str, default: &str) -> String {
            std::env::var(key).unwrap_or_else(|_| default.to_string())
        }
        let (client_email, private_key_pem) = load_sa_credentials().unwrap_or_default();
        Self {
            bucket: var("GCS_BUCKET", "ruh-time-tracker"),
            host: var("GCS_HOST", GCS_HOST),
            location: var("GCS_LOCATION", "auto"),
            client_email,
            private_key_pem,
        }
    }
}

/// Load `(client_email, private_key_pem)` from a GCS service-account key.
fn load_sa_credentials() -> Option<(String, String)> {
    let json = load_sa_key_json()?;
    parse_sa_key_json(&json)
}

/// Read the raw service-account key JSON from the environment, trying (in order):
/// base64-encoded (`GCS_SA_KEY_BASE64`), raw inline (`GCS_SA_KEY_JSON`), then a
/// key file path (`GOOGLE_APPLICATION_CREDENTIALS`). Returns the first present.
fn load_sa_key_json() -> Option<String> {
    // 1. Base64-encoded JSON — primary. A single safe line for `.env`/Docker
    //    (raw JSON breaks on the multi-line `private_key`).
    if let Ok(b64) = std::env::var("GCS_SA_KEY_BASE64") {
        if !b64.trim().is_empty() {
            let bytes = match STANDARD.decode(b64.trim()) {
                Ok(b) => b,
                Err(e) => {
                    tracing::error!(error = %e, "decoding GCS_SA_KEY_BASE64 failed");
                    return None;
                }
            };
            return match String::from_utf8(bytes) {
                Ok(s) => Some(s),
                Err(e) => {
                    tracing::error!(error = %e, "GCS_SA_KEY_BASE64 did not decode to UTF-8 JSON");
                    None
                }
            };
        }
    }
    // 2. Raw inline JSON.
    if let Ok(inline) = std::env::var("GCS_SA_KEY_JSON") {
        if !inline.trim().is_empty() {
            return Some(inline);
        }
    }
    // 3. Key file path.
    if let Ok(path) = std::env::var("GOOGLE_APPLICATION_CREDENTIALS") {
        return match std::fs::read_to_string(&path) {
            Ok(s) => Some(s),
            Err(e) => {
                tracing::error!(%path, error = %e, "reading GCS service-account key file failed");
                None
            }
        };
    }
    None
}

/// Extract `(client_email, private_key)` from service-account key JSON. Pure (no
/// env/filesystem) so it is unit-testable.
fn parse_sa_key_json(json: &str) -> Option<(String, String)> {
    let v: serde_json::Value = match serde_json::from_str(json) {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(error = %e, "parsing GCS service-account key JSON failed");
            return None;
        }
    };
    let email = v.get("client_email").and_then(|x| x.as_str())?.to_string();
    let key = v.get("private_key").and_then(|x| x.as_str())?.to_string();
    Some((email, key))
}

#[derive(Clone)]
pub struct StorageClient {
    bucket: String,
    host: String,
    location: String,
    client_email: String,
    /// Parsed signing key; `None` when GCS credentials are absent or invalid.
    signer: Option<SigningKey<Sha256>>,
}

impl StorageClient {
    /// Build a client from config. A missing or unparseable key does not fail
    /// construction — it leaves the client unconfigured, so the error surfaces
    /// only when an upload/view URL is actually requested.
    pub fn new(cfg: GcsConfig) -> Self {
        let signer = if cfg.private_key_pem.trim().is_empty() {
            tracing::warn!("GCS credentials not configured; presigning disabled");
            None
        } else {
            match RsaPrivateKey::from_pkcs8_pem(&cfg.private_key_pem) {
                Ok(key) => Some(SigningKey::<Sha256>::new(key)),
                Err(e) => {
                    tracing::error!(error = %e, "parsing GCS private key failed; presigning disabled");
                    None
                }
            }
        };
        Self {
            bucket: cfg.bucket,
            host: cfg.host,
            location: cfg.location,
            client_email: cfg.client_email,
            signer,
        }
    }

    pub fn bucket(&self) -> &str {
        &self.bucket
    }

    /// Build a V4 signed `PUT` URL for `key`, valid for `expires_secs`.
    pub fn presign_put(&self, key: &str, expires_secs: u64, now: DateTime<Utc>) -> Result<String> {
        self.presign("PUT", key, expires_secs, now)
    }

    /// Build a V4 signed `GET` URL for `key` (short-lived viewing — Rule 5).
    pub fn presign_get(&self, key: &str, expires_secs: u64, now: DateTime<Utc>) -> Result<String> {
        self.presign("GET", key, expires_secs, now)
    }

    /// Download an object's bytes (used server-side for AI analysis). Fetches via
    /// a short-lived signed GET so it goes straight to GCS.
    pub async fn fetch_object(&self, key: &str) -> Result<Vec<u8>, String> {
        let url = self
            .presign_get(key, 300, Utc::now())
            .map_err(|e| format!("signing GET URL failed: {e}"))?;
        let resp = reqwest::get(&url)
            .await
            .map_err(|e| format!("storage GET failed: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("storage GET {key} -> HTTP {}", resp.status()));
        }
        resp.bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| format!("reading object bytes failed: {e}"))
    }

    fn presign(
        &self,
        method: &str,
        key: &str,
        expires_secs: u64,
        now: DateTime<Utc>,
    ) -> Result<String> {
        let signer = self
            .signer
            .as_ref()
            .ok_or_else(|| anyhow!("GCS storage is not configured (missing service-account key)"))?;

        let (canonical_querystring, encoded_uri, string_to_sign) =
            self.canonical(method, key, expires_secs, now);

        let signature = signer
            .try_sign(string_to_sign.as_bytes())
            .context("RSA signing of GCS V4 string-to-sign failed")?;
        let signature_hex = hex::encode(signature.to_bytes());

        Ok(format!(
            "https://{}{encoded_uri}?{canonical_querystring}&X-Goog-Signature={signature_hex}",
            self.host
        ))
    }

    /// Build the V4 canonical request and string-to-sign. Returns
    /// `(canonical_querystring, encoded_uri, string_to_sign)`. Split out from
    /// signing so it can be exercised independently in tests.
    fn canonical(
        &self,
        method: &str,
        key: &str,
        expires_secs: u64,
        now: DateTime<Utc>,
    ) -> (String, String, String) {
        let request_time = now.format("%Y%m%dT%H%M%SZ").to_string();
        let date_stamp = now.format("%Y%m%d").to_string();
        let scope = format!("{date_stamp}/{}/storage/goog4_request", self.location);
        let credential = format!("{}/{scope}", self.client_email);

        // Path-style canonical resource: /<bucket>/<object>. Slashes within the
        // object key are preserved (not percent-encoded), per the GCS V4 spec.
        let canonical_uri = format!("/{}/{}", self.bucket, key);
        let encoded_uri = uri_encode(&canonical_uri, false);

        // Query params must be sorted by key and percent-encoded (slashes in the
        // credential are encoded as %2F).
        let canonical_querystring = format!(
            "X-Goog-Algorithm=GOOG4-RSA-SHA256\
             &X-Goog-Credential={}\
             &X-Goog-Date={}\
             &X-Goog-Expires={}\
             &X-Goog-SignedHeaders=host",
            uri_encode(&credential, true),
            request_time,
            expires_secs,
        );

        let canonical_headers = format!("host:{}\n", self.host);
        let canonical_request = format!(
            "{method}\n{encoded_uri}\n{canonical_querystring}\n{canonical_headers}\nhost\nUNSIGNED-PAYLOAD"
        );

        let string_to_sign = format!(
            "GOOG4-RSA-SHA256\n{request_time}\n{scope}\n{}",
            sha256_hex(canonical_request.as_bytes())
        );

        (canonical_querystring, encoded_uri, string_to_sign)
    }
}

fn sha256_hex(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    hex::encode(h.finalize())
}

/// RFC 3986 encoding. Unreserved chars pass through; `/` is kept when
/// `encode_slash` is false (used for the resource path).
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

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;
    use chrono::TimeZone;
    use rsa::pkcs1v15::{Signature, VerifyingKey};
    use rsa::signature::Verifier;

    // Throwaway 2048-bit RSA key generated solely for these tests. Not a real
    // credential — never used outside this module.
    const TEST_KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----
MIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQDb2b7INI66sG4u
aUnNfgu+QIRtVTI/VH7VQr/ls5RKeS64QGiSjn3jmcDJDe5yYDbJkXVZPNdXjw3F
DTXl1TNNTXBcW2IlWDAdYXVmhbA9j2ufkZSCGy2u2t3/xXo/8Vuybv9QVDYP+msY
zPK5fHJHQ+Jw/IwSIlaH225sQHyiqWw88b7j35k2M25tXGQakoO6I/ICRKF09acP
QsIl05KCNc/gnoe7zueiJY7xyR96eqsZLPMEZ/BlqjWTmOBvDTYAnk9gIsD1pEbJ
xkposWjTGvRH15TNnj6qtyuuACqFqQjAD4JIL7/jkkR+oSuCYUl1Isjo3GhaKxq/
F3cLZgH1AgMBAAECggEAGhDrzgxvNMLI2X9ZxP+yED1Rhi4A/mG8BPh8TJV5Wkga
ajWWwHOdCPB4m03nzM9MB6OTpIAybvry0L9zebvIsJxjG0wpa5CUadNdikmE/xOD
WOC3mSJ0TOQ9yH7HMxAghVwkMMe/Pi88Ze0cxTG0Ue9CSI+k4tyH4mU5pyORM27H
OPMOaSVlvz+BJtb/3KqgeEzwSUeCipVJRajgmDOAivZeixqMvIK6H7z1lTzrS97q
UC306LRROvQmyC0B1WLtsXvD7rF2L8o/R4WbPTstSZbKs89GEhvRvZWhVCzg9LLK
hbSFhDyW7xGZaejCwShK62C9Hw2cN2yRRkU/8zBrqQKBgQDuvWgzdUI8vAHlr/uN
MnoLJTkbXjzOBjvntWLAkR0K84bbNn9DqOElDQYt15UFnJV3pDowqzTOPY9VyqC0
am0LSd4epW4IxAd/wc0O5Zt174435DzAfOsYA8sFs1XPbZdx73eVjRi0qsRYoRBp
fARioENfB0ZBVPrrOze3vSG0PQKBgQDrvrw+xJX35WZ5RLdXW7h2HPadJHpaS2vT
iEqP7Bl7yduOcRKgymx1WX7Yw7x47GSU0WDL5cPvGxp2enDg0E6GCybnHuYTqzXv
BvS8JcKeevFrBFVf4QyBh2wD61J50FQZpOnjM2wjX9xo9jpoqTsSWQoP/HaGAMF/
E8Ybn3eIGQKBgQDLmeaKs/64MxJfIvUh1qF0bdtE9tpeTby6/9rnJRqio0soGlLw
jc7svS2+3LLvZTmnHfyuUuQm2yMpZDQniPfT/7wZYLX1bdCDaPELOk+ZhaJBEjmp
Yx9PjLSQ+P8+dlUIytOuS/V85oGqoDmR7D0FK24pVdfZkbKn+AmtffOHdQKBgQCW
Aep67lCXS0ungsg/OEfqMIfkel+4sSFNmv4HEq8si8e/ncfCtMvK6EXE3NhDAfkG
kSzM0cmZjl+KYGMPSr2MofABRTIV9e39ySlVx5lNiuGvvTQuLF/u1CMxYz458Qxy
vPNu9TusTsVLrNKB4p2KEUpL3dP1dOmrWQPIc/J1kQKBgH4k2hbjKYQ8zCEi1Jai
5zmXRx5OZr+puVWrRBjFY3aFKVNuErJinJzms7kY6EcuQzvFZIuiLlgNFsZSB5d6
b4orvu45D2w47+r8JIc8mszOaRKTZWxIvH0kJlq7D7rpNyNhM4pZJ+RSs7F4xYcR
h7FsDMgjGcJldURWe/fduat6
-----END PRIVATE KEY-----
";

    fn test_client() -> StorageClient {
        StorageClient::new(GcsConfig {
            bucket: "ruh-time-tracker".into(),
            host: GCS_HOST.into(),
            location: "auto".into(),
            client_email: "sa@proj.iam.gserviceaccount.com".into(),
            private_key_pem: TEST_KEY_PEM.into(),
        })
    }

    #[test]
    fn canonical_request_is_v4_scoped_and_sorted() {
        let now = Utc.with_ymd_and_hms(2026, 6, 26, 12, 0, 0).unwrap();
        let (qs, uri, sts) = test_client().canonical("PUT", "user/abc.jpg", 900, now);

        assert_eq!(uri, "/ruh-time-tracker/user/abc.jpg");
        assert!(qs.starts_with("X-Goog-Algorithm=GOOG4-RSA-SHA256&X-Goog-Credential="));
        // Credential scope has slashes encoded as %2F.
        assert!(qs.contains("%2F20260626%2Fauto%2Fstorage%2Fgoog4_request"));
        assert!(qs.contains("&X-Goog-Date=20260626T120000Z"));
        assert!(qs.contains("&X-Goog-Expires=900"));
        assert!(qs.ends_with("&X-Goog-SignedHeaders=host"));
        assert!(sts.starts_with(
            "GOOG4-RSA-SHA256\n20260626T120000Z\n20260626/auto/storage/goog4_request\n"
        ));
    }

    #[test]
    fn signed_url_round_trips() {
        let now = Utc.with_ymd_and_hms(2026, 6, 26, 12, 0, 0).unwrap();
        let client = test_client();
        let url = client.presign_put("user/abc.jpg", 900, now).unwrap();

        assert!(url.starts_with("https://storage.googleapis.com/ruh-time-tracker/user/abc.jpg?"));
        assert!(url.contains("X-Goog-Algorithm=GOOG4-RSA-SHA256"));
        assert!(url.contains("&X-Goog-Signature="));

        // The signature must verify as a real RSA-SHA256 signature over the
        // V4 string-to-sign produced for the same request.
        let (_, _, sts) = client.canonical("PUT", "user/abc.jpg", 900, now);
        let sig_hex = url.rsplit("X-Goog-Signature=").next().unwrap();
        let sig = Signature::try_from(hex::decode(sig_hex).unwrap().as_slice()).unwrap();
        let private = RsaPrivateKey::from_pkcs8_pem(TEST_KEY_PEM).unwrap();
        let verifying = VerifyingKey::<Sha256>::new(private.to_public_key());
        verifying
            .verify(sts.as_bytes(), &sig)
            .expect("signed URL signature must verify against the public key");
    }

    #[test]
    fn unconfigured_client_errors() {
        let client = StorageClient::new(GcsConfig {
            bucket: "ruh-time-tracker".into(),
            host: GCS_HOST.into(),
            location: "auto".into(),
            client_email: String::new(),
            private_key_pem: String::new(),
        });
        assert!(client.presign_put("user/abc.jpg", 900, Utc::now()).is_err());
        assert!(client.presign_get("user/abc.jpg", 900, Utc::now()).is_err());
    }

    #[test]
    fn parse_sa_key_json_extracts_email_and_key() {
        // Shape of a real GCS service-account key (private_key embeds newlines —
        // serde escapes them, which is exactly why we ship it base64-encoded).
        let json = serde_json::json!({
            "type": "service_account",
            "client_email": "sa@proj.iam.gserviceaccount.com",
            "private_key": TEST_KEY_PEM,
        })
        .to_string();

        let (email, key) = parse_sa_key_json(&json).expect("valid SA JSON parses");
        assert_eq!(email, "sa@proj.iam.gserviceaccount.com");
        assert_eq!(key, TEST_KEY_PEM);

        // Base64 round-trip (the GCS_SA_KEY_BASE64 path) yields the same JSON.
        let decoded = String::from_utf8(STANDARD.decode(STANDARD.encode(&json)).unwrap()).unwrap();
        assert_eq!(
            parse_sa_key_json(&decoded).unwrap().0,
            "sa@proj.iam.gserviceaccount.com"
        );

        // The recovered key is usable — it signs a V4 URL.
        let client = StorageClient::new(GcsConfig {
            bucket: "ruh-time-tracker".into(),
            host: GCS_HOST.into(),
            location: "auto".into(),
            client_email: email,
            private_key_pem: key,
        });
        assert!(client.presign_put("user/abc.jpg", 900, Utc::now()).is_ok());
    }
}

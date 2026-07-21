//! JWT issuing and verification (Rule 6).
//!
//! Payload shape (per STEP 1):
//! ```json
//! { "sub": "<user-uuid>", "role": "employee", "team": "<team-uuid|null>", "exp": 1234567890 }
//! ```
//! Signing is HS256 (server-side secret) by default. RS256 signing can be
//! enabled additively for cross-system verification (the HRMS integration):
//! when an RSA key is configured the server can sign with RS256 and publishes
//! the public key as a JWKS document, so external verifiers hold no secret and
//! can verify but never mint. Verification always tries RS256 first (when
//! configured) and falls back to HS256, so tokens minted before a signing-alg
//! flip keep working for their remaining TTL. Expiry is validated on decode.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::{Duration, Utc};
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use rsa::pkcs1::DecodeRsaPrivateKey;
use rsa::pkcs8::{DecodePrivateKey, EncodePublicKey, LineEnding};
use rsa::traits::PublicKeyParts;
use rsa::RsaPrivateKey;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::AppError;
use crate::role::UserRole;

/// Issuer/audience the tokens are bound to (SEC-25) — set on issue, required on
/// verify, so a token minted elsewhere for a different audience is rejected.
const JWT_ISSUER: &str = "timetracker-api";
const JWT_AUDIENCE: &str = "timetracker-clients";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    /// Subject — the user id.
    pub sub: String,
    pub role: UserRole,
    /// Team id, if the user belongs to one.
    pub team: Option<String>,
    /// Issuer.
    pub iss: String,
    /// Audience.
    pub aud: String,
    /// Expiry as a UNIX timestamp (seconds).
    pub exp: usize,
    /// Login email — additive claim for external verifiers (display only).
    /// Omitted from the payload when absent so pre-existing consumers see an
    /// unchanged token shape.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub email: Option<String>,
}

/// RS256 signing material, present only when an RSA key is configured.
struct RsaKeys {
    encoding: EncodingKey,
    decoding: DecodingKey,
    kid: String,
    /// Pre-rendered JWKS document (public key only), served verbatim by the
    /// `/.well-known/jwks.json` route.
    jwks_json: String,
}

/// Holds the signing/verification keys and token lifetime. Stored in `AppState`.
pub struct JwtKeys {
    hs_encoding: EncodingKey,
    hs_decoding: DecodingKey,
    rsa: Option<RsaKeys>,
    /// When true (JWT_SIGNING_ALG=RS256) new tokens are signed with RSA;
    /// otherwise HS256 exactly as before. Verification accepts both either way.
    sign_rs256: bool,
    access_ttl_seconds: i64,
}

impl JwtKeys {
    pub fn new(access_secret: &str, access_ttl_seconds: i64) -> Self {
        Self {
            hs_encoding: EncodingKey::from_secret(access_secret.as_bytes()),
            hs_decoding: DecodingKey::from_secret(access_secret.as_bytes()),
            rsa: None,
            sign_rs256: false,
            access_ttl_seconds,
        }
    }

    /// Attach an RS256 keypair (private key PEM, PKCS#8 or PKCS#1). The public
    /// half is derived from it — one env var, no separate public-key config —
    /// and rendered into the JWKS document under `kid`. `sign_rs256` selects
    /// the signing algorithm for NEW tokens; verification always accepts both.
    pub fn with_rs256(
        mut self,
        private_key_pem: &str,
        kid: &str,
        sign_rs256: bool,
    ) -> anyhow::Result<Self> {
        let private = RsaPrivateKey::from_pkcs8_pem(private_key_pem)
            .or_else(|_| RsaPrivateKey::from_pkcs1_pem(private_key_pem))
            .map_err(|e| {
                anyhow::anyhow!("JWT_RS256_PRIVATE_KEY_PEM is not a valid RSA key: {e}")
            })?;

        let public = private.to_public_key();
        let public_pem = public
            .to_public_key_pem(LineEnding::LF)
            .map_err(|e| anyhow::anyhow!("failed to encode RSA public key: {e}"))?;

        let encoding = EncodingKey::from_rsa_pem(private_key_pem.as_bytes())
            .map_err(|e| anyhow::anyhow!("jsonwebtoken rejected the RSA private key: {e}"))?;
        let decoding = DecodingKey::from_rsa_pem(public_pem.as_bytes())
            .map_err(|e| anyhow::anyhow!("jsonwebtoken rejected the RSA public key: {e}"))?;

        // RFC 7517 JWK: modulus + exponent, base64url without padding.
        let jwks_json = serde_json::json!({
            "keys": [{
                "kty": "RSA",
                "use": "sig",
                "alg": "RS256",
                "kid": kid,
                "n": URL_SAFE_NO_PAD.encode(public.n().to_bytes_be()),
                "e": URL_SAFE_NO_PAD.encode(public.e().to_bytes_be()),
            }]
        })
        .to_string();

        self.rsa = Some(RsaKeys {
            encoding,
            decoding,
            kid: kid.to_string(),
            jwks_json,
        });
        self.sign_rs256 = sign_rs256;
        Ok(self)
    }

    /// Access-token lifetime in seconds (returned to clients as `expires_in`).
    pub fn access_ttl_seconds(&self) -> i64 {
        self.access_ttl_seconds
    }

    /// The JWKS document for `/.well-known/jwks.json`, when RS256 is configured.
    pub fn jwks_json(&self) -> Option<&str> {
        self.rsa.as_ref().map(|r| r.jwks_json.as_str())
    }

    /// Issue a signed access token for the given user.
    pub fn issue(
        &self,
        user_id: Uuid,
        role: UserRole,
        team: Option<Uuid>,
        email: Option<String>,
    ) -> Result<String, AppError> {
        let exp = (Utc::now() + Duration::seconds(self.access_ttl_seconds)).timestamp();
        let claims = Claims {
            sub: user_id.to_string(),
            role,
            team: team.map(|t| t.to_string()),
            iss: JWT_ISSUER.to_string(),
            aud: JWT_AUDIENCE.to_string(),
            exp: exp as usize,
            email,
        };
        let result = match (&self.rsa, self.sign_rs256) {
            (Some(rsa), true) => {
                let mut header = Header::new(Algorithm::RS256);
                header.kid = Some(rsa.kid.clone());
                encode(&header, &claims, &rsa.encoding)
            }
            _ => encode(&Header::new(Algorithm::HS256), &claims, &self.hs_encoding),
        };
        result.map_err(|e| AppError::Internal(anyhow::anyhow!("failed to sign jwt: {e}")))
    }

    /// Verify a token's signature, expiry, issuer, and audience, returning its
    /// claims. Tries RS256 first when configured, then HS256 (the pre-flip
    /// path), so a signing-alg change never invalidates live sessions. Any
    /// failure maps to `401 Unauthorized` (never leaks the reason).
    pub fn verify(&self, token: &str) -> Result<Claims, AppError> {
        if let Some(rsa) = &self.rsa {
            let mut validation = Validation::new(Algorithm::RS256);
            validation.set_issuer(&[JWT_ISSUER]);
            validation.set_audience(&[JWT_AUDIENCE]);
            if let Ok(data) = decode::<Claims>(token, &rsa.decoding, &validation) {
                return Ok(data.claims);
            }
        }
        let mut validation = Validation::new(Algorithm::HS256);
        validation.set_issuer(&[JWT_ISSUER]);
        validation.set_audience(&[JWT_AUDIENCE]);
        decode::<Claims>(token, &self.hs_decoding, &validation)
            .map(|data| data.claims)
            .map_err(|_| AppError::Unauthorized)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Throwaway 2048-bit key generated for these tests only — never used
    /// anywhere else. Real keys come from the environment (never committed).
    const TEST_RSA_PEM: &str = "-----BEGIN PRIVATE KEY-----
MIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQCvqkJHLgRqzZ4Z
KJ7+UEK+7fIJ8olFqtBAnsC01IPUAgz8MZ0Ag7oPWgUtjh1NdGrYoM3u6+72KsGl
UOXYAbkzh7ckO1fMrbfD9K5i5ypchvZgeHRxRpLMNwIrNiwtQDmjrS0Vr1HS0gYD
R/gEJTQGtjht2lSiBtpsYMoAgO0mqXLbGov/JNtr54jgec+/7kkCcr3VBg1ukvsB
OT/AjUorTA9+xpvOzXTjh2mEoISiPszIBwRHaM/U18CooXbi9genfsZQHKaiwUUJ
rcR8YFGU5znEf+Kq1zTWCd+ZCAeNfN6fMb4wARoHvxJB9PuXR2wZLA0k44WyqI2D
WWv0g4HVAgMBAAECggEAAzj2HMyrD5OEkDUWz5cE1nBShZyxqHnE9r6Czf5tCa1c
9B5r4SfJeMEfwbFu964ITL+Mb5Lb8B+iLEhy4vbLITce5LcC+hIAwkNBlLZlaEug
hu4GwtI23LCwpT7J6i2YsRg68MAl2d549kPyeI7/KJVcrSU5crx58s2GvyIeV5xx
3XSW2pc1ePcwcbh8TNQhSc4wIPWDAESHJ4omL5Cx8Om9E0OO/NOQs+ReJtq14Keo
jHQ81ylm/tBhJgf637cZF71fDfazDUAvGh+vfsrXC3k+FKiUa14TfUawW1uf6un9
D3Gj9Pj0BN88jcCuBn5f0Cd36GK2cFjPghBRL++cyQKBgQDlFL+GGmc6X9ZVqFYD
58mjih9KwJ2hl6HQ0rG+FDDrNf6GlWvxhVho86+PBfbxMSdA7/RREeHV4rv8Jsgp
INqgCVcN6G6u17xIgpBD9SlyputpI1JHCOuagNSirN4+ui1vI9JofA+nh4Bbm15P
9sO75REiq+2e6RwnNzQvgGIh+QKBgQDETqSh4fWQZrB+dRtvJq1ufliuMNugiNeh
F07fDW5PLZXvVRIASJgfDos9ul3UGI6WGBrxFn5fTz2UMLGO+yBKNtYoOwQLEv6F
9uXLYRS6xUCHIpugAkYhXVCSWjNqD8AVQ/d/TrZOf8PChixAKjKZ/8UXHQAo5AFw
EOFqmz8VvQKBgQCiJKxnS/ry23DG0k6HlyEmqxX5ABrSjJQRTYBXTM1PPrl4Ivzz
R3HLX2hMsGkYULU7BlyEoUaa8FAkaoLUcZetHM9MXc8x3Cs9m4RnRXjf2PvZQBH1
i3/PrQ5UUk1AUuDjwJQmNmGSL5uPu4nmyX/DV6BoTQ70ooS10SIT2SgcaQKBgEF1
8BRDYz254JTqvDE+dorn+u5rQHGx8CW5vgVHV0dz7hGXDp+llYSVtvzojq1RWwrW
EvXeCFoD9IbcDfmd6FCXJKbZtylyNgm6ITrGrsDxUek+MyMSXwx1L47+ITL2+Zid
x4ReXRCkLkVJtKvY5nLQBYsFkq4QCiKynliMWGcJAoGBAJrEiOLDu+YlrNrsjK7J
UJLvPmI5lMm7cX1is7JhzWhwd8AxpWicFNon1Gr0i4hHgDe7KKu4nYse4VxITh2E
o7L5Aj+RiNK6RItJUoE7L+J+hwkzsyuZwe5i+yNBVeVqI1zlIjsTvYSioZOTYtif
Pu1oG3FlQdKJ5oOCmVP5mZTo
-----END PRIVATE KEY-----
";

    fn rs256_keys(sign_rs256: bool) -> JwtKeys {
        JwtKeys::new("test-secret-not-used-for-rs256", 900)
            .with_rs256(TEST_RSA_PEM, "test-kid-1", sign_rs256)
            .unwrap()
    }

    #[test]
    fn issues_and_verifies_round_trip() {
        let keys = JwtKeys::new("test-secret", 900);
        let uid = Uuid::new_v4();
        let token = keys.issue(uid, UserRole::Hr, None, None).unwrap();

        let claims = keys.verify(&token).unwrap();
        assert_eq!(claims.sub, uid.to_string());
        assert_eq!(claims.role, UserRole::Hr);
        assert!(claims.team.is_none());
        assert!(claims.email.is_none());
    }

    #[test]
    fn rejects_token_signed_with_other_secret() {
        let issuer = JwtKeys::new("secret-a", 900);
        let verifier = JwtKeys::new("secret-b", 900);
        let token = issuer
            .issue(Uuid::new_v4(), UserRole::Employee, None, None)
            .unwrap();
        assert!(verifier.verify(&token).is_err());
    }

    #[test]
    fn rejects_expired_token() {
        // Expired well beyond jsonwebtoken's default 60s clock-skew leeway.
        let keys = JwtKeys::new("test-secret", -3600);
        let token = keys
            .issue(Uuid::new_v4(), UserRole::Employee, None, None)
            .unwrap();
        assert!(keys.verify(&token).is_err());
    }

    #[test]
    fn rs256_round_trip_with_kid_header() {
        let keys = rs256_keys(true);
        let uid = Uuid::new_v4();
        let token = keys
            .issue(uid, UserRole::Employee, None, Some("a@ruh.ai".into()))
            .unwrap();

        let header = jsonwebtoken::decode_header(&token).unwrap();
        assert_eq!(header.alg, Algorithm::RS256);
        assert_eq!(header.kid.as_deref(), Some("test-kid-1"));

        let claims = keys.verify(&token).unwrap();
        assert_eq!(claims.sub, uid.to_string());
        assert_eq!(claims.email.as_deref(), Some("a@ruh.ai"));
    }

    #[test]
    fn hs256_tokens_still_verify_after_rs256_flip() {
        // A token minted pre-flip (plain HS256 keys) must validate on keys that
        // now sign RS256 with the SAME HS secret — the no-logout guarantee.
        let pre_flip = JwtKeys::new("shared-hs-secret", 900);
        let token = pre_flip
            .issue(Uuid::new_v4(), UserRole::Hr, None, None)
            .unwrap();

        let post_flip = JwtKeys::new("shared-hs-secret", 900)
            .with_rs256(TEST_RSA_PEM, "test-kid-1", true)
            .unwrap();
        assert!(post_flip.verify(&token).is_ok());
    }

    #[test]
    fn rs256_configured_but_flag_off_signs_hs256() {
        // Ship-with-flag-off mode: key present, signing unchanged.
        let keys = rs256_keys(false);
        let token = keys
            .issue(Uuid::new_v4(), UserRole::Employee, None, None)
            .unwrap();
        let header = jsonwebtoken::decode_header(&token).unwrap();
        assert_eq!(header.alg, Algorithm::HS256);
        assert!(keys.verify(&token).is_ok());
        // The JWKS is still published so external verifiers can pre-wire.
        assert!(keys.jwks_json().is_some());
    }

    #[test]
    fn jwks_document_is_well_formed() {
        let keys = rs256_keys(true);
        let jwks: serde_json::Value = serde_json::from_str(keys.jwks_json().unwrap()).unwrap();
        let key = &jwks["keys"][0];
        assert_eq!(key["kty"], "RSA");
        assert_eq!(key["alg"], "RS256");
        assert_eq!(key["use"], "sig");
        assert_eq!(key["kid"], "test-kid-1");
        assert_eq!(key["e"], "AQAB");
        let n = key["n"].as_str().unwrap();
        assert!(!n.is_empty());
        // base64url without padding: no '+', '/', or '='.
        assert!(!n.contains(['+', '/', '=']));
    }

    #[test]
    fn hs256_only_keys_publish_no_jwks() {
        let keys = JwtKeys::new("test-secret", 900);
        assert!(keys.jwks_json().is_none());
    }
}

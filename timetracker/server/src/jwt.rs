//! JWT issuing and verification (Rule 6).
//!
//! Payload shape (per STEP 1):
//! ```json
//! { "sub": "<user-uuid>", "role": "employee", "team": "<team-uuid|null>", "exp": 1234567890 }
//! ```
//! HS256 with a server-side secret. Expiry is validated on decode.

use chrono::{Duration, Utc};
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::AppError;
use crate::role::UserRole;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    /// Subject — the user id.
    pub sub: String,
    pub role: UserRole,
    /// Team id, if the user belongs to one.
    pub team: Option<String>,
    /// Expiry as a UNIX timestamp (seconds).
    pub exp: usize,
}

/// Holds the signing/verification keys and token lifetime. Stored in `AppState`.
pub struct JwtKeys {
    encoding: EncodingKey,
    decoding: DecodingKey,
    access_ttl_seconds: i64,
}

impl JwtKeys {
    pub fn new(access_secret: &str, access_ttl_seconds: i64) -> Self {
        Self {
            encoding: EncodingKey::from_secret(access_secret.as_bytes()),
            decoding: DecodingKey::from_secret(access_secret.as_bytes()),
            access_ttl_seconds,
        }
    }

    /// Access-token lifetime in seconds (returned to clients as `expires_in`).
    pub fn access_ttl_seconds(&self) -> i64 {
        self.access_ttl_seconds
    }

    /// Issue a signed access token for the given user.
    pub fn issue(
        &self,
        user_id: Uuid,
        role: UserRole,
        team: Option<Uuid>,
    ) -> Result<String, AppError> {
        let exp = (Utc::now() + Duration::seconds(self.access_ttl_seconds)).timestamp();
        let claims = Claims {
            sub: user_id.to_string(),
            role,
            team: team.map(|t| t.to_string()),
            exp: exp as usize,
        };
        encode(&Header::new(Algorithm::HS256), &claims, &self.encoding)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("failed to sign jwt: {e}")))
    }

    /// Verify a token's signature and expiry, returning its claims.
    /// Any failure maps to `401 Unauthorized` (never leaks the reason).
    pub fn verify(&self, token: &str) -> Result<Claims, AppError> {
        let validation = Validation::new(Algorithm::HS256);
        decode::<Claims>(token, &self.decoding, &validation)
            .map(|data| data.claims)
            .map_err(|_| AppError::Unauthorized)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issues_and_verifies_round_trip() {
        let keys = JwtKeys::new("test-secret", 900);
        let uid = Uuid::new_v4();
        let token = keys.issue(uid, UserRole::Hr, None).unwrap();

        let claims = keys.verify(&token).unwrap();
        assert_eq!(claims.sub, uid.to_string());
        assert_eq!(claims.role, UserRole::Hr);
        assert!(claims.team.is_none());
    }

    #[test]
    fn rejects_token_signed_with_other_secret() {
        let issuer = JwtKeys::new("secret-a", 900);
        let verifier = JwtKeys::new("secret-b", 900);
        let token = issuer
            .issue(Uuid::new_v4(), UserRole::Employee, None)
            .unwrap();
        assert!(verifier.verify(&token).is_err());
    }

    #[test]
    fn rejects_expired_token() {
        // Expired well beyond jsonwebtoken's default 60s clock-skew leeway.
        let keys = JwtKeys::new("test-secret", -3600);
        let token = keys
            .issue(Uuid::new_v4(), UserRole::Employee, None)
            .unwrap();
        assert!(keys.verify(&token).is_err());
    }
}

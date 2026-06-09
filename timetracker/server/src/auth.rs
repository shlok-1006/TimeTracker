//! Authentication: Argon2 password hashing and the login flow (Rule 6).

use argon2::password_hash::rand_core::{OsRng, RngCore};
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::db::{refresh_tokens, users};
use crate::error::AppError;
use crate::role::UserRole;
use crate::state::AppState;

/// Generate a high-entropy opaque refresh token (returned to the client once).
fn generate_refresh_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// SHA-256 hash of a refresh token (only the hash is stored — no plaintext).
fn hash_refresh_token(token: &str) -> String {
    let mut h = Sha256::new();
    h.update(token.as_bytes());
    hex::encode(h.finalize())
}

/// Hash a plaintext password with Argon2id (default params) and a random salt.
/// Returns the PHC-encoded string suitable for storage.
pub fn hash_password(password: &str) -> anyhow::Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("failed to hash password: {e}"))?
        .to_string();
    Ok(hash)
}

/// Verify a plaintext password against a stored PHC hash. Constant-time within
/// Argon2; returns `false` for malformed hashes (never panics).
pub fn verify_password(password: &str, password_hash: &str) -> bool {
    match PasswordHash::new(password_hash) {
        Ok(parsed) => Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok(),
        Err(_) => false,
    }
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct AuthenticatedUser {
    pub id: Uuid,
    pub name: String,
    pub email: String,
    pub role: UserRole,
    pub team: Option<Uuid>,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub token_type: &'static str,
    pub expires_in: i64,
    pub user: AuthenticatedUser,
}

#[derive(Debug, Deserialize)]
pub struct RefreshRequest {
    pub refresh_token: String,
}

#[derive(Debug, Serialize)]
pub struct TokenPair {
    pub access_token: String,
    pub refresh_token: String,
    pub token_type: &'static str,
    pub expires_in: i64,
}

/// Mint + persist a new refresh token for `user_id`, returning the plaintext.
async fn issue_refresh_token(state: &AppState, user_id: Uuid) -> Result<String, AppError> {
    let token = generate_refresh_token();
    let expires_at = Utc::now() + Duration::seconds(state.refresh_ttl_seconds);
    refresh_tokens::insert(&state.db, user_id, &hash_refresh_token(&token), expires_at).await?;
    Ok(token)
}

/// Authenticate a user and issue a JWT access token.
///
/// Invalid email and invalid password both return `401 Unauthorized` with the
/// same message — we never reveal which part was wrong. Role enforcement is the
/// responsibility of the guards on protected endpoints and of each client.
pub async fn login(state: &AppState, req: LoginRequest) -> Result<LoginResponse, AppError> {
    let user = users::find_by_email(&state.db, &req.email)
        .await?
        .ok_or(AppError::Unauthorized)?;

    if !verify_password(&req.password, &user.password_hash) {
        return Err(AppError::Unauthorized);
    }

    let access_token = state.jwt.issue(user.id, user.role, user.team_id)?;
    let refresh_token = issue_refresh_token(state, user.id).await?;

    Ok(LoginResponse {
        access_token,
        refresh_token,
        token_type: "Bearer",
        expires_in: state.jwt.access_ttl_seconds(),
        user: AuthenticatedUser {
            id: user.id,
            name: user.name,
            email: user.email,
            role: user.role,
            team: user.team_id,
        },
    })
}

/// Exchange a valid refresh token for a new access token + a NEW refresh token
/// (rotation: the presented token is revoked immediately).
pub async fn refresh(state: &AppState, req: RefreshRequest) -> Result<TokenPair, AppError> {
    let hash = hash_refresh_token(&req.refresh_token);
    let (id, user_id) = refresh_tokens::find_valid(&state.db, &hash)
        .await?
        .ok_or(AppError::Unauthorized)?;

    // Rotate: revoke the used token before issuing a new pair.
    refresh_tokens::revoke(&state.db, id).await?;

    let user = users::find_by_id(&state.db, user_id)
        .await?
        .ok_or(AppError::Unauthorized)?;

    let access_token = state.jwt.issue(user.id, user.role, user.team_id)?;
    let refresh_token = issue_refresh_token(state, user.id).await?;

    Ok(TokenPair {
        access_token,
        refresh_token,
        token_type: "Bearer",
        expires_in: state.jwt.access_ttl_seconds(),
    })
}

/// Revoke a refresh token (logout). Idempotent.
pub async fn logout(state: &AppState, req: RefreshRequest) -> Result<(), AppError> {
    refresh_tokens::revoke_by_hash(&state.db, &hash_refresh_token(&req.refresh_token)).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_and_verifies_password() {
        let hash = hash_password("correct horse battery staple").unwrap();
        assert!(verify_password("correct horse battery staple", &hash));
        assert!(!verify_password("wrong password", &hash));
    }

    #[test]
    fn hash_is_argon2id_phc_and_salted() {
        let hash = hash_password("same-password").unwrap();
        let hash2 = hash_password("same-password").unwrap();
        assert!(hash.starts_with("$argon2id$"));
        // Random salt => identical inputs produce different hashes.
        assert_ne!(hash, hash2);
    }

    #[test]
    fn malformed_hash_does_not_panic() {
        assert!(!verify_password("whatever", "not-a-valid-phc-string"));
    }

    #[test]
    fn refresh_tokens_are_unique_and_hash_is_stable() {
        let a = generate_refresh_token();
        let b = generate_refresh_token();
        assert_eq!(a.len(), 64); // 32 bytes hex
        assert_ne!(a, b); // high entropy
                          // Hash is deterministic for the same token, different across tokens.
        assert_eq!(hash_refresh_token(&a), hash_refresh_token(&a));
        assert_ne!(hash_refresh_token(&a), hash_refresh_token(&b));
        assert_ne!(hash_refresh_token(&a), a); // never store plaintext
    }
}

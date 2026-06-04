//! Authentication: Argon2 password hashing and the login flow (Rule 6).

use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::db::users;
use crate::error::AppError;
use crate::role::UserRole;
use crate::state::AppState;

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
    pub token_type: &'static str,
    pub user: AuthenticatedUser,
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

    Ok(LoginResponse {
        access_token,
        token_type: "Bearer",
        user: AuthenticatedUser {
            id: user.id,
            name: user.name,
            email: user.email,
            role: user.role,
            team: user.team_id,
        },
    })
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
}

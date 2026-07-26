//! Authentication: Argon2 password hashing and the login flow (Rule 6).

use argon2::password_hash::rand_core::{OsRng, RngCore};
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::db::{audit, refresh_tokens, users};
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

/// Generate a strong random temporary password (SEC-33): 20 characters from a
/// CSPRNG over a mixed alphabet (ambiguous chars removed). Replaces the old
/// 32-bit `Tt-<8 hex>!` format; HR hands it over once and the user resets it.
pub fn generate_temp_password() -> String {
    const CHARSET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789!@#$%*?";
    const LEN: usize = 20;
    let mut buf = [0u8; LEN];
    OsRng.fill_bytes(&mut buf);
    buf.iter()
        .map(|b| CHARSET[(*b as usize) % CHARSET.len()] as char)
        .collect()
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

/// A fixed valid Argon2 hash, computed once, used to equalize login timing when
/// the email doesn't exist — otherwise a missing account returns measurably
/// faster than a wrong password, enabling account enumeration (SEC-23).
fn dummy_password_hash() -> &'static str {
    static H: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    H.get_or_init(|| {
        hash_password("timing-equalizer-not-a-real-password").expect("hash dummy password")
    })
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

#[derive(Debug, Deserialize)]
pub struct ChangePasswordRequest {
    pub email: String,
    pub current_password: String,
    pub new_password: String,
}

/// Minimum length for a user-chosen password.
const MIN_PASSWORD_LEN: usize = 8;

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
    let user = match users::find_by_email(&state.db, &req.email).await? {
        Some(u) => u,
        None => {
            // SEC-23: verify against a dummy hash so an unknown email takes the
            // same time as a wrong password (no enumeration by timing).
            let _ = verify_password(&req.password, dummy_password_hash());
            tracing::warn!("login failed: no account for the supplied email");
            return Err(AppError::Unauthorized);
        }
    };

    if !verify_password(&req.password, &user.password_hash) {
        // SEC-22: record the failed attempt against the known account.
        audit::log(
            &state.db,
            user.id,
            "auth.login_failed",
            "user",
            Some(user.id),
        )
        .await;
        return Err(AppError::Unauthorized);
    }

    let access_token =
        state
            .jwt
            .issue(user.id, user.role, user.team_id, Some(user.email.clone()))?;
    let refresh_token = issue_refresh_token(state, user.id).await?;
    audit::log(&state.db, user.id, "auth.login", "user", Some(user.id)).await;

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

/// Verify the current password, set a new one, and return a fresh session.
///
/// Public endpoint (no access token) so a user can rotate a temporary password
/// straight from the login screen. Authenticates exactly like `login` — the same
/// `401` for a bad email or password, with timing equalization against a dummy
/// hash to prevent account enumeration (SEC-23). On success it updates the hash,
/// revokes ALL existing refresh tokens for the user (SEC-18, so any leaked temp
/// credential is fully retired), and issues a new token pair — the caller ends
/// up logged in with the new password.
pub async fn change_password(
    state: &AppState,
    req: ChangePasswordRequest,
) -> Result<LoginResponse, AppError> {
    if req.new_password.chars().count() < MIN_PASSWORD_LEN {
        return Err(AppError::BadRequest(format!(
            "new password must be at least {MIN_PASSWORD_LEN} characters"
        )));
    }
    if req.new_password == req.current_password {
        return Err(AppError::BadRequest(
            "new password must be different from the current password".to_string(),
        ));
    }

    let user = match users::find_by_email(&state.db, &req.email).await? {
        Some(u) => u,
        None => {
            // SEC-23: equalize timing for an unknown email (same as login).
            let _ = verify_password(&req.current_password, dummy_password_hash());
            tracing::warn!("change-password failed: no account for the supplied email");
            return Err(AppError::Unauthorized);
        }
    };

    if !verify_password(&req.current_password, &user.password_hash) {
        audit::log(
            &state.db,
            user.id,
            "auth.change_password_failed",
            "user",
            Some(user.id),
        )
        .await;
        return Err(AppError::Unauthorized);
    }

    let new_hash = hash_password(&req.new_password)?;
    users::set_password(&state.db, user.id, &new_hash).await?;

    // Retire every existing session (SEC-18), then mint a fresh pair.
    refresh_tokens::revoke_all_for_user(&state.db, user.id).await?;
    audit::log(
        &state.db,
        user.id,
        "auth.password_changed",
        "user",
        Some(user.id),
    )
    .await;

    let access_token =
        state
            .jwt
            .issue(user.id, user.role, user.team_id, Some(user.email.clone()))?;
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

/// How recently a rotated token may be re-presented before we treat it as a
/// stolen-token signal. Within this window a re-presented (just-rotated) token
/// is assumed benign — either a concurrent-refresh race, or (the common one) a
/// client that never received its rotated successor because the response was
/// dropped (e.g. a server restart mid-refresh). Instead of forcing a re-login we
/// RECOVER it by minting a fresh pair (see `refresh`). Kept modest so genuine
/// theft is still caught quickly. Widened from 30s → 120s so a client's next
/// retry (heartbeat/sync/session poll cadence) reliably lands inside the window.
const REFRESH_REUSE_GRACE_SECONDS: i64 = 120;

/// Decide whether replaying a non-consumable refresh token is a genuine
/// stolen-token signal (→ revoke every session) or a benign event to ignore.
///
/// It's real reuse only when the family is STILL live (a valid successor token
/// exists, so this old one was truly superseded) AND the replay is outside the
/// grace window (not a just-now concurrent-refresh race). If the family is
/// already fully revoked, this is a stale replay of an incident we've handled —
/// ignoring it stops the retry storm (repeated revoke/audit/warn on every poll).
/// Split out so the policy is unit-tested without a database.
fn is_genuine_reuse(family_live: bool, within_grace: bool) -> bool {
    family_live && !within_grace
}

/// Exchange a valid refresh token for a new access token + a NEW refresh token
/// (rotation: the presented token is revoked immediately).
pub async fn refresh(state: &AppState, req: RefreshRequest) -> Result<TokenPair, AppError> {
    let hash = hash_refresh_token(&req.refresh_token);

    // SEC-17: atomically consume the token (revoke-if-valid in one statement) so
    // two concurrent refreshes can't both succeed on the same token.
    let user_id = match refresh_tokens::consume(&state.db, &hash).await? {
        Some((_id, user_id)) => user_id,
        None => {
            // The token wasn't consumable (missing, revoked, or expired). If the
            // hash is known, classify the replay before crying theft (SEC-17):
            //
            //  * within the grace window  -> benign: a concurrent-refresh race,
            //    or a client that lost its rotated successor (dropped response,
            //    e.g. a server restart mid-refresh). The token was rotated moments
            //    ago and a successor exists that the client never received. We
            //    RECOVER it by minting a fresh pair — no forced re-login — which
            //    is what stops a wave of restart-time logouts.
            //  * family already fully revoked (no live token) -> a stale replay of
            //    an already-handled revocation. The session is genuinely gone.
            //  * rotated LONG ago while the family is STILL live -> the real
            //    stolen-token signal: revoke everything.
            match refresh_tokens::replay_info(&state.db, &hash).await? {
                Some(info) => {
                    let within_grace = info.revoked_at.is_some_and(|t| {
                        Utc::now() - t < Duration::seconds(REFRESH_REUSE_GRACE_SECONDS)
                    });
                    if is_genuine_reuse(info.family_live, within_grace) {
                        refresh_tokens::revoke_all_for_user(&state.db, info.user_id).await?;
                        audit::log(
                            &state.db,
                            info.user_id,
                            "auth.refresh_reuse_detected",
                            "user",
                            Some(info.user_id),
                        )
                        .await;
                        tracing::warn!(user_id = %info.user_id, "refresh token reuse detected — revoked all sessions");
                        return Err(AppError::Unauthorized);
                    }
                    if within_grace {
                        // Recover: fall through with the owner and mint a fresh
                        // pair below (the client stores it and keeps working).
                        tracing::debug!(
                            user_id = %info.user_id,
                            "recovering recently-rotated refresh token (lost successor / concurrent race)"
                        );
                        info.user_id
                    } else {
                        // Stale replay after the family was already revoked.
                        return Err(AppError::Unauthorized);
                    }
                }
                None => return Err(AppError::Unauthorized), // token was never issued
            }
        }
    };

    let user = users::find_by_id(&state.db, user_id)
        .await?
        .ok_or(AppError::Unauthorized)?;

    let access_token =
        state
            .jwt
            .issue(user.id, user.role, user.team_id, Some(user.email.clone()))?;
    let refresh_token = issue_refresh_token(state, user.id).await?;

    Ok(TokenPair {
        access_token,
        refresh_token,
        token_type: "Bearer",
        expires_in: state.jwt.access_ttl_seconds(),
    })
}

/// Revoke a refresh token (logout). Idempotent. Audits the logout (SEC-22).
///
/// Revocation model (SEC-18): access tokens are stateless and can't be revoked
/// mid-flight, so logout revokes the refresh token — no new access token can be
/// minted, and the outstanding one expires within the short, capped access TTL
/// (see `config`). Password reset additionally revokes ALL of a user's refresh
/// tokens (`revoke_all_for_user`).
pub async fn logout(state: &AppState, req: RefreshRequest) -> Result<(), AppError> {
    if let Some(user_id) =
        refresh_tokens::revoke_by_hash(&state.db, &hash_refresh_token(&req.refresh_token)).await?
    {
        audit::log(&state.db, user_id, "auth.logout", "user", Some(user_id)).await;
    }
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

    #[test]
    fn genuine_reuse_only_when_family_live_and_outside_grace() {
        // Real stolen-token replay: an old token used again while a valid
        // successor still exists, long after it was rotated.
        assert!(is_genuine_reuse(true, false));

        // Benign concurrent-refresh race: rotated moments ago (within grace).
        assert!(!is_genuine_reuse(true, true));

        // Stale replay after the family was already revoked — not a new
        // incident, so we must stay quiet (this is what stopped the retry storm
        // of repeated "reuse detected — revoked all sessions" log lines).
        assert!(!is_genuine_reuse(false, false));
        assert!(!is_genuine_reuse(false, true));
    }
}

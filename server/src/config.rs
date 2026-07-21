//! Runtime configuration, loaded from environment variables (see `.env.example`).
//!
//! Configuration is read once at startup into an immutable struct and injected
//! via `AppState` — no global mutable state (Coding Standards).

use std::net::{IpAddr, SocketAddr};

use anyhow::Context;

#[derive(Debug, Clone)]
pub struct Config {
    pub socket_addr: SocketAddr,
    pub database_url: String,
    pub database_max_connections: u32,
    /// HS256 signing secret for JWT access tokens (Rule 6).
    pub jwt_access_secret: String,
    /// Access-token lifetime in seconds.
    pub jwt_access_ttl_seconds: i64,
    /// Refresh-token lifetime in seconds.
    pub jwt_refresh_ttl_seconds: i64,
    /// RSA private key (PEM) for RS256 signing + the JWKS endpoint (HRMS
    /// integration). Optional — absent means HS256-only, exactly as before.
    pub jwt_rs256_private_key_pem: Option<String>,
    /// Key id advertised in the JWT header and the JWKS document.
    pub jwt_kid: String,
    /// True when JWT_SIGNING_ALG=RS256 — new tokens are RSA-signed. The flag is
    /// independent of key presence so the key can ship first (flag off), then
    /// the flip is a one-var change with instant rollback.
    pub jwt_sign_rs256: bool,
}

/// Exact browser origins allowed by CORS (SEC-02) — no wildcard. Comma-separated
/// `CORS_ALLOWED_ORIGINS`, defaulting to the local dev dashboards. The API is a
/// bearer-token API (no cookies), so CORS credentials are never enabled.
pub fn cors_allowed_origins() -> Vec<String> {
    env_or(
        "CORS_ALLOWED_ORIGINS",
        "http://localhost:3001,http://localhost:3002",
    )
    .split(',')
    .map(|s| s.trim().to_string())
    .filter(|s| !s.is_empty())
    .collect()
}

impl Config {
    /// Build configuration from the process environment.
    ///
    /// `dotenvy` has already populated the environment in `main`, so this only
    /// reads `std::env`. Missing required values are hard errors — we never start
    /// with a half-configured server.
    pub fn from_env() -> anyhow::Result<Self> {
        let host: IpAddr = env_or("SERVER_HOST", "0.0.0.0")
            .parse()
            .context("SERVER_HOST is not a valid IP address")?;

        let port: u16 = env_or("SERVER_PORT", "8080")
            .parse()
            .context("SERVER_PORT is not a valid port")?;

        let database_url =
            std::env::var("DATABASE_URL").context("DATABASE_URL must be set (see .env.example)")?;

        let database_max_connections: u32 = env_or("DATABASE_MAX_CONNECTIONS", "10")
            .parse()
            .context("DATABASE_MAX_CONNECTIONS must be an integer")?;

        let jwt_access_secret =
            std::env::var("JWT_ACCESS_SECRET").context("JWT_ACCESS_SECRET must be set")?;
        // SEC-07: refuse to start on a weak/placeholder signing key — HS256
        // security depends entirely on secret strength. Require >=32 chars of
        // high-entropy data (e.g. `openssl rand -base64 48`).
        validate_jwt_secret(&jwt_access_secret)?;

        // SEC-18: a stateless access token can't be revoked before it expires,
        // so keep the lifetime short (default 5 min) and cap it — logout/reset
        // revoke the refresh token, so no new access token can be minted and the
        // outstanding one lapses within this bounded window.
        const MAX_ACCESS_TTL: i64 = 3600;
        let configured_access_ttl: i64 = env_or("JWT_ACCESS_TTL_SECONDS", "300")
            .parse()
            .context("JWT_ACCESS_TTL_SECONDS must be an integer")?;
        if configured_access_ttl < 1 {
            anyhow::bail!("JWT_ACCESS_TTL_SECONDS must be a positive number of seconds");
        }
        let jwt_access_ttl_seconds = if configured_access_ttl > MAX_ACCESS_TTL {
            tracing::warn!(
                "JWT_ACCESS_TTL_SECONDS {configured_access_ttl} exceeds the {MAX_ACCESS_TTL}s \
                 cap; clamping (SEC-18)"
            );
            MAX_ACCESS_TTL
        } else {
            configured_access_ttl
        };

        let jwt_refresh_ttl_seconds: i64 = env_or("JWT_REFRESH_TTL_SECONDS", "2592000")
            .parse()
            .context("JWT_REFRESH_TTL_SECONDS must be an integer")?;

        let jwt_sign_rs256 = parse_signing_alg(&env_or("JWT_SIGNING_ALG", "HS256"))?;

        // PEMs often arrive through env files with literal `\n` sequences —
        // normalize them so both real newlines and escaped ones work.
        let jwt_rs256_private_key_pem = std::env::var("JWT_RS256_PRIVATE_KEY_PEM")
            .ok()
            .map(|v| v.replace("\\n", "\n"))
            .filter(|v| !v.trim().is_empty());

        if jwt_sign_rs256 && jwt_rs256_private_key_pem.is_none() {
            anyhow::bail!(
                "JWT_SIGNING_ALG=RS256 requires JWT_RS256_PRIVATE_KEY_PEM to be set \
                 (generate with `openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048`)"
            );
        }

        let jwt_kid = env_or("JWT_KID", "tt-1");

        Ok(Self {
            socket_addr: SocketAddr::new(host, port),
            database_url,
            database_max_connections,
            jwt_access_secret,
            jwt_access_ttl_seconds,
            jwt_refresh_ttl_seconds,
            jwt_rs256_private_key_pem,
            jwt_kid,
            jwt_sign_rs256,
        })
    }
}

/// Parse JWT_SIGNING_ALG: HS256 (default, today's behavior) or RS256.
/// Anything else is a hard startup error — a typo must not silently fall back.
fn parse_signing_alg(raw: &str) -> anyhow::Result<bool> {
    match raw.trim().to_ascii_uppercase().as_str() {
        "HS256" => Ok(false),
        "RS256" => Ok(true),
        other => anyhow::bail!("JWT_SIGNING_ALG must be HS256 or RS256, got {other:?}"),
    }
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

/// Minimum acceptable length (chars) for the JWT signing secret.
const MIN_JWT_SECRET_LEN: usize = 32;

/// Known placeholder secrets that must never reach production.
const JWT_SECRET_PLACEHOLDERS: [&str; 3] = ["change-me-access", "change-me-refresh", "change-me"];

/// Reject short, placeholder, or low-entropy JWT secrets at startup (SEC-07).
/// Validates the exact string used for signing — not a trimmed copy (RA-13).
fn validate_jwt_secret(secret: &str) -> anyhow::Result<()> {
    if JWT_SECRET_PLACEHOLDERS.contains(&secret.trim()) {
        anyhow::bail!(
            "JWT_ACCESS_SECRET is a placeholder value — set a real secret \
             (generate with `openssl rand -base64 48`)"
        );
    }
    if secret.len() < MIN_JWT_SECRET_LEN {
        anyhow::bail!(
            "JWT_ACCESS_SECRET must be at least {MIN_JWT_SECRET_LEN} characters of \
             high-entropy random data (generate with `openssl rand -base64 48`); \
             refusing to start with a weak secret"
        );
    }
    // Guard against a long-but-trivial secret (e.g. 32 repeated chars): require a
    // minimum number of distinct bytes so the length check can't be gamed (RA-13).
    let distinct = secret
        .bytes()
        .collect::<std::collections::HashSet<u8>>()
        .len();
    if distinct < 8 {
        anyhow::bail!(
            "JWT_ACCESS_SECRET has too few distinct characters ({distinct}); it looks \
             low-entropy — generate a random secret with `openssl rand -base64 48`"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_placeholder_secret() {
        assert!(validate_jwt_secret("change-me-access").is_err());
    }

    #[test]
    fn rejects_short_secret() {
        assert!(validate_jwt_secret("too-short").is_err());
    }

    #[test]
    fn accepts_strong_secret() {
        // 44-char base64-looking value.
        assert!(validate_jwt_secret("Zm9vYmFyYmF6cXV4MTIzNDU2Nzg5MGFiY2RlZmdoaQ==").is_ok());
    }

    #[test]
    fn rejects_long_but_low_entropy_secret() {
        // 40 identical chars: passes length but has 1 distinct byte (RA-13).
        assert!(validate_jwt_secret(&"a".repeat(40)).is_err());
    }

    #[test]
    fn parses_signing_alg() {
        assert!(!parse_signing_alg("HS256").unwrap());
        assert!(parse_signing_alg("RS256").unwrap());
        assert!(parse_signing_alg(" rs256 ").unwrap()); // case/space tolerant
        assert!(parse_signing_alg("ES256").is_err()); // unsupported → hard error
        assert!(parse_signing_alg("").is_err());
    }
}

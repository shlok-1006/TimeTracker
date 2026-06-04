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

        let jwt_access_ttl_seconds: i64 = env_or("JWT_ACCESS_TTL_SECONDS", "900")
            .parse()
            .context("JWT_ACCESS_TTL_SECONDS must be an integer")?;

        Ok(Self {
            socket_addr: SocketAddr::new(host, port),
            database_url,
            database_max_connections,
            jwt_access_secret,
            jwt_access_ttl_seconds,
        })
    }
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

//! In-process rate limiting for authentication endpoints (SEC-08).
//!
//! Fixed-window per-client counter guarding `/auth/*`. The client key is the
//! true client IP:
//!   * Behind a reverse proxy (`RATE_LIMIT_TRUST_PROXY=true`, the default —
//!     deployment is behind nginx) we trust `X-Real-IP`, which our nginx SETS
//!     (overwrites) to the real peer. We deliberately NEVER key on the left-most
//!     `X-Forwarded-For`, which is client-supplied and trivially spoofable
//!     (RA-01) — that would hand every attacker a fresh bucket per request.
//!   * With `RATE_LIMIT_TRUST_PROXY=false` (the API is directly exposed) only
//!     the TCP peer IP is used and all forwarded headers are ignored.
//!
//! The key map is swept of expired windows once it grows large, so a flood of
//! distinct keys can't grow it without bound (RA-06). Unidentifiable clients
//! (e.g. unit tests with no connect info) pass through rather than sharing one
//! bucket.
//!
//! NOTE: `trust_proxy=true` is only safe when the API is reachable *solely* via
//! the proxy (see SEC-04 — bind the API to loopback so nginx is the only path).
//! If the API port is publicly reachable, an attacker can bypass nginx and forge
//! `X-Real-IP`; keep SEC-04 in place.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, Request, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

/// Cap on distinct tracked keys; when exceeded, expired windows are swept so the
/// map stays bounded to roughly the number of active clients (RA-06).
const MAX_TRACKED_KEYS: usize = 100_000;

/// Fixed-window counter: at most `max` requests per `window` per key.
pub struct RateLimiter {
    max: u32,
    window: Duration,
    trust_proxy: bool,
    hits: Mutex<HashMap<String, (Instant, u32)>>,
}

impl RateLimiter {
    pub fn new(max: u32, window: Duration, trust_proxy: bool) -> Self {
        Self {
            max,
            window,
            trust_proxy,
            hits: Mutex::new(HashMap::new()),
        }
    }

    /// From env: `RATE_LIMIT_AUTH_MAX` (default 10) requests per
    /// `RATE_LIMIT_AUTH_WINDOW_SECS` (default 60); `RATE_LIMIT_TRUST_PROXY`
    /// (default true — set to "false" only for direct, proxy-less exposure).
    pub fn from_env() -> Self {
        let max = std::env::var("RATE_LIMIT_AUTH_MAX")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(10);
        let secs = std::env::var("RATE_LIMIT_AUTH_WINDOW_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(60);
        let trust_proxy = std::env::var("RATE_LIMIT_TRUST_PROXY")
            .map(|v| v != "false")
            .unwrap_or(true);
        Self::new(max, Duration::from_secs(secs), trust_proxy)
    }

    /// Record a hit for `key`. Returns `Err(retry_after_secs)` when over limit.
    pub fn check(&self, key: &str, now: Instant) -> Result<(), u64> {
        let mut hits = self.hits.lock().expect("rate limiter mutex poisoned");
        if hits.len() >= MAX_TRACKED_KEYS {
            evict_expired(&mut hits, self.window, now);
        }
        let entry = hits.entry(key.to_string()).or_insert((now, 0));
        if now.duration_since(entry.0) >= self.window {
            *entry = (now, 0); // window elapsed — reset
        }
        if entry.1 >= self.max {
            let elapsed = now.duration_since(entry.0).as_secs();
            return Err(self.window.as_secs().saturating_sub(elapsed).max(1));
        }
        entry.1 += 1;
        Ok(())
    }
}

/// Drop entries whose window has fully elapsed.
fn evict_expired(map: &mut HashMap<String, (Instant, u32)>, window: Duration, now: Instant) {
    map.retain(|_, (start, _)| now.duration_since(*start) < window);
}

/// The true client IP: `X-Real-IP` when the proxy is trusted (nginx overwrites
/// it), else the TCP peer. The left-most `X-Forwarded-For` is never trusted.
fn client_key(headers: &HeaderMap, peer: Option<SocketAddr>, trust_proxy: bool) -> Option<String> {
    if trust_proxy {
        if let Some(xr) = headers.get("x-real-ip").and_then(|v| v.to_str().ok()) {
            let xr = xr.trim();
            if !xr.is_empty() {
                return Some(xr.to_string());
            }
        }
    }
    peer.map(|s| s.ip().to_string())
}

/// Axum middleware: returns 429 (+ `Retry-After`) when the client exceeds the
/// configured auth rate limit.
pub async fn rate_limit(
    State(limiter): State<Arc<RateLimiter>>,
    req: Request,
    next: Next,
) -> Response {
    let peer = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0);
    match client_key(req.headers(), peer, limiter.trust_proxy) {
        Some(k) => match limiter.check(&k, Instant::now()) {
            Ok(()) => next.run(req).await,
            Err(retry) => (
                StatusCode::TOO_MANY_REQUESTS,
                [(header::RETRY_AFTER, retry.to_string())],
                "too many requests — slow down and try again shortly",
            )
                .into_response(),
        },
        None => next.run(req).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_up_to_limit_then_blocks() {
        let rl = RateLimiter::new(3, Duration::from_secs(60), true);
        let t = Instant::now();
        assert!(rl.check("1.2.3.4", t).is_ok());
        assert!(rl.check("1.2.3.4", t).is_ok());
        assert!(rl.check("1.2.3.4", t).is_ok());
        assert!(rl.check("1.2.3.4", t).is_err()); // 4th over the limit
    }

    #[test]
    fn window_resets_after_elapse() {
        let rl = RateLimiter::new(1, Duration::from_secs(60), true);
        let t = Instant::now();
        assert!(rl.check("k", t).is_ok());
        assert!(rl.check("k", t).is_err());
        assert!(rl.check("k", t + Duration::from_secs(61)).is_ok());
    }

    #[test]
    fn keys_are_independent() {
        let rl = RateLimiter::new(1, Duration::from_secs(60), true);
        let t = Instant::now();
        assert!(rl.check("a", t).is_ok());
        assert!(rl.check("b", t).is_ok()); // different key, own budget
    }

    #[test]
    fn spoofed_forwarded_for_is_ignored() {
        // RA-01: a client-supplied X-Forwarded-For must NOT create a fresh key;
        // we fall back to the real peer IP.
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-for", "9.9.9.9, 10.0.0.1".parse().unwrap());
        let peer: SocketAddr = "5.6.7.8:443".parse().unwrap();
        assert_eq!(client_key(&h, Some(peer), true).as_deref(), Some("5.6.7.8"));
    }

    #[test]
    fn trusts_x_real_ip_behind_proxy() {
        let mut h = HeaderMap::new();
        h.insert("x-real-ip", "9.9.9.9".parse().unwrap());
        assert_eq!(client_key(&h, None, true).as_deref(), Some("9.9.9.9"));
    }

    #[test]
    fn ignores_headers_when_not_trusting_proxy() {
        let mut h = HeaderMap::new();
        h.insert("x-real-ip", "9.9.9.9".parse().unwrap());
        let peer: SocketAddr = "5.6.7.8:443".parse().unwrap();
        assert_eq!(client_key(&h, Some(peer), false).as_deref(), Some("5.6.7.8"));
    }

    #[test]
    fn none_when_unidentifiable() {
        assert_eq!(client_key(&HeaderMap::new(), None, true), None);
    }

    #[test]
    fn evicts_expired_entries() {
        let mut m: HashMap<String, (Instant, u32)> = HashMap::new();
        let t = Instant::now();
        m.insert("stale".into(), (t, 1));
        m.insert("fresh".into(), (t + Duration::from_secs(59), 1));
        evict_expired(&mut m, Duration::from_secs(60), t + Duration::from_secs(61));
        assert!(!m.contains_key("stale")); // window elapsed → removed
        assert!(m.contains_key("fresh")); // still within window → kept
    }
}

//! Authenticated HTTP to the API with transparent token refresh.
//!
//! Adds the bearer access token; on `401` it rotates the refresh token once
//! (`auth::do_refresh`) and retries. Keeps tokens out of the webview entirely.
//!
//! Every request the app makes goes through the one [`client`] built here, so
//! the VPN-proofing below applies everywhere (login, sync, screenshots, …).

use std::sync::OnceLock;
use std::time::Duration;

use reqwest::StatusCode;
use serde_json::Value;
use tokio::sync::Mutex;

use crate::auth;

/// Give up on a TCP/TLS connect after this long. A VPN that silently drops our
/// traffic otherwise leaves a worker stuck on the OS default (75s+).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Whole-request ceiling for API calls (small JSON bodies). Screenshot uploads
/// override it per request with [`UPLOAD_TIMEOUT`].
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Ceiling for a screenshot PUT to storage — larger body, slower links.
pub const UPLOAD_TIMEOUT: Duration = Duration::from_secs(120);
/// Drop idle pooled connections quickly: when a VPN connects or disconnects the
/// route changes, and a connection opened on the old route would just hang.
const POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

fn builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .user_agent(concat!("TimeTracker-Desktop/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .pool_idle_timeout(POOL_IDLE_TIMEOUT)
        .tcp_keepalive(Duration::from_secs(30))
}

/// The shared HTTP client.
///
/// Trusts the OS certificate store as well as the bundled Mozilla roots, so a
/// VPN that inspects HTTPS with a company root CA installed on the laptop is
/// accepted (certificates are still fully verified — exactly what the browser
/// does). Follows the OS proxy settings. If the OS store can't be read at all,
/// falls back to the bundled roots alone, which is how every earlier build
/// behaved.
pub fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| match builder().build() {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("OS certificate store unusable ({e}); using bundled roots only");
            builder()
                .tls_built_in_native_certs(false)
                .build()
                .unwrap_or_else(|e| {
                    tracing::error!("HTTP client setup failed ({e}); using defaults");
                    reqwest::Client::new()
                })
        }
    })
}

/// Serializes refresh-token rotation across concurrent requests. Without it,
/// several requests hitting 401 at the same time each rotate the *same* refresh
/// token; the server's reuse-detection then treats the consumed token as a
/// stolen-token replay and revokes ALL sessions, forcing the user to log in
/// again. Single-flight: exactly one task refreshes; the others wait and reuse
/// the freshly-rotated token.
fn refresh_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

async fn send(
    method: reqwest::Method,
    path: &str,
    token: &str,
    body: Option<&Value>,
) -> Result<reqwest::Response, String> {
    let url = format!("{}{}", auth::api_base(), path);
    let mut req = client().request(method, url).bearer_auth(token);
    if let Some(b) = body {
        req = req.json(b);
    }
    req.send()
        .await
        .map_err(|e| format!("request failed: {}", describe(&e)))
}

/// A transport error with its whole cause chain. reqwest's own message stops at
/// "error sending request for url (…)"; the reason — `invalid peer certificate:
/// UnknownIssuer` (a VPN inspecting HTTPS), a DNS failure, a timeout — is in the
/// sources, and it is what tells us why a given VPN blocks the app.
pub fn describe(e: &(dyn std::error::Error + 'static)) -> String {
    let mut out = e.to_string();
    let mut source = e.source();
    while let Some(s) = source {
        out.push_str(": ");
        out.push_str(&s.to_string());
        source = s.source();
    }
    out
}

/// Prefix of the error `parse_json` returns for a non-2xx answer.
const SERVER_RETURNED: &str = "server returned ";

/// Did the server answer — and refuse — this particular request, as opposed to
/// being unreachable, down, or the session needing a refresh? Only such errors
/// mean retrying the same payload is pointless: a 4xx other than 401 (session),
/// 408 (timeout) or 429 (rate limit).
pub fn is_rejection(err: &str) -> bool {
    err.strip_prefix(SERVER_RETURNED)
        .and_then(|rest| rest.get(..3))
        .and_then(|code| code.parse::<u16>().ok())
        .and_then(|code| StatusCode::from_u16(code).ok())
        .is_some_and(is_rejection_status)
}

/// See [`is_rejection`].
pub fn is_rejection_status(status: StatusCode) -> bool {
    status.is_client_error()
        && !matches!(
            status,
            StatusCode::UNAUTHORIZED | StatusCode::REQUEST_TIMEOUT | StatusCode::TOO_MANY_REQUESTS
        )
}

/// Perform an authenticated request, refreshing once on 401.
async fn authed(
    method: reqwest::Method,
    path: &str,
    body: Option<Value>,
) -> Result<reqwest::Response, String> {
    let token = auth::stored_access().ok_or_else(|| "not authenticated".to_string())?;
    let resp = send(method.clone(), path, &token, body.as_ref()).await?;

    if resp.status() == StatusCode::UNAUTHORIZED {
        // Access token likely expired — refresh once (single-flight) and retry.
        {
            let _guard = refresh_lock().lock().await;
            // Another task may have rotated the token while we waited for the
            // lock. Only refresh if the stored token is still the one we just
            // used with a 401 — otherwise reuse the token that task obtained.
            if auth::stored_access().as_deref() == Some(token.as_str()) {
                auth::do_refresh().await?;
            }
        }
        let token = auth::stored_access().ok_or_else(|| "not authenticated".to_string())?;
        return send(method, path, &token, body.as_ref()).await;
    }
    Ok(resp)
}

async fn parse_json(resp: reqwest::Response) -> Result<Value, String> {
    if !resp.status().is_success() {
        return Err(format!("{SERVER_RETURNED}{}", resp.status()));
    }
    resp.json::<Value>().await.map_err(|e| e.to_string())
}

pub async fn get_json(path: &str) -> Result<Value, String> {
    parse_json(authed(reqwest::Method::GET, path, None).await?).await
}

pub async fn post_json(path: &str, body: Value) -> Result<Value, String> {
    parse_json(authed(reqwest::Method::POST, path, Some(body)).await?).await
}

pub async fn patch_json(path: &str, body: Value) -> Result<Value, String> {
    parse_json(authed(reqwest::Method::PATCH, path, Some(body)).await?).await
}

pub async fn delete_json(path: &str) -> Result<Value, String> {
    parse_json(authed(reqwest::Method::DELETE, path, None).await?).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejection_is_a_4xx_the_payload_caused() {
        assert!(is_rejection("server returned 400 Bad Request"));
        assert!(is_rejection("server returned 404 Not Found"));
        assert!(is_rejection("server returned 422 Unprocessable Entity"));
        // Session, timeout and rate limit are not the payload's fault.
        assert!(!is_rejection("server returned 401 Unauthorized"));
        assert!(!is_rejection("server returned 408 Request Timeout"));
        assert!(!is_rejection("server returned 429 Too Many Requests"));
        // Server down / behind a VPN: retry later.
        assert!(!is_rejection("server returned 502 Bad Gateway"));
        assert!(!is_rejection("request failed: error sending request"));
        assert!(!is_rejection("refresh rejected (401 Unauthorized)"));
    }

    #[test]
    fn describe_includes_the_cause_chain() {
        let inner = std::io::Error::new(std::io::ErrorKind::Other, "invalid peer certificate");
        let outer = anyhow::Error::new(inner).context("error sending request");
        let e: &(dyn std::error::Error + 'static) = outer.as_ref();
        assert_eq!(
            describe(e),
            "error sending request: invalid peer certificate"
        );
    }
}

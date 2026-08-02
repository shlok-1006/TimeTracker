//! Vision provider for screenshot analysis — supports the **RUH vision bridge**,
//! **Anthropic Claude**, and **xAI Grok**, selected at startup by which env is present:
//!   * `VISION_BRIDGE_URL` + `VISION_BRIDGE_TOKEN` → RUH bridge (model from
//!     `VISION_BRIDGE_MODEL`, default `claude-haiku-4-5`) — Claude runs on RUH's
//!     subscription, no Anthropic key needed (FOR_SHLOK_VISION_BRIDGE.md)
//!   * `XAI_API_KEY`       → Grok  (model from `XAI_MODEL`, default `grok-4.5`)
//!   * `ANTHROPIC_API_KEY` → Claude (model from `ANTHROPIC_MODEL`)
//! Bridge takes precedence over XAI over Anthropic. Keys/tokens are held
//! server-side only and never exposed to clients.
//!
//! The APIs have different wire shapes — the bridge's `{user, images[], model}` →
//! `{text}`, Anthropic's Messages API, xAI's OpenAI-compatible Chat Completions —
//! so requests and responses are branched per backend. JSON output is requested
//! via the prompt contract itself (callers parse + validate and retry on
//! malformed output).
//!
//! (The type is still named `ClaudeProvider` for historical call-site
//! compatibility; it is the general vision provider.)

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use serde_json::{json, Value};
use std::time::Duration;

const DEFAULT_ANTHROPIC_MODEL: &str = "claude-haiku-4-5-20251001";
const ANTHROPIC_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";

const DEFAULT_XAI_MODEL: &str = "grok-4.5";
const XAI_URL: &str = "https://api.x.ai/v1/chat/completions";

/// The bridge validates models against {claude-haiku-4-5, claude-sonnet-5,
/// claude-opus-4-8}. Haiku is the default here because the analyzer is a
/// high-volume workload (every screenshot of a range job is one call).
const DEFAULT_BRIDGE_MODEL: &str = "claude-haiku-4-5";
/// The bridge runs the model itself (~10–60s per call); its doc says 120s.
const BRIDGE_TIMEOUT: Duration = Duration::from_secs(120);
/// 429 = "bridge busy" (max 3 concurrent) — back off and retry.
const BRIDGE_BUSY_RETRIES: u32 = 3;
const BRIDGE_BUSY_BACKOFF: Duration = Duration::from_secs(5);

/// Generous enough for a short JSON verdict or summary paragraph, bounded to
/// keep cost/latency predictable.
const MAX_TOKENS: u32 = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Backend {
    Bridge,
    Anthropic,
    Xai,
}

#[derive(Debug, thiserror::Error)]
pub enum ClaudeError {
    #[error("Vision AI is not configured (set VISION_BRIDGE_URL+VISION_BRIDGE_TOKEN, XAI_API_KEY, or ANTHROPIC_API_KEY)")]
    NotConfigured,
    #[error("vision request failed: {0}")]
    Http(String),
    #[error("vision API error: {0}")]
    Api(String),
    #[error("vision model returned no text content")]
    Empty,
}

pub struct ClaudeProvider {
    backend: Backend,
    api_key: Option<String>,
    /// Bridge only: the base URL (no trailing slash), e.g. `https://tapan-dev-vm.tail436a50.ts.net`.
    base_url: Option<String>,
    model: String,
    client: reqwest::Client,
}

fn env_nonempty(var: &str) -> Option<String> {
    std::env::var(var).ok().filter(|s| !s.is_empty())
}

fn env_model(var: &str, default: &str) -> String {
    env_nonempty(var).unwrap_or_else(|| default.to_string())
}

impl ClaudeProvider {
    /// Build from the environment. `VISION_BRIDGE_URL`+`VISION_BRIDGE_TOKEN`
    /// selects the RUH bridge; else `XAI_API_KEY` selects Grok; otherwise
    /// `ANTHROPIC_API_KEY` selects Claude. If none is set the provider is
    /// unconfigured (analysis is skipped with a clear error).
    pub fn from_env() -> Self {
        if let (Some(url), Some(token)) = (
            env_nonempty("VISION_BRIDGE_URL"),
            env_nonempty("VISION_BRIDGE_TOKEN"),
        ) {
            return Self {
                backend: Backend::Bridge,
                api_key: Some(token),
                base_url: Some(url.trim_end_matches('/').to_string()),
                model: env_model("VISION_BRIDGE_MODEL", DEFAULT_BRIDGE_MODEL),
                client: reqwest::Client::builder()
                    .timeout(BRIDGE_TIMEOUT)
                    .build()
                    .unwrap_or_else(|_| reqwest::Client::new()),
            };
        }
        if let Some(key) = env_nonempty("XAI_API_KEY") {
            return Self {
                backend: Backend::Xai,
                api_key: Some(key),
                base_url: None,
                model: env_model("XAI_MODEL", DEFAULT_XAI_MODEL),
                client: reqwest::Client::new(),
            };
        }
        Self {
            backend: Backend::Anthropic,
            api_key: env_nonempty("ANTHROPIC_API_KEY"),
            base_url: None,
            model: env_model("ANTHROPIC_MODEL", DEFAULT_ANTHROPIC_MODEL),
            client: reqwest::Client::new(),
        }
    }

    pub fn is_configured(&self) -> bool {
        self.api_key.is_some()
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    /// Send `prompt` + an inline image, requesting a JSON response. Returns the
    /// text of the response (expected to be a JSON object).
    pub async fn generate_json(
        &self,
        prompt: &str,
        image: &[u8],
        image_mime: &str,
    ) -> Result<String, ClaudeError> {
        let body = match self.backend {
            // RUH bridge: `{user, images: [{name, b64}], model}` → `{text}`.
            // b64 is raw base64, no data: prefix; one screenshot per call.
            Backend::Bridge => json!({
                "user": prompt,
                "images": [{
                    "name": if image_mime.contains("png") { "shot.png" } else { "shot.jpg" },
                    "b64": BASE64.encode(image)
                }],
                "model": self.model
            }),
            Backend::Anthropic => json!({
                "model": self.model,
                "max_tokens": MAX_TOKENS,
                "temperature": 0.1,
                "messages": [{
                    "role": "user",
                    "content": [
                        { "type": "image", "source": { "type": "base64", "media_type": image_mime, "data": BASE64.encode(image) } },
                        { "type": "text", "text": prompt }
                    ]
                }]
            }),
            // xAI is OpenAI-compatible: image goes as a data-URL image_url part.
            Backend::Xai => json!({
                "model": self.model,
                "max_tokens": MAX_TOKENS,
                "temperature": 0.1,
                "messages": [{
                    "role": "user",
                    "content": [
                        { "type": "text", "text": prompt },
                        { "type": "image_url", "image_url": { "url": format!("data:{};base64,{}", image_mime, BASE64.encode(image)) } }
                    ]
                }]
            }),
        };
        self.post(body, "/v1/vision").await
    }

    /// Send a TEXT-only prompt requesting a JSON response (no image). Used for
    /// the daily summary generator. The bridge exposes this as `/v1/prompt`.
    pub async fn generate_text_json(&self, prompt: &str) -> Result<String, ClaudeError> {
        let body = match self.backend {
            Backend::Bridge => json!({ "user": prompt, "model": self.model }),
            _ => json!({
                "model": self.model,
                "max_tokens": MAX_TOKENS,
                "temperature": 0.2,
                "messages": [{ "role": "user", "content": prompt }]
            }),
        };
        self.post(body, "/v1/prompt").await
    }

    /// POST the request to the active backend and return the response text.
    /// `bridge_path` picks the bridge endpoint (`/v1/vision` or `/v1/prompt`);
    /// the other backends have a single fixed URL and ignore it.
    async fn post(&self, body: Value, bridge_path: &str) -> Result<String, ClaudeError> {
        let key = self.api_key.as_ref().ok_or(ClaudeError::NotConfigured)?;

        // 429 from the bridge means "busy" (max 3 concurrent) — back off and retry.
        let mut attempt = 0u32;
        let resp = loop {
            let req = match self.backend {
                Backend::Bridge => {
                    let base = self.base_url.as_ref().ok_or(ClaudeError::NotConfigured)?;
                    self.client.post(format!("{base}{bridge_path}")).bearer_auth(key)
                }
                Backend::Anthropic => self
                    .client
                    .post(ANTHROPIC_URL)
                    .header("x-api-key", key)
                    .header("anthropic-version", ANTHROPIC_VERSION),
                Backend::Xai => self.client.post(XAI_URL).bearer_auth(key),
            };

            let resp = req
                .json(&body)
                .send()
                .await
                .map_err(|e| ClaudeError::Http(e.to_string()))?;

            if self.backend == Backend::Bridge
                && resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS
                && attempt < BRIDGE_BUSY_RETRIES
            {
                attempt += 1;
                tokio::time::sleep(BRIDGE_BUSY_BACKOFF * attempt).await;
                continue;
            }
            break resp;
        };

        if !resp.status().is_success() {
            let status = resp.status();
            let detail = resp.text().await.unwrap_or_default();
            return Err(ClaudeError::Api(format!("HTTP {status}: {detail}")));
        }

        let v: Value = resp
            .json()
            .await
            .map_err(|e| ClaudeError::Http(e.to_string()))?;
        let text = match self.backend {
            Backend::Bridge => extract_text_bridge(&v),
            Backend::Anthropic => extract_text_anthropic(&v),
            Backend::Xai => extract_text_openai(&v),
        };
        text.ok_or(ClaudeError::Empty)
    }
}

/// RUH bridge: the reply is `{ "text": "<model's answer>", "model": "..." }`.
fn extract_text_bridge(v: &Value) -> Option<String> {
    let t = v.get("text")?.as_str()?;
    (!t.is_empty()).then(|| t.to_string())
}

/// Anthropic: concatenate the text blocks of the response's `content` array.
fn extract_text_anthropic(v: &Value) -> Option<String> {
    let blocks = v.get("content")?.as_array()?;
    let mut out = String::new();
    for b in blocks {
        if b.get("type").and_then(|t| t.as_str()) == Some("text") {
            if let Some(t) = b.get("text").and_then(|x| x.as_str()) {
                out.push_str(t);
            }
        }
    }
    (!out.is_empty()).then_some(out)
}

/// OpenAI-compatible (xAI): `choices[0].message.content`, which is either a
/// plain string or an array of `{type,text}` parts.
fn extract_text_openai(v: &Value) -> Option<String> {
    let content = v
        .get("choices")?
        .as_array()?
        .first()?
        .get("message")?
        .get("content")?;
    if let Some(s) = content.as_str() {
        return (!s.is_empty()).then(|| s.to_string());
    }
    if let Some(parts) = content.as_array() {
        let mut out = String::new();
        for p in parts {
            if let Some(t) = p.get("text").and_then(|x| x.as_str()) {
                out.push_str(t);
            }
        }
        return (!out.is_empty()).then_some(out);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_anthropic_text_blocks() {
        let v = json!({ "content": [{ "type": "text", "text": "{\"verdict\":\"aligned\"}" }] });
        assert_eq!(
            extract_text_anthropic(&v).unwrap(),
            "{\"verdict\":\"aligned\"}"
        );
    }

    #[test]
    fn extracts_openai_string_content() {
        let v = json!({ "choices": [{ "message": { "content": "{\"verdict\":\"aligned\"}" } }] });
        assert_eq!(
            extract_text_openai(&v).unwrap(),
            "{\"verdict\":\"aligned\"}"
        );
    }

    #[test]
    fn extracts_openai_array_content() {
        let v = json!({ "choices": [{ "message": { "content": [{ "type": "text", "text": "ok" }] } }] });
        assert_eq!(extract_text_openai(&v).unwrap(), "ok");
    }

    #[test]
    fn extracts_bridge_text() {
        let v = json!({ "text": "{\"verdict\":\"aligned\"}", "model": "claude-haiku-4-5" });
        assert_eq!(extract_text_bridge(&v).unwrap(), "{\"verdict\":\"aligned\"}");
    }

    #[test]
    fn missing_text_is_none() {
        assert!(extract_text_anthropic(&json!({ "content": [] })).is_none());
        assert!(extract_text_openai(&json!({ "choices": [] })).is_none());
        assert!(extract_text_openai(&json!({})).is_none());
        assert!(extract_text_bridge(&json!({})).is_none());
        assert!(extract_text_bridge(&json!({ "text": "" })).is_none());
    }
}

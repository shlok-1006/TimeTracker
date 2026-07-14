//! Vision provider for screenshot analysis — supports **Anthropic Claude** and
//! **xAI Grok**, selected at startup by which API key is present:
//!   * `XAI_API_KEY`       → Grok  (model from `XAI_MODEL`, default `grok-4.5`)
//!   * `ANTHROPIC_API_KEY` → Claude (model from `ANTHROPIC_MODEL`)
//! XAI takes precedence if both are set. The key is held server-side only and
//! never exposed to clients.
//!
//! The two APIs have different wire shapes — Anthropic's Messages API vs xAI's
//! OpenAI-compatible Chat Completions — so requests and responses are branched
//! per backend. JSON output is requested via the prompt contract itself
//! (callers parse + validate and retry on malformed output).
//!
//! (The type is still named `ClaudeProvider` for historical call-site
//! compatibility; it is the general vision provider.)

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use serde_json::{json, Value};

const DEFAULT_ANTHROPIC_MODEL: &str = "claude-haiku-4-5-20251001";
const ANTHROPIC_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";

const DEFAULT_XAI_MODEL: &str = "grok-4.5";
const XAI_URL: &str = "https://api.x.ai/v1/chat/completions";

/// Generous enough for a short JSON verdict or summary paragraph, bounded to
/// keep cost/latency predictable.
const MAX_TOKENS: u32 = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Backend {
    Anthropic,
    Xai,
}

#[derive(Debug, thiserror::Error)]
pub enum ClaudeError {
    #[error("Vision AI is not configured (set XAI_API_KEY or ANTHROPIC_API_KEY)")]
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
    /// Build from the environment. `XAI_API_KEY` selects Grok; otherwise
    /// `ANTHROPIC_API_KEY` selects Claude. If neither is set the provider is
    /// unconfigured (analysis is skipped with a clear error).
    pub fn from_env() -> Self {
        if let Some(key) = env_nonempty("XAI_API_KEY") {
            return Self {
                backend: Backend::Xai,
                api_key: Some(key),
                model: env_model("XAI_MODEL", DEFAULT_XAI_MODEL),
                client: reqwest::Client::new(),
            };
        }
        Self {
            backend: Backend::Anthropic,
            api_key: env_nonempty("ANTHROPIC_API_KEY"),
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
        self.post(body).await
    }

    /// Send a TEXT-only prompt requesting a JSON response (no image). Used for
    /// the daily summary generator.
    pub async fn generate_text_json(&self, prompt: &str) -> Result<String, ClaudeError> {
        let body = json!({
            "model": self.model,
            "max_tokens": MAX_TOKENS,
            "temperature": 0.2,
            "messages": [{ "role": "user", "content": prompt }]
        });
        self.post(body).await
    }

    /// POST the request to the active backend and return the response text.
    async fn post(&self, body: Value) -> Result<String, ClaudeError> {
        let key = self.api_key.as_ref().ok_or(ClaudeError::NotConfigured)?;

        let req = match self.backend {
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
            Backend::Anthropic => extract_text_anthropic(&v),
            Backend::Xai => extract_text_openai(&v),
        };
        text.ok_or(ClaudeError::Empty)
    }
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
    fn missing_text_is_none() {
        assert!(extract_text_anthropic(&json!({ "content": [] })).is_none());
        assert!(extract_text_openai(&json!({ "choices": [] })).is_none());
        assert!(extract_text_openai(&json!({})).is_none());
    }
}

//! Anthropic Claude vision provider (screenshot analysis).
//!
//! Sends a multimodal prompt (instruction text + screenshot bytes) to the
//! Claude Messages API and returns the model's raw text response. The API key
//! (`ANTHROPIC_API_KEY`) is held server-side only and never exposed to
//! clients. JSON output is requested via the prompt contract itself (Claude
//! has no `response_format` flag); callers parse + validate the response and
//! retry on malformed output.

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use serde_json::{json, Value};

const DEFAULT_MODEL: &str = "claude-haiku-4-5-20251001";
const API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";
/// Generous enough for a short JSON verdict or summary paragraph, bounded to
/// keep cost/latency predictable.
const MAX_TOKENS: u32 = 1024;

#[derive(Debug, thiserror::Error)]
pub enum ClaudeError {
    #[error("Claude is not configured (set ANTHROPIC_API_KEY)")]
    NotConfigured,
    #[error("Claude request failed: {0}")]
    Http(String),
    #[error("Claude API error: {0}")]
    Api(String),
    #[error("Claude returned no text content")]
    Empty,
}

pub struct ClaudeProvider {
    api_key: Option<String>,
    model: String,
    client: reqwest::Client,
}

impl ClaudeProvider {
    /// Build from the environment: `ANTHROPIC_API_KEY` (required to be active)
    /// and optional `ANTHROPIC_MODEL` (defaults to `claude-haiku-4-5-20251001`).
    pub fn from_env() -> Self {
        Self {
            api_key: std::env::var("ANTHROPIC_API_KEY").ok().filter(|s| !s.is_empty()),
            model: std::env::var("ANTHROPIC_MODEL")
                .ok()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| DEFAULT_MODEL.to_string()),
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
    /// concatenated text of the response (expected to be a JSON object).
    pub async fn generate_json(
        &self,
        prompt: &str,
        image: &[u8],
        image_mime: &str,
    ) -> Result<String, ClaudeError> {
        let body = json!({
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
        });
        self.post_messages(body).await
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
        self.post_messages(body).await
    }

    /// POST a Messages API request body and return the concatenated text content.
    async fn post_messages(&self, body: Value) -> Result<String, ClaudeError> {
        let key = self.api_key.as_ref().ok_or(ClaudeError::NotConfigured)?;

        let resp = self
            .client
            .post(API_URL)
            .header("x-api-key", key)
            .header("anthropic-version", ANTHROPIC_VERSION)
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
        extract_text(&v).ok_or(ClaudeError::Empty)
    }
}

/// Concatenate the text blocks of the response's `content` array.
fn extract_text(v: &Value) -> Option<String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_text_blocks() {
        let v = json!({
            "content": [{ "type": "text", "text": "{\"verdict\":\"aligned\"}" }]
        });
        assert_eq!(extract_text(&v).unwrap(), "{\"verdict\":\"aligned\"}");
    }

    #[test]
    fn missing_text_is_none() {
        assert!(extract_text(&json!({ "content": [] })).is_none());
        assert!(extract_text(&json!({})).is_none());
    }
}

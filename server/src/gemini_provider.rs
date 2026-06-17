//! Google Gemini 2.5 Flash vision provider (STEP 10).
//!
//! Sends a multimodal prompt (instruction text + screenshot bytes) to the Gemini
//! `generateContent` REST endpoint and returns the model's raw text response.
//! The API key (`GEMINI_API_KEY`) is held server-side only and never exposed to
//! clients. JSON-mode is requested via `responseMimeType: application/json`.

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use serde_json::{json, Value};

const DEFAULT_MODEL: &str = "gemini-2.5-flash";
const API_BASE: &str = "https://generativelanguage.googleapis.com/v1beta/models";

#[derive(Debug, thiserror::Error)]
pub enum GeminiError {
    #[error("Gemini is not configured (set GEMINI_API_KEY)")]
    NotConfigured,
    #[error("Gemini request failed: {0}")]
    Http(String),
    #[error("Gemini API error: {0}")]
    Api(String),
    #[error("Gemini returned no text content")]
    Empty,
}

pub struct GeminiProvider {
    api_key: Option<String>,
    model: String,
    client: reqwest::Client,
}

impl GeminiProvider {
    /// Build from the environment: `GEMINI_API_KEY` (required to be active) and
    /// optional `GEMINI_MODEL` (defaults to `gemini-2.5-flash`).
    pub fn from_env() -> Self {
        Self {
            api_key: std::env::var("GEMINI_API_KEY").ok().filter(|s| !s.is_empty()),
            model: std::env::var("GEMINI_MODEL")
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
    /// raw text of the first candidate (expected to be a JSON object).
    pub async fn generate_json(
        &self,
        prompt: &str,
        image: &[u8],
        image_mime: &str,
    ) -> Result<String, GeminiError> {
        let body = json!({
            "contents": [{
                "parts": [
                    { "text": prompt },
                    { "inline_data": { "mime_type": image_mime, "data": BASE64.encode(image) } }
                ]
            }],
            "generationConfig": { "responseMimeType": "application/json", "temperature": 0.1 }
        });
        self.post_generate(body).await
    }

    /// Send a TEXT-only prompt requesting a JSON response (no image). Used for
    /// the daily summary generator (Feature 1 Phase 3).
    pub async fn generate_text_json(&self, prompt: &str) -> Result<String, GeminiError> {
        let body = json!({
            "contents": [{ "parts": [ { "text": prompt } ] }],
            "generationConfig": { "responseMimeType": "application/json", "temperature": 0.2 }
        });
        self.post_generate(body).await
    }

    /// POST a generateContent request body and return the first candidate's text.
    async fn post_generate(&self, body: Value) -> Result<String, GeminiError> {
        let key = self.api_key.as_ref().ok_or(GeminiError::NotConfigured)?;
        let url = format!("{API_BASE}/{}:generateContent?key={key}", self.model);

        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| GeminiError::Http(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let detail = resp.text().await.unwrap_or_default();
            return Err(GeminiError::Api(format!("HTTP {status}: {detail}")));
        }

        let v: Value = resp
            .json()
            .await
            .map_err(|e| GeminiError::Http(e.to_string()))?;
        extract_text(&v).ok_or(GeminiError::Empty)
    }
}

/// Concatenate the text parts of the first candidate.
fn extract_text(v: &Value) -> Option<String> {
    let parts = v.pointer("/candidates/0/content/parts")?.as_array()?;
    let mut out = String::new();
    for p in parts {
        if let Some(t) = p.get("text").and_then(|x| x.as_str()) {
            out.push_str(t);
        }
    }
    (!out.is_empty()).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_candidate_text() {
        let v = json!({
            "candidates": [{ "content": { "parts": [{ "text": "{\"verdict\":\"aligned\"}" }] } }]
        });
        assert_eq!(extract_text(&v).unwrap(), "{\"verdict\":\"aligned\"}");
    }

    #[test]
    fn missing_text_is_none() {
        assert!(extract_text(&json!({ "candidates": [] })).is_none());
        assert!(extract_text(&json!({})).is_none());
    }
}

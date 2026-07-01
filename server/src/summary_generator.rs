//! Daily human-readable summary generation (Feature 1 Phase 3).
//!
//! Takes all per-screenshot rationales for a day and asks an LLM to produce one
//! short manager-facing paragraph (`summary_text` in `analysis_reports`).
//!
//! Design:
//!   * Provider abstraction (`SummaryProvider`) — decouples the generator from
//!     Claude so it can be unit-tested with a mock and swapped later.
//!   * Retry handling — re-prompts on a provider error or malformed output, up
//!     to `MAX_ATTEMPTS`.
//!   * Strict JSON validation — the model must return `{ "summary": "<text>" }`.

use axum::async_trait;
use serde::Deserialize;

use crate::claude_provider::ClaudeProvider;
use crate::error::AppError;

const MAX_ATTEMPTS: usize = 3;
/// Cap how many rationales go into the prompt (keeps it bounded/cheap).
const MAX_RATIONALES: usize = 60;

/// Abstraction over a text LLM that returns a JSON object for a prompt.
#[async_trait]
pub trait SummaryProvider: Send + Sync {
    fn is_configured(&self) -> bool;
    fn model(&self) -> &str;
    /// Complete `prompt`, expecting a JSON object as text. `Err` is a transport
    /// or API failure (retryable by the caller).
    async fn complete_json(&self, prompt: &str) -> Result<String, String>;
}

/// Claude Haiku as the concrete provider.
#[async_trait]
impl SummaryProvider for ClaudeProvider {
    fn is_configured(&self) -> bool {
        ClaudeProvider::is_configured(self)
    }
    fn model(&self) -> &str {
        ClaudeProvider::model(self)
    }
    async fn complete_json(&self, prompt: &str) -> Result<String, String> {
        self.generate_text_json(prompt).await.map_err(|e| e.to_string())
    }
}

/// The model's required JSON contract.
#[derive(Debug, Deserialize)]
struct RawSummary {
    summary: String,
}

/// Build the summary prompt from the day's rationales.
pub fn build_prompt(rationales: &[String]) -> String {
    let list = rationales
        .iter()
        .take(MAX_RATIONALES)
        .enumerate()
        .map(|(i, r)| format!("{}. {}", i + 1, r.trim()))
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "You are summarising one employee's working day for their manager. Below are the \
per-screenshot observations from an automated work-verification system.\n\n\
OBSERVATIONS:\n{list}\n\n\
Write ONE concise, neutral paragraph (2–4 sentences) describing what the employee appeared \
to work on overall and how well it matched their assigned work. Do not invent details beyond \
the observations.\n\n\
Respond with ONLY a JSON object, no markdown:\n{{ \"summary\": \"<your paragraph>\" }}"
    )
}

/// Extract the JSON object from a response, tolerating code fences / prose.
fn extract_json_object(s: &str) -> &str {
    match (s.find('{'), s.rfind('}')) {
        (Some(a), Some(b)) if b > a => &s[a..=b],
        _ => s.trim(),
    }
}

/// Parse + validate the `{ "summary": "..." }` contract.
fn parse_summary(text: &str) -> Result<String, String> {
    let json = extract_json_object(text);
    let raw: RawSummary = serde_json::from_str(json).map_err(|e| format!("invalid JSON ({e})"))?;
    let summary = raw.summary.trim();
    if summary.is_empty() {
        return Err("summary field was empty".into());
    }
    Ok(summary.to_string())
}

/// Generate the daily summary from all rationales.
///
/// * No rationales → a trivial summary (the provider is never called).
/// * Provider not configured → `Err` (the caller falls back to a deterministic
///   summary; see `report_service`).
/// * Otherwise retries up to `MAX_ATTEMPTS` on provider/parse failure.
pub async fn generate_summary(
    provider: &dyn SummaryProvider,
    rationales: &[String],
) -> Result<String, AppError> {
    let rationales: Vec<String> = rationales
        .iter()
        .filter(|r| !r.trim().is_empty())
        .cloned()
        .collect();

    if rationales.is_empty() {
        return Ok("No activity was analysed for this day.".to_string());
    }
    if !provider.is_configured() {
        return Err(AppError::BadRequest(
            "summary AI is not configured (set ANTHROPIC_API_KEY)".into(),
        ));
    }

    let prompt = build_prompt(&rationales);
    let mut last_err = String::new();

    for attempt in 1..=MAX_ATTEMPTS {
        match provider.complete_json(&prompt).await {
            Ok(text) => match parse_summary(&text) {
                Ok(summary) => return Ok(summary),
                Err(e) => {
                    last_err = e;
                    tracing::warn!(attempt, "summary: malformed output: {last_err}");
                }
            },
            Err(e) => {
                last_err = e;
                tracing::warn!(attempt, "summary: provider error: {last_err}");
            }
        }
    }

    Err(AppError::Internal(anyhow::anyhow!(
        "summary generation failed after {MAX_ATTEMPTS} attempts: {last_err}"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Mock provider returning a queued sequence of responses; records call count.
    struct MockProvider {
        configured: bool,
        responses: Mutex<Vec<Result<String, String>>>,
        calls: Mutex<usize>,
    }
    impl MockProvider {
        fn new(configured: bool, responses: Vec<Result<String, String>>) -> Self {
            Self { configured, responses: Mutex::new(responses), calls: Mutex::new(0) }
        }
        fn calls(&self) -> usize {
            *self.calls.lock().unwrap()
        }
    }
    #[async_trait]
    impl SummaryProvider for MockProvider {
        fn is_configured(&self) -> bool {
            self.configured
        }
        fn model(&self) -> &str {
            "mock"
        }
        async fn complete_json(&self, _prompt: &str) -> Result<String, String> {
            *self.calls.lock().unwrap() += 1;
            let mut r = self.responses.lock().unwrap();
            if r.is_empty() {
                Err("no more mock responses".into())
            } else {
                r.remove(0)
            }
        }
    }

    fn r(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_valid_and_fenced() {
        assert_eq!(parse_summary(r#"{"summary":"Worked on auth."}"#).unwrap(), "Worked on auth.");
        let fenced = "```json\n{\"summary\":\"Did X then Y.\"}\n```";
        assert_eq!(parse_summary(fenced).unwrap(), "Did X then Y.");
    }

    #[test]
    fn rejects_bad_json_and_empty_summary() {
        assert!(parse_summary("not json").is_err());
        assert!(parse_summary(r#"{"summary":"   "}"#).is_err());
        assert!(parse_summary(r#"{"notsummary":"x"}"#).is_err());
    }

    #[test]
    fn prompt_includes_rationales() {
        let p = build_prompt(&r(&["edited login.rs", "reviewed PR"]));
        assert!(p.contains("edited login.rs"));
        assert!(p.contains("reviewed PR"));
        assert!(p.contains("\"summary\""));
    }

    #[tokio::test]
    async fn empty_rationales_skip_provider() {
        let mock = MockProvider::new(true, vec![]);
        let out = generate_summary(&mock, &[]).await.unwrap();
        assert!(out.contains("No activity"));
        assert_eq!(mock.calls(), 0, "provider must not be called with no rationales");

        // whitespace-only rationales are filtered → also skipped
        let out2 = generate_summary(&mock, &r(&["  ", ""])).await.unwrap();
        assert!(out2.contains("No activity"));
        assert_eq!(mock.calls(), 0);
    }

    #[tokio::test]
    async fn unconfigured_provider_errors_without_calling() {
        let mock = MockProvider::new(false, vec![Ok(r#"{"summary":"x"}"#.into())]);
        let out = generate_summary(&mock, &r(&["did work"])).await;
        assert!(out.is_err());
        assert_eq!(mock.calls(), 0);
    }

    #[tokio::test]
    async fn succeeds_first_try() {
        let mock = MockProvider::new(true, vec![Ok(r#"{"summary":"Solid progress on the API."}"#.into())]);
        let out = generate_summary(&mock, &r(&["coded the API"])).await.unwrap();
        assert_eq!(out, "Solid progress on the API.");
        assert_eq!(mock.calls(), 1);
    }

    #[tokio::test]
    async fn retries_malformed_then_succeeds() {
        let mock = MockProvider::new(
            true,
            vec![Ok("garbage".into()), Ok(r#"{"summary":"recovered"}"#.into())],
        );
        let out = generate_summary(&mock, &r(&["x"])).await.unwrap();
        assert_eq!(out, "recovered");
        assert_eq!(mock.calls(), 2);
    }

    #[tokio::test]
    async fn retries_provider_error_then_succeeds() {
        let mock = MockProvider::new(
            true,
            vec![Err("503".into()), Ok(r#"{"summary":"ok now"}"#.into())],
        );
        let out = generate_summary(&mock, &r(&["x"])).await.unwrap();
        assert_eq!(out, "ok now");
        assert_eq!(mock.calls(), 2);
    }

    #[tokio::test]
    async fn fails_after_max_attempts() {
        let mock = MockProvider::new(
            true,
            vec![Ok("nope".into()), Ok("still bad".into()), Ok("garbage".into())],
        );
        let out = generate_summary(&mock, &r(&["x"])).await;
        assert!(out.is_err());
        assert_eq!(mock.calls(), MAX_ATTEMPTS);
    }
}

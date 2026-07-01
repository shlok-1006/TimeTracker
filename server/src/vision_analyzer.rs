//! Vision AI screenshot analyzer.
//!
//! Compares a sampled screenshot against an employee's assigned Linear tickets
//! using Claude Haiku, and produces a strictly-validated verdict.
//!
//! Pipeline: build prompt → call provider (JSON mode) → parse + validate →
//! retry on malformed output (up to `MAX_ATTEMPTS`) → apply confidence threshold.
//! Below `CONFIDENCE_THRESHOLD` the verdict is forced to `inconclusive`.

use serde::{Deserialize, Serialize};

use crate::claude_provider::ClaudeProvider;
use crate::error::AppError;
use crate::ticket_cache::Ticket;

const MAX_ATTEMPTS: usize = 3;
const CONFIDENCE_THRESHOLD: f64 = 0.4;

/// The four valid verdicts.
pub const VERDICTS: [&str; 4] = ["aligned", "partially_aligned", "not_aligned", "inconclusive"];

/// A validated analysis result, ready to persist.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AnalysisResult {
    pub verdict: String,
    pub matched_ticket_id: Option<String>,
    pub confidence: f64,
    pub observed: String,
    pub rationale: String,
    pub inconclusive_reason: Option<String>,
    pub model: String,
}

/// Outcome of an analysis attempt. A non-working screenshot is skipped *before*
/// any model call, so it can never produce a stored result (Feature 2 Phase 4).
#[derive(Debug, Clone, PartialEq)]
pub enum AnalysisOutcome {
    Analyzed(AnalysisResult),
    /// The screenshot was not captured while working — analysis was aborted and
    /// Claude was never called.
    SkippedMeetingScreenshot,
}

/// Only screenshots captured while *working* may be analysed. Meeting/break/idle
/// shots are stored and viewable but never sent to the model.
pub fn is_analyzable(captured_status: &str) -> bool {
    captured_status == "working"
}

/// Raw model output (the required JSON contract).
#[derive(Debug, Deserialize)]
struct RawOutput {
    verdict: String,
    #[serde(default)]
    matched_ticket_id: Option<String>,
    #[serde(default)]
    confidence: f64,
    #[serde(default)]
    observed: String,
    #[serde(default)]
    rationale: String,
    #[serde(default)]
    inconclusive_reason: Option<String>,
}

/// Build the analyzer prompt embedding the ticket context as JSON.
pub fn build_prompt(tickets: &[Ticket]) -> String {
    let context: Vec<_> = tickets
        .iter()
        .map(|t| {
            serde_json::json!({
                "id": t.id,
                "title": t.title,
                "state": t.state,
                "labels": t.labels,
                "description_excerpt": t.description_excerpt,
            })
        })
        .collect();
    let tickets_json =
        serde_json::to_string_pretty(&context).unwrap_or_else(|_| "[]".to_string());

    format!(
        "You are a work-verification assistant. Compare the attached SCREENSHOT of an \
employee's screen against their assigned tickets and judge whether the visible activity \
matches the described work.\n\n\
ASSIGNED TICKETS (JSON array):\n{tickets_json}\n\n\
Choose exactly one verdict:\n\
- \"aligned\": the screen clearly shows work on one of the tickets.\n\
- \"partially_aligned\": the work is related but not clearly tied to a specific ticket.\n\
- \"not_aligned\": the screen shows work unrelated to every ticket.\n\
- \"inconclusive\": the screen is ambiguous, blank, locked, or you cannot tell.\n\n\
Respond with ONLY a single JSON object (no markdown, no prose) with EXACTLY these keys:\n\
{{\n\
  \"verdict\": one of [\"aligned\", \"partially_aligned\", \"not_aligned\", \"inconclusive\"],\n\
  \"matched_ticket_id\": the id of the best-matching ticket, or null,\n\
  \"confidence\": a number between 0 and 1 (your certainty in the verdict),\n\
  \"observed\": a short factual description of what is visible on screen,\n\
  \"rationale\": one or two sentences justifying the verdict,\n\
  \"inconclusive_reason\": if verdict is \"inconclusive\", a brief reason; otherwise null\n\
}}"
    )
}

/// Extract the JSON object from a model response, tolerating code fences or
/// surrounding prose (the substring from the first `{` to the last `}`).
fn extract_json_object(s: &str) -> &str {
    match (s.find('{'), s.rfind('}')) {
        (Some(a), Some(b)) if b > a => &s[a..=b],
        _ => s.trim(),
    }
}

/// Parse + strictly validate a raw response against the contract.
fn parse_and_validate(text: &str) -> Result<RawOutput, String> {
    let json = extract_json_object(text);
    let raw: RawOutput =
        serde_json::from_str(json).map_err(|e| format!("invalid JSON ({e})"))?;

    if !VERDICTS.contains(&raw.verdict.as_str()) {
        return Err(format!("invalid verdict: {:?}", raw.verdict));
    }
    if !raw.confidence.is_finite() || !(0.0..=1.0).contains(&raw.confidence) {
        return Err(format!("confidence out of range [0,1]: {}", raw.confidence));
    }
    Ok(raw)
}

/// Apply business rules: drop unknown ticket ids, enforce the confidence
/// threshold, and keep `matched_ticket_id` only for (partially) aligned verdicts.
fn finalize(raw: RawOutput, valid_ids: &[String], model: &str) -> AnalysisResult {
    let matched = raw
        .matched_ticket_id
        .filter(|m| !m.is_empty() && valid_ids.iter().any(|id| id == m));

    let mut verdict = raw.verdict;
    let mut inconclusive_reason = raw.inconclusive_reason.filter(|r| !r.trim().is_empty());

    // Confidence threshold (Requirement): below 0.4 → inconclusive.
    if raw.confidence < CONFIDENCE_THRESHOLD {
        verdict = "inconclusive".to_string();
        if inconclusive_reason.is_none() {
            inconclusive_reason = Some(format!(
                "confidence {:.2} below threshold {CONFIDENCE_THRESHOLD:.2}",
                raw.confidence
            ));
        }
    }

    // A matched ticket only makes sense for (partially) aligned verdicts.
    let matched_ticket_id = match verdict.as_str() {
        "aligned" | "partially_aligned" => matched,
        _ => None,
    };

    AnalysisResult {
        verdict,
        matched_ticket_id,
        confidence: raw.confidence,
        observed: raw.observed,
        rationale: raw.rationale,
        inconclusive_reason,
        model: model.to_string(),
    }
}

/// Analyze one screenshot against the ticket context. Retries on malformed
/// model output, then validates and applies the confidence threshold.
///
/// Phase 4 protection: if `captured_status` is anything other than `working`
/// (e.g. a meeting screenshot accidentally passed in), analysis is aborted
/// immediately with `SkippedMeetingScreenshot` and Claude is never called.
pub async fn analyze_screenshot(
    claude: &ClaudeProvider,
    image: &[u8],
    image_mime: &str,
    captured_status: &str,
    tickets: &[Ticket],
) -> Result<AnalysisOutcome, AppError> {
    // Hard guard FIRST — before the provider check and before any network call.
    if !is_analyzable(captured_status) {
        tracing::info!(
            status = captured_status,
            "analyzer: non-working screenshot skipped (Claude not called)"
        );
        return Ok(AnalysisOutcome::SkippedMeetingScreenshot);
    }

    if !claude.is_configured() {
        return Err(AppError::BadRequest(
            "Vision AI is not configured (set ANTHROPIC_API_KEY)".into(),
        ));
    }

    let prompt = build_prompt(tickets);
    let valid_ids: Vec<String> = tickets.iter().map(|t| t.id.clone()).collect();
    let mut last_err = String::new();

    for attempt in 1..=MAX_ATTEMPTS {
        match claude.generate_json(&prompt, image, image_mime).await {
            Ok(text) => match parse_and_validate(&text) {
                Ok(raw) => {
                    return Ok(AnalysisOutcome::Analyzed(finalize(
                        raw,
                        &valid_ids,
                        claude.model(),
                    )))
                }
                Err(e) => {
                    last_err = e;
                    tracing::warn!(attempt, "vision analysis: malformed output: {last_err}");
                }
            },
            Err(e) => {
                last_err = e.to_string();
                tracing::warn!(attempt, "vision analysis: provider error: {last_err}");
            }
        }
    }

    Err(AppError::Internal(anyhow::anyhow!(
        "vision analysis failed after {MAX_ATTEMPTS} attempts: {last_err}"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ticket(id: &str, title: &str) -> Ticket {
        Ticket {
            id: id.into(),
            title: title.into(),
            state: "In Progress".into(),
            project: None,
            labels: vec!["backend".into()],
            description_excerpt: "do the thing".into(),
        }
    }

    fn ids() -> Vec<String> {
        vec!["ENG-1".to_string(), "ENG-2".to_string()]
    }

    // ---- Phase 4: analyzer protection ----

    #[test]
    fn only_working_status_is_analyzable() {
        assert!(is_analyzable("working"));
        for status in ["meeting", "break", "idle", "not_working", ""] {
            assert!(!is_analyzable(status), "{status:?} must not be analyzable");
        }
    }

    #[tokio::test]
    async fn meeting_screenshot_is_skipped_without_calling_claude() {
        // Bogus bytes + (possibly unconfigured) provider: if Claude were ever
        // called this would error. Instead the guard returns Skipped first.
        let claude = crate::claude_provider::ClaudeProvider::from_env();
        let out = analyze_screenshot(&claude, b"not-an-image", "image/jpeg", "meeting", &[])
            .await
            .expect("guard returns Ok(Skipped), never an error");
        assert_eq!(out, AnalysisOutcome::SkippedMeetingScreenshot);
    }

    #[tokio::test]
    async fn break_idle_notworking_are_also_skipped() {
        let claude = crate::claude_provider::ClaudeProvider::from_env();
        for status in ["break", "idle", "not_working"] {
            let out = analyze_screenshot(&claude, b"x", "image/jpeg", status, &[])
                .await
                .unwrap();
            assert_eq!(out, AnalysisOutcome::SkippedMeetingScreenshot, "status {status}");
        }
    }

    #[test]
    fn prompt_includes_tickets_and_keys() {
        let p = build_prompt(&[ticket("ENG-1", "Fix login")]);
        assert!(p.contains("ENG-1"));
        assert!(p.contains("Fix login"));
        assert!(p.contains("\"verdict\""));
        assert!(p.contains("partially_aligned"));
    }

    #[test]
    fn parses_strict_json() {
        let raw = parse_and_validate(
            r#"{"verdict":"aligned","matched_ticket_id":"ENG-1","confidence":0.9,
                "observed":"VS Code open","rationale":"editing login.rs","inconclusive_reason":null}"#,
        )
        .unwrap();
        assert_eq!(raw.verdict, "aligned");
        assert_eq!(raw.confidence, 0.9);
    }

    #[test]
    fn tolerates_code_fences_and_prose() {
        let text = "Here you go:\n```json\n{\"verdict\":\"not_aligned\",\"confidence\":0.8,\
                    \"observed\":\"x\",\"rationale\":\"y\"}\n```";
        let raw = parse_and_validate(text).unwrap();
        assert_eq!(raw.verdict, "not_aligned");
    }

    #[test]
    fn rejects_unknown_verdict() {
        let err = parse_and_validate(r#"{"verdict":"maybe","confidence":0.5}"#).unwrap_err();
        assert!(err.contains("invalid verdict"));
    }

    #[test]
    fn rejects_out_of_range_confidence() {
        assert!(parse_and_validate(r#"{"verdict":"aligned","confidence":1.5}"#)
            .unwrap_err()
            .contains("out of range"));
    }

    #[test]
    fn rejects_non_json() {
        assert!(parse_and_validate("the screen shows code").is_err());
    }

    #[test]
    fn low_confidence_forced_inconclusive() {
        let raw = parse_and_validate(
            r#"{"verdict":"aligned","matched_ticket_id":"ENG-1","confidence":0.3,
                "observed":"blurry","rationale":"maybe"}"#,
        )
        .unwrap();
        let r = finalize(raw, &ids(), "claude-haiku-4-5-20251001");
        assert_eq!(r.verdict, "inconclusive");
        assert!(r.matched_ticket_id.is_none());
        assert!(r.inconclusive_reason.unwrap().contains("threshold"));
    }

    #[test]
    fn unknown_matched_id_is_dropped() {
        let raw = parse_and_validate(
            r#"{"verdict":"aligned","matched_ticket_id":"ENG-999","confidence":0.9,
                "observed":"x","rationale":"y"}"#,
        )
        .unwrap();
        let r = finalize(raw, &ids(), "m");
        assert_eq!(r.verdict, "aligned");
        assert!(r.matched_ticket_id.is_none()); // ENG-999 not in the supplied set
    }

    #[test]
    fn matched_id_kept_for_aligned() {
        let raw = parse_and_validate(
            r#"{"verdict":"aligned","matched_ticket_id":"ENG-2","confidence":0.95,
                "observed":"x","rationale":"y"}"#,
        )
        .unwrap();
        let r = finalize(raw, &ids(), "m");
        assert_eq!(r.matched_ticket_id.as_deref(), Some("ENG-2"));
    }

    #[test]
    fn matched_id_cleared_for_not_aligned() {
        let raw = parse_and_validate(
            r#"{"verdict":"not_aligned","matched_ticket_id":"ENG-1","confidence":0.9,
                "observed":"x","rationale":"y"}"#,
        )
        .unwrap();
        let r = finalize(raw, &ids(), "m");
        assert!(r.matched_ticket_id.is_none());
    }
}

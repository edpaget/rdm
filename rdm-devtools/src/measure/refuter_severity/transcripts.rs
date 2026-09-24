//! Reading refuter and finder transcripts: the initiating prompt (a bare
//! string on the first `user` turn; later user turns are tool-result arrays)
//! and the last `StructuredOutput` tool call.

use std::path::Path;

use serde_json::Value;

use super::extract::{Context, extract_finder_context, extract_finding, extract_refuter_context};
use crate::measure::jsjson::JsValue;
use crate::measure::sidecar::{read_text, transcript_entries};

/// What a refuter transcript recovers.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RefuterTranscript {
    /// The graded finding's severity, when it is a string.
    pub severity: Option<String>,
    /// The returned verdict (last boolean `refuted` wins).
    pub refuted: Option<bool>,
    /// Dimension and unit identity from the prompt header.
    pub context: Context,
    /// The whole graded finding.
    pub finding: Option<JsValue>,
}

/// What a finder transcript recovers.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FinderTranscript {
    /// The reported findings; `None` only when no `StructuredOutput` was ever
    /// seen (so "unknown" and "zero findings" stay distinct).
    pub findings: Option<Vec<JsValue>>,
    /// The `ac` dimension's structured table, when the last output had one.
    pub ac_table: Option<JsValue>,
    /// Dimension and unit identity from the prompt.
    pub context: Context,
}

impl FinderTranscript {
    /// `findingsCount`: `None` exactly when `findings` is.
    pub fn findings_count(&self) -> Option<usize> {
        self.findings.as_ref().map(Vec::len)
    }
}

fn initiating_prompt(entry: &Value) -> Option<Option<&str>> {
    // Some(Some(p)): the initiating prompt; Some(None): a user line that is
    // not it; None: not a user line.
    if entry.get("type").and_then(Value::as_str) != Some("user") {
        return None;
    }
    Some(
        entry
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(Value::as_str),
    )
}

/// The `input` objects of every `StructuredOutput` tool call on an assistant
/// line, in order.
pub fn structured_outputs(entry: &Value) -> Vec<Value> {
    if entry.get("type").and_then(Value::as_str) != Some("assistant") {
        return Vec::new();
    }
    let Some(content) = entry
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    content
        .iter()
        .filter(|b| {
            b.get("type").and_then(Value::as_str) == Some("tool_use")
                && b.get("name").and_then(Value::as_str) == Some("StructuredOutput")
        })
        .map(|b| match b.get("input") {
            Some(v) if crate::measure::sidecar::truthy(Some(v)) => v.clone(),
            _ => Value::Object(serde_json::Map::new()),
        })
        .collect()
}

/// Reads one refuter transcript; an unreadable file recovers nothing.
pub fn read_refuter_transcript(path: &Path) -> RefuterTranscript {
    let Ok(raw) = read_text(path) else {
        return RefuterTranscript::default();
    };
    let mut out = RefuterTranscript::default();
    let mut seen = false;
    for entry in transcript_entries(&raw) {
        if !seen && let Some(prompt) = initiating_prompt(&entry) {
            let Some(prompt) = prompt else { continue };
            seen = true;
            out.finding = extract_finding(prompt);
            out.severity = out
                .finding
                .as_ref()
                .and_then(|f| f.get("severity"))
                .and_then(JsValue::as_str)
                .map(str::to_owned);
            out.context = extract_refuter_context(prompt);
            continue;
        }
        for input in structured_outputs(&entry) {
            if let Some(b) = input.get("refuted").and_then(Value::as_bool) {
                out.refuted = Some(b);
            }
        }
    }
    out
}

/// Reads one finder transcript; an unreadable file recovers nothing.
pub fn read_finder_transcript(path: &Path) -> FinderTranscript {
    let Ok(raw) = read_text(path) else {
        return FinderTranscript::default();
    };
    let mut out = FinderTranscript::default();
    let mut seen = false;
    for entry in transcript_entries(&raw) {
        if !seen && let Some(prompt) = initiating_prompt(&entry) {
            let Some(prompt) = prompt else { continue };
            seen = true;
            out.context = extract_finder_context(prompt);
            continue;
        }
        for input in structured_outputs(&entry) {
            out.findings = Some(
                input
                    .get("findings")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().map(JsValue::from_json).collect())
                    .unwrap_or_default(),
            );
            out.ac_table = input
                .get("ac")
                .filter(|v| v.is_array())
                .map(JsValue::from_json);
        }
    }
    out
}

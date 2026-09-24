//! Prompts: recovering `refutePrompt`'s inputs from a historical prompt,
//! regenerating a corpus item's prompt through the REAL `refutePrompt`
//! (canonical, via [`ReviewRules`]), and the experiment's own batched prompt.

use super::corpus::{CorpusItem, sha256};
use crate::measure::jsjson::JsValue;
use crate::measure::refuter_severity::extract::{
    FINDING_AGAINST, HEADER_MARKER, SENTINEL, extract_finding, match_brace,
};
use crate::measure::review_rules::ReviewRules;

/// `refutePrompt`'s inputs recovered from a historical prompt.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RefuteInputs {
    /// `code` or `plan`, from the stance sentence.
    pub mode: Option<&'static str>,
    /// The dimension key.
    pub dim_key: Option<String>,
    /// The whole (possibly multi-line) target.
    pub target: Option<String>,
    /// The finding, in its original key order.
    pub finding: Option<JsValue>,
}

/// Index of the finding's opening `{`: the line-anchored brace whose match
/// closes immediately before the sentinel (no JSON parse required).
pub fn find_finding_start(prompt: &str) -> Option<usize> {
    let sentinel_at = prompt.find(SENTINEL)?;
    let close_at = sentinel_at.checked_sub(1)?;
    let bytes = prompt.as_bytes();
    if bytes.get(close_at) != Some(&b'}') {
        return None;
    }
    (0..=close_at).find(|&i| {
        bytes[i] == b'{'
            && (i == 0 || bytes[i - 1] == b'\n')
            && match_brace(prompt, i) == Some(close_at)
    })
}

/// Recovers `refutePrompt(mode, dim, finding, context)`'s inputs. The target
/// is inline and routinely multi-line, so it ends at the `:` before the
/// newline that precedes the finding's line-anchored brace, not at the end of
/// the header line.
pub fn reconstruct_refute_inputs(prompt: &str) -> RefuteInputs {
    let mut out = RefuteInputs::default();
    let finding_at = find_finding_start(prompt);
    if let Some(header_at) = prompt.find(HEADER_MARKER)
        && let Some(rel) = prompt[header_at..].find(FINDING_AGAINST)
    {
        let against_at = header_at + rel;
        out.dim_key = Some(prompt[header_at + HEADER_MARKER.len()..against_at].to_owned());
        let target_from = against_at + FINDING_AGAINST.len();
        match finding_at.filter(|&f| f > target_from) {
            Some(f) => {
                let mut target_to = f - 1;
                if prompt.as_bytes().get(target_to - 1) == Some(&b':') {
                    target_to -= 1;
                }
                out.target = Some(prompt[target_from..target_to].to_owned());
            }
            None => {
                let rest = &prompt[target_from..];
                let line = rest.split('\n').next().unwrap_or(rest);
                out.target = Some(line.strip_suffix(':').unwrap_or(line).to_owned());
            }
        }
    }
    if prompt.contains("unless the code proves otherwise") {
        out.mode = Some("code");
    } else if prompt.contains("unless the plan proves otherwise") {
        out.mode = Some("plan");
    }
    out.finding = extract_finding(prompt);
    out
}

/// Regenerates an item's prompt through the canonical `refutePrompt`.
///
/// # Errors
///
/// When the canonical call fails.
pub fn regenerate_prompt(item: &CorpusItem, rules: &mut dyn ReviewRules) -> Result<String, String> {
    rules.refute_prompt(item.mode(), item.dim(), item.finding(), item.target())
}

/// Whether a regenerated prompt no longer matches the recorded sha
/// (`promptDrift`: `refutePrompt` changed after the item was mined).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fidelity {
    /// The item id.
    pub id: String,
    /// The regenerated prompt's sha differs from the recorded one.
    pub drifted: bool,
    /// The recorded sha.
    pub expected: String,
    /// The regenerated sha.
    pub actual: String,
}

/// Checks one item's prompt fidelity.
///
/// # Errors
///
/// When the canonical call fails.
pub fn check_prompt_fidelity(
    item: &CorpusItem,
    rules: &mut dyn ReviewRules,
) -> Result<Fidelity, String> {
    let actual = sha256(&regenerate_prompt(item, rules)?);
    Ok(Fidelity {
        id: item.id().to_owned(),
        drifted: actual != item.prompt_sha256(),
        expected: item.prompt_sha256().to_owned(),
        actual,
    })
}

/// The batched refuter prompt: a MINIMAL delta from `refutePrompt` (same
/// stance sentence; the dimension's gating findings rendered as one JSON array
/// with a `refute_id` each, and one verdict asked per id). This is the
/// experiment's own prompt, deliberately not in `review.mjs`.
pub fn build_batch_prompt(
    mode: &str,
    dim_key: &str,
    findings: &[JsValue],
    target: Option<&str>,
) -> String {
    let target = target
        .filter(|t| !t.is_empty())
        .unwrap_or("(the target described in your working directory)");
    [
        "You are a READ-ONLY refuter. Do not edit any files.".to_owned(),
        format!("A prior reviewer raised these {dim_key} findings against {target}:"),
        JsValue::Array(findings.to_vec()).stringify_pretty(),
        format!(
            "Start from the stance: this is NOT a real issue unless the {} proves otherwise. Read the actual cited location and its surrounding context before deciding.",
            if mode == "code" { "code" } else { "plan" }
        ),
        "Return JSON matching the BATCH_VERDICT schema: verdicts, an array with ONE entry per refute_id above, each { id (the refute_id), refuted (boolean — true if that finding does not hold up), confidence (0-100 in your verdict), rationale }.".to_owned(),
    ]
    .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multi_line_json_target_is_recovered_whole() {
        let finding = "{\n  \"id\": \"f-2\",\n  \"severity\": \"concern\"\n}";
        let target = "{\n  \"plan\": {\"steps_per_ac\": 2}\n}";
        let prompt = format!(
            "You are a READ-ONLY refuter. Do not edit any files.\nA prior reviewer raised this coherence finding against {target}:\n{finding}\nStart from the stance: this is NOT a real issue unless the plan proves otherwise."
        );
        let r = reconstruct_refute_inputs(&prompt);
        assert_eq!(r.dim_key.as_deref(), Some("coherence"));
        assert_eq!(r.target.as_deref(), Some(target));
        assert_eq!(r.mode, Some("plan"));
        assert_eq!(
            r.finding
                .as_ref()
                .and_then(|f| f.get("id"))
                .and_then(JsValue::as_str),
            Some("f-2")
        );
    }

    #[test]
    fn batch_prompt_is_a_minimal_delta() {
        let f = JsValue::parse(r#"{"refute_id":"a","id":"x"}"#).unwrap_or(JsValue::Null);
        let p = build_batch_prompt("code", "correctness", &[f], Some("task/x"));
        assert!(p.starts_with(
            "You are a READ-ONLY refuter. Do not edit any files.\nA prior reviewer raised these correctness findings against task/x:\n[\n  {\n    \"refute_id\": \"a\""
        ));
        assert!(p.contains("unless the code proves otherwise"));
        let q = build_batch_prompt("plan", "coherence", &[], None);
        assert!(q.contains("against (the target described in your working directory):\n[]"));
    }
}

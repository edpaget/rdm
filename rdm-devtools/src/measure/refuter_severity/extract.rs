//! Recovering a finding, its dimension and its review unit from the prompts
//! `refutePrompt` and `findPrompt` wrote.
//!
//! `refutePrompt` builds exactly:
//!
//! ```text
//! You are a READ-ONLY refuter. Do not edit any files.
//! A prior reviewer raised this <dim> finding against <target>:
//! <JSON.stringify(finding, null, 2)>
//! Start from the stance: ...
//! ```
//!
//! `<target>` is inline and may itself be pretty-printed JSON (the
//! `--implementation-plan` target), so "the first `{`" finds the target. The
//! finding is instead the brace-matched object that starts a line and closes
//! immediately before the `Start from the stance:` sentinel.
//!
//! Byte offsets are used throughout; every delimiter is ASCII, so they agree
//! with the JavaScript UTF-16 offsets the original used. Plausibility lengths
//! are measured in UTF-16 units, as in JavaScript.

use crate::measure::jsjson::{JsValue, js_len};

/// The sentinel line that follows the finding JSON.
pub const SENTINEL: &str = "\nStart from the stance:";
/// The fixed header marker before the dimension key.
pub const HEADER_MARKER: &str = "A prior reviewer raised this ";
/// The separator between the dimension key and the target.
pub const FINDING_AGAINST: &str = " finding against ";
/// `findPrompt`'s target marker.
pub const FIND_TARGET_MARKER: &str = "Review target: ";
/// The two `diffHint` lines that terminate `findPrompt`'s target.
pub const FIND_DIFF_HINTS: [&str; 2] = [
    "\nInspect the implementation diff (use git log / git diff in the worktree).",
    "\nInspect the plan document text.",
];
/// `findPrompt`'s dimension marker.
pub const FIND_DIMENSION_MARKER: &str = "Your single dimension is ";
/// A candidate unit identity longer than this (UTF-16 units) is rejected.
pub const MAX_UNIT_IDENT_LENGTH: usize = 200;

/// Forward brace-match from `start` (which must index a `{`), honouring JSON
/// string literals and escapes. Returns the matching `}`'s index.
pub fn match_brace(text: &str, start: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut depth: i64 = 0;
    let mut in_string = false;
    let mut escaped = false;
    for (i, &ch) in bytes.iter().enumerate().skip(start) {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == b'\\' {
                escaped = true;
            } else if ch == b'"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// The finding object's `(open, close)` brace indices in a refuter prompt.
pub fn locate_finding_span(prompt: &str) -> Option<(usize, usize)> {
    let sentinel_at = prompt.find(SENTINEL)?;
    let close_at = sentinel_at.checked_sub(1)?;
    let bytes = prompt.as_bytes();
    if bytes.get(close_at) != Some(&b'}') {
        return None;
    }
    for i in 0..=close_at {
        if bytes[i] != b'{' || (i != 0 && bytes[i - 1] != b'\n') {
            continue;
        }
        if match_brace(prompt, i) != Some(close_at) {
            continue;
        }
        if matches!(
            JsValue::parse(&prompt[i..=close_at]),
            Ok(JsValue::Object(_))
        ) {
            return Some((i, close_at));
        }
    }
    None
}

/// The finding embedded in a refuter prompt, in its original key order.
pub fn extract_finding(prompt: &str) -> Option<JsValue> {
    let (start, end) = locate_finding_span(prompt)?;
    JsValue::parse(&prompt[start..=end]).ok()
}

/// A candidate unit identity, or `None` when it is empty, carries raw JSON
/// structure (`{` or `"` — the JSON-target shape) or is implausibly long.
/// The one unit-identity rule every instrument shares.
pub fn unit_ident(candidate: &str) -> Option<String> {
    if candidate.is_empty()
        || candidate.contains(['{', '"'])
        || js_len(candidate) > MAX_UNIT_IDENT_LENGTH
    {
        return None;
    }
    Some(candidate.to_owned())
}

/// A prompt's recovered dimension key and unit identity.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Context {
    /// The dimension key.
    pub dim_key: Option<String>,
    /// The review-unit identity.
    pub unit_ident: Option<String>,
}

/// The first line of `target` when it is multi-line (plan mode), else the
/// single line with one trailing `suffix` stripped (code mode).
fn first_line_or_stripped(target: &str, suffix: char) -> &str {
    match target.find('\n') {
        Some(at) => &target[..at],
        None => target.strip_suffix(suffix).unwrap_or(target),
    }
}

/// The dimension and unit identity from a refuter prompt's header line.
pub fn extract_refuter_context(prompt: &str) -> Context {
    let Some((start, _)) = locate_finding_span(prompt) else {
        return Context::default();
    };
    let header = &prompt[..start];
    let header = header.strip_suffix('\n').unwrap_or(header);
    let Some(marker_at) = header.find(HEADER_MARKER) else {
        return Context::default();
    };
    let after = &header[marker_at + HEADER_MARKER.len()..];
    let Some(sep_at) = after.find(FINDING_AGAINST) else {
        return Context::default();
    };
    let dim_key = Some(&after[..sep_at])
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    let target = &after[sep_at + FINDING_AGAINST.len()..];
    Context {
        dim_key,
        unit_ident: unit_ident(first_line_or_stripped(target, ':')),
    }
}

/// The parenthesised key on the `Your single dimension is <title> (<key>).`
/// line: the first `(...)` on that line with no nested parentheses.
fn find_dimension_key(rest: &str) -> Option<String> {
    let line = rest.split('\n').next().unwrap_or("");
    // JS: /^[^\n]*?\(([^()\n]+)\)/ — the leftmost `(` whose content up to the
    // next `)` holds no `(`, `)` or newline.
    let bytes = line.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b != b'(' {
            continue;
        }
        let inner_start = i + 1;
        let mut j = inner_start;
        while j < bytes.len() && bytes[j] != b'(' && bytes[j] != b')' {
            j += 1;
        }
        if j < bytes.len() && bytes[j] == b')' && j > inner_start {
            return Some(line[inner_start..j].to_owned());
        }
    }
    None
}

/// The dimension and unit identity from a finder's own prompt. The target is
/// `context.target` exactly as the refuter header interpolates it, so both
/// sides of the finder↔refuter join recover the same identity.
pub fn extract_finder_context(prompt: &str) -> Context {
    let Some(marker_at) = prompt.find(FIND_TARGET_MARKER) else {
        return Context::default();
    };
    let rest = &prompt[marker_at + FIND_TARGET_MARKER.len()..];
    let Some(end) = FIND_DIFF_HINTS.iter().filter_map(|h| rest.find(h)).min() else {
        return Context::default();
    };
    let target = &rest[..end];
    let dim_key = prompt[marker_at..]
        .find(FIND_DIMENSION_MARKER)
        .and_then(|at| find_dimension_key(&prompt[marker_at + at + FIND_DIMENSION_MARKER.len()..]));
    Context {
        dim_key,
        unit_ident: unit_ident(first_line_or_stripped(target, '.')),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refute_prompt(dim: &str, target: &str, finding: &str) -> String {
        format!(
            "You are a READ-ONLY refuter. Do not edit any files.\nA prior reviewer raised this {dim} finding against {target}:\n{finding}\nStart from the stance: this is NOT a real issue."
        )
    }

    #[test]
    fn finding_extraction_requires_sentinel_anchor() {
        let finding = "{\n  \"id\": \"f-1\",\n  \"severity\": \"blocking\"\n}";
        // A braced, multi-line JSON target precedes the finding.
        let target = "the plan {\n  \"steps\": {\"a\": 1}\n}";
        let prompt = refute_prompt("coherence", target, finding);
        let got = extract_finding(&prompt).unwrap_or(JsValue::Null);
        assert_eq!(got.get("id").and_then(JsValue::as_str), Some("f-1"));
        assert_eq!(
            got.get("severity").and_then(JsValue::as_str),
            Some("blocking")
        );
        // Without the sentinel line nothing is recovered.
        let broken = prompt.replace("\nStart from the stance:", "\nNEVER MATCHES THIS SENTINEL:");
        assert_eq!(extract_finding(&broken), None);
        assert_eq!(extract_refuter_context(&broken), Context::default());
        // A finding whose close does not immediately precede the sentinel is
        // not recovered either.
        let gap = prompt.replace("}\nStart from", "}\n\nStart from");
        assert_eq!(extract_finding(&gap), None);
    }

    #[test]
    fn unit_and_dimension_come_from_prompt_header() {
        let f = "{\n  \"id\": \"x\"\n}";
        let code = refute_prompt("correctness", "widget/phase-1-alpha", f);
        assert_eq!(
            extract_refuter_context(&code),
            Context {
                dim_key: Some("correctness".to_owned()),
                unit_ident: Some("widget/phase-1-alpha".to_owned())
            },
            "code mode strips the trailing colon on the single-line target"
        );
        let plan = refute_prompt("coherence", "phase widget/phase-6-zeta\n\nbody text", f);
        assert_eq!(
            extract_refuter_context(&plan).unit_ident.as_deref(),
            Some("phase widget/phase-6-zeta"),
            "plan mode keeps the first line"
        );
        let json_target = refute_prompt("coherence", "{\n  \"plan\": 1\n}", f);
        assert_eq!(extract_refuter_context(&json_target).unit_ident, None);
        let broken = code.replace(HEADER_MARKER, "NEVER MATCHES THIS MARKER");
        assert_eq!(extract_refuter_context(&broken), Context::default());
    }

    #[test]
    fn json_target_is_not_a_unit_identity() {
        assert_eq!(unit_ident("{"), None);
        assert_eq!(unit_ident("a \"quoted\" plan"), None);
        assert_eq!(unit_ident(""), None);
        assert_eq!(unit_ident(&"x".repeat(201)), None);
        assert_eq!(unit_ident(&"x".repeat(200)).map(|s| s.len()), Some(200));
    }

    #[test]
    fn finder_and_refuter_resolve_byte_identical_unit_key() {
        let finder = |target: &str, hint: &str| {
            format!(
                "You are a reviewer.\nReview target: {target}.{hint}\nYour single dimension is Correctness & error handling (correctness).\n"
            )
        };
        let code_finder = finder(
            "widget/phase-1-alpha",
            "\nInspect the implementation diff (use git log / git diff in the worktree).",
        );
        let code_refuter =
            refute_prompt("correctness", "widget/phase-1-alpha", "{\n  \"id\": 1\n}");
        let fc = extract_finder_context(&code_finder);
        assert_eq!(fc.unit_ident.as_deref(), Some("widget/phase-1-alpha"));
        assert_eq!(fc.dim_key.as_deref(), Some("correctness"));
        assert_eq!(
            fc.unit_ident,
            extract_refuter_context(&code_refuter).unit_ident
        );

        let body = "phase widget/phase-6-zeta\n\nThe plan body. It ends here";
        let plan_finder = finder(body, "\nInspect the plan document text.");
        let plan_refuter = refute_prompt("coherence", body, "{\n  \"id\": 1\n}");
        let fp = extract_finder_context(&plan_finder);
        assert_eq!(fp.unit_ident.as_deref(), Some("phase widget/phase-6-zeta"));
        assert_eq!(
            fp.unit_ident,
            extract_refuter_context(&plan_refuter).unit_ident
        );
        assert_eq!(extract_finder_context("no marker"), Context::default());
    }
}

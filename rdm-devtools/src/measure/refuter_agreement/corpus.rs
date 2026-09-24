//! The adjudicated finding corpus: schema, loading, and composition.
//!
//! Every field is required and unknown top-level keys are rejected, so a
//! typo'd hand edit cannot pass as `undefined` into the scorer. Ground truth is
//! adjudicated against the pinned tree recorded in
//! `groundTruth.adjudicatedAgainstCommit`, the tree a replay actually reads.

use serde::Serialize;

use crate::measure::jsjson::{JsMap, JsValue};
use crate::measure::jsnum::{js_round, js_trim};

/// Bumped whenever the record shape changes incompatibly.
pub const CORPUS_SCHEMA_VERSION: f64 = 1.0;

/// The closed ground-truth classes.
pub const GROUND_TRUTH_CLASSES: [&str; 6] = [
    "real-defect",
    "mechanically-true-not-a-defect",
    "false-premise",
    "stale-fact",
    "misread-scope",
    "style-preference",
];

/// The divergence class the corpus is deliberately weighted toward.
pub const DIVERGENCE_CLASS: &str = "mechanically-true-not-a-defect";
/// Legal `groundTruth.authority` values.
pub const AUTHORITIES: [&str; 2] = ["authoritative", "judgement-call"];
/// Legal modes.
pub const MODES: [&str; 2] = ["code", "plan"];
/// Legal `provenance.kind` values.
pub const PROVENANCE_KINDS: [&str; 2] = ["mined", "constructed"];
/// Severities that still spawn a refuter.
pub const GATING_SEVERITIES: [&str; 2] = ["blocking", "concern"];
/// Severities recorded but excluded from the headline rates (no refuter is
/// spawned for them since `NON_GATING_SEVERITIES = ['suggestion']`).
pub const HISTORICAL_ONLY_SEVERITIES: [&str; 1] = ["suggestion"];

/// Corpus floor: minimum size.
pub const MIN_CORPUS_SIZE: usize = 45;
/// Corpus floor: divergence-class share.
pub const MIN_DIVERGENCE_CLASS_SHARE: f64 = 0.35;
/// Corpus floor: mined share.
pub const MIN_MINED_SHARE: f64 = 0.6;
/// Corpus floor: authoritative share.
pub const MIN_AUTHORITATIVE_SHARE: f64 = 0.5;

/// The four token classes, in report order.
pub const TOKEN_CLASSES: [&str; 4] = ["output", "uncachedInput", "cacheWrite", "cacheRead"];

const FINDING_FIELDS: [&str; 6] = [
    "id",
    "concern",
    "location",
    "severity",
    "confidence",
    "what_fails",
];

const TOP_LEVEL_FIELDS: [&str; 10] = [
    "id",
    "schemaVersion",
    "mode",
    "dim",
    "target",
    "finding",
    "promptSha256",
    "promptDrift",
    "provenance",
    "groundTruth",
];

/// sha256 hex of a UTF-8 string.
pub fn sha256(text: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(text.as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// The authoritative-evidence rule: evidence must cite a source path
/// (`.rs`/`.mjs`/`.js`/`.sh`/`.md`/`.json`/`.toml`), a 7–40 char hex sha, a
/// `§` reference, or an `AC-`/`AC<digit>` criterion — the JS
/// `AUTHORITATIVE_EVIDENCE_RE`, with its ASCII word boundaries.
pub fn authoritative_evidence(text: &str) -> bool {
    if text.contains('§') {
        return true;
    }
    let chars: Vec<char> = text.chars().collect();
    let boundary_before = |i: usize| i == 0 || !is_word(chars[i - 1]);
    let boundary_after = |i: usize| i >= chars.len() || !is_word(chars[i]);
    for ext in [".rs", ".mjs", ".js", ".sh", ".md", ".json", ".toml"] {
        let e: Vec<char> = ext.chars().collect();
        for i in 0..chars.len() {
            if chars[i..].starts_with(&e) && boundary_after(i + e.len()) {
                return true;
            }
        }
    }
    for i in 0..chars.len() {
        if boundary_before(i) && chars[i..].starts_with(&['A', 'C']) {
            match chars.get(i + 2) {
                Some('-') => return true,
                Some(c) if c.is_ascii_digit() => return true,
                _ => {}
            }
        }
    }
    // `\b[0-9a-f]{7,40}\b`: a whole word token of 7–40 lowercase hex digits.
    let mut i = 0;
    while i < chars.len() {
        if !is_word(chars[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && is_word(chars[i]) {
            i += 1;
        }
        let token = &chars[start..i];
        if (7..=40).contains(&token.len())
            && token
                .iter()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(c))
        {
            return true;
        }
    }
    false
}

fn is_hex(s: &str, min: usize, max: usize) -> bool {
    (min..=max).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn non_empty_string(v: Option<&JsValue>) -> bool {
    v.and_then(JsValue::as_str)
        .is_some_and(|s| !js_trim(s).is_empty())
}

/// `JSON.stringify(v)` of an optional value (`undefined` when absent).
pub fn json_of(v: Option<&JsValue>) -> String {
    v.map_or_else(|| "undefined".to_owned(), JsValue::stringify)
}

/// Validates one record. Returns every problem found.
pub fn validate_corpus_item(item: &JsValue) -> Vec<String> {
    let mut errors = Vec::new();
    let Some(o) = item.as_object() else {
        return vec!["item is not an object".to_owned()];
    };
    for key in o.keys() {
        if !TOP_LEVEL_FIELDS.contains(&key.as_str()) {
            errors.push(format!("unknown top-level key \"{key}\""));
        }
    }
    for key in TOP_LEVEL_FIELDS {
        if !o.contains_key(key) {
            errors.push(format!("missing required key \"{key}\""));
        }
    }
    if !non_empty_string(o.get("id")) {
        errors.push("id must be a non-empty string".to_owned());
    }
    if o.get("schemaVersion").and_then(JsValue::as_f64) != Some(CORPUS_SCHEMA_VERSION) {
        errors.push(format!(
            "schemaVersion must be 1, got {}",
            json_of(o.get("schemaVersion"))
        ));
    }
    if !o
        .get("mode")
        .and_then(JsValue::as_str)
        .is_some_and(|m| MODES.contains(&m))
    {
        errors.push(format!(
            "mode must be one of {}, got {}",
            MODES.join("|"),
            json_of(o.get("mode"))
        ));
    }
    let dim_ok = o
        .get("dim")
        .is_some_and(|d| d.is_object() && non_empty_string(d.get("key")));
    if !dim_ok {
        errors.push("dim must be an object with a non-empty key".to_owned());
    }
    if !non_empty_string(o.get("target")) {
        errors.push("target must be a non-empty string".to_owned());
    }
    let sha_ok = o
        .get("promptSha256")
        .and_then(JsValue::as_str)
        .is_some_and(|s| is_hex(s, 64, 64));
    if !sha_ok {
        errors.push("promptSha256 must be a 64-char lowercase hex digest".to_owned());
    }
    if o.get("promptDrift").and_then(JsValue::as_bool).is_none() {
        errors.push("promptDrift must be a boolean".to_owned());
    }

    match o.get("finding").filter(|f| f.is_object()) {
        None => errors.push("finding must be an object".to_owned()),
        Some(f) => {
            for field in FINDING_FIELDS {
                if f.get(field).is_none() {
                    errors.push(format!("finding.{field} is required"));
                }
            }
            if !non_empty_string(f.get("severity")) {
                errors.push("finding.severity must be a non-empty string".to_owned());
            }
            if f.get("confidence").and_then(JsValue::as_f64).is_none() {
                errors.push("finding.confidence must be a number".to_owned());
            }
        }
    }

    match o.get("provenance").filter(|p| p.is_object()) {
        None => errors.push("provenance must be an object".to_owned()),
        Some(p) => match p.get("kind").and_then(JsValue::as_str) {
            Some("mined") => {
                for field in ["projectSlug", "sessionId", "runId", "agentId", "workflow"] {
                    if !non_empty_string(p.get(field)) {
                        errors.push(format!(
                            "mined provenance.{field} must be a non-empty string"
                        ));
                    }
                }
                let hv_ok = p
                    .get("historicalVerdict")
                    .filter(|h| h.is_object())
                    .and_then(|h| h.get("refuted"))
                    .and_then(JsValue::as_bool)
                    .is_some();
                if !hv_ok {
                    errors.push(
                        "mined provenance.historicalVerdict must be an object with a boolean refuted"
                            .to_owned(),
                    );
                }
            }
            Some("constructed") => {
                if !non_empty_string(p.get("builtAgainstCommit")) {
                    errors.push(
                        "constructed provenance.builtAgainstCommit must be a non-empty string"
                            .to_owned(),
                    );
                }
                if !non_empty_string(p.get("rationale")) {
                    errors.push(
                        "constructed provenance.rationale must be a non-empty string".to_owned(),
                    );
                }
            }
            _ => errors.push(format!(
                "provenance.kind must be one of {}, got {}",
                PROVENANCE_KINDS.join("|"),
                json_of(p.get("kind"))
            )),
        },
    }

    match o.get("groundTruth").filter(|g| g.is_object()) {
        None => errors.push(
            "groundTruth must be an object (the miner emits null; adjudicate before checking in)"
                .to_owned(),
        ),
        Some(g) => {
            let defect = g.get("defect").and_then(JsValue::as_bool);
            if defect.is_none() {
                errors.push("groundTruth.defect must be a boolean".to_owned());
            }
            let class = g.get("class").and_then(JsValue::as_str);
            if !class.is_some_and(|c| GROUND_TRUTH_CLASSES.contains(&c)) {
                errors.push(format!(
                    "groundTruth.class must be one of {}, got {}",
                    GROUND_TRUTH_CLASSES.join("|"),
                    json_of(g.get("class"))
                ));
            }
            let authority = g.get("authority").and_then(JsValue::as_str);
            if !authority.is_some_and(|a| AUTHORITIES.contains(&a)) {
                errors.push(format!(
                    "groundTruth.authority must be one of {}, got {}",
                    AUTHORITIES.join("|"),
                    json_of(g.get("authority"))
                ));
            }
            if !non_empty_string(g.get("evidence")) {
                errors.push("groundTruth.evidence must be a non-empty string".to_owned());
            } else if authority == Some("authoritative")
                && !authoritative_evidence(
                    g.get("evidence").and_then(JsValue::as_str).unwrap_or(""),
                )
            {
                errors.push("an authoritative item's evidence must cite a concrete artifact (a source path, a 7+ hex sha, or a §/AC- reference)".to_owned());
            }
            let commit = match g.get("adjudicatedAgainstCommit") {
                Some(v) if v.truthy() => v.js_string(),
                _ => String::new(),
            };
            if !is_hex(&commit, 7, 40) {
                errors.push(
                    "groundTruth.adjudicatedAgainstCommit must be a 7-40 char hex commit sha"
                        .to_owned(),
                );
            }
            let should_be_defect = class == Some("real-defect");
            if let Some(d) = defect
                && d != should_be_defect
            {
                errors.push(format!(
                    "groundTruth.defect ({d}) contradicts class \"{}\" (only \"real-defect\" carries defect: true)",
                    g.get("class").map_or_else(|| "undefined".to_owned(), JsValue::js_string)
                ));
            }
        }
    }
    errors
}

/// One validated corpus record, in its original key order.
#[derive(Debug, Clone, PartialEq)]
pub struct CorpusItem(pub JsValue);

impl CorpusItem {
    /// The record.
    pub fn value(&self) -> &JsValue {
        &self.0
    }
    /// `id`.
    pub fn id(&self) -> &str {
        self.str_at(&["id"]).unwrap_or("")
    }
    /// `mode`.
    pub fn mode(&self) -> &str {
        self.str_at(&["mode"]).unwrap_or("")
    }
    /// `target`.
    pub fn target(&self) -> &str {
        self.str_at(&["target"]).unwrap_or("")
    }
    /// `dim`.
    pub fn dim(&self) -> &JsValue {
        self.0.get("dim").unwrap_or(&JsValue::Null)
    }
    /// `finding`.
    pub fn finding(&self) -> &JsValue {
        self.0.get("finding").unwrap_or(&JsValue::Null)
    }
    /// `finding.severity`.
    pub fn severity(&self) -> &str {
        self.str_at(&["finding", "severity"]).unwrap_or("")
    }
    /// `groundTruth.defect`.
    pub fn defect(&self) -> bool {
        self.at(&["groundTruth", "defect"])
            .and_then(JsValue::as_bool)
            .unwrap_or(false)
    }
    /// `groundTruth.class`.
    pub fn class(&self) -> &str {
        self.str_at(&["groundTruth", "class"]).unwrap_or("")
    }
    /// `groundTruth.authority`.
    pub fn authority(&self) -> &str {
        self.str_at(&["groundTruth", "authority"]).unwrap_or("")
    }
    /// `provenance.kind`.
    pub fn provenance_kind(&self) -> &str {
        self.str_at(&["provenance", "kind"]).unwrap_or("")
    }
    /// `promptSha256`.
    pub fn prompt_sha256(&self) -> &str {
        self.str_at(&["promptSha256"]).unwrap_or("")
    }
    /// `promptDrift`.
    pub fn prompt_drift(&self) -> bool {
        self.at(&["promptDrift"])
            .and_then(JsValue::as_bool)
            .unwrap_or(false)
    }
    /// A nested member.
    pub fn at(&self, path: &[&str]) -> Option<&JsValue> {
        path.iter().try_fold(&self.0, |v, k| v.get(k))
    }
    fn str_at(&self, path: &[&str]) -> Option<&str> {
        self.at(path).and_then(JsValue::as_str)
    }
}

/// Parses a JSONL corpus. Blank lines are skipped; a duplicate id is
/// rejected. Returns the valid items and every error (with line numbers).
pub fn load_corpus(text: &str) -> (Vec<CorpusItem>, Vec<String>) {
    let mut items = Vec::new();
    let mut errors = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for (i, raw) in text.split('\n').enumerate() {
        let line = js_trim(raw);
        if line.is_empty() {
            continue;
        }
        let n = i + 1;
        let parsed = match JsValue::parse(line) {
            Ok(v) => v,
            Err(e) => {
                errors.push(format!("line {n}: not parseable JSON: {e}"));
                continue;
            }
        };
        let item_errors = validate_corpus_item(&parsed);
        if !item_errors.is_empty() {
            // `${parsed && parsed.id}`: a null record prints "null".
            let id = if parsed == JsValue::Null {
                "null".to_owned()
            } else {
                parsed
                    .get("id")
                    .map_or_else(|| "undefined".to_owned(), JsValue::js_string)
            };
            for e in item_errors {
                errors.push(format!("line {n} ({id}): {e}"));
            }
            continue;
        }
        let id = parsed
            .get("id")
            .and_then(JsValue::as_str)
            .unwrap_or("")
            .to_owned();
        if seen.contains(&id) {
            errors.push(format!("line {n}: duplicate id \"{id}\""));
            continue;
        }
        seen.push(id);
        items.push(CorpusItem(parsed));
    }
    (items, errors)
}

/// The corpus composition.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CorpusSummary {
    /// Items.
    pub size: usize,
    /// By ground-truth class (first-appearance order).
    pub by_class: JsMap<usize>,
    /// By authority.
    pub by_authority: JsMap<usize>,
    /// By provenance kind.
    pub by_provenance: JsMap<usize>,
    /// By finding severity.
    pub by_severity: JsMap<usize>,
    /// By mode.
    pub by_mode: JsMap<usize>,
    /// Divergence-class share (percent).
    #[serde(serialize_with = "crate::measure::jsnum::serialize_js_number")]
    pub divergence_class_share: f64,
    /// Authoritative share (percent).
    #[serde(serialize_with = "crate::measure::jsnum::serialize_js_number")]
    pub authoritative_share: f64,
    /// Mined share (percent).
    #[serde(serialize_with = "crate::measure::jsnum::serialize_js_number")]
    pub mined_share: f64,
    /// Defect-truth items.
    pub defect_items: usize,
    /// Non-defect-truth items.
    pub non_defect_items: usize,
    /// Items recorded as prompt-drifted.
    pub drifted_items: usize,
}

/// Summarizes a corpus's composition.
pub fn summarize_corpus(corpus: &[CorpusItem]) -> CorpusSummary {
    let mut by_class = JsMap::new();
    let mut by_authority = JsMap::new();
    let mut by_provenance = JsMap::new();
    let mut by_severity = JsMap::new();
    let mut by_mode = JsMap::new();
    for item in corpus {
        *by_class.entry(item.class(), || 0) += 1;
        *by_authority.entry(item.authority(), || 0) += 1;
        *by_provenance.entry(item.provenance_kind(), || 0) += 1;
        *by_severity.entry(item.severity(), || 0) += 1;
        *by_mode.entry(item.mode(), || 0) += 1;
    }
    let size = corpus.len();
    #[allow(clippy::cast_precision_loss)]
    let share = |n: Option<&usize>| -> f64 {
        if size == 0 {
            0.0
        } else {
            js_round((*n.unwrap_or(&0) as f64 / size as f64) * 1000.0) / 10.0
        }
    };
    CorpusSummary {
        size,
        divergence_class_share: share(by_class.get(DIVERGENCE_CLASS)),
        authoritative_share: share(by_authority.get("authoritative")),
        mined_share: share(by_provenance.get("mined")),
        by_class,
        by_authority,
        by_provenance,
        by_severity,
        by_mode,
        defect_items: corpus.iter().filter(|i| i.defect()).count(),
        non_defect_items: corpus.iter().filter(|i| !i.defect()).count(),
        drifted_items: corpus.iter().filter(|i| i.prompt_drift()).count(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authoritative_evidence_matches_the_js_rule() {
        for yes in [
            "see rdm-core/src/lib.rs line 3",
            "docs/x.md",
            "fixed in 1a2b3c4",
            "§ Scope",
            "violates AC-2",
            "AC3 says",
            "(AC-1)",
        ] {
            assert!(authoritative_evidence(yes), "{yes}");
        }
        for no in [
            "clearly not a bug",
            "the rsx file",
            "a.rsx",
            "1a2b3c",
            "1a2b3c4g",
            "XAC-1",
            "ACME",
            "1A2B3C4D",
        ] {
            assert!(!authoritative_evidence(no), "{no}");
        }
    }
}

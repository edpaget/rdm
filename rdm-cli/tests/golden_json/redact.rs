//! The six redaction rules that make a capture reproducible, applied to the
//! raw text so every other byte is preserved, and the byte comparator
//! against `tests/golden/`.
//!
//! 1. The fixture's temp root, in its raw and canonical forms (macOS prints
//!    `/var/…` from `rdm info` and `/private/var/…` from `rdm worktree`),
//!    longest first → `<TMPDIR>`.
//! 2. Review `created`/`submitted` RFC3339 datetimes → `<DATETIME>`.
//! 3. `created`/`completed`/`updated` dates → `<DATE>`.
//! 4. `commit`/`applied_commit`/`created_commit`/`review_sha` → `<SHA>`.
//! 5. The review id (`YYYY-MM-DD-HHMM-hex`) → `<REVIEW-ID>`; the small
//!    integer comment `id` is untouched.
//! 6. `estimate_snapshot` (a digest over the whole phase document, dates
//!    included) → `<SNAPSHOT>`.

use std::collections::BTreeMap;
use std::path::Path;

use regex::Regex;

/// The command that rewrites `tests/golden/` from a fresh capture.
pub const BLESS: &str =
    "cargo nextest run -p rdm-cli --test golden_json --run-ignored only -E 'test(=bless)'";

/// Applies the redaction rules for one fixture root.
pub struct Redactor {
    roots: Vec<String>,
    rules: Vec<(Regex, &'static str)>,
}

impl Redactor {
    /// A redactor for a fixture rooted at `root`.
    pub fn for_root(root: &Path) -> Self {
        let raw = root.to_string_lossy().into_owned();
        let canonical = root
            .canonicalize()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| raw.clone());
        let mut roots = vec![raw, canonical];
        roots.sort_by_key(|r| std::cmp::Reverse(r.len()));
        roots.dedup();
        let rule = |re: &str| Regex::new(re).expect("a valid redaction pattern");
        let rules = vec![
            (
                rule(r#""(created|submitted)": "[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9:.]+Z""#),
                r#""$1": "<DATETIME>""#,
            ),
            (
                rule(r#""(created|completed|updated)": "[0-9]{4}-[0-9]{2}-[0-9]{2}""#),
                r#""$1": "<DATE>""#,
            ),
            (
                rule(r#""(commit|applied_commit|created_commit|review_sha)": "[0-9a-f]{7,40}""#),
                r#""$1": "<SHA>""#,
            ),
            (
                rule(r#""id": "[0-9]{4}-[0-9]{2}-[0-9]{2}-[0-9]{4}-[0-9a-f]+""#),
                r#""id": "<REVIEW-ID>""#,
            ),
            (
                rule(r#""estimate_snapshot": "[0-9a-f]{64}""#),
                r#""estimate_snapshot": "<SNAPSHOT>""#,
            ),
        ];
        Self { roots, rules }
    }

    /// The temp-root forms this redactor replaces, longest first.
    pub fn roots(&self) -> &[String] {
        &self.roots
    }

    /// Redacts one captured file.
    pub fn redact(&self, text: &str) -> String {
        let mut out = text.to_owned();
        for root in &self.roots {
            out = out.replace(root.as_str(), "<TMPDIR>");
        }
        for (re, with) in &self.rules {
            out = re.replace_all(&out, *with).into_owned();
        }
        out
    }

    /// Redacts every captured file.
    pub fn redact_all(&self, raw: &BTreeMap<String, String>) -> BTreeMap<String, String> {
        raw.iter()
            .map(|(k, v)| (k.clone(), self.redact(v)))
            .collect()
    }
}

/// Every `*.json` file directly under `dir`, by stem.
pub fn read_goldens(dir: &Path) -> std::io::Result<BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.extension().is_some_and(|e| e == "json")
            && let Some(stem) = path.file_stem()
        {
            out.insert(
                stem.to_string_lossy().into_owned(),
                std::fs::read_to_string(&path)?,
            );
        }
    }
    Ok(out)
}

/// Compares a redacted capture with the committed goldens byte for byte,
/// naming every drifted, missing or uncaptured file.
pub fn compare(
    captured: &BTreeMap<String, String>,
    goldens: &BTreeMap<String, String>,
) -> Vec<String> {
    let mut problems = Vec::new();
    for (name, text) in captured {
        match goldens.get(name) {
            None => problems.push(format!("{name}.json: no committed golden")),
            Some(g) if g != text => problems.push(format!("{name}.json: drifted")),
            Some(_) => {}
        }
    }
    for name in goldens.keys() {
        if !captured.contains_key(name) {
            problems.push(format!("{name}.json: committed but not captured"));
        }
    }
    problems
}

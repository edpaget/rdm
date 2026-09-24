//! rdm's machine-facing `--format json` contract, frozen as the committed
//! `tests/golden/*.json` files and reproduced here from a fresh, sandboxed
//! fixture on every run.
//!
//! [`capture`] builds the fixture and runs the 24-command inventory;
//! [`redact`] applies the six redaction rules and compares byte for byte.
//! A changed shape is re-blessed deliberately with the ignored [`bless`]
//! case (see [`redact::BLESS`] and `tests/golden/README.md`); it is the only
//! test that writes into the checkout. Ported from the retired
//! `scripts/verify-golden-json.sh`, `scripts/capture-golden.sh` and their
//! two shell libraries; see `docs/test-migration-inventory.md` § 8.

#[macro_use]
#[path = "../common/workflow_support.rs"]
mod workflow_support;

#[path = "../git_test_support.rs"]
mod git_test_support;

#[path = "../common/plan_fixture.rs"]
mod plan_fixture;

#[path = "../common/seeded_plan.rs"]
mod seeded_plan;

mod capture;
mod redact;

use std::collections::BTreeMap;
use std::path::PathBuf;

use regex::Regex;
use serde_json::Value;

use crate::capture::{NAMES, capture};
use crate::redact::{BLESS, Redactor, compare, read_goldens};
use crate::workflow_support::repo_root;

fn golden_dir() -> PathBuf {
    repo_root().join("tests/golden")
}

fn goldens() -> BTreeMap<String, String> {
    read_goldens(&golden_dir()).expect("read tests/golden")
}

/// A fresh capture, redacted, plus the temp-root forms it redacted.
fn redacted_capture() -> (BTreeMap<String, String>, Vec<String>) {
    let cap = capture().unwrap_or_else(|f| panic!("{f}"));
    let redactor = Redactor::for_root(cap.fixture.dir.path());
    (redactor.redact_all(&cap.raw), redactor.roots().to_vec())
}

fn bare_digest() -> Regex {
    Regex::new(r#""[0-9a-f]{64}""#).expect("valid pattern")
}

#[test]
fn capture_matches_committed_goldens() {
    let (captured, _) = redacted_capture();
    assert_eq!(
        captured.keys().map(String::as_str).collect::<Vec<_>>(),
        {
            let mut names = NAMES.to_vec();
            names.sort_unstable();
            names
        },
        "every inventory command was captured"
    );
    let problems = compare(&captured, &goldens());
    assert!(
        problems.is_empty(),
        "the --format json contract drifted from tests/golden/:\n  {}\nIf the change is intentional, re-bless with:\n  {BLESS}\nand review `git diff tests/golden/`.",
        problems.join("\n  ")
    );
}

#[test]
fn same_day_captures_are_identical_and_leak_no_temp_path() {
    let (first, first_roots) = redacted_capture();
    let (second, second_roots) = redacted_capture();
    assert!(
        first == second,
        "two independent same-day captures differ after redaction: {:?}",
        compare(&first, &second)
    );
    for (name, text) in first.iter().chain(&second) {
        for root in first_roots.iter().chain(&second_roots) {
            assert!(
                !text.contains(root.as_str()),
                "{name}.json leaks the temp root {root}"
            );
        }
    }
}

#[test]
fn digest_fields_are_redacted_not_frozen() {
    let cap = capture().unwrap_or_else(|f| panic!("{f}"));
    let raw: Value = serde_json::from_str(&cap.raw["phase-show"]).expect("phase-show is JSON");
    let digest = raw["estimate_snapshot"].as_str().unwrap_or_default();
    assert!(
        digest.len() == 64
            && digest
                .chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
        "phase-show carries a 64-hex estimate_snapshot before redaction: {raw}"
    );
    let redactor = Redactor::for_root(cap.fixture.dir.path());
    let redacted: Value =
        serde_json::from_str(&redactor.redact(&cap.raw["phase-show"])).expect("still JSON");
    assert_eq!(redacted["estimate_snapshot"], "<SNAPSHOT>");
    let bare = bare_digest();
    for (name, text) in redactor.redact_all(&cap.raw).iter().chain(&goldens()) {
        assert!(
            !bare.is_match(text),
            "{name}.json holds a bare 64-hex value that should have been redacted"
        );
    }
}

#[test]
fn redactor_redacts_a_planted_digest_and_every_volatile_field() {
    let root = tempfile::TempDir::new().expect("tempdir");
    let r = Redactor::for_root(root.path());
    let planted = format!(
        "{{\n  \"root\": \"{}/plan\",\n  \"estimate_snapshot\": \"{}\",\n  \"created\": \"2026-01-02\",\n  \"submitted\": \"2026-01-02T03:04:05.678Z\",\n  \"commit\": \"abc1234\",\n  \"id\": \"2026-01-02-0304-abcd\",\n  \"comments\": [{{ \"id\": 1 }}]\n}}\n",
        root.path().display(),
        "0123456789abcdef".repeat(4)
    );
    let out: Value = serde_json::from_str(&r.redact(&planted)).expect("JSON");
    assert_eq!(out["root"], "<TMPDIR>/plan");
    assert_eq!(out["estimate_snapshot"], "<SNAPSHOT>");
    assert_eq!(out["created"], "<DATE>");
    assert_eq!(out["submitted"], "<DATETIME>");
    assert_eq!(out["commit"], "<SHA>");
    assert_eq!(out["id"], "<REVIEW-ID>");
    assert_eq!(out["comments"][0]["id"], 1, "comment ids are untouched");
}

#[test]
fn comparator_names_a_mutated_golden() {
    let committed = goldens();
    assert!(compare(&committed, &committed).is_empty());
    let mut mutated = committed.clone();
    let text = mutated.get_mut("task-show").expect("task-show golden");
    text.push(' ');
    assert_eq!(
        compare(&mutated, &committed),
        vec!["task-show.json: drifted".to_owned()]
    );
    let mut missing = committed.clone();
    missing.remove("info");
    assert_eq!(
        compare(&missing, &committed),
        vec!["info.json: committed but not captured".to_owned()]
    );
}

/// Rewrites `tests/golden/*.json` from a fresh capture. Run deliberately
/// (see [`redact::BLESS`]) after an intentional shape change, then review
/// `git diff tests/golden/`.
#[test]
#[ignore = "writes tests/golden/; run deliberately to re-bless the JSON contract"]
fn bless() {
    let (captured, _) = redacted_capture();
    for (name, text) in &captured {
        std::fs::write(golden_dir().join(format!("{name}.json")), text)
            .unwrap_or_else(|e| panic!("writing {name}.json: {e}"));
    }
    eprintln!(
        "re-blessed {} goldens under {}; review with `git diff tests/golden/`",
        captured.len(),
        golden_dir().display()
    );
}

//! `rdm-measure refuter-severity` against the checked-in
//! `tests/fixtures/token-refuter-severity` and `token-determining-rank` trees.
//!
//! Workflow-dependent (need Node, reached only through the phase-2 binding via
//! `NodeReviewRules`, which replays the canonical `rankFindings`/`survives`/
//! `hasBlocking`/`acTableHasGap` in `.claude/workflows/lib/review.mjs`): every
//! test that measures a corpus — `severity_counts_verdicts_and_token_classes_match_fixture`,
//! `fanout_distributions_match_fixture`, `determining_rank_block_matches_fixture`,
//! `json_and_text_match_goldens`, `check_accepts_fixture_doc`,
//! `check_and_audit_reject_edited_severity_figure`,
//! `check_and_audit_reject_edited_rank_figure`, `until_pins_run_set_at_both_edges`,
//! `check_applies_doc_window_and_until_overrides` and
//! `rank_uses_canonical_rank_findings`. Node resolves from `RDM_TEST_NODE` or
//! `PATH`; a missing runtime fails them.
//!
//! No JavaScript runtime: `audit_committed_baseline_ok` (run with an empty
//! `PATH` to prove it) and `until_rejects_unparseable_date`.

mod measure_support;

use std::path::{Path, PathBuf};

use measure_support::{
    edited_doc, fails, fixture, golden, json, json_without_instrument, normalize, ok, repo_root,
    run, run_with,
};
use rdm_devtools::workflow::MutantTree;
use serde_json::Value;

fn severity_root() -> PathBuf {
    fixture("token-refuter-severity")
}

fn rank_root() -> PathBuf {
    fixture("token-determining-rank")
}

fn severity_doc() -> PathBuf {
    severity_root().join("expected-nonGatingRefutationSkip.json")
}

fn rank_doc() -> PathBuf {
    rank_root().join("expected-determiningFindingRank.json")
}

fn s(p: &Path) -> &str {
    p.to_str().expect("utf-8 path")
}

fn measure(root: &Path, extra: &[&str]) -> Value {
    let mut args = vec!["refuter-severity", "--root", s(root), "--format", "json"];
    args.extend_from_slice(extra);
    json(&ok(&run(&args)))
}

fn row<'a>(rows: &'a Value, key: &str) -> Option<&'a Value> {
    rows.as_array()?.iter().find(|r| r["key"] == key)
}

#[test]
fn severity_counts_verdicts_and_token_classes_match_fixture() {
    let report = measure(&severity_root(), &[]);
    let expected = json(&golden(&severity_doc()))["nonGatingRefutationSkip"].clone();
    let got_keys: Vec<&Value> = report["refuteBySeverity"]
        .as_array()
        .expect("rows")
        .iter()
        .map(|r| &r["key"])
        .collect();
    let want_keys: Vec<&Value> = expected["refuteBySeverity"]
        .as_array()
        .expect("rows")
        .iter()
        .map(|r| &r["key"])
        .collect();
    assert_eq!(
        got_keys, want_keys,
        "exactly the expected rows, in report order"
    );
    for want in expected["refuteBySeverity"].as_array().expect("rows") {
        let key = want["key"].as_str().expect("key");
        let got = row(&report["refuteBySeverity"], key).expect("row");
        for f in [
            "agentCount",
            "graded",
            "refuted",
            "refutedRate",
            "output",
            "uncachedInput",
            "cacheWrite",
            "cacheRead",
        ] {
            assert_eq!(got[f], want[f], "refuteBySeverity.{key}.{f}");
        }
    }
    let rows = &report["refuteBySeverity"];
    assert_eq!(
        row(rows, "blocking").expect("blocking")["agentCount"],
        1,
        "the braced-target refuter resolved to its real severity"
    );
    assert!(row(rows, "unrecoverable:unparseable").is_none());
    let none = row(rows, "unrecoverable:no-transcript").expect("no-transcript bucket");
    assert_eq!(none["agentCount"], 1);
    assert_eq!(none["graded"], 0, "an unrecoverable refuter grades nothing");
    for f in [
        "agentCount",
        "output",
        "uncachedInput",
        "cacheWrite",
        "cacheRead",
    ] {
        assert_eq!(
            report["refuteTotals"][f], expected["refuteTotals"][f],
            "refuteTotals.{f}"
        );
        assert_eq!(
            report["laneTotals"][f], expected["laneTotals"][f],
            "laneTotals.{f}"
        );
    }
    for f in [
        "agentsNotSpawned",
        "allTokens",
        "freshTokens",
        "percentOfRefuteAgents",
        "percentOfRefuteTokens",
        "percentOfLaneTokens",
    ] {
        assert_eq!(
            report["projected"][f], expected["projected"][f],
            "projected.{f}"
        );
    }
    assert_eq!(
        report["projected"]["severities"],
        serde_json::json!(["suggestion"]),
        "the non-gating set is read from review.mjs"
    );
}

#[test]
fn fanout_distributions_match_fixture() {
    let report = measure(&severity_root(), &[]);
    let expected = json(&golden(&severity_doc()))["refuterFanout"].clone();
    let fp = &report["refuterFanout"]["findingsPerFinder"];
    let exp_fp = &expected["findingsPerFinder"];
    let keys = |v: &Value| -> Vec<Value> {
        v["rows"]
            .as_array()
            .expect("rows")
            .iter()
            .map(|r| r["key"].clone())
            .collect()
    };
    assert_eq!(keys(fp), keys(exp_fp));
    for want in exp_fp["rows"].as_array().expect("rows") {
        let got = row(&fp["rows"], want["key"].as_str().expect("key")).expect("row");
        for f in ["n", "min", "p50", "p90", "max", "refutersDispatched"] {
            assert_eq!(got[f], want[f], "findingsPerFinder.{}.{f}", want["key"]);
        }
    }
    assert_eq!(fp["unreadableFinderCount"], exp_fp["unreadableFinderCount"]);
    assert_eq!(fp["unresolvedLabelCount"], exp_fp["unresolvedLabelCount"]);
    assert_eq!(
        row(&fp["rows"], "plan:unit-of-work").expect("row")["refutersDispatched"],
        1,
        "the dimension-shadow refuter counts on its PROMPT-derived dimension"
    );
    assert!(row(&fp["rows"], "plan:coherence").is_none());
    let unit = &report["refuterFanout"]["refuterCountsByUnit"];
    for f in [
        "n",
        "min",
        "p50",
        "p90",
        "max",
        "totalRefuters",
        "recoveredRefuters",
        "unrecoverableRefuterCount",
        "recoveryRatePercent",
    ] {
        assert_eq!(
            unit[f], expected["refuterCountsByUnit"][f],
            "refuterCountsByUnit.{f}"
        );
    }
}

#[test]
fn determining_rank_block_matches_fixture() {
    let report = measure(&rank_root(), &[]);
    let got = &report["determiningFindingRank"];
    let exp = &json(&golden(&rank_doc()))["determiningFindingRank"];
    assert_eq!(got, exp, "the whole block, every figure");
    assert_eq!(
        got["units"]["total"], 7,
        "no phantom unit for orphan agent E"
    );
    assert_eq!(got["units"]["determining"], 3);
    assert_eq!(got["units"]["nonDetermining"], 2);
    assert_eq!(got["units"]["unrecoverable"], 2);
    assert_eq!(
        got["rankHistogram"],
        serde_json::json!([{"rank": 1, "count": 1}, {"rank": 2, "count": 1}, {"rank": 4, "count": 1}])
    );
    let within: Vec<u64> = got["withinTop"]
        .as_array()
        .expect("rows")
        .iter()
        .filter_map(|w| w["count"].as_u64())
        .collect();
    assert_eq!(within, [2, 3], "top-3 and top-5 are different figures");
    assert_eq!(got["orphanAgents"]["finders"], 1);
    assert_eq!(got["orphanAgents"]["runsAffected"], 1);
    assert_eq!(got["acTableGapUnits"], 1);
}

#[test]
fn json_and_text_match_goldens() {
    for root in [severity_root(), rank_root()] {
        let out = ok(&run(&[
            "refuter-severity",
            "--root",
            s(&root),
            "--format",
            "json",
        ]));
        let got = normalize(&out, &root);
        assert_eq!(json(&got)["instrument"], "rdm-measure refuter-severity");
        assert_eq!(
            json_without_instrument(&got),
            json_without_instrument(&golden(&root.join("expected/report.json"))),
            "{}",
            root.display()
        );
        let text = ok(&run(&["refuter-severity", "--root", s(&root)]));
        assert_eq!(
            normalize(&text, &root),
            golden(&root.join("expected/report.txt")),
            "{}",
            root.display()
        );
    }
    let root = severity_root();
    let out = ok(&run(&[
        "refuter-severity",
        "--root",
        s(&root),
        "--until",
        "2026-07-24T00:00:00Z",
        "--format",
        "json",
    ]));
    assert_eq!(
        json_without_instrument(&normalize(&out, &root)),
        json_without_instrument(&golden(&root.join("expected/report-until.json")))
    );
}

#[test]
fn check_accepts_fixture_doc() {
    let out = ok(&run(&[
        "refuter-severity",
        "--root",
        s(&severity_root()),
        "--check",
        s(&severity_doc()),
    ]));
    assert!(out.contains("--check OK"), "{out}");
    ok(&run(&[
        "refuter-severity",
        "--root",
        s(&rank_root()),
        "--check",
        s(&rank_doc()),
    ]));
    ok(&run(&["refuter-severity", "--audit", s(&severity_doc())]));
    ok(&run(&["refuter-severity", "--audit", s(&rank_doc())]));
}

#[test]
fn check_and_audit_reject_edited_severity_figure() {
    let doc = edited_doc(&severity_doc(), |d| {
        d["nonGatingRefutationSkip"]["projected"]["agentsNotSpawned"] = 2.into();
    });
    let err = fails(&run(&["refuter-severity", "--audit", s(doc.path())]));
    assert!(err.contains("projected.agentsNotSpawned"), "{err}");
    let err = fails(&run(&[
        "refuter-severity",
        "--root",
        s(&severity_root()),
        "--check",
        s(doc.path()),
    ]));
    assert!(
        err.contains("projected.agentsNotSpawned: doc 2 vs corpus 1"),
        "{err}"
    );
}

#[test]
fn check_and_audit_reject_edited_rank_figure() {
    let doc = edited_doc(&rank_doc(), |d| {
        d["determiningFindingRank"]["units"]["determining"] = 4.into();
    });
    let err = fails(&run(&["refuter-severity", "--audit", s(doc.path())]));
    assert!(err.contains("determiningFindingRank.units"), "{err}");
    let err = fails(&run(&[
        "refuter-severity",
        "--root",
        s(&rank_root()),
        "--check",
        s(doc.path()),
    ]));
    assert!(
        err.contains("units.determining: doc 4 vs corpus 3"),
        "{err}"
    );
}

#[test]
fn audit_committed_baseline_ok() {
    // An empty PATH and no RDM_TEST_NODE: the audit must not need JavaScript.
    let home = tempfile::tempdir().expect("home");
    let out = ok(&run_with(
        &["refuter-severity", "--audit", "docs/token-baseline.json"],
        &[
            ("PATH", ""),
            ("RDM_TEST_NODE", ""),
            ("HOME", s(home.path())),
        ],
    ));
    assert!(out.contains("--audit OK"), "{out}");
    let _ = repo_root();
}

#[test]
fn until_pins_run_set_at_both_edges() {
    let root = severity_root();
    let before = measure(&root, &["--until", "2026-07-24T00:00:00Z"]);
    assert_eq!(before["corpus"]["runCount"], 0);
    assert_eq!(before["corpus"]["agentRecordCount"], 0);
    assert_eq!(before["refuteBySeverity"], serde_json::json!([]));
    assert_eq!(before["projected"]["agentsNotSpawned"], 0);
    let after = measure(&root, &["--until", "2026-07-26T00:00:00Z"]);
    assert_eq!(after["corpus"]["runCount"], 1);
    assert_eq!(after["corpus"]["agentRecordCount"], 11);
    let unwindowed = measure(&root, &[]);
    assert_eq!(after["refuteBySeverity"], unwindowed["refuteBySeverity"]);
    assert_eq!(after["projected"], unwindowed["projected"]);
    // The run starts exactly at 2026-07-25T09:00:00Z: the edge is inclusive.
    let edge = measure(&root, &["--until", "2026-07-25T09:00:00Z"]);
    assert_eq!(edge["corpus"]["runCount"], 1);
    let just_before = measure(&root, &["--until", "2026-07-25T08:59:59.999Z"]);
    assert_eq!(just_before["corpus"]["runCount"], 0);
}

#[test]
fn until_rejects_unparseable_date() {
    let home = tempfile::tempdir().expect("home");
    let err = fails(&run_with(
        &[
            "refuter-severity",
            "--root",
            s(&severity_root()),
            "--until",
            "not-a-date",
        ],
        &[
            ("PATH", ""),
            ("RDM_TEST_NODE", ""),
            ("HOME", s(home.path())),
        ],
    ));
    assert!(
        err.contains("--until value is not a parseable date"),
        "{err}"
    );
}

#[test]
fn check_applies_doc_window_and_until_overrides() {
    let doc = edited_doc(&severity_doc(), |d| {
        d["nonGatingRefutationSkip"]["measurementWindow"] =
            serde_json::json!({"until": "2026-07-24T00:00:00Z"});
    });
    fails(&run(&[
        "refuter-severity",
        "--root",
        s(&severity_root()),
        "--check",
        s(doc.path()),
    ]));
    ok(&run(&[
        "refuter-severity",
        "--root",
        s(&severity_root()),
        "--until",
        "2026-07-26T00:00:00Z",
        "--check",
        s(doc.path()),
    ]));
}

#[test]
fn rank_uses_canonical_rank_findings() {
    let tree = MutantTree::copy(&repo_root(), &[".claude/workflows/lib/review.mjs"])
        .expect("copy review.mjs");
    tree.replace_once(
        ".claude/workflows/lib/review.mjs",
        "identity-ranking",
        "function rankFindings(findings) {\n",
        "function rankFindings(findings) {\n  return findings.slice();\n",
    )
    .expect("plant mutant");
    let lib = tree.root().join(".claude/workflows/lib/review.mjs");
    let mutant = measure(&rank_root(), &["--review-lib", s(&lib)]);
    let real = measure(&rank_root(), &[]);
    assert_ne!(
        mutant["determiningFindingRank"]["rankHistogram"],
        real["determiningFindingRank"]["rankHistogram"],
        "unit B's out-of-order findings rank 4 only under the canonical ranking"
    );
    assert_eq!(
        real["determiningFindingRank"],
        json(&golden(&rank_doc()))["determiningFindingRank"]
    );
}

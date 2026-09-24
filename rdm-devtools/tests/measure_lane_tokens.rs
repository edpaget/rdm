//! `rdm-measure lane-tokens` against the checked-in `tests/fixtures/token-sidecar`
//! tree: the hand-computed totals, the goldens captured from the JavaScript
//! tool before it was deleted, the filters, and argument handling.
//!
//! No test here needs a JavaScript runtime, a live model, or the real
//! `~/.claude` (every run passes `--root` or points `HOME` at a temp dir).

mod measure_support;

use std::path::PathBuf;

use measure_support::{fails, fixture, golden, json, normalize, ok, run, run_with};
use serde_json::Value;

fn root() -> PathBuf {
    fixture("token-sidecar")
}

fn report_json(extra: &[&str]) -> Value {
    let root = root();
    let mut args = vec![
        "lane-tokens",
        "--root",
        root.to_str().expect("utf-8"),
        "--format",
        "json",
    ];
    args.extend_from_slice(extra);
    json(&ok(&run(&args)))
}

fn keyed<'a>(rows: &'a Value, key: &str) -> Option<&'a Value> {
    rows.as_array()?.iter().find(|r| r["key"] == key)
}

#[test]
fn fixture_totals_match_hand_computed() {
    let report = report_json(&[]);
    let expected = json(&golden(&root().join("expected-totals.json")));
    for group in ["byAgentClass", "byLabel", "byModel", "byWorkflow"] {
        let exp = expected[group].as_object().expect("keyed expectation");
        for (key, want) in exp {
            let got = keyed(&report[group], key).unwrap_or_else(|| panic!("{group} lacks {key}"));
            for field in [
                "agentCount",
                "dedupedRequestCount",
                "output",
                "uncachedInput",
                "cacheWrite",
                "cacheRead",
            ] {
                assert_eq!(got[field], want[field], "{group}.{key}.{field}");
            }
        }
    }
    for (key, want) in expected["floorByAgentClass"].as_object().expect("floors") {
        let got = keyed(&report["floorByAgentClass"], key).expect("floor row");
        for field in ["n", "minTokens", "p10Tokens", "medianTokens", "meanTokens"] {
            assert_eq!(got[field], want[field], "floorByAgentClass.{key}.{field}");
        }
    }
    assert_eq!(report["runsConsidered"], expected["runsConsidered"]);
    assert_eq!(report["recordCount"], expected["recordCount"]);
    assert_eq!(report["totalsDiscrepancy"], expected["totalsDiscrepancy"]);
    // The refute class rolls up 2+ distinct full labels.
    let refute_labels: Vec<&Value> = report["byLabel"]
        .as_array()
        .expect("rows")
        .iter()
        .filter(|r| r["key"].as_str().is_some_and(|k| k.starts_with("refute:")))
        .collect();
    assert!(refute_labels.len() >= 2);
    let summed: u64 = refute_labels
        .iter()
        .filter_map(|r| r["agentCount"].as_u64())
        .sum();
    assert_eq!(
        keyed(&report["byAgentClass"], "refute").expect("refute")["agentCount"],
        summed
    );
}

#[test]
fn json_matches_golden() {
    let root = root();
    let out = ok(&run(&[
        "lane-tokens",
        "--root",
        root.to_str().expect("utf-8"),
        "--format",
        "json",
    ]));
    let got = normalize(&out, &root);
    let want = golden(&root.join("expected/lane-tokens.json"));
    assert_eq!(json(&got), json(&want), "structural golden");
    assert_eq!(got, want, "byte golden (the JS tool's formatting)");
}

#[test]
fn text_matches_golden() {
    let root = root();
    let out = ok(&run(&[
        "lane-tokens",
        "--root",
        root.to_str().expect("utf-8"),
    ]));
    assert_eq!(out, golden(&root.join("expected/lane-tokens.txt")));
}

#[test]
fn workflow_filter_is_ored() {
    let root = root();
    let got = report_json(&["--workflow", "autopilot", "--workflow", "no-such-workflow"]);
    let want = json(&golden(&root.join("expected/lane-tokens-workflow-or.json")));
    let mut got_norm = got.clone();
    got_norm["projectsRoot"] = Value::String("<ROOT>".to_owned());
    assert_eq!(got_norm, want);
    assert_eq!(
        got["runsConsidered"], 1,
        "OR: the matching name keeps its run"
    );
    let both = report_json(&["--workflow", "autopilot,dispatch-phase"]);
    assert_eq!(
        both["runsConsidered"], 0,
        "a comma is part of a workflow name, not a separator"
    );
    let widened = report_json(&["--workflow", "autopilot", "--workflow", "dispatch-phase"]);
    assert_eq!(
        widened["runsConsidered"], 2,
        "repeating --workflow widens, never narrows"
    );
}

#[test]
fn since_filter_excludes_earlier_runs() {
    let root = root();
    let mut got = report_json(&["--since", "2026-07-20T00:00:00Z"]);
    assert_eq!(got["runsConsidered"], 1);
    got["projectsRoot"] = Value::String("<ROOT>".to_owned());
    assert_eq!(
        got,
        json(&golden(&root.join("expected/lane-tokens-since.json")))
    );
    let bad = fails(&run(&[
        "lane-tokens",
        "--root",
        root.to_str().expect("utf-8"),
        "--since",
        "not-a-date",
    ]));
    assert!(bad.contains("--since"), "{bad}");
}

#[test]
fn worktree_slug_sessions_included() {
    let report = report_json(&[]);
    let autopilot = keyed(&report["byWorkflow"], "autopilot").expect("the worktree-slug run");
    assert!(autopilot["agentCount"].as_u64().unwrap_or(0) > 0);
    let workflows: Vec<&str> = report["byWorkflow"]
        .as_array()
        .expect("rows")
        .iter()
        .filter_map(|r| r["key"].as_str())
        .collect();
    assert_eq!(workflows, ["autopilot", "dispatch-phase"]);
}

#[test]
fn discrepancy_reported_never_reconciled() {
    let report = report_json(&[]);
    let d = &report["totalsDiscrepancy"];
    assert_eq!(d["sidecarTotalTokens"], 9200);
    assert_eq!(d["dedupedTotalTokens"], 8180);
    assert_eq!(
        d["delta"], 1020,
        "both sides and the delta, never reconciled to 0"
    );
    let text = ok(&run(&[
        "lane-tokens",
        "--root",
        root().to_str().expect("utf-8"),
    ]));
    assert!(text.lines().any(|l| l
        == "Sidecar totalTokens vs deduped-sum discrepancy: 1020 (sidecar=9200, deduped=8180)"));
}

#[test]
fn floor_by_agent_class_excludes_cached_and_omits_empty() {
    let report = report_json(&[]);
    let floor = report["floorByAgentClass"].as_array().expect("rows");
    let keys: Vec<&str> = floor.iter().filter_map(|r| r["key"].as_str()).collect();
    assert_eq!(
        keys,
        ["fetch", "find", "plan"],
        "gate/implement/refute/stamp are all cached or sidecar-only: omitted, not n:0"
    );
    let fetch = keyed(&report["floorByAgentClass"], "fetch").expect("fetch");
    assert_eq!(
        fetch["medianTokens"], 800,
        "the FIRST request (100+500+200), not 380 or 1180"
    );
    assert_eq!(
        keyed(&report["floorByAgentClass"], "find").expect("find")["n"],
        1
    );
}

#[test]
fn missing_flag_value_rejected() {
    let root = root();
    let err = fails(&run(&[
        "lane-tokens",
        "--root",
        root.to_str().expect("utf-8"),
        "--workflow",
    ]));
    assert!(err.contains("--workflow"), "{err}");
}

#[test]
fn flag_taken_as_value_rejected() {
    let root = root();
    let err = fails(&run(&[
        "lane-tokens",
        "--root",
        root.to_str().expect("utf-8"),
        "--since",
        "--format",
        "json",
    ]));
    assert!(err.contains("--since"), "{err}");
}

#[test]
fn out_into_missing_directory_fails() {
    let dir = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("does-not-exist/out.json");
    let root = root();
    let err = fails(&run(&[
        "lane-tokens",
        "--root",
        root.to_str().expect("utf-8"),
        "--out",
        target.to_str().expect("utf-8"),
    ]));
    assert!(err.contains("does not exist"), "{err}");
    let good = dir.path().join("out.json");
    let stdout = ok(&run(&[
        "lane-tokens",
        "--root",
        root.to_str().expect("utf-8"),
        "--format",
        "json",
        "--out",
        good.to_str().expect("utf-8"),
    ]));
    assert!(stdout.is_empty(), "--out writes nothing to stdout");
    let written = std::fs::read_to_string(&good).expect("written");
    assert_eq!(
        normalize(&written, &root),
        golden(&root.join("expected/lane-tokens.json")),
        "--out writes the same bytes stdout would"
    );
}

#[test]
fn unknown_argument_and_bad_format_rejected() {
    let err = fails(&run(&["lane-tokens", "--nonsense"]));
    assert!(err.contains("--nonsense"), "{err}");
    let err = fails(&run(&["lane-tokens", "--format", "yaml"]));
    assert!(err.contains("--format"), "{err}");
    assert!(
        run(&["lane-tokens", "--help"]).status.success(),
        "--help exits 0"
    );
}

#[test]
fn default_root_is_home_claude_projects() {
    let home = tempfile::tempdir().expect("home");
    let home_str = home.path().to_str().expect("utf-8");
    let err = fails(&run_with(&["lane-tokens"], &[("HOME", home_str)]));
    let projects = home.path().join(".claude/projects");
    assert!(
        err.contains(projects.to_str().expect("utf-8")),
        "an absent default root is named: {err}"
    );
    std::fs::create_dir_all(&projects).expect("mkdir");
    let src = root().join("-Users-edward-Projects-rdm");
    copy_tree(&src, &projects.join("-Users-edward-Projects-rdm"));
    let out = ok(&run_with(
        &["lane-tokens", "--format", "json"],
        &[("HOME", home_str)],
    ));
    let report = json(&out);
    assert_eq!(report["projectsRoot"], projects.to_str().expect("utf-8"));
    assert_eq!(report["runsConsidered"], 1);
}

fn copy_tree(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).expect("mkdir");
    for entry in std::fs::read_dir(from).expect("read_dir") {
        let entry = entry.expect("entry");
        let dest = to.join(entry.file_name());
        if entry.file_type().expect("type").is_dir() {
            copy_tree(&entry.path(), &dest);
        } else {
            std::fs::copy(entry.path(), dest).expect("copy");
        }
    }
}

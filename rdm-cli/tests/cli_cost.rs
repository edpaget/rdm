//! `rdm cost` over the checked-in fixture tree
//! `tests/fixtures/cost-session/home/.claude/projects/`.
//!
//! Every invocation points `HOME` and `CLAUDE_CONFIG_DIR` at the fixture and
//! clears `CLAUDE_CODE_SESSION_ID`, `RDM_ROOT` and `RDM_PROJECT`, because the
//! suite itself often runs inside a Claude Code session. The goldens under
//! `tests/fixtures/cost-session/expected/` are written only by the ignored
//! [`bless_cost_goldens`] case, which runs the binary; no token figure is
//! typed into this file.

use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use serde_json::Value;

const S1: &str = "11111111-2222-4333-8444-555555555555";
const S2: &str = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";
const BLESS: &str = "cargo nextest run -p rdm-cli --test cli_cost --run-ignored only -E 'test(=bless_cost_goldens)'";

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("rdm-cli has a parent directory")
        .join("tests/fixtures/cost-session")
}

fn config_dir() -> PathBuf {
    fixture().join("home/.claude")
}

fn expected_dir() -> PathBuf {
    fixture().join("expected")
}

/// `rdm` with the host's session and plan-repo variables cleared and no
/// Claude data directory set.
fn bare_rdm() -> Command {
    let mut cmd = Command::cargo_bin("rdm").expect("rdm binary");
    cmd.env_remove("CLAUDE_CODE_SESSION_ID")
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env("XDG_CONFIG_HOME", "/dev/null/nonexistent");
    cmd
}

/// `rdm` pointed at the fixture.
fn rdm() -> Command {
    let mut cmd = bare_rdm();
    cmd.env("HOME", fixture().join("home"))
        .env("CLAUDE_CONFIG_DIR", config_dir());
    cmd
}

struct Run {
    ok: bool,
    stdout: String,
    stderr: String,
}

fn run(cmd: &mut Command) -> Run {
    let out = cmd.output().expect("run rdm");
    Run {
        ok: out.status.success(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

fn json(args: &[&str]) -> Value {
    let r = run(rdm().args(args).args(["--format", "json"]));
    assert!(r.ok, "rdm {args:?} failed: {}", r.stderr);
    assert!(
        r.stderr.is_empty(),
        "json stderr must be empty: {}",
        r.stderr
    );
    serde_json::from_str(&r.stdout).expect("stdout is JSON")
}

/// Replaces the fixture's Claude data directory, raw and canonical, with a
/// placeholder so the goldens do not depend on the checkout path.
fn redact(text: &str) -> String {
    let raw = config_dir().display().to_string();
    let canonical = fs::canonicalize(config_dir())
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| raw.clone());
    let mut forms = [raw, canonical];
    forms.sort_by_key(|f| std::cmp::Reverse(f.len()));
    forms.iter().fold(text.to_owned(), |t, f| {
        t.replace(f.as_str(), "<CLAUDE_CONFIG_DIR>")
    })
}

/// The four goldens, captured from the binary and redacted.
fn capture() -> Vec<(&'static str, String)> {
    let session = ["cost", "--session", S1];
    let workflow = ["cost", "--workflow-run", "wf_R1"];
    let mut out = Vec::new();
    for (stem, args) in [("session", session), ("workflow-run", workflow)] {
        let j = run(rdm().args(args).args(["--format", "json"]));
        assert!(j.ok, "rdm {args:?} --format json failed: {}", j.stderr);
        out.push((
            if stem == "session" {
                "session.json"
            } else {
                "workflow-run.json"
            },
            redact(&j.stdout),
        ));
        let h = run(rdm().args(args));
        assert!(h.ok, "rdm {args:?} failed: {}", h.stderr);
        out.push((
            if stem == "session" {
                "session.txt"
            } else {
                "workflow-run.txt"
            },
            redact(&format!(
                "--- stdout ---\n{}--- stderr ---\n{}",
                h.stdout, h.stderr
            )),
        ));
    }
    out
}

#[test]
fn output_matches_committed_goldens() {
    for (name, captured) in capture() {
        let path = expected_dir().join(name);
        let committed = fs::read_to_string(&path).unwrap_or_default();
        assert!(
            captured == committed,
            "rdm cost output drifted from {}.\n--- captured ---\n{captured}\n--- committed ---\n{committed}\nIf the change is intentional, re-bless with:\n  {BLESS}\nand review `git diff tests/fixtures/cost-session/expected/`.",
            path.display()
        );
    }
}

/// Rewrites the goldens from the binary. Run deliberately (see [`BLESS`]),
/// then review the diff.
#[test]
#[ignore = "writes tests/fixtures/cost-session/expected/; run deliberately to re-bless"]
fn bless_cost_goldens() {
    fs::create_dir_all(expected_dir()).expect("create expected dir");
    for (name, text) in capture() {
        fs::write(expected_dir().join(name), text).expect("write golden");
    }
}

fn source<'a>(report: &'a Value, id: &str) -> &'a Value {
    report["sources"]
        .as_array()
        .and_then(|a| a.iter().find(|s| s["id"] == id))
        .unwrap_or_else(|| panic!("no source {id}"))
}

fn warnings(report: &Value) -> Vec<String> {
    report["warnings"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|w| w.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn session_report_flags_anchors_and_degradations() {
    let report = json(&["cost", "--session", S1]);

    // The errored Workflow agent is flagged; the cached one costs nothing.
    let errored: Vec<&Value> = report["sources"]
        .as_array()
        .map(|a| a.iter().filter(|s| s["errored"] == true).collect())
        .unwrap_or_default();
    assert_eq!(errored.len(), 1);
    assert_eq!(errored[0]["kind"], "workflow_agent");
    assert_eq!(errored[0]["run_id"], "wf_R1");
    let cached = source(&report, "w3");
    assert_eq!(cached["cached"], true);
    assert_eq!(cached["totals"]["total"], 0);

    // The nested A3 carries its root ancestor A2's anchor.
    let a2 = source(&report, "a2");
    let a3 = source(&report, "a3");
    assert!(a2["anchor"].is_string());
    assert_eq!(a3["anchor"], a2["anchor"]);
    assert_eq!(a3["parent_agent_id"], "a2");

    // A4 and wf_R2 are unanchored, with labels.
    let unanchored: Vec<(String, String)> = report["unanchored"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|u| {
                    (
                        u["id"].as_str().unwrap_or_default().to_owned(),
                        u["label"].as_str().unwrap_or_default().to_owned(),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let ids: Vec<&str> = unanchored.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids, ["a4", "wf_R2"]);
    assert!(unanchored.iter().all(|(_, label)| !label.is_empty()));

    // The nested-unverified warning and the bad sidecar's path, while the
    // rest of the session still reports.
    let w = warnings(&report);
    assert!(
        w.iter()
            .any(|w| w.starts_with("workflow-nested agents unverified")),
        "{w:?}"
    );
    let bad = config_dir().join(format!("projects/-fixture-proj/{S1}/workflows/wf_bad.json"));
    assert!(
        w.iter().any(|w| w.contains(&bad.display().to_string())),
        "no warning names {}: {w:?}",
        bad.display()
    );
    assert!(report["sources"].as_array().is_some_and(|a| a.len() > 1));
    assert_eq!(report["workflow_runs"].as_array().map(Vec::len), Some(2));
}

#[test]
fn workflow_run_report_is_restricted_to_that_run() {
    let report = json(&["cost", "--workflow-run", "wf_R1"]);
    assert_eq!(report["scope"]["run_id"], "wf_R1");
    let sources = report["sources"].as_array().cloned().unwrap_or_default();
    assert!(!sources.is_empty());
    assert!(sources.iter().all(|s| s["run_id"] == "wf_R1"));
    assert!(
        warnings(&report)
            .iter()
            .any(|w| w.starts_with("workflow-nested agents unverified"))
    );
    // The prefix is optional.
    assert_eq!(json(&["cost", "--workflow-run", "R1"]), report);
}

#[test]
fn a_session_under_a_worktree_slug_is_located() {
    let report = json(&["cost", "--session", S2]);
    assert_eq!(
        report["project_slug"],
        "-fixture-proj--worktrees-roadmap-demo"
    );
    assert_eq!(report["session_id"], S2);
}

#[test]
fn no_selector_reads_claude_code_session_id() {
    let explicit = json(&["cost", "--session", S1]);
    let r = run(rdm()
        .env("CLAUDE_CODE_SESSION_ID", S1)
        .args(["cost", "--format", "json"]));
    assert!(r.ok, "{}", r.stderr);
    let from_env: Value = serde_json::from_str(&r.stdout).expect("JSON");
    assert_eq!(from_env, explicit);

    let unset = run(rdm().arg("cost"));
    assert!(!unset.ok);
    for needle in ["--session", "--workflow-run", "CLAUDE_CODE_SESSION_ID"] {
        assert!(unset.stderr.contains(needle), "{needle}: {}", unset.stderr);
    }
}

#[test]
fn an_unknown_session_names_the_uuid_root_and_slugs() {
    let r = run(rdm().args(["cost", "--session", "00000000-0000-4000-8000-000000000000"]));
    assert!(!r.ok);
    let stderr = redact(&r.stderr);
    for needle in [
        "00000000-0000-4000-8000-000000000000",
        "<CLAUDE_CONFIG_DIR>/projects",
        "-fixture-proj\n",
        "-fixture-proj--worktrees-roadmap-demo",
    ] {
        assert!(stderr.contains(needle), "{needle:?} missing from {stderr}");
    }
}

#[test]
fn works_without_a_plan_repo() {
    let empty = tempfile::TempDir::new().expect("tempdir");
    let r = run(rdm()
        .arg("--root")
        .arg(empty.path())
        .args(["cost", "--session", S1]));
    assert!(r.ok, "{}", r.stderr);
}

#[test]
fn selectors_conflict_and_bad_ids_are_rejected() {
    let both = run(rdm().args(["cost", "--session", S1, "--workflow-run", "wf_R1"]));
    assert!(!both.ok);
    assert!(
        both.stderr.contains("cannot be used with"),
        "{}",
        both.stderr
    );

    let escape = run(rdm().args(["cost", "--session", "../x"]));
    assert!(!escape.ok);
    assert!(escape.stderr.contains("invalid id"), "{}", escape.stderr);
}

#[test]
fn home_is_used_when_claude_config_dir_is_unset() {
    let r = run(bare_rdm()
        .env_remove("CLAUDE_CONFIG_DIR")
        .env("HOME", fixture().join("home"))
        .args(["cost", "--session", S2, "--format", "json"]));
    assert!(r.ok, "{}", r.stderr);
}

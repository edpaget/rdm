//! Integration tests for `rdm verify resolve` / `rdm verify run` — the CLI
//! surface over the repo-only `dispatch.verify` key (`docs/verify-gate.md`).

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use std::path::Path;
use tempfile::TempDir;

#[path = "git_test_support.rs"]
mod git_test_support;
use git_test_support::git;

fn rdm() -> Command {
    let mut cmd = Command::cargo_bin("rdm").unwrap();
    cmd.env("XDG_CONFIG_HOME", "/dev/null/nonexistent");
    cmd.env_remove("RDM_PROJECT")
        .env_remove("RDM_ROOT")
        .env_remove("RDM_DISPATCH_VERIFY");
    cmd
}

fn init_plan_repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    let p = dir.path();
    rdm().arg("--root").arg(p).arg("init").assert().success();
    rdm()
        .arg("--root")
        .arg(p)
        .args(["project", "create", "demo"])
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(p)
        .args([
            "roadmap",
            "create",
            "auth",
            "--title",
            "Auth",
            "--no-edit",
            "--project",
            "demo",
        ])
        .assert()
        .success();
    dir
}

/// A temp source repo whose git root is a **subdirectory** of the `TempDir`.
///
/// `rdm worktree add` places a worktree at
/// `repo_root.parent()/<name>__worktrees/<item>` — a sibling of the repo. If
/// the repo root were the `TempDir` itself that sibling would land in the
/// system temp directory and outlive `TempDir::drop`, leaking a populated git
/// worktree on every run. Rooting the repo one level down keeps the sibling
/// inside the `TempDir`, so it is removed with everything else.
struct SourceRepo {
    _dir: TempDir,
    root: std::path::PathBuf,
}

impl SourceRepo {
    fn path(&self) -> &Path {
        &self.root
    }
}

fn init_source_repo() -> SourceRepo {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("repo");
    std::fs::create_dir_all(&root).unwrap();
    let p = root.as_path();
    git(p, &["init", "-b", "main"]);
    std::fs::write(p.join("README.md"), "# project\n").unwrap();
    git(p, &["add", "."]);
    git(p, &["commit", "-m", "initial"]);
    SourceRepo { _dir: dir, root }
}

fn set_verify(plan: &Path, cmd: &str) {
    rdm()
        .arg("--root")
        .arg(plan)
        .args(["config", "set", "dispatch.verify", cmd])
        .assert()
        .success();
}

/// Runs `rdm verify <args…>` in `cwd`, returning (exit code, stdout).
fn verify(plan: &Path, cwd: &Path, args: &[&str]) -> (i32, String) {
    let out = rdm()
        .arg("--root")
        .arg(plan)
        .arg("verify")
        .args(args)
        .current_dir(cwd)
        .assert()
        .get_output()
        .clone();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).to_string(),
    )
}

// --- resolve ---------------------------------------------------------------

#[test]
fn verify_resolve_reports_command_or_unresolved() {
    let plan = init_plan_repo();
    let cwd = init_source_repo();

    // Unset.
    let (code, out) = verify(
        plan.path(),
        cwd.path(),
        &["resolve", "--format", "json", "--project", "demo"],
    );
    assert_eq!(code, 0, "resolve is a query and always exits 0");
    let j: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(j["resolved"], false);
    assert!(j["command"].is_null());

    let (_, out) = verify(plan.path(), cwd.path(), &["resolve", "--project", "demo"]);
    assert_eq!(out.trim(), "unresolved");

    // Set.
    set_verify(plan.path(), "bash scripts/ci.sh");
    let (code, out) = verify(
        plan.path(),
        cwd.path(),
        &["resolve", "--format", "json", "--project", "demo"],
    );
    assert_eq!(code, 0);
    let j: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(j["resolved"], true);
    assert_eq!(j["command"], "bash scripts/ci.sh");

    let (_, out) = verify(plan.path(), cwd.path(), &["resolve", "--project", "demo"]);
    assert_eq!(out.trim(), "bash scripts/ci.sh");
}

#[test]
fn verify_resolve_agrees_with_config_get() {
    // The two surfaces read through the same accessor, so they can never
    // disagree about what is configured. Assert it rather than trusting it.
    let plan = init_plan_repo();
    let cwd = init_source_repo();
    set_verify(plan.path(), "bash scripts/ci.sh");
    let (_, resolved) = verify(plan.path(), cwd.path(), &["resolve", "--project", "demo"]);
    let got = rdm()
        .arg("--root")
        .arg(plan.path())
        .args(["config", "get", "dispatch.verify", "--raw"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(
        resolved.trim(),
        String::from_utf8_lossy(&got).trim(),
        "`verify resolve` and `config get` must never disagree"
    );

    set_project_verify(plan.path(), "demo", "echo demo-own");
    let (_, resolved) = verify(plan.path(), cwd.path(), &["resolve", "--project", "demo"]);
    let got = rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "config",
            "get",
            "dispatch.verify",
            "--raw",
            "--project",
            "demo",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(resolved.trim(), "echo demo-own");
    assert_eq!(
        resolved.trim(),
        String::from_utf8_lossy(&got).trim(),
        "`verify resolve --project` and `config get --project` must never disagree"
    );
}

// --- run -------------------------------------------------------------------

#[test]
fn verify_run_reports_exit_and_bounded_tail() {
    let plan = init_plan_repo();
    let cwd = init_source_repo();

    // Passing command.
    set_verify(plan.path(), "echo all good");
    let (code, out) = verify(
        plan.path(),
        cwd.path(),
        &["run", "--format", "json", "--project", "demo"],
    );
    assert_eq!(code, 0, "a passing command must exit 0 so `&&` composes");
    let j: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(j["resolved"], true);
    assert_eq!(j["exit"], 0);
    assert!(j["tail"].as_str().unwrap().contains("all good"));

    // Failing command, with its diagnosis on stderr.
    set_verify(plan.path(), "echo boom >&2; exit 7");
    let (code, out) = verify(
        plan.path(),
        cwd.path(),
        &["run", "--format", "json", "--project", "demo"],
    );
    assert_eq!(code, 7, "rdm must exit with the command's own code");
    let j: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(j["exit"], 7);
    assert!(
        j["tail"].as_str().unwrap().contains("boom"),
        "stderr must be merged into the tail: {j}"
    );
}

#[test]
fn verify_run_truncates_the_tail_to_the_last_4000_chars() {
    let plan = init_plan_repo();
    let cwd = init_source_repo();
    // 20000 'x' between two markers; only the LAST 4000 characters survive.
    set_verify(
        plan.path(),
        "printf 'FIRSTMARKER'; for i in $(seq 1 2000); do printf 'xxxxxxxxxx'; done; printf 'LASTMARKER'",
    );
    let (_, out) = verify(
        plan.path(),
        cwd.path(),
        &["run", "--format", "json", "--project", "demo"],
    );
    let j: Value = serde_json::from_str(&out).unwrap();
    let tail = j["tail"].as_str().unwrap();
    assert_eq!(tail.chars().count(), 4000, "the tail bound is 4000 chars");
    assert!(tail.ends_with("LASTMARKER"), "the tail must be the END");
    assert!(!tail.contains("FIRSTMARKER"), "the head must be dropped");
}

#[test]
fn verify_run_reports_unresolved_and_exits_2() {
    let plan = init_plan_repo();
    let cwd = init_source_repo();
    let (code, out) = verify(
        plan.path(),
        cwd.path(),
        &["run", "--format", "json", "--project", "demo"],
    );
    assert_eq!(code, 2);
    let j: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(j["resolved"], false);
    assert!(j["command"].is_null());
    assert!(j["exit"].is_null());
    assert!(j["tail"].is_null());

    let (code, out) = verify(plan.path(), cwd.path(), &["run", "--project", "demo"]);
    assert_eq!(code, 2);
    assert_eq!(out.trim(), "unresolved");
}

#[test]
fn verify_run_refuses_a_multiline_command() {
    let plan = init_plan_repo();
    let cwd = init_source_repo();
    // `config set` accepts it; `verify run` refuses to decompose it.
    set_verify(plan.path(), "cargo fmt --check\ncargo clippy");
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args(["verify", "run", "--project", "demo"])
        .current_dir(cwd.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("multi-line"))
        .stderr(predicate::str::contains("bash scripts/ci.sh"));
}

#[test]
fn verify_run_executes_in_the_item_worktree() {
    let plan = init_plan_repo();
    let src = init_source_repo();
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "phase",
            "create",
            "design",
            "--title",
            "Design",
            "--number",
            "1",
            "--no-edit",
            "--roadmap",
            "auth",
            "--project",
            "demo",
        ])
        .assert()
        .success();
    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .args(["worktree", "add", "auth", "--project", "demo"])
        .current_dir(src.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let wt = std::path::PathBuf::from(String::from_utf8_lossy(&out).trim().to_string());

    set_verify(plan.path(), "printf ran > sentinel.txt");
    // Invoked from the MAIN checkout, targeting the worktree by --item.
    let (code, _) = verify(
        plan.path(),
        src.path(),
        &[
            "run",
            "--item",
            "auth",
            "--format",
            "json",
            "--project",
            "demo",
        ],
    );
    assert_eq!(code, 0);
    assert!(
        wt.join("sentinel.txt").exists(),
        "the command must run in the item's worktree"
    );
    assert!(
        !src.path().join("sentinel.txt").exists(),
        "and NOT in the invoking cwd"
    );
}

#[test]
fn verify_run_errors_actionably_for_an_item_with_no_worktree() {
    let plan = init_plan_repo();
    let src = init_source_repo();
    set_verify(plan.path(), "true");
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args(["verify", "run", "--item", "auth", "--project", "demo"])
        .current_dir(src.path())
        .assert()
        .code(3)
        .stderr(predicate::str::contains("rdm worktree add auth"))
        .stdout(
            predicate::str::contains("not run to completion")
                .and(predicate::str::contains("signal").not()),
        );
}

#[test]
fn verify_run_exits_a_distinct_code_when_the_item_does_not_resolve() {
    let plan = init_plan_repo();
    let src = init_source_repo();
    // A command that would have succeeded had it run — proving the distinct
    // exit is about resolution, not about the configured command failing.
    set_verify(plan.path(), "true");
    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "verify",
            "run",
            "--item",
            "auth",
            "--format",
            "json",
            "--project",
            "demo",
        ])
        .current_dir(src.path())
        .assert()
        .code(3)
        .stderr(predicate::str::contains("rdm worktree add auth"))
        .get_output()
        .clone();
    let j: Value = serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    assert_eq!(
        j["resolved"], true,
        "the command IS configured — only the checkout is unknown"
    );
    assert_eq!(j["command"], "true");
    assert!(
        j["exit"].is_null(),
        "nothing ran, so exit must be null: {j}"
    );
}

#[test]
fn verify_run_resolves_a_phase_item_to_its_roadmap_worktree() {
    let plan = init_plan_repo();
    let src = init_source_repo();
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "phase",
            "create",
            "design",
            "--title",
            "Design",
            "--number",
            "1",
            "--no-edit",
            "--roadmap",
            "auth",
            "--project",
            "demo",
        ])
        .assert()
        .success();
    // The roadmap worktree, never a per-phase one.
    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .args(["worktree", "add", "auth", "--project", "demo"])
        .current_dir(src.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let wt = std::path::PathBuf::from(String::from_utf8_lossy(&out).trim().to_string());

    // Unprefixed worktree grammar.
    set_verify(plan.path(), "printf unprefixed > sentinel-unprefixed.txt");
    let (code, _) = verify(
        plan.path(),
        src.path(),
        &[
            "run",
            "--item",
            "auth/phase-1-design",
            "--format",
            "json",
            "--project",
            "demo",
        ],
    );
    assert_eq!(code, 0);
    assert!(
        wt.join("sentinel-unprefixed.txt").exists(),
        "the unprefixed phase form must resolve to the roadmap worktree"
    );

    // Kind-prefixed grammar, the shape `--on`/`--implements` use.
    set_verify(plan.path(), "printf prefixed > sentinel-prefixed.txt");
    let (code, _) = verify(
        plan.path(),
        src.path(),
        &[
            "run",
            "--item",
            "phase/auth/phase-1-design",
            "--format",
            "json",
            "--project",
            "demo",
        ],
    );
    assert_eq!(code, 0);
    assert!(
        wt.join("sentinel-prefixed.txt").exists(),
        "the phase/<roadmap>/<stem> form must also resolve to the roadmap worktree"
    );
}

#[test]
fn verify_run_prefers_the_roadmap_worktree_over_a_stale_per_phase_one() {
    let plan = init_plan_repo();
    let src = init_source_repo();
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "phase",
            "create",
            "design",
            "--title",
            "Design",
            "--number",
            "1",
            "--no-edit",
            "--roadmap",
            "auth",
            "--project",
            "demo",
        ])
        .assert()
        .success();
    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .args(["worktree", "add", "auth", "--project", "demo"])
        .current_dir(src.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let roadmap_wt = std::path::PathBuf::from(String::from_utf8_lossy(&out).trim().to_string());

    // Construct a stale per-phase worktree the same way leftover state does.
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "worktree",
            "add",
            "auth/phase-1-design",
            "--project",
            "demo",
        ])
        .current_dir(src.path())
        .assert()
        .success();

    set_verify(plan.path(), "printf ran > sentinel.txt");
    let (code, _) = verify(
        plan.path(),
        src.path(),
        &[
            "run",
            "--item",
            "auth/phase-1-design",
            "--format",
            "json",
            "--project",
            "demo",
        ],
    );
    assert_eq!(code, 0);
    assert!(
        roadmap_wt.join("sentinel.txt").exists(),
        "the shared roadmap worktree must win over a stale per-phase one"
    );
}

#[test]
fn verify_run_never_suggests_a_per_phase_worktree_for_a_phase_item() {
    let plan = init_plan_repo();
    let src = init_source_repo();
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "phase",
            "create",
            "design",
            "--title",
            "Design",
            "--number",
            "1",
            "--no-edit",
            "--roadmap",
            "auth",
            "--project",
            "demo",
        ])
        .assert()
        .success();
    set_verify(plan.path(), "true");
    // No worktree registered at all.
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "verify",
            "run",
            "--item",
            "auth/phase-1-design",
            "--project",
            "demo",
        ])
        .current_dir(src.path())
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("rdm worktree add auth")
                .and(predicate::str::contains("rdm worktree add auth/phase-1-design").not()),
        );
}

#[test]
fn verify_run_refuses_a_plan_or_change_item_naming_the_accepted_grammar() {
    let plan = init_plan_repo();
    let src = init_source_repo();
    set_verify(plan.path(), "true");

    // `plan/<slug>` names no worktree — it must be refused up front, never
    // silently reinterpreted as a phase of a roadmap literally named `plan`
    // (the garbled `no rdm worktree found for 'phase '...' not found` shape
    // the code review reproduced).
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args(["verify", "run", "--item", "plan/foo", "--project", "demo"])
        .current_dir(src.path())
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("names no worktree")
                .and(predicate::str::contains("rdm phase list").not())
                .and(predicate::str::contains("task/<slug>")),
        );

    // `change/<sha>` — the canonical `--on change/HEAD` shape the dispatch
    // skill's own review triage uses — must be refused the same way.
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "verify",
            "run",
            "--item",
            "change/HEAD",
            "--project",
            "demo",
        ])
        .current_dir(src.path())
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("names no worktree")
                .and(predicate::str::contains("rdm phase list").not()),
        );
}

#[test]
fn verify_run_refuses_a_malformed_task_item_naming_the_accepted_grammar() {
    let plan = init_plan_repo();
    let src = init_source_repo();
    set_verify(plan.path(), "true");

    // `task` is a reserved roadmap slug (like `plan`/`change`), so a
    // malformed `task/<slug>` reference (an extra segment) must be refused
    // up front, naming the accepted grammar, rather than being silently
    // reinterpreted by `ItemRef::parse` as a task whose slug is literally
    // `a/b`.
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args(["verify", "run", "--item", "task/a/b", "--project", "demo"])
        .current_dir(src.path())
        .assert()
        .code(3)
        .stderr(
            predicate::str::contains("names no worktree")
                .and(predicate::str::contains("task/<slug>"))
                .and(predicate::str::contains("rdm phase list").not()),
        );
}

#[test]
fn verify_run_refuses_a_malformed_phase_prefixed_item_naming_the_accepted_grammar() {
    let plan = init_plan_repo();
    let src = init_source_repo();
    set_verify(plan.path(), "true");

    // `phase/<roadmap>` with the stem omitted (no roadmap named `phase`
    // exists in this repo) must get the actionable grammar message, never
    // the garbled nested "unknown item 'phase/auth' — check `rdm phase
    // list`" error that comes from `ItemRef::parse` misreading it as
    // `ItemRef::Phase { roadmap: "phase", stem: "auth" }`.
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args(["verify", "run", "--item", "phase/auth", "--project", "demo"])
        .current_dir(src.path())
        .assert()
        .code(3)
        .stderr(
            predicate::str::contains("names no worktree")
                .and(predicate::str::contains("phase/<roadmap>/<stem>"))
                .and(predicate::str::contains("rdm phase list").not())
                .and(predicate::str::contains("unknown item").not()),
        );

    // Same shape as `verify_run_exits_a_distinct_code_when_the_item_does_not_resolve`:
    // the command IS configured — only the reference is malformed — so the
    // JSON payload must report `resolved: true` and a null `exit` (nothing
    // ran to completion).
    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "verify",
            "run",
            "--item",
            "phase/auth",
            "--format",
            "json",
            "--project",
            "demo",
        ])
        .current_dir(src.path())
        .assert()
        .code(3)
        .get_output()
        .clone();
    let j: Value = serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    assert_eq!(
        j["resolved"], true,
        "the command IS configured — only the reference is malformed"
    );
    assert!(
        j["exit"].is_null(),
        "nothing ran, so exit must be null: {j}"
    );
}

#[test]
fn verify_run_still_resolves_a_roadmap_literally_named_phase() {
    let plan = init_plan_repo();
    let src = init_source_repo();
    // `roadmap`/`phase` are NOT reserved roadmap slugs (unlike
    // `task`/`plan`/`src`/`change`), so a roadmap literally named `phase`
    // must still resolve through the ordinary 2-segment `phase/<stem>`
    // grammar — the malformed-reference fix above must not break this.
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "roadmap",
            "create",
            "phase",
            "--title",
            "Phase",
            "--no-edit",
            "--project",
            "demo",
        ])
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "phase",
            "create",
            "design",
            "--title",
            "Design",
            "--number",
            "1",
            "--no-edit",
            "--roadmap",
            "phase",
            "--project",
            "demo",
        ])
        .assert()
        .success();
    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .args(["worktree", "add", "phase", "--project", "demo"])
        .current_dir(src.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let wt = std::path::PathBuf::from(String::from_utf8_lossy(&out).trim().to_string());

    set_verify(plan.path(), "printf ran > sentinel.txt");
    let (code, _) = verify(
        plan.path(),
        src.path(),
        &[
            "run",
            "--item",
            "phase/phase-1-design",
            "--format",
            "json",
            "--project",
            "demo",
        ],
    );
    assert_eq!(code, 0);
    assert!(
        wt.join("sentinel.txt").exists(),
        "a roadmap literally named `phase` must still resolve via `phase/<stem>`"
    );
}

#[test]
fn verify_run_surfaces_the_real_unknown_stem_error_for_a_roadmap_literally_named_phase() {
    let plan = init_plan_repo();
    let src = init_source_repo();
    set_verify(plan.path(), "true");
    // A roadmap literally named `phase` exists, so `phase/no-such-stem` is a
    // WELL-FORMED reference into it (roadmap `phase`, stem `no-such-stem`) —
    // just one whose stem does not exist. It must surface the real,
    // actionable "phase not found" error `resolve_item` produces, never the
    // generic "names no worktree" grammar message: the reference itself is
    // not malformed, only the stem is unknown.
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "roadmap",
            "create",
            "phase",
            "--title",
            "Phase",
            "--no-edit",
            "--project",
            "demo",
        ])
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "verify",
            "run",
            "--item",
            "phase/no-such-stem",
            "--project",
            "demo",
        ])
        .current_dir(src.path())
        .assert()
        .code(3)
        .stderr(
            predicate::str::contains("phase 'phase/no-such-stem' not found")
                .and(predicate::str::contains("rdm phase list"))
                .and(predicate::str::contains("names no worktree").not()),
        );
}

#[test]
fn verify_run_resolves_a_roadmap_item_and_a_numeric_phase_item() {
    let plan = init_plan_repo();
    let src = init_source_repo();
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "phase",
            "create",
            "design",
            "--title",
            "Design",
            "--number",
            "1",
            "--no-edit",
            "--roadmap",
            "auth",
            "--project",
            "demo",
        ])
        .assert()
        .success();
    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .args(["worktree", "add", "auth", "--project", "demo"])
        .current_dir(src.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let wt = std::path::PathBuf::from(String::from_utf8_lossy(&out).trim().to_string());

    // `roadmap/<slug>` — whole-roadmap kind-prefixed grammar.
    set_verify(plan.path(), "printf roadmap-form > sentinel-roadmap.txt");
    let (code, _) = verify(
        plan.path(),
        src.path(),
        &[
            "run",
            "--item",
            "roadmap/auth",
            "--format",
            "json",
            "--project",
            "demo",
        ],
    );
    assert_eq!(code, 0);
    assert!(
        wt.join("sentinel-roadmap.txt").exists(),
        "roadmap/<slug> must resolve to the roadmap worktree"
    );

    // `phase/<roadmap>/<number>` — the numeric-stem form, resolved via
    // `resolve_phase_stem`.
    set_verify(
        plan.path(),
        "printf numeric-phase-form > sentinel-numeric.txt",
    );
    let (code, _) = verify(
        plan.path(),
        src.path(),
        &[
            "run",
            "--item",
            "phase/auth/1",
            "--format",
            "json",
            "--project",
            "demo",
        ],
    );
    assert_eq!(code, 0);
    assert!(
        wt.join("sentinel-numeric.txt").exists(),
        "phase/<roadmap>/<number> must resolve to the roadmap worktree"
    );
}

#[test]
fn verify_run_resolves_a_phase_of_a_roadmap_literally_named_roadmap() {
    let plan = init_plan_repo(); // already has a roadmap `auth`
    let src = init_source_repo();
    // A project roadmap literally named `roadmap` (legal — `roadmap` is not
    // in RESERVED_ROADMAP_SLUGS, only task/plan/src/change are). Before this
    // fix, `roadmap/<stem>` could never address one of ITS OWN phases:
    // `ReviewTarget::from_str` parses `roadmap/<rest>` as
    // `ReviewTarget::Roadmap { roadmap: rest }` whenever `rest` has no `/`,
    // so `normalize_item_grammar` collapsed it to the bare slug `<rest>` and
    // it resolved (or failed to) as an unrelated roadmap named `<rest>`.
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "roadmap",
            "create",
            "roadmap",
            "--title",
            "Roadmap",
            "--no-edit",
            "--project",
            "demo",
        ])
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "phase",
            "create",
            "design",
            "--title",
            "Design",
            "--number",
            "1",
            "--no-edit",
            "--roadmap",
            "roadmap",
            "--project",
            "demo",
        ])
        .assert()
        .success();
    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .args(["worktree", "add", "roadmap", "--project", "demo"])
        .current_dir(src.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let roadmap_wt = std::path::PathBuf::from(String::from_utf8_lossy(&out).trim().to_string());

    set_verify(plan.path(), "printf phase-form > sentinel-phase.txt");
    let (code, _) = verify(
        plan.path(),
        src.path(),
        &[
            "run",
            "--item",
            "roadmap/phase-1-design",
            "--format",
            "json",
            "--project",
            "demo",
        ],
    );
    assert_eq!(code, 0);
    assert!(
        roadmap_wt.join("sentinel-phase.txt").exists(),
        "roadmap/<stem> must resolve to the phase of the roadmap literally named `roadmap`, \
         not fail looking for a roadmap named `phase-1-design`"
    );

    // An ordinary `--item roadmap/<slug>` reference must keep resolving as a
    // WHOLE roadmap even in the presence of the `roadmap`-named roadmap —
    // the carve-out must not fire for a slug that is not one of its phases.
    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .args(["worktree", "add", "auth", "--project", "demo"])
        .current_dir(src.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let auth_wt = std::path::PathBuf::from(String::from_utf8_lossy(&out).trim().to_string());

    set_verify(plan.path(), "printf roadmap-form > sentinel-roadmap.txt");
    let (code, _) = verify(
        plan.path(),
        src.path(),
        &[
            "run",
            "--item",
            "roadmap/auth",
            "--format",
            "json",
            "--project",
            "demo",
        ],
    );
    assert_eq!(code, 0);
    assert!(
        auth_wt.join("sentinel-roadmap.txt").exists(),
        "an ordinary roadmap/<slug> reference must still resolve as a whole roadmap"
    );
}

#[test]
fn verify_run_resolves_a_task_item_to_its_task_worktree() {
    let plan = init_plan_repo();
    let src = init_source_repo();
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "task",
            "create",
            "fix-bug",
            "--title",
            "Fix bug",
            "--no-edit",
            "--project",
            "demo",
        ])
        .assert()
        .success();
    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .args(["worktree", "add", "task/fix-bug", "--project", "demo"])
        .current_dir(src.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let wt = std::path::PathBuf::from(String::from_utf8_lossy(&out).trim().to_string());

    set_verify(plan.path(), "printf ran > sentinel.txt");
    let (code, _) = verify(
        plan.path(),
        src.path(),
        &[
            "run",
            "--item",
            "task/fix-bug",
            "--format",
            "json",
            "--project",
            "demo",
        ],
    );
    assert_eq!(code, 0);
    assert!(
        wt.join("sentinel.txt").exists(),
        "a task item must resolve to its own task worktree"
    );
}

#[test]
fn verify_run_never_suggests_a_per_phase_worktree_for_a_task_item_miss() {
    let plan = init_plan_repo();
    let src = init_source_repo();
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "task",
            "create",
            "fix-bug",
            "--title",
            "Fix bug",
            "--no-edit",
            "--project",
            "demo",
        ])
        .assert()
        .success();
    set_verify(plan.path(), "true");
    // No worktree registered for the task at all.
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "verify",
            "run",
            "--item",
            "task/fix-bug",
            "--project",
            "demo",
        ])
        .current_dir(src.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("rdm worktree add task/fix-bug"));
}

// --- per-project dispatch.verify --------------------------------------------

fn init_multi_project_plan_repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    let p = dir.path();
    rdm().arg("--root").arg(p).arg("init").assert().success();
    for name in ["a", "b", "c"] {
        rdm()
            .arg("--root")
            .arg(p)
            .args(["project", "create", name])
            .assert()
            .success();
    }
    dir
}

fn set_project_verify(plan: &Path, project: &str, cmd: &str) {
    rdm()
        .arg("--root")
        .arg(plan)
        .args([
            "config",
            "set",
            "dispatch.verify",
            cmd,
            "--project",
            project,
        ])
        .assert()
        .success();
}

fn json_of(out: &str) -> Value {
    serde_json::from_str(out).unwrap()
}

#[test]
fn verify_resolve_reports_each_projects_own_command() {
    let plan = init_multi_project_plan_repo();
    let cwd = init_source_repo();
    set_project_verify(plan.path(), "a", "echo from-a");
    set_project_verify(plan.path(), "b", "echo from-b");
    for (p, want) in [("a", "echo from-a"), ("b", "echo from-b")] {
        let (code, out) = verify(
            plan.path(),
            cwd.path(),
            &["resolve", "--format", "json", "--project", p],
        );
        assert_eq!(code, 0);
        let j = json_of(&out);
        assert_eq!(j["resolved"], true);
        assert_eq!(j["command"], want);
    }
}

#[test]
fn verify_run_runs_each_projects_own_command() {
    let plan = init_multi_project_plan_repo();
    let cwd = init_source_repo();
    set_project_verify(plan.path(), "a", "echo from-a");
    set_project_verify(plan.path(), "b", "echo from-b");
    for (p, want, other) in [("a", "from-a", "from-b"), ("b", "from-b", "from-a")] {
        let (code, out) = verify(
            plan.path(),
            cwd.path(),
            &["run", "--format", "json", "--project", p],
        );
        assert_eq!(code, 0);
        let tail = json_of(&out)["tail"].as_str().unwrap().to_string();
        assert!(tail.contains(want), "{p}: {tail}");
        assert!(!tail.contains(other), "{p}: {tail}");
    }
}

#[test]
fn verify_uses_the_plan_repo_wide_command_for_a_project_without_an_override() {
    let plan = init_multi_project_plan_repo();
    let cwd = init_source_repo();
    set_verify(plan.path(), "echo repo-wide");
    set_project_verify(plan.path(), "a", "echo from-a");
    let (_, out) = verify(
        plan.path(),
        cwd.path(),
        &["resolve", "--format", "json", "--project", "c"],
    );
    assert_eq!(json_of(&out)["command"], "echo repo-wide");
    let (code, out) = verify(
        plan.path(),
        cwd.path(),
        &["run", "--format", "json", "--project", "c"],
    );
    assert_eq!(code, 0);
    assert!(
        json_of(&out)["tail"]
            .as_str()
            .unwrap()
            .contains("repo-wide")
    );
}

#[test]
fn verify_run_is_unresolved_for_a_project_with_no_override_and_no_repo_value() {
    let plan = init_multi_project_plan_repo();
    let cwd = init_source_repo();
    set_project_verify(plan.path(), "a", "echo from-a");
    let (code, out) = verify(
        plan.path(),
        cwd.path(),
        &["run", "--format", "json", "--project", "c"],
    );
    assert_eq!(code, 2);
    assert_eq!(json_of(&out)["resolved"], false);
    let (code, out) = verify(
        plan.path(),
        cwd.path(),
        &["resolve", "--format", "json", "--project", "c"],
    );
    assert_eq!(code, 0);
    assert_eq!(json_of(&out)["resolved"], false);
}

#[test]
fn verify_run_resolves_an_item_in_its_projects_command() {
    let plan = init_multi_project_plan_repo();
    let src = init_source_repo();
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "task",
            "create",
            "fix-a",
            "--title",
            "Fix a",
            "--no-edit",
            "--project",
            "a",
        ])
        .assert()
        .success();
    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .args(["worktree", "add", "task/fix-a", "--project", "a"])
        .current_dir(src.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let wt = std::path::PathBuf::from(String::from_utf8_lossy(&out).trim().to_string());
    set_project_verify(plan.path(), "a", "printf a > sentinel.txt");
    set_project_verify(plan.path(), "b", "printf b > sentinel.txt");
    let (code, _) = verify(
        plan.path(),
        src.path(),
        &[
            "run",
            "--item",
            "task/fix-a",
            "--format",
            "json",
            "--project",
            "a",
        ],
    );
    assert_eq!(code, 0);
    assert_eq!(
        std::fs::read_to_string(wt.join("sentinel.txt")).unwrap(),
        "a"
    );
}

#[test]
fn verify_run_refusal_names_project_for_a_multiline_project_override() {
    let plan = init_multi_project_plan_repo();
    let cwd = init_source_repo();
    set_project_verify(plan.path(), "a", "cargo fmt --check\ncargo test");
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args(["verify", "run", "--project", "a"])
        .current_dir(cwd.path())
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("multi-line").and(predicate::str::contains("--project a")),
        );
}

#[test]
fn verify_env_override_wins_over_the_project_override() {
    let plan = init_multi_project_plan_repo();
    let cwd = init_source_repo();
    set_project_verify(plan.path(), "a", "echo from-a");
    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .env("RDM_DISPATCH_VERIFY", "echo from-env")
        .args(["verify", "resolve", "--format", "json", "--project", "a"])
        .current_dir(cwd.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(
        json_of(&String::from_utf8_lossy(&out))["command"],
        "echo from-env"
    );
}

#[test]
fn verify_empty_env_override_falls_through_to_the_project_override() {
    let plan = init_multi_project_plan_repo();
    let cwd = init_source_repo();
    set_project_verify(plan.path(), "a", "echo from-a");
    for blank in ["", "   \t "] {
        let out = rdm()
            .arg("--root")
            .arg(plan.path())
            .env("RDM_DISPATCH_VERIFY", blank)
            .args(["verify", "run", "--format", "json", "--project", "a"])
            .current_dir(cwd.path())
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let j = json_of(&String::from_utf8_lossy(&out));
        assert_eq!(j["command"], "echo from-a", "{blank:?}");
        assert!(j["tail"].as_str().unwrap().contains("from-a"), "{blank:?}");
    }
}

#[test]
fn verify_empty_env_override_with_nothing_configured_is_unresolved() {
    let plan = init_plan_repo();
    let cwd = init_source_repo();
    for blank in ["", "  "] {
        let out = rdm()
            .arg("--root")
            .arg(plan.path())
            .env("RDM_DISPATCH_VERIFY", blank)
            .args(["verify", "run", "--format", "json", "--project", "demo"])
            .current_dir(cwd.path())
            .assert()
            .code(2)
            .get_output()
            .stdout
            .clone();
        let j = json_of(&String::from_utf8_lossy(&out));
        assert_eq!(j["resolved"], false, "{blank:?}");
    }
}

#[test]
fn verify_run_refuses_a_multi_line_env_override_naming_the_variable() {
    let plan = init_plan_repo();
    let cwd = init_source_repo();
    rdm()
        .arg("--root")
        .arg(plan.path())
        .env("RDM_DISPATCH_VERIFY", "echo one\necho two")
        .args(["verify", "run", "--project", "demo"])
        .current_dir(cwd.path())
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("RDM_DISPATCH_VERIFY")
                .and(predicate::str::contains("rdm config set").not()),
        );
}

#[test]
fn verify_run_executes_a_single_line_env_override() {
    let plan = init_plan_repo();
    let cwd = init_source_repo();
    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .env("RDM_DISPATCH_VERIFY", "echo env-ran")
        .args(["verify", "run", "--format", "json", "--project", "demo"])
        .current_dir(cwd.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let j = json_of(&String::from_utf8_lossy(&out));
    assert_eq!(j["exit"], 0);
    assert!(j["tail"].as_str().unwrap().contains("env-ran"));
}

#[test]
fn verify_blank_project_override_falls_through_to_the_repo_wide_command() {
    let plan = init_multi_project_plan_repo();
    let cwd = init_source_repo();
    let toml_path = plan.path().join("rdm.toml");
    let mut toml = std::fs::read_to_string(&toml_path).unwrap();
    toml.push_str(
        "\n[dispatch]\nverify = \"echo repo-wide\"\n\n[projects.a.dispatch]\nverify = \"  \"\n",
    );
    std::fs::write(&toml_path, toml).unwrap();
    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .env_remove("RDM_DISPATCH_VERIFY")
        .args(["verify", "run", "--format", "json", "--project", "a"])
        .current_dir(cwd.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let j = json_of(&String::from_utf8_lossy(&out));
    assert!(j["tail"].as_str().unwrap().contains("repo-wide"));
}

#[test]
fn verify_blank_everywhere_is_unresolved() {
    let plan = init_multi_project_plan_repo();
    let cwd = init_source_repo();
    let toml_path = plan.path().join("rdm.toml");
    let mut toml = std::fs::read_to_string(&toml_path).unwrap();
    toml.push_str("\n[dispatch]\nverify = \" \"\n\n[projects.a.dispatch]\nverify = \"  \"\n");
    std::fs::write(&toml_path, toml).unwrap();
    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .env("RDM_DISPATCH_VERIFY", " ")
        .args(["verify", "run", "--format", "json", "--project", "a"])
        .current_dir(cwd.path())
        .assert()
        .code(2)
        .get_output()
        .stdout
        .clone();
    let j = json_of(&String::from_utf8_lossy(&out));
    assert_eq!(j["resolved"], false);
}

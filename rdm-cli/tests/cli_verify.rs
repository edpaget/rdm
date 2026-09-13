//! Integration tests for `rdm verify resolve` / `rdm verify run` — the CLI
//! surface over the repo-only `dispatch.verify` key (`docs/verify-gate.md`).

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use std::path::Path;
use tempfile::TempDir;

fn rdm() -> Command {
    let mut cmd = Command::cargo_bin("rdm").unwrap();
    cmd.env("XDG_CONFIG_HOME", "/dev/null/nonexistent");
    cmd.env_remove("RDM_PROJECT").env_remove("RDM_ROOT");
    cmd
}

fn git(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@test.com")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@test.com")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
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

fn init_source_repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    let p = dir.path();
    git(p, &["init", "-b", "main"]);
    std::fs::write(p.join("README.md"), "# project\n").unwrap();
    git(p, &["add", "."]);
    git(p, &["commit", "-m", "initial"]);
    dir
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
        .failure()
        .stderr(predicate::str::contains("rdm worktree add auth"));
}

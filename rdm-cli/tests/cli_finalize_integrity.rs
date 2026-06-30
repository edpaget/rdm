//! Integration tests for the empty-finalize data-integrity guard.
//!
//! When a phase or task is moved to `needs-review` the CLI checks, against the
//! source repo discovered from the command's CWD, whether HEAD actually carries
//! a committed diff worth reviewing. If it does not, a non-blocking warning is
//! printed to stderr (the status transition still succeeds).
//!
//! Baseline rules under test:
//! - PHASE: baseline against the nearest prior finalized sibling phase in the
//!   same roadmap (the one-worktree-per-roadmap model shares a long-lived branch
//!   that is never an ancestor of the default branch). Warn when HEAD has not
//!   advanced past that sibling's stamped commit.
//! - First phase of a roadmap / standalone TASK (no sibling): fall back to the
//!   default-branch baseline (warn when HEAD has no commits beyond it).

use assert_cmd::Command;
use predicates::prelude::*;
use std::path::Path;
use tempfile::TempDir;

fn rdm() -> Command {
    let mut cmd = Command::cargo_bin("rdm").unwrap();
    cmd.env("XDG_CONFIG_HOME", "/dev/null/nonexistent");
    cmd
}

fn git(dir: &Path, args: &[&str]) -> std::process::Output {
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
    out
}

/// Initialize a source/code repo on `branch` with a single base commit.
fn init_source_repo(branch: &str) -> TempDir {
    let dir = TempDir::new().unwrap();
    git(dir.path(), &["init", "-b", branch]);
    std::fs::write(dir.path().join("base.txt"), "base").unwrap();
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["commit", "-m", "base"]);
    dir
}

/// Commit a new file in `dir`, returning nothing (HEAD advances).
fn commit_file(dir: &Path, name: &str) {
    std::fs::write(dir.join(name), name).unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-m", name]);
}

/// A plan repo with project `demo`, a roadmap `rm` carrying two phases
/// (`phase-1-one`, `phase-2-two`), and a standalone task `solo`.
fn init_plan_repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("init")
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "create", "demo"])
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "roadmap",
            "create",
            "rm",
            "--title",
            "Roadmap",
            "--no-edit",
            "--project",
            "demo",
        ])
        .assert()
        .success();
    for (slug, num) in [("one", "1"), ("two", "2")] {
        rdm()
            .arg("--root")
            .arg(dir.path())
            .args([
                "phase",
                "create",
                slug,
                "--title",
                slug,
                "--number",
                num,
                "--no-edit",
                "--roadmap",
                "rm",
                "--project",
                "demo",
            ])
            .assert()
            .success();
    }
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "task",
            "create",
            "solo",
            "--title",
            "Solo",
            "--no-edit",
            "--project",
            "demo",
        ])
        .assert()
        .success();
    dir
}

/// Set a `default_branch` in the plan repo's `rdm.toml`.
fn set_default_branch(plan: &TempDir, branch: &str) {
    let path = plan.path().join("rdm.toml");
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    std::fs::write(
        &path,
        format!("{existing}\ndefault_branch = \"{branch}\"\n"),
    )
    .unwrap();
}

/// Run `phase update <stem> --status needs-review` from `cwd`, returning the
/// assert handle so callers can inspect stdout/stderr.
fn finalize_phase(plan: &TempDir, cwd: &Path, stem: &str) -> assert_cmd::assert::Assert {
    rdm()
        .arg("--root")
        .arg(plan.path())
        .current_dir(cwd)
        .args([
            "phase",
            "update",
            stem,
            "--status",
            "needs-review",
            "--no-edit",
            "--roadmap",
            "rm",
            "--project",
            "demo",
        ])
        .assert()
}

/// Run `task update <slug> --status needs-review` from `cwd`.
fn finalize_task(plan: &TempDir, cwd: &Path, slug: &str) -> assert_cmd::assert::Assert {
    rdm()
        .arg("--root")
        .arg(plan.path())
        .current_dir(cwd)
        .args([
            "task",
            "update",
            slug,
            "--status",
            "needs-review",
            "--no-edit",
            "--project",
            "demo",
        ])
        .assert()
}

// ---------------------------------------------------------------------------
// Sibling-baseline phase tests (the dominant one-worktree-per-roadmap path).
// ---------------------------------------------------------------------------

#[test]
fn phase_update_second_phase_with_no_new_commit_warns() {
    // Source repo on a long-lived feature branch (never merged to main).
    let src = init_source_repo("main");
    git(src.path(), &["checkout", "-b", "roadmap/rm"]);
    commit_file(src.path(), "phase1.txt"); // phase-1's real work

    let plan = init_plan_repo();

    // Finalize phase-1: stamps review_sha = current HEAD (the phase1 commit).
    finalize_phase(&plan, src.path(), "phase-1-one").success();

    // Finalize phase-2 WITHOUT any new commit: HEAD is still phase-1's commit,
    // so there is nothing new to review for phase-2.
    finalize_phase(&plan, src.path(), "phase-2-two")
        .success()
        .stderr(predicate::str::contains("nothing to review"));
}

#[test]
fn phase_update_second_phase_with_new_commit_emits_no_warning() {
    let src = init_source_repo("main");
    git(src.path(), &["checkout", "-b", "roadmap/rm"]);
    commit_file(src.path(), "phase1.txt");

    let plan = init_plan_repo();
    finalize_phase(&plan, src.path(), "phase-1-one").success();

    // Phase-2 makes a real commit before finalizing.
    commit_file(src.path(), "phase2.txt");
    finalize_phase(&plan, src.path(), "phase-2-two")
        .success()
        .stderr(predicate::str::contains("nothing to review").not());
}

#[test]
fn phase_update_out_of_order_finalize_with_no_new_commit_warns() {
    // 3-phase roadmap on a shared feature branch (one-worktree-per-roadmap
    // model). Finalize phase-1 (commit A), then finalize phase-3 out of order
    // (commit C), then finalize phase-2 with NO new commit (HEAD still C).
    //
    // The old "highest phase number below current" baseline picks phase-1
    // (commit A) as phase-2's baseline, since phase-3's number is >= 2 and
    // gets filtered out. review_sha (C) != baseline (A), so the old logic
    // missed the warning even though nothing was committed since the last
    // finalize (phase-3, commit C). The fix must pick the nearest-prior
    // *reachable* sibling by recency, not by phase number, which is phase-3
    // here.
    let src = init_source_repo("main");
    git(src.path(), &["checkout", "-b", "roadmap/rm"]);

    let plan = init_plan_repo();
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "phase",
            "create",
            "three",
            "--title",
            "three",
            "--number",
            "3",
            "--no-edit",
            "--roadmap",
            "rm",
            "--project",
            "demo",
        ])
        .assert()
        .success();

    commit_file(src.path(), "phase1.txt"); // commit A
    finalize_phase(&plan, src.path(), "phase-1-one").success();

    commit_file(src.path(), "phase3.txt"); // commit C
    finalize_phase(&plan, src.path(), "phase-3-three").success();

    // Finalize phase-2 with HEAD still at commit C: nothing new since the
    // nearest prior finalize (phase-3).
    finalize_phase(&plan, src.path(), "phase-2-two")
        .success()
        .stderr(predicate::str::contains("nothing to review"));
}

// ---------------------------------------------------------------------------
// First-phase / default-branch fallback tests.
// ---------------------------------------------------------------------------

#[test]
fn phase_update_first_phase_with_no_new_commits_warns() {
    // Feature branch tip == main tip: no work committed at all.
    let src = init_source_repo("main");
    git(src.path(), &["checkout", "-b", "roadmap/rm"]);

    let plan = init_plan_repo();
    finalize_phase(&plan, src.path(), "phase-1-one")
        .success()
        .stderr(predicate::str::contains("nothing to review"));
}

#[test]
fn phase_update_first_phase_with_diverged_branch_emits_no_warning() {
    let src = init_source_repo("main");
    git(src.path(), &["checkout", "-b", "roadmap/rm"]);
    commit_file(src.path(), "work.txt"); // real divergence from main

    let plan = init_plan_repo();
    finalize_phase(&plan, src.path(), "phase-1-one")
        .success()
        .stderr(predicate::str::contains("nothing to review").not());
}

#[test]
fn phase_update_on_default_branch_emits_no_warning() {
    // Running directly on the default branch: "diff vs trunk" is degenerate, so
    // the guard is skipped even though HEAD trivially has no diff vs itself.
    let src = init_source_repo("main");

    let plan = init_plan_repo();
    finalize_phase(&plan, src.path(), "phase-1-one")
        .success()
        .stderr(predicate::str::contains("nothing to review").not());
}

#[test]
fn phase_update_respects_custom_default_branch() {
    let src = init_source_repo("develop");
    git(src.path(), &["checkout", "-b", "roadmap/rm"]); // no new commit

    let plan = init_plan_repo();
    set_default_branch(&plan, "develop");

    finalize_phase(&plan, src.path(), "phase-1-one")
        .success()
        .stderr(
            predicate::str::contains("nothing to review").and(predicate::str::contains("develop")),
        );
}

// ---------------------------------------------------------------------------
// Task tests (no siblings → always the default-branch fallback).
// ---------------------------------------------------------------------------

#[test]
fn task_update_with_no_new_commits_warns() {
    let src = init_source_repo("main");
    git(src.path(), &["checkout", "-b", "feature"]); // no new commit beyond main

    let plan = init_plan_repo();
    finalize_task(&plan, src.path(), "solo")
        .success()
        .stderr(predicate::str::contains("nothing to review"));
}

#[test]
fn task_update_with_new_commit_emits_no_warning() {
    let src = init_source_repo("main");
    git(src.path(), &["checkout", "-b", "feature"]);
    commit_file(src.path(), "work.txt");

    let plan = init_plan_repo();
    finalize_task(&plan, src.path(), "solo")
        .success()
        .stderr(predicate::str::contains("nothing to review").not());
}

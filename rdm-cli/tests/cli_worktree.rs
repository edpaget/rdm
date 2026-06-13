//! Integration tests for `rdm worktree` against separate temp plan + project
//! repos. Commands are run with `.current_dir(project_repo)` so the project
//! repo is discovered from CWD, the way a user would invoke them.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn rdm() -> Command {
    let mut cmd = Command::cargo_bin("rdm").unwrap();
    cmd.env("XDG_CONFIG_HOME", "/dev/null/nonexistent");
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

/// A plan repo with a project, one roadmap and one phase, plus a task.
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
            "my-roadmap",
            "--title",
            "My Roadmap",
            "--project",
            "demo",
        ])
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "create",
            "foo",
            "--title",
            "Phase One",
            "--number",
            "1",
            "--no-edit",
            "--roadmap",
            "my-roadmap",
            "--project",
            "demo",
        ])
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
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
    dir
}

/// A project (code) repo with one commit on `main`.
fn init_project_repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    git(dir.path(), &["init", "-b", "main"]);
    fs::write(dir.path().join("README.md"), "# project").unwrap();
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["commit", "-m", "initial"]);
    dir
}

/// Builds an `rdm worktree` command rooted at `plan`, running in `project`.
fn worktree_cmd(plan: &TempDir, project: &TempDir) -> Command {
    let mut cmd = rdm();
    cmd.arg("--root")
        .arg(plan.path())
        .current_dir(project.path())
        .arg("worktree");
    cmd
}

#[test]
fn add_prints_path_and_is_idempotent() {
    let plan = init_plan_repo();
    let project = init_project_repo();

    let assert = worktree_cmd(&plan, &project)
        .args(["add", "task/fix-bug", "--project", "demo"])
        .assert()
        .success();
    let path = String::from_utf8_lossy(&assert.get_output().stdout)
        .trim()
        .to_string();
    assert!(Path::new(&path).exists(), "printed path should exist");

    // Second add is idempotent: same path, exit 0.
    worktree_cmd(&plan, &project)
        .args(["add", "task/fix-bug", "--project", "demo"])
        .assert()
        .success()
        .stdout(predicate::str::contains(&path));
}

#[test]
fn add_json_format_parses() {
    let plan = init_plan_repo();
    let project = init_project_repo();

    let assert = worktree_cmd(&plan, &project)
        .args([
            "add",
            "task/fix-bug",
            "--project",
            "demo",
            "--format",
            "json",
        ])
        .assert()
        .success();
    let v: serde_json::Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("valid json");
    assert_eq!(v["item"], "task/fix-bug");
    assert_eq!(v["branch"], "task/fix-bug");
    assert_eq!(v["created"], true);
    assert!(v["path"].as_str().unwrap().contains("__worktrees"));
}

#[test]
fn add_resolves_phase_by_number() {
    let plan = init_plan_repo();
    let project = init_project_repo();

    worktree_cmd(&plan, &project)
        .args([
            "add",
            "my-roadmap/1",
            "--project",
            "demo",
            "--format",
            "json",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("my-roadmap/phase-1-foo"))
        .stdout(predicate::str::contains("phase/my-roadmap/phase-1-foo"));
}

#[test]
fn add_unknown_item_fails_cleanly() {
    let plan = init_plan_repo();
    let project = init_project_repo();

    worktree_cmd(&plan, &project)
        .args(["add", "my-roadmap/nope", "--project", "demo"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));

    // No worktree should have been created.
    worktree_cmd(&plan, &project)
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains("No rdm worktrees"));
}

#[test]
fn list_shows_dirty_flag() {
    let plan = init_plan_repo();
    let project = init_project_repo();

    let assert = worktree_cmd(&plan, &project)
        .args(["add", "task/fix-bug", "--project", "demo"])
        .assert()
        .success();
    let path = String::from_utf8_lossy(&assert.get_output().stdout)
        .trim()
        .to_string();

    // Clean worktree.
    worktree_cmd(&plan, &project)
        .args(["list", "--format", "json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"dirty\": false"));

    // Dirty it.
    fs::write(Path::new(&path).join("scratch.txt"), "wip").unwrap();
    worktree_cmd(&plan, &project)
        .args(["list", "--format", "json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"dirty\": true"));
}

#[test]
fn remove_guards_dirty_then_force() {
    let plan = init_plan_repo();
    let project = init_project_repo();

    let assert = worktree_cmd(&plan, &project)
        .args(["add", "task/fix-bug", "--project", "demo"])
        .assert()
        .success();
    let path = String::from_utf8_lossy(&assert.get_output().stdout)
        .trim()
        .to_string();
    fs::write(Path::new(&path).join("scratch.txt"), "wip").unwrap();

    // Dirty remove refused.
    worktree_cmd(&plan, &project)
        .args(["remove", "task/fix-bug"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("uncommitted changes"));
    assert!(Path::new(&path).exists());

    // Forced remove succeeds.
    worktree_cmd(&plan, &project)
        .args(["remove", "task/fix-bug", "--force"])
        .assert()
        .success();
    assert!(!Path::new(&path).exists());
}

#[test]
fn remove_delete_branch() {
    let plan = init_plan_repo();
    let project = init_project_repo();

    worktree_cmd(&plan, &project)
        .args(["add", "task/fix-bug", "--project", "demo"])
        .assert()
        .success();
    worktree_cmd(&plan, &project)
        .args(["remove", "task/fix-bug", "--delete-branch"])
        .assert()
        .success();

    // Branch is gone.
    let out = std::process::Command::new("git")
        .args(["branch", "--list", "task/fix-bug"])
        .current_dir(project.path())
        .output()
        .unwrap();
    assert!(out.stdout.is_empty(), "branch should be deleted");
}

#[test]
fn remove_by_phase_number() {
    let plan = init_plan_repo();
    let project = init_project_repo();

    // Add by stem, then remove by the phase-number shorthand.
    worktree_cmd(&plan, &project)
        .args(["add", "my-roadmap/phase-1-foo", "--project", "demo"])
        .assert()
        .success();
    worktree_cmd(&plan, &project)
        .args(["remove", "my-roadmap/1", "--project", "demo"])
        .assert()
        .success();
    worktree_cmd(&plan, &project)
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains("No rdm worktrees"));
}

#[test]
fn not_a_git_repo_errors() {
    let plan = init_plan_repo();
    // A non-git directory as CWD.
    let bare = TempDir::new().unwrap();

    let mut cmd = rdm();
    cmd.arg("--root")
        .arg(plan.path())
        .current_dir(bare.path())
        .args(["worktree", "add", "task/fix-bug", "--project", "demo"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not inside a git repository"));
}

#[test]
fn inside_plan_repo_errors() {
    let plan = init_plan_repo();

    // Run from inside the plan repo itself — should be refused.
    let mut cmd = rdm();
    cmd.arg("--root")
        .arg(plan.path())
        .current_dir(plan.path())
        .args(["worktree", "add", "task/fix-bug", "--project", "demo"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("plan repo"));
}

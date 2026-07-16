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
    // Keep tests hermetic: don't let an ambient RDM_PROJECT/RDM_ROOT (e.g. from
    // .mise.toml during local runs) leak in and mask missing flags. CI has
    // neither set, so tests that rely on them pass locally but fail in CI.
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

/// Builds an `rdm worktree` command rooted at `plan`, running in an arbitrary
/// `cwd` — used when the invoking checkout is a linked worktree rather than
/// the project's main working tree.
fn worktree_cmd_at(plan: &TempDir, cwd: &Path) -> Command {
    let mut cmd = rdm();
    cmd.arg("--root")
        .arg(plan.path())
        .current_dir(cwd)
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
fn current_reports_item_from_inside_worktree() {
    let plan = init_plan_repo();
    let project = init_project_repo();

    let assert = worktree_cmd(&plan, &project)
        .args(["add", "task/fix-bug", "--project", "demo"])
        .assert()
        .success();
    let wt_path = String::from_utf8_lossy(&assert.get_output().stdout)
        .trim()
        .to_string();

    // `worktree current` run from INSIDE the created worktree reports the item.
    let assert = rdm()
        .arg("--root")
        .arg(plan.path())
        .current_dir(&wt_path)
        .args(["worktree", "current", "--format", "json"])
        .assert()
        .success();
    let v: serde_json::Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("valid json");
    assert_eq!(v["item"], "task/fix-bug");
    assert_eq!(v["branch"], "task/fix-bug");
    assert_eq!(v["rdm_managed"], true);
}

#[test]
fn current_is_null_from_main_checkout() {
    let plan = init_plan_repo();
    let project = init_project_repo();

    // Text: main checkout on `main` is not an item context.
    worktree_cmd(&plan, &project)
        .args(["current"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Not in an rdm worktree."));

    // JSON: null, machine-clean.
    let assert = worktree_cmd(&plan, &project)
        .args(["current", "--format", "json"])
        .assert()
        .success();
    let v: serde_json::Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("valid json");
    assert!(v.is_null(), "expected null, got {v}");
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
        .args(["remove", "task/fix-bug", "--project", "demo"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("uncommitted changes"));
    assert!(Path::new(&path).exists());

    // Forced remove succeeds.
    worktree_cmd(&plan, &project)
        .args(["remove", "task/fix-bug", "--force", "--project", "demo"])
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
        .args([
            "remove",
            "task/fix-bug",
            "--delete-branch",
            "--project",
            "demo",
        ])
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
fn prune_removes_done_keeps_open() {
    let plan = init_plan_repo();
    let project = init_project_repo();

    // Add a second phase and mark phase 1 done.
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "phase",
            "create",
            "bar",
            "--title",
            "Phase Two",
            "--number",
            "2",
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
        .arg(plan.path())
        .args([
            "phase",
            "update",
            "phase-1-foo",
            "--status",
            "done",
            "--no-edit",
            "--roadmap",
            "my-roadmap",
            "--project",
            "demo",
        ])
        .assert()
        .success();

    // Stand up a worktree for each phase.
    worktree_cmd(&plan, &project)
        .args(["add", "my-roadmap/phase-1-foo", "--project", "demo"])
        .assert()
        .success();
    worktree_cmd(&plan, &project)
        .args(["add", "my-roadmap/phase-2-bar", "--project", "demo"])
        .assert()
        .success();

    // Batch prune removes only the done phase's worktree.
    worktree_cmd(&plan, &project)
        .args(["prune", "--project", "demo"])
        .assert()
        .success()
        .stdout(predicate::str::contains("phase-1-foo"));

    // The done one is gone; the open one remains.
    worktree_cmd(&plan, &project)
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains("phase-2-bar"))
        .stdout(predicate::str::contains("phase-1-foo").not());
}

/// Stands up a done-phase worktree carrying an unmerged commit on its branch,
/// then returns `(plan, project)`. The done phase is `phase-1-foo` / branch
/// `phase/my-roadmap/phase-1-foo`.
fn setup_done_worktree_with_unmerged_branch() -> (TempDir, TempDir) {
    let plan = init_plan_repo();
    let project = init_project_repo();

    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "phase",
            "update",
            "phase-1-foo",
            "--status",
            "done",
            "--no-edit",
            "--roadmap",
            "my-roadmap",
            "--project",
            "demo",
        ])
        .assert()
        .success();

    let assert = worktree_cmd(&plan, &project)
        .args(["add", "my-roadmap/phase-1-foo", "--project", "demo"])
        .assert()
        .success();
    let wt_path = String::from_utf8_lossy(&assert.get_output().stdout)
        .trim()
        .to_string();

    // Unmerged commit on the worktree's branch → `git branch -d` will refuse.
    fs::write(Path::new(&wt_path).join("feature.txt"), "work").unwrap();
    git(Path::new(&wt_path), &["add", "."]);
    git(Path::new(&wt_path), &["commit", "-m", "feature work"]);

    (plan, project)
}

#[test]
fn prune_reports_branch_kept_for_unmerged() {
    let (plan, project) = setup_done_worktree_with_unmerged_branch();

    // Prune with --delete-branch (force off): worktree removed, branch retained.
    worktree_cmd(&plan, &project)
        .args(["prune", "--delete-branch", "--project", "demo"])
        .assert()
        .success()
        .stdout(predicate::str::contains("branch kept"))
        .stdout(predicate::str::contains("1 branch kept"));

    // The worktree is gone...
    worktree_cmd(&plan, &project)
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains("phase-1-foo").not());

    // ...but the branch survives.
    let out = std::process::Command::new("git")
        .args(["branch", "--list", "phase/my-roadmap/phase-1-foo"])
        .current_dir(project.path())
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .unwrap();
    assert!(
        !String::from_utf8_lossy(&out.stdout).trim().is_empty(),
        "unmerged branch should be retained"
    );
}

#[test]
fn prune_json_reports_branch_kept() {
    let (plan, project) = setup_done_worktree_with_unmerged_branch();

    let assert = worktree_cmd(&plan, &project)
        .args([
            "prune",
            "--delete-branch",
            "--project",
            "demo",
            "--format",
            "json",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    assert_eq!(json["branch_kept"], 1);
    assert_eq!(json["removed"], 0);
    assert_eq!(json["results"][0]["action"], "removed-branch-kept");
    let reason = json["results"][0]["reason"].as_str().unwrap();
    assert!(!reason.is_empty(), "per-result reason should be present");
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

/// Stands up a project repo with a second linked worktree on branch
/// `feature-x` that has diverged from `main` by one commit (`feature.txt`).
/// Returns `(plan, project, wt_dir, feature_path)`; `wt_dir` must be kept
/// alive by the caller for the duration of the test (its `Drop` cleans up
/// the linked worktree's directory).
fn setup_diverged_feature_worktree() -> (TempDir, TempDir, TempDir, std::path::PathBuf) {
    let plan = init_plan_repo();
    let project = init_project_repo();

    let wt_dir = TempDir::new().unwrap();
    let feature_path = wt_dir.path().join("feature-wt");

    git(
        project.path(),
        &[
            "worktree",
            "add",
            "-b",
            "feature-x",
            feature_path.to_str().unwrap(),
            "main",
        ],
    );
    fs::write(feature_path.join("feature.txt"), "wip").unwrap();
    git(&feature_path, &["add", "."]);
    git(&feature_path, &["commit", "-m", "feature work"]);

    (plan, project, wt_dir, feature_path)
}

#[test]
fn add_defaults_base_to_invoking_checkout_branch() {
    let (plan, _project, _wt_dir, feature_path) = setup_diverged_feature_worktree();

    // No --base: the new worktree should be based on feature-x, not main, so
    // it should inherit feature.txt.
    let assert = worktree_cmd_at(&plan, &feature_path)
        .args(["add", "task/fix-bug", "--project", "demo"])
        .assert()
        .success();
    let path = String::from_utf8_lossy(&assert.get_output().stdout)
        .trim()
        .to_string();
    assert!(
        Path::new(&path).join("feature.txt").exists(),
        "new worktree should be based on the invoking checkout's branch (feature-x), \
         not main — feature.txt should be present"
    );
}

#[test]
fn add_base_flag_overrides_invoking_branch() {
    let (plan, _project, _wt_dir, feature_path) = setup_diverged_feature_worktree();

    // Explicit --base main should win over the invoking checkout's branch.
    let assert = worktree_cmd_at(&plan, &feature_path)
        .args(["add", "task/fix-bug", "--project", "demo", "--base", "main"])
        .assert()
        .success();
    let path = String::from_utf8_lossy(&assert.get_output().stdout)
        .trim()
        .to_string();
    assert!(
        !Path::new(&path).join("feature.txt").exists(),
        "--base main should override the invoking branch default"
    );
}

#[test]
fn add_detached_head_falls_back_to_head_default() {
    let plan = init_plan_repo();
    let project = init_project_repo();

    let wt_dir = TempDir::new().unwrap();
    let detached_path = wt_dir.path().join("detached-wt");

    git(
        project.path(),
        &[
            "worktree",
            "add",
            "--detach",
            detached_path.to_str().unwrap(),
            "main",
        ],
    );

    // Advance `main` past the detached checkout's commit so the fallback ref
    // (HEAD at the main working tree) is distinguishable from the detached
    // SHA: only a worktree based on the fallback contains `main-only.txt`.
    fs::write(project.path().join("main-only.txt"), "post-detach").unwrap();
    git(project.path(), &["add", "."]);
    git(
        project.path(),
        &["commit", "-m", "advance main past detach"],
    );

    let assert = worktree_cmd_at(&plan, &detached_path)
        .args(["add", "task/fix-bug", "--project", "demo"])
        .assert()
        .success();
    let path = String::from_utf8_lossy(&assert.get_output().stdout)
        .trim()
        .to_string();
    assert!(
        Path::new(&path).join("main-only.txt").exists(),
        "detached HEAD should fall back to the prior default (HEAD at the \
         main working tree), not the detached checkout's own commit"
    );
}

#[test]
fn add_unborn_invoking_branch_falls_back_to_head_default() {
    let plan = init_plan_repo();
    let project = init_project_repo();

    let wt_dir = TempDir::new().unwrap();
    let orphan_path = wt_dir.path().join("orphan-wt");

    // An orphan linked worktree: `git symbolic-ref HEAD` reports the branch
    // name (`scratch`) even though it has no commits, so it cannot be used
    // as a base ref — the add must fall back to the prior HEAD default
    // instead of failing with `fatal: invalid reference: scratch`.
    git(
        project.path(),
        &[
            "worktree",
            "add",
            "--orphan",
            "-b",
            "scratch",
            orphan_path.to_str().unwrap(),
        ],
    );

    let assert = worktree_cmd_at(&plan, &orphan_path)
        .args(["add", "task/fix-bug", "--project", "demo"])
        .assert()
        .success();
    let path = String::from_utf8_lossy(&assert.get_output().stdout)
        .trim()
        .to_string();
    assert!(
        Path::new(&path).join("README.md").exists(),
        "unborn invoking branch should fall back to the prior HEAD default"
    );
}

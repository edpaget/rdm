//! Integration tests for `rdm review pending` against separate temp plan +
//! source repos. The plan repo is addressed via `--root`; reachability is keyed
//! off the source repo discovered from the command's CWD — the way the review
//! Stop hook invokes it from inside a worktree/branch.

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use std::fs;
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

/// A plan repo with a `demo` project, three tasks, and a roadmap with one phase
/// (`roadmap-z/phase-1-build`) so phase scoping can be exercised end-to-end too.
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
    for slug in ["item-x", "item-y", "legacy"] {
        rdm()
            .arg("--root")
            .arg(dir.path())
            .args([
                "task",
                "create",
                slug,
                "--title",
                slug,
                "--no-edit",
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
            "roadmap",
            "create",
            "roadmap-z",
            "--title",
            "Roadmap Z",
            "--no-edit",
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
            "build",
            "--title",
            "Build",
            "--number",
            "1",
            "--no-edit",
            "--roadmap",
            "roadmap-z",
            "--project",
            "demo",
        ])
        .assert()
        .success();
    dir
}

/// Sets task `slug` to needs-review with the command running in `cwd`.
fn finalize_task(plan: &TempDir, cwd: &Path, slug: &str) {
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
        .success();
}

/// Sets phase `stem` in `roadmap` to needs-review with the command running in `cwd`.
fn finalize_phase(plan: &TempDir, cwd: &Path, roadmap: &str, stem: &str) {
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
            roadmap,
            "--project",
            "demo",
        ])
        .assert()
        .success();
}

#[test]
fn pending_scopes_to_current_branch_and_fails_open() {
    let plan = init_plan_repo();

    // Source repo with two divergent branches off an initial commit.
    let src = TempDir::new().unwrap();
    git(src.path(), &["init", "-b", "main"]);
    fs::write(src.path().join("README.md"), "# project").unwrap();
    git(src.path(), &["add", "."]);
    git(src.path(), &["commit", "-m", "initial"]);

    git(src.path(), &["checkout", "-b", "branch-a"]);
    fs::write(src.path().join("a.txt"), "a").unwrap();
    git(src.path(), &["add", "."]);
    git(src.path(), &["commit", "-m", "work on a"]);

    git(src.path(), &["checkout", "main"]);
    git(src.path(), &["checkout", "-b", "branch-b"]);
    fs::write(src.path().join("b.txt"), "b").unwrap();
    git(src.path(), &["add", "."]);
    git(src.path(), &["commit", "-m", "work on b"]);

    // Finalize item-x on branch B (stamps B's HEAD).
    finalize_task(&plan, src.path(), "item-x");

    // Finalize item-y (task) and the phase on branch A (stamps A's HEAD). The
    // phase exercises the `roadmap/stem` identifier path through the CLI.
    git(src.path(), &["checkout", "branch-a"]);
    finalize_task(&plan, src.path(), "item-y");
    finalize_phase(&plan, src.path(), "roadmap-z", "phase-1-build");

    // Finalize the legacy item from a non-git directory → unstamped (fail open).
    let nongit = TempDir::new().unwrap();
    finalize_task(&plan, nongit.path(), "legacy");

    // From branch A: item-y (in-branch task), the in-branch phase, and legacy
    // (unstamped) are in scope; item-x (finalized on branch B) is out of scope.
    let output = rdm()
        .arg("--root")
        .arg(plan.path())
        .current_dir(src.path())
        .args(["review", "pending", "--project", "demo", "--format", "json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).unwrap();
    let ids: Vec<&str> = json
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["identifier"].as_str().unwrap())
        .collect();

    assert!(ids.contains(&"item-y"), "expected item-y in scope: {ids:?}");
    assert!(
        ids.contains(&"legacy"),
        "expected legacy (fail open): {ids:?}"
    );
    assert!(
        ids.contains(&"roadmap-z/phase-1-build"),
        "expected in-branch phase (roadmap/stem identifier) in scope: {ids:?}"
    );
    assert!(
        !ids.contains(&"item-x"),
        "item-x finalized on branch B must be out of scope: {ids:?}"
    );

    // The phase item reports kind "phase"; the task items report kind "task".
    let phase_item = json
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["identifier"] == "roadmap-z/phase-1-build")
        .expect("phase item present");
    assert_eq!(phase_item["kind"], "phase");

    // Each emitted item carries the documented fields, but no review_sha.
    let first = &json.as_array().unwrap()[0];
    assert!(first.get("kind").is_some());
    assert!(first.get("identifier").is_some());
    assert!(first.get("project").is_some());
    assert!(first.get("title").is_some());
    assert!(first.get("review_sha").is_none());

    // Human (non-JSON) output lists the in-scope items, including the phase, and
    // omits the out-of-scope item-x.
    rdm()
        .arg("--root")
        .arg(plan.path())
        .current_dir(src.path())
        .args(["review", "pending", "--project", "demo"])
        .assert()
        .success()
        .stdout(predicate::str::contains("roadmap-z/phase-1-build"))
        .stdout(predicate::str::contains("item-y"))
        .stdout(predicate::str::contains("item-x").not());
}

#[test]
fn pending_empty_lists_emit_clean_output() {
    let plan = init_plan_repo();
    let src = TempDir::new().unwrap();
    git(src.path(), &["init", "-b", "main"]);
    fs::write(src.path().join("README.md"), "# project").unwrap();
    git(src.path(), &["add", "."]);
    git(src.path(), &["commit", "-m", "initial"]);

    // No items in needs-review yet.
    rdm()
        .arg("--root")
        .arg(plan.path())
        .current_dir(src.path())
        .args(["review", "pending", "--project", "demo"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No items pending review."));

    let output = rdm()
        .arg("--root")
        .arg(plan.path())
        .current_dir(src.path())
        .args(["review", "pending", "--project", "demo", "--format", "json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json.as_array().unwrap().len(), 0);
}

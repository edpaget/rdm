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

/// Runs `rdm review pending --format json` with the command's CWD set to `cwd`
/// (which determines source-repo branch/reachability) and returns the in-scope
/// identifiers.
fn pending_ids(plan: &TempDir, cwd: &Path) -> Vec<String> {
    let output = rdm()
        .arg("--root")
        .arg(plan.path())
        .current_dir(cwd)
        .args(["review", "pending", "--project", "demo", "--format", "json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).unwrap();
    json.as_array()
        .unwrap()
        .iter()
        .map(|i| i["identifier"].as_str().unwrap().to_string())
        .collect()
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
fn pending_scopes_by_branch_identity_and_falls_back_to_reachability() {
    let plan = init_plan_repo();

    // A second roadmap so cross-roadmap isolation is exercised: roadmap-z's
    // trigger must never pick up roadmap-w's item stamped a different branch.
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "roadmap",
            "create",
            "roadmap-w",
            "--title",
            "Roadmap W",
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
            "build",
            "--title",
            "Build",
            "--number",
            "1",
            "--no-edit",
            "--roadmap",
            "roadmap-w",
            "--project",
            "demo",
        ])
        .assert()
        .success();

    // Source repo: a base commit on main, then two divergent branches.
    let src = TempDir::new().unwrap();
    git(src.path(), &["init", "-b", "main"]);
    fs::write(src.path().join("README.md"), "# project").unwrap();
    git(src.path(), &["add", "."]);
    git(src.path(), &["commit", "-m", "initial"]);

    // Legacy item: finalize at a *detached* HEAD on the base commit. A detached
    // HEAD stamps `review_sha` (the commit) but no `review_branch`, simulating a
    // pre-stamp item that must survive via the reachability fallback.
    git(src.path(), &["checkout", "--detach"]);
    finalize_task(&plan, src.path(), "legacy");

    git(src.path(), &["checkout", "main"]);
    git(src.path(), &["checkout", "-b", "branch-a"]);
    fs::write(src.path().join("a.txt"), "a").unwrap();
    git(src.path(), &["add", "."]);
    git(src.path(), &["commit", "-m", "work on a"]);

    git(src.path(), &["checkout", "main"]);
    git(src.path(), &["checkout", "-b", "branch-b"]);
    fs::write(src.path().join("b.txt"), "b").unwrap();
    git(src.path(), &["add", "."]);
    git(src.path(), &["commit", "-m", "work on b"]);

    // roadmap-w's phase is finalized on branch B (stamps branch-b).
    git(src.path(), &["checkout", "branch-b"]);
    finalize_phase(&plan, src.path(), "roadmap-w", "phase-1-build");

    // roadmap-z's phase is finalized on branch A (stamps branch-a).
    git(src.path(), &["checkout", "branch-a"]);
    finalize_phase(&plan, src.path(), "roadmap-z", "phase-1-build");

    // From branch A: keep only branch-a items plus the reachability-fallback
    // legacy. roadmap-w's phase (stamped branch-b) is excluded — exact roadmap
    // isolation. The legacy item's base commit is an ancestor of branch-a, so
    // the SHA fallback keeps it.
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
    let entries = json.as_array().unwrap();
    let ids: Vec<&str> = entries
        .iter()
        .map(|i| i["identifier"].as_str().unwrap())
        .collect();

    assert!(
        ids.contains(&"roadmap-z/phase-1-build"),
        "branch-a phase must be in scope: {ids:?}"
    );
    assert!(
        ids.contains(&"legacy"),
        "legacy item must survive via reachability fallback: {ids:?}"
    );
    assert!(
        !ids.contains(&"roadmap-w/phase-1-build"),
        "roadmap-w phase stamped branch-b must be excluded (roadmap isolation): {ids:?}"
    );

    // The JSON exposes the stamped branch for the phase-6 skill to consume.
    let phase_item = entries
        .iter()
        .find(|i| i["identifier"] == "roadmap-z/phase-1-build")
        .expect("phase item present");
    assert_eq!(phase_item["branch"], "branch-a");
    // The fallback legacy item carries a null branch.
    let legacy_item = entries
        .iter()
        .find(|i| i["identifier"] == "legacy")
        .expect("legacy item present");
    assert!(legacy_item["branch"].is_null());
}

#[test]
fn pending_excludes_legacy_item_whose_sha_is_unreachable() {
    // The legacy (no-branch) fallback must DROP an item whose stamped sha is not
    // reachable from the current HEAD — the mirror of the keep direction, and the
    // guard against cross-branch contamination for pre-stamp items.
    let plan = init_plan_repo();
    let src = TempDir::new().unwrap();
    git(src.path(), &["init", "-b", "main"]);
    fs::write(src.path().join("README.md"), "# project").unwrap();
    git(src.path(), &["add", "."]);
    git(src.path(), &["commit", "-m", "initial"]);

    git(src.path(), &["checkout", "-b", "branch-a"]);
    fs::write(src.path().join("a.txt"), "a").unwrap();
    git(src.path(), &["add", "."]);
    git(src.path(), &["commit", "-m", "work on a"]);

    // Divergent branch-b, then finalize `legacy` at a DETACHED HEAD on branch-b's
    // tip: review_branch = None (detached), review_sha = a commit NOT reachable
    // from branch-a.
    git(src.path(), &["checkout", "main"]);
    git(src.path(), &["checkout", "-b", "branch-b"]);
    fs::write(src.path().join("b.txt"), "b").unwrap();
    git(src.path(), &["add", "."]);
    git(src.path(), &["commit", "-m", "work on b"]);
    git(src.path(), &["checkout", "--detach"]);
    finalize_task(&plan, src.path(), "legacy");

    // From branch-a, the legacy item's stamped sha is unreachable → excluded.
    git(src.path(), &["checkout", "branch-a"]);
    let ids = pending_ids(&plan, src.path());
    assert!(
        !ids.contains(&"legacy".to_string()),
        "legacy item stamped an unreachable sha must be excluded: {ids:?}"
    );
}

#[test]
fn pending_keeps_branch_stamped_items_when_branch_unresolvable() {
    // When the firing checkout has no resolvable branch (here: a non-git CWD, the
    // same path git-unavailable takes), a branch-stamped item must FAIL OPEN via
    // SHA reachability rather than being silently hidden. Regression guard: a
    // trigger firing from the wrong place must over-report, never drop work.
    let plan = init_plan_repo();
    let src = TempDir::new().unwrap();
    git(src.path(), &["init", "-b", "branch-a"]);
    fs::write(src.path().join("README.md"), "# project").unwrap();
    git(src.path(), &["add", "."]);
    git(src.path(), &["commit", "-m", "initial"]);
    // Stamp item-x with review_branch=branch-a (and a sha) by finalizing here.
    finalize_task(&plan, src.path(), "item-x");

    // Evaluate `review pending` from a directory that is not a git repo at all:
    // current branch is unresolvable and reachability errors → fail open.
    let nongit = TempDir::new().unwrap();
    let ids = pending_ids(&plan, nongit.path());
    assert!(
        ids.contains(&"item-x".to_string()),
        "branch-stamped item must fail open when the branch is unresolvable: {ids:?}"
    );
}

#[test]
fn blocked_lists_parked_phases_with_recorded_reasons() {
    let plan = init_plan_repo();

    // Empty queue first: a clean message and an empty JSON array.
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args(["review", "blocked", "--project", "demo"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No blocked phases."));

    // Park the phase as blocked and record an escalation reason in one update.
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "phase",
            "update",
            "phase-1-build",
            "--status",
            "blocked",
            "--reason",
            "AC 2 is ambiguous about which crate owns parsing",
            "--no-edit",
            "--roadmap",
            "roadmap-z",
            "--project",
            "demo",
        ])
        .assert()
        .success();

    // `phase show` surfaces the reason (queryable).
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "phase",
            "show",
            "phase-1-build",
            "--no-body",
            "--roadmap",
            "roadmap-z",
            "--project",
            "demo",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Blocked reason: AC 2 is ambiguous about which crate owns parsing",
        ));

    // `review blocked` lists the parked phase with its reason (human output).
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args(["review", "blocked", "--project", "demo"])
        .assert()
        .success()
        .stdout(predicate::str::contains("roadmap-z/phase-1-build"))
        .stdout(predicate::str::contains(
            "AC 2 is ambiguous about which crate owns parsing",
        ));

    // JSON output carries identifier, title, and reason.
    let output = rdm()
        .arg("--root")
        .arg(plan.path())
        .args(["review", "blocked", "--project", "demo", "--format", "json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).unwrap();
    let arr = json.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["identifier"], "roadmap-z/phase-1-build");
    assert_eq!(
        arr[0]["reason"],
        "AC 2 is ambiguous about which crate owns parsing"
    );

    // Resuming the phase preserves the recorded reason but drops it from the
    // blocked queue (status is no longer blocked).
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "phase",
            "update",
            "phase-1-build",
            "--status",
            "in-progress",
            "--no-edit",
            "--roadmap",
            "roadmap-z",
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
            "show",
            "phase-1-build",
            "--no-body",
            "--roadmap",
            "roadmap-z",
            "--project",
            "demo",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Blocked reason: AC 2 is ambiguous about which crate owns parsing",
        ));
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args(["review", "blocked", "--project", "demo"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No blocked phases."));
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

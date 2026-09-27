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
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["commit", "-m", "seed: init plan repo fixture"])
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

    // Each emitted item carries the documented fields and observable review SHA.
    let first = &json.as_array().unwrap()[0];
    assert!(first.get("kind").is_some());
    assert!(first.get("identifier").is_some());
    assert!(first.get("project").is_some());
    assert!(first.get("title").is_some());
    assert!(first.get("review_sha").is_some());

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

/// Returns the current HEAD SHA of the git repo at `dir`.
fn head_sha(dir: &Path) -> String {
    let out = git(dir, &["rev-parse", "HEAD"]);
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Runs `rdm review restamp --format json` with CWD set to `cwd` (which
/// determines the source-repo HEAD/branch used to refresh stamps) and returns
/// the parsed array of restamped entries.
fn restamp_entries(plan: &TempDir, cwd: &Path) -> Vec<Value> {
    let output = rdm()
        .arg("--root")
        .arg(plan.path())
        .current_dir(cwd)
        .args(["review", "restamp", "--project", "demo", "--format", "json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).unwrap();
    json.as_array().unwrap().clone()
}

/// Runs `rdm review restamp` (default text format) with CWD set to `cwd` and
/// returns the raw stdout.
fn restamp_text(plan: &TempDir, cwd: &Path) -> String {
    let output = rdm()
        .arg("--root")
        .arg(plan.path())
        .current_dir(cwd)
        .args(["review", "restamp", "--project", "demo"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    String::from_utf8(output).unwrap()
}

/// Returns the stamped branch for a single pending item identifier, or `None`
/// if the item is absent or carries a null branch.
fn pending_branch(plan: &TempDir, cwd: &Path, identifier: &str) -> Option<String> {
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
        .find(|i| i["identifier"] == identifier)
        .and_then(|i| i["branch"].as_str().map(str::to_string))
}

#[test]
fn restamp_refreshes_stale_review_sha_after_amend() {
    // Finalize a phase on branch-a (stamps sha1), then amend the commit so HEAD
    // moves to sha2. `rdm review restamp` must refresh the stamp to the new HEAD.
    let plan = init_plan_repo();
    let src = TempDir::new().unwrap();
    git(src.path(), &["init", "-b", "branch-a"]);
    fs::write(src.path().join("README.md"), "# project").unwrap();
    git(src.path(), &["add", "."]);
    git(src.path(), &["commit", "-m", "work"]);

    finalize_phase(&plan, src.path(), "roadmap-z", "phase-1-build");
    let sha1 = head_sha(src.path());

    // Amend the implementation commit while the phase is still needs-review.
    fs::write(src.path().join("more.txt"), "more").unwrap();
    git(src.path(), &["add", "."]);
    git(src.path(), &["commit", "--amend", "-m", "work amended"]);
    let sha2 = head_sha(src.path());
    assert_ne!(sha1, sha2, "amend must move HEAD");

    let entries = restamp_entries(&plan, src.path());
    let entry = entries
        .iter()
        .find(|e| e["identifier"] == "roadmap-z/phase-1-build")
        .expect("phase must be restamped");
    assert_eq!(entry["sha"], sha2, "stamp must refresh to the new HEAD");
    assert_eq!(entry["branch"], "branch-a");
}

#[test]
fn restamp_is_idempotent_when_nothing_changed() {
    // A second restamp with no intervening commit must report nothing (no
    // spurious plan-repo writes / commit churn).
    let plan = init_plan_repo();
    let src = TempDir::new().unwrap();
    git(src.path(), &["init", "-b", "branch-a"]);
    fs::write(src.path().join("README.md"), "# project").unwrap();
    git(src.path(), &["add", "."]);
    git(src.path(), &["commit", "-m", "work"]);

    finalize_phase(&plan, src.path(), "roadmap-z", "phase-1-build");

    // First restamp may or may not change anything (the finalize already stamped
    // the current HEAD/branch); the SECOND restamp must be a no-op.
    let _ = restamp_entries(&plan, src.path());
    let entries = restamp_entries(&plan, src.path());
    assert!(
        entries.is_empty(),
        "restamp with no intervening commit must be a no-op: {entries:?}"
    );
}

#[test]
fn restamp_upgrades_legacy_detached_item_with_branch_stamp() {
    // An item finalized at a detached HEAD carries no review_branch (a legacy /
    // fallback-only item). Restamping from the branch whose tip it sits on must
    // upgrade it with a branch stamp, which then survives a later amend.
    let plan = init_plan_repo();
    let src = TempDir::new().unwrap();
    git(src.path(), &["init", "-b", "branch-a"]);
    fs::write(src.path().join("README.md"), "# project").unwrap();
    git(src.path(), &["add", "."]);
    git(src.path(), &["commit", "-m", "work"]);

    // Finalize at a detached HEAD on branch-a's tip: review_branch = None.
    git(src.path(), &["checkout", "--detach"]);
    finalize_task(&plan, src.path(), "item-x");
    assert!(
        pending_branch(&plan, src.path(), "item-x").is_none(),
        "detached finalize must leave review_branch null"
    );

    // From branch-a (item in scope via SHA reachability), restamp upgrades it.
    git(src.path(), &["checkout", "branch-a"]);
    let entries = restamp_entries(&plan, src.path());
    let entry = entries
        .iter()
        .find(|e| e["identifier"] == "item-x")
        .expect("legacy item must be restamped");
    assert_eq!(entry["branch"], "branch-a");
    assert_eq!(
        pending_branch(&plan, src.path(), "item-x").as_deref(),
        Some("branch-a"),
        "item must now carry a branch stamp"
    );

    // The branch stamp now protects the item across an amend (branch identity,
    // not SHA reachability) — closing the original staleness gap.
    fs::write(src.path().join("more.txt"), "more").unwrap();
    git(src.path(), &["add", "."]);
    git(src.path(), &["commit", "--amend", "-m", "work amended"]);
    let ids = pending_ids(&plan, src.path());
    assert!(
        ids.contains(&"item-x".to_string()),
        "branch-stamped item must survive an amend: {ids:?}"
    );
}

#[test]
fn restamp_never_touches_out_of_scope_items() {
    // Restamping from branch-a must never touch an item stamped for branch-b
    // (roadmap isolation), mirroring the pending-scoping guarantee.
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

    git(src.path(), &["checkout", "main"]);
    git(src.path(), &["checkout", "-b", "branch-b"]);
    fs::write(src.path().join("b.txt"), "b").unwrap();
    git(src.path(), &["add", "."]);
    git(src.path(), &["commit", "-m", "work on b"]);

    // roadmap-z/phase-1-build finalized on branch-b; item-x finalized on branch-a.
    git(src.path(), &["checkout", "branch-b"]);
    finalize_phase(&plan, src.path(), "roadmap-z", "phase-1-build");
    git(src.path(), &["checkout", "branch-a"]);
    finalize_task(&plan, src.path(), "item-x");

    // Make item-x genuinely STALE: an extra commit on branch-a after finalizing
    // it, so restamp actually refreshes it (otherwise the idempotency guard would
    // skip it and the test would pass vacuously with an empty output for BOTH).
    fs::write(src.path().join("a2.txt"), "a2").unwrap();
    git(src.path(), &["add", "."]);
    git(src.path(), &["commit", "-m", "more on a"]);

    // Restamp from branch-a: item-x is in scope and stale → refreshed; the
    // branch-b phase is out of scope → never touched. Proves real isolation:
    // one side touched, the other not.
    let entries = restamp_entries(&plan, src.path());
    let ids: Vec<&str> = entries
        .iter()
        .map(|e| e["identifier"].as_str().unwrap())
        .collect();
    assert!(
        ids.contains(&"item-x"),
        "in-scope stale item-x must be restamped: {ids:?}"
    );
    assert!(
        !ids.contains(&"roadmap-z/phase-1-build"),
        "out-of-scope branch-b phase must never be restamped from branch-a: {ids:?}"
    );
}

#[test]
fn restamp_from_detached_head_preserves_branch_stamp() {
    // Regression: restamp from a detached HEAD (unresolvable branch) whose HEAD is
    // still SHA-reachable from the item's stamp must NOT downgrade an already
    // branch-stamped item to review_branch = None. Otherwise a sibling branch
    // sharing history would pick it up via the SHA-reachability fallback — the
    // exact cross-branch leakage branch-identity scoping exists to prevent.
    let plan = init_plan_repo();
    let src = TempDir::new().unwrap();
    git(src.path(), &["init", "-b", "branch-a"]);
    fs::write(src.path().join("README.md"), "# project").unwrap();
    git(src.path(), &["add", "."]);
    git(src.path(), &["commit", "-m", "work"]);

    // Finalize item-x on branch-a (resolvable) → stamps review_branch = branch-a.
    finalize_task(&plan, src.path(), "item-x");
    assert_eq!(
        pending_branch(&plan, src.path(), "item-x").as_deref(),
        Some("branch-a"),
        "finalize on branch-a must stamp branch-a"
    );

    // Detach HEAD at that same commit and restamp. The branch stamp must survive.
    git(src.path(), &["checkout", "--detach"]);
    let _ = restamp_entries(&plan, src.path());
    assert_eq!(
        pending_branch(&plan, src.path(), "item-x").as_deref(),
        Some("branch-a"),
        "detached restamp must NOT downgrade the branch stamp to null"
    );

    // A sibling branch created from the same commit must NOT see item-x via
    // pending — branch identity (branch-a) still excludes it from branch-c.
    git(src.path(), &["checkout", "-b", "branch-c"]);
    let ids = pending_ids(&plan, src.path());
    assert!(
        !ids.contains(&"item-x".to_string()),
        "sibling branch-c must not pick up branch-a's item: {ids:?}"
    );
}

#[test]
fn restamp_fails_open_when_head_unresolvable() {
    // Run restamp from a non-git directory (HEAD unresolvable): it must exit 0
    // and report nothing, pinning the advertised fail-open contract for the hook.
    let plan = init_plan_repo();
    let src = TempDir::new().unwrap();
    git(src.path(), &["init", "-b", "branch-a"]);
    fs::write(src.path().join("README.md"), "# project").unwrap();
    git(src.path(), &["add", "."]);
    git(src.path(), &["commit", "-m", "work"]);
    finalize_task(&plan, src.path(), "item-x");

    // A non-git CWD: head_commit_info_at returns None → restamp is a no-op.
    let nongit = TempDir::new().unwrap();
    let entries = restamp_entries(&plan, nongit.path());
    assert!(
        entries.is_empty(),
        "restamp from a non-git dir must restamp nothing: {entries:?}"
    );
    // Text format confirms the human-readable no-op message and exit 0.
    let text = restamp_text(&plan, nongit.path());
    assert!(
        text.contains("Nothing to restamp."),
        "fail-open restamp must report nothing: {text:?}"
    );
}

#[test]
fn restamp_text_format_reports_noop_and_refresh() {
    // The human-readable output: "Nothing to restamp." on a no-op, and a
    // "restamped <kind> <identifier> -> <sha>" line after a genuine refresh.
    let plan = init_plan_repo();
    let src = TempDir::new().unwrap();
    git(src.path(), &["init", "-b", "branch-a"]);
    fs::write(src.path().join("README.md"), "# project").unwrap();
    git(src.path(), &["add", "."]);
    git(src.path(), &["commit", "-m", "work"]);

    finalize_task(&plan, src.path(), "item-x");

    // No-op: finalize already stamped the current HEAD/branch (drain any change
    // first), so the next call reports nothing.
    let _ = restamp_text(&plan, src.path());
    let noop = restamp_text(&plan, src.path());
    assert!(
        noop.contains("Nothing to restamp."),
        "no-op restamp must print the nothing message: {noop:?}"
    );

    // Genuine refresh: amend so HEAD moves, then the line names the item + sha.
    fs::write(src.path().join("more.txt"), "more").unwrap();
    git(src.path(), &["add", "."]);
    git(src.path(), &["commit", "--amend", "-m", "work amended"]);
    let sha = head_sha(src.path());
    let refreshed = restamp_text(&plan, src.path());
    assert!(
        refreshed.contains(&format!("restamped task item-x -> {sha}")),
        "refresh must print the restamped line with the new sha: {refreshed:?}"
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

// ---------------------------------------------------------------------------
// Document reviews: rdm review start/comment/submit/list/show/update/delete/
// requests (the authoring family, distinct from the needs-review queue above).
// ---------------------------------------------------------------------------

/// Creates a task with a known body so quote anchors have text to bite on.
fn create_task_with_body(plan: &TempDir, slug: &str, body: &str) {
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "task",
            "create",
            slug,
            "--title",
            slug,
            "--no-edit",
            "--project",
            "demo",
            "--body",
            body,
        ])
        .assert()
        .success();
}

/// Starts a review of `on` and returns its id (parsed from the JSON output).
fn start_review(plan: &TempDir, on: &str) -> String {
    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "start",
            "--on",
            on,
            "--author",
            "tester",
            "--no-edit",
            "--project",
            "demo",
            "--format",
            "json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&out).unwrap();
    json["id"].as_str().unwrap().to_string()
}

/// Adds a whole-document comment with `body` to review `id`.
fn add_plain_comment(plan: &TempDir, id: &str, body: &str) {
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "comment",
            id,
            "--body",
            body,
            "--no-edit",
            "--project",
            "demo",
        ])
        .assert()
        .success();
}

/// Runs `review show --format json` and returns the parsed value.
fn show_review_json(plan: &TempDir, id: &str) -> Value {
    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "show",
            id,
            "--project",
            "demo",
            "--format",
            "json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&out).unwrap()
}

#[test]
fn review_start_creates_draft_and_prints_id() {
    let plan = init_plan_repo();
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "start",
            "--on",
            "task/item-x",
            "--author",
            "tester",
            "--no-edit",
            "--project",
            "demo",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Started review '"))
        .stdout(predicate::str::contains("on task/item-x (draft)"));
}

#[test]
fn review_start_json_includes_target_and_created_commit() {
    let plan = init_plan_repo();
    let id = start_review(&plan, "task/item-x");
    let json = show_review_json(&plan, &id);
    assert_eq!(json["target"]["kind"], "task");
    assert_eq!(json["target"]["slug"], "item-x");
    assert_eq!(json["state"], "draft");
    assert!(
        json["created_commit"].as_str().is_some(),
        "created_commit must be stamped: {json}"
    );
}

#[test]
fn review_start_phase_target_resolves_number_to_stem() {
    let plan = init_plan_repo();
    let id = start_review(&plan, "phase/roadmap-z/1");
    let json = show_review_json(&plan, &id);
    assert_eq!(json["target"]["kind"], "phase");
    assert_eq!(json["target"]["roadmap"], "roadmap-z");
    assert_eq!(json["target"]["stem"], "phase-1-build");
}

#[test]
fn review_start_missing_target_errors() {
    let plan = init_plan_repo();
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "start",
            "--on",
            "task/no-such-task",
            "--author",
            "tester",
            "--no-edit",
            "--project",
            "demo",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("review target not found"));
}

#[test]
fn review_start_malformed_ref_shows_accepted_forms() {
    let plan = init_plan_repo();
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "start",
            "--on",
            "not-a-ref",
            "--author",
            "tester",
            "--no-edit",
            "--project",
            "demo",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("roadmap/<slug>"))
        .stderr(predicate::str::contains("task/<slug>"));
}

#[test]
fn review_comment_quote_derives_text_quote_anchor() {
    let plan = init_plan_repo();
    create_task_with_body(
        &plan,
        "quoted",
        "Intro paragraph. The auth flow needs work here. Outro.",
    );
    // Land a real commit so the review's created_commit pins to a revision
    // where the task actually exists in history.
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args(["commit", "-m", "seed: add quoted task"])
        .assert()
        .success();
    let id = start_review(&plan, "task/quoted");
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "comment",
            &id,
            "--quote",
            "auth flow",
            "--body",
            "Which auth flow?",
            "--no-edit",
            "--project",
            "demo",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "Added comment 1 to review '{id}'"
        )));

    let json = show_review_json(&plan, &id);
    let c = &json["comments"][0];
    assert_eq!(c["anchor"]["anchor_type"], "text-quote");
    assert_eq!(c["anchor"]["quote"], "auth flow");
    assert_eq!(c["anchor"]["prefix"], "Intro paragraph. The ");
    assert!(
        c["anchor"]["suffix"]
            .as_str()
            .unwrap()
            .starts_with(" needs work here."),
        "suffix must capture trailing context: {c}"
    );
    assert_eq!(c["resolution"]["state"], "resolved");
    assert_eq!(c["resolution"]["quote"], "auth flow");
    assert_eq!(c["resolution"]["body"], "original");
}

#[test]
fn review_comment_quote_is_derived_at_created_commit_and_reports_drift() {
    let plan = init_plan_repo();
    create_task_with_body(
        &plan,
        "drifty",
        "Alpha. The original wording sits here. Omega.",
    );
    // Land a real commit so the review's created_commit pins to a revision
    // where the task actually exists in history.
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args(["commit", "-m", "seed: add drifty task"])
        .assert()
        .success();
    let id = start_review(&plan, "task/drifty");

    // Edit the task body after the review started: the quoted text vanishes
    // from the current document.
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "task",
            "update",
            "drifty",
            "--no-edit",
            "--project",
            "demo",
            "--body",
            "Alpha. Completely rewritten text sits here. Omega.",
        ])
        .assert()
        .success();

    // The quote still anchors — it is located in the body at created_commit.
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "comment",
            &id,
            "--quote",
            "original wording",
            "--body",
            "Keep this term.",
            "--no-edit",
            "--project",
            "demo",
        ])
        .assert()
        .success();

    let json = show_review_json(&plan, &id);
    let r = &json["comments"][0]["resolution"];
    assert_eq!(r["state"], "drifted", "current body changed: {json}");
    assert_eq!(r["quote"], "original wording");
    assert_eq!(
        r["body"], "original",
        "drifted ranges index the created_commit body"
    );
}

#[test]
fn review_comment_quote_not_found_names_created_commit() {
    let plan = init_plan_repo();
    create_task_with_body(&plan, "missing-quote", "Some body without the phrase.");
    // Land a real commit so the review's created_commit pins to a revision
    // where the task actually exists in history.
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args(["commit", "-m", "seed: add missing-quote task"])
        .assert()
        .success();
    let id = start_review(&plan, "task/missing-quote");
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "comment",
            &id,
            "--quote",
            "nonexistent phrase",
            "--body",
            "x",
            "--no-edit",
            "--project",
            "demo",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("created commit"))
        .stderr(predicate::str::contains("--at "))
        .stderr(predicate::str::contains("fresh review"));
}

#[test]
fn review_comment_ambiguous_quote_lists_occurrences_then_occurrence_selects() {
    let plan = init_plan_repo();
    create_task_with_body(
        &plan,
        "dup",
        "call foo() at the start, then call foo() at the end.",
    );
    let id = start_review(&plan, "task/dup");

    // Ambiguous without --occurrence: lists 1-based positions and the fix.
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "comment",
            &id,
            "--quote",
            "call foo()",
            "--body",
            "x",
            "--no-edit",
            "--project",
            "demo",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--occurrence"))
        .stderr(predicate::str::contains("1: "))
        .stderr(predicate::str::contains("2: "));

    // Out-of-range occurrence is rejected with the valid range.
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "comment",
            &id,
            "--quote",
            "call foo()",
            "--occurrence",
            "5",
            "--body",
            "x",
            "--no-edit",
            "--project",
            "demo",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("valid: 1..=2"));

    // --occurrence 2 anchors the second occurrence.
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "comment",
            &id,
            "--quote",
            "call foo()",
            "--occurrence",
            "2",
            "--body",
            "The second call.",
            "--no-edit",
            "--project",
            "demo",
        ])
        .assert()
        .success();
    let json = show_review_json(&plan, &id);
    let prefix = json["comments"][0]["anchor"]["prefix"].as_str().unwrap();
    assert!(
        prefix.ends_with("then "),
        "prefix must come from the second occurrence: {prefix:?}"
    );
}

#[test]
fn review_comment_occurrence_requires_quote() {
    let plan = init_plan_repo();
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "comment",
            "any-id",
            "--occurrence",
            "2",
            "--body",
            "x",
            "--no-edit",
            "--project",
            "demo",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--quote"));
}

#[test]
fn review_comment_doc_scopes_roadmap_review_to_phase() {
    let plan = init_plan_repo();
    let id = start_review(&plan, "roadmap/roadmap-z");
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "comment",
            &id,
            "--doc",
            "phase/1",
            "--body",
            "About phase one.",
            "--no-edit",
            "--project",
            "demo",
        ])
        .assert()
        .success();
    let json = show_review_json(&plan, &id);
    assert_eq!(json["comments"][0]["doc"]["stem"], "phase-1-build");
}

#[test]
fn review_comment_doc_out_of_scope_errors() {
    let plan = init_plan_repo();
    let id = start_review(&plan, "roadmap/roadmap-z");
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "comment",
            &id,
            "--doc",
            "phase/ghost-phase",
            "--body",
            "x",
            "--no-edit",
            "--project",
            "demo",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not part of roadmap"));
}

#[test]
fn review_comment_doc_on_task_review_errors() {
    let plan = init_plan_repo();
    let id = start_review(&plan, "task/item-x");
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "comment",
            &id,
            "--doc",
            "phase/1",
            "--body",
            "x",
            "--no-edit",
            "--project",
            "demo",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("roadmap review"));
}

#[test]
fn review_comment_on_submitted_review_errors() {
    let plan = init_plan_repo();
    let id = start_review(&plan, "task/item-x");
    add_plain_comment(&plan, &id, "First comment.");
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "submit",
            &id,
            "--verdict",
            "comment",
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
            "review",
            "comment",
            &id,
            "--body",
            "Too late.",
            "--no-edit",
            "--project",
            "demo",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not a draft"));
}

#[test]
fn review_comment_empty_body_errors() {
    let plan = init_plan_repo();
    let id = start_review(&plan, "task/item-x");
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args(["review", "comment", &id, "--no-edit", "--project", "demo"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("comment body must not be empty"));
}

#[test]
fn review_submit_requires_verdict_flag() {
    let plan = init_plan_repo();
    let id = start_review(&plan, "task/item-x");
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args(["review", "submit", &id, "--no-edit", "--project", "demo"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--verdict"));
}

#[test]
fn review_submit_empty_review_errors() {
    let plan = init_plan_repo();
    let id = start_review(&plan, "task/item-x");
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "submit",
            &id,
            "--verdict",
            "approve",
            "--no-edit",
            "--project",
            "demo",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no comments and no summary"));
}

#[test]
fn review_submit_empty_body_against_summary_is_refused() {
    let plan = init_plan_repo();
    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "start",
            "--on",
            "task/item-x",
            "--author",
            "tester",
            "--no-edit",
            "--project",
            "demo",
            "--body",
            "Existing summary.",
            "--format",
            "json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let id = serde_json::from_slice::<Value>(&out).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "submit",
            &id,
            "--verdict",
            "approve",
            "--body",
            "",
            "--no-edit",
            "--project",
            "demo",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("non-empty summary"));
}

#[test]
fn review_update_comment_before_submit_errors() {
    let plan = init_plan_repo();
    let id = start_review(&plan, "task/item-x");
    add_plain_comment(&plan, &id, "A comment.");
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "update",
            &id,
            "--comment",
            "1",
            "--status",
            "addressed",
            "--project",
            "demo",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("has not been submitted"));
}

#[test]
fn review_update_state_addressed_with_open_comments_errors() {
    let plan = init_plan_repo();
    let id = start_review(&plan, "task/item-x");
    add_plain_comment(&plan, &id, "A comment.");
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "submit",
            &id,
            "--verdict",
            "request-changes",
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
            "review",
            "update",
            &id,
            "--state",
            "addressed",
            "--project",
            "demo",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("open comment"));
}

#[test]
fn review_update_without_any_change_errors() {
    let plan = init_plan_repo();
    let id = start_review(&plan, "task/item-x");
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args(["review", "update", &id, "--project", "demo"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("nothing to update"));
    // --comment without a resolution field is also rejected.
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "update",
            &id,
            "--comment",
            "1",
            "--project",
            "demo",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("at least one of"));
}

#[test]
fn review_full_authoring_loop_end_to_end() {
    let plan = init_plan_repo();
    // `init_plan_repo` ends with a real `rdm commit`, so the plan repo's
    // HEAD at this point is a resolvable commit — a short prefix of it
    // exercises `--applied-commit`'s abbreviated-SHA resolution below.
    let plan_head = String::from_utf8(git(plan.path(), &["rev-parse", "HEAD"]).stdout)
        .unwrap()
        .trim()
        .to_string();
    let plan_head_short = &plan_head[..8];
    create_task_with_body(
        &plan,
        "loop-task",
        "Step one is unclear. Step two is fine. Step three is missing tests.",
    );
    let id = start_review(&plan, "task/loop-task");

    // Two anchored comments.
    for (quote, body) in [
        ("Step one is unclear", "Clarify step one."),
        ("missing tests", "Add tests for step three."),
    ] {
        rdm()
            .arg("--root")
            .arg(plan.path())
            .args([
                "review",
                "comment",
                &id,
                "--quote",
                quote,
                "--body",
                body,
                "--no-edit",
                "--project",
                "demo",
            ])
            .assert()
            .success();
    }

    // Submit with request-changes.
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "submit",
            &id,
            "--verdict",
            "request-changes",
            "--body",
            "Two things to fix.",
            "--no-edit",
            "--project",
            "demo",
        ])
        .assert()
        .success();

    // The agent queue picks it up.
    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "requests",
            "--project",
            "demo",
            "--format",
            "json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let queue: Value = serde_json::from_slice(&out).unwrap();
    let arr = queue.as_array().unwrap();
    assert_eq!(arr.len(), 1, "requests must list the submitted review");
    assert_eq!(arr[0]["id"], id.as_str());
    assert_eq!(arr[0]["verdict"], "request-changes");
    // Full anchor + resolution ride along, so agents need no second call.
    assert_eq!(arr[0]["comments"][0]["anchor"]["anchor_type"], "text-quote");
    assert_eq!(arr[0]["comments"][0]["resolution"]["state"], "resolved");

    // Resolve both comments (addressed with commit + reply; wont-fix).
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "update",
            &id,
            "--comment",
            "1",
            "--status",
            "addressed",
            "--applied-commit",
            plan_head_short,
            "--reply",
            "Clarified in the seed commit.",
            "--project",
            "demo",
        ])
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "update",
            &id,
            "--comment",
            "2",
            "--status",
            "wont-fix",
            "--reply",
            "Covered elsewhere.",
            "--project",
            "demo",
        ])
        .assert()
        .success();

    // Close the loop.
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "update",
            &id,
            "--state",
            "addressed",
            "--project",
            "demo",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("state: addressed"));

    // The queue is empty again, and show reflects the resolutions.
    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "requests",
            "--project",
            "demo",
            "--format",
            "json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(
        serde_json::from_slice::<Value>(&out)
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        0
    );
    let json = show_review_json(&plan, &id);
    assert_eq!(json["state"], "addressed");
    assert_eq!(json["comments"][0]["status"], "addressed");
    // The abbreviated SHA passed above is stored resolved to its full
    // 40-character form, not the operator's literal input.
    assert_eq!(json["comments"][0]["applied_commit"], plan_head.as_str());
    assert_eq!(
        json["comments"][0]["reply"],
        "Clarified in the seed commit."
    );
    assert_eq!(json["comments"][1]["status"], "wont-fix");
}

#[test]
fn review_applied_commit_rejects_nonexistent_sha_in_plan_repo() {
    let plan = init_plan_repo();
    create_task_with_body(&plan, "loop-task", "Step one is unclear.");
    let id = start_review(&plan, "task/loop-task");
    add_plain_comment(&plan, &id, "Clarify step one.");
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "submit",
            &id,
            "--verdict",
            "request-changes",
            "--no-edit",
            "--project",
            "demo",
        ])
        .assert()
        .success();

    // A well-formed but nonexistent SHA is refused, naming the plan repo,
    // and leaves the comment's applied_commit unset.
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "update",
            &id,
            "--comment",
            "1",
            "--applied-commit",
            "0123456789abcdef0123456789abcdef01234567",
            "--project",
            "demo",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("does not resolve to a commit"));

    let json = show_review_json(&plan, &id);
    assert!(json["comments"][0]["applied_commit"].is_null());
}

#[test]
fn review_applied_commit_correction_after_addressed_keeps_state() {
    let plan = init_plan_repo();
    let plan_head = String::from_utf8(git(plan.path(), &["rev-parse", "HEAD"]).stdout)
        .unwrap()
        .trim()
        .to_string();
    create_task_with_body(&plan, "loop-task", "Step one is unclear.");
    let id = start_review(&plan, "task/loop-task");
    add_plain_comment(&plan, &id, "Clarify step one.");
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "submit",
            &id,
            "--verdict",
            "request-changes",
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
            "review",
            "update",
            &id,
            "--comment",
            "1",
            "--status",
            "addressed",
            "--applied-commit",
            &plan_head[..8],
            "--project",
            "demo",
        ])
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "update",
            &id,
            "--state",
            "addressed",
            "--project",
            "demo",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("state: addressed"));

    // A second, later commit gives the correction a genuinely different
    // resolvable SHA to write. Re-using `plan_head` here would let this test
    // pass even if the CLI silently ignored `--applied-commit` on a closed
    // review, since the stored value would already equal it from the
    // `--status addressed` write above.
    std::fs::write(plan.path().join("second-commit-marker.txt"), "marker\n").unwrap();
    git(plan.path(), &["add", "second-commit-marker.txt"]);
    git(
        plan.path(),
        &["commit", "-m", "second commit for the correction test"],
    );
    let second_head = String::from_utf8(git(plan.path(), &["rev-parse", "HEAD"]).stdout)
        .unwrap()
        .trim()
        .to_string();
    assert_ne!(
        plan_head, second_head,
        "the correction must target a genuinely different commit"
    );

    // Correcting applied_commit/reply after the review has closed succeeds,
    // and the review's state stays addressed.
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "update",
            &id,
            "--comment",
            "1",
            "--applied-commit",
            &second_head[..8],
            "--reply",
            "Corrected note.",
            "--project",
            "demo",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("state: addressed"));

    // A status change is still refused, naming the actual state.
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "update",
            &id,
            "--comment",
            "1",
            "--status",
            "wont-fix",
            "--project",
            "demo",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("addressed"));

    let json = show_review_json(&plan, &id);
    assert_eq!(json["state"], "addressed");
    // The corrected value — the SECOND commit, not the one stored before the
    // review closed — is what `review show` reports. This is the assertion
    // that would fail if `--applied-commit` were silently ignored on a
    // closed review.
    assert_eq!(json["comments"][0]["applied_commit"], second_head.as_str());
    assert_eq!(json["comments"][0]["reply"], "Corrected note.");
}

#[test]
fn review_list_filters_by_on_state_verdict_author() {
    let plan = init_plan_repo();
    let draft = start_review(&plan, "task/item-x");
    let submitted = start_review(&plan, "task/item-y");
    add_plain_comment(&plan, &submitted, "A comment.");
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "submit",
            &submitted,
            "--verdict",
            "approve",
            "--no-edit",
            "--project",
            "demo",
        ])
        .assert()
        .success();

    let list = |extra: &[&str]| -> Vec<String> {
        let mut args = vec!["review", "list", "--project", "demo", "--format", "json"];
        args.extend_from_slice(extra);
        let out = rdm()
            .arg("--root")
            .arg(plan.path())
            .args(&args)
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        serde_json::from_slice::<Value>(&out)
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["id"].as_str().unwrap().to_string())
            .collect()
    };

    assert_eq!(list(&[]).len(), 2);
    assert_eq!(list(&["--state", "draft"]), vec![draft.clone()]);
    assert_eq!(list(&["--verdict", "approve"]), vec![submitted.clone()]);
    assert_eq!(list(&["--on", "task/item-x"]), vec![draft.clone()]);
    assert_eq!(list(&["--author", "tester"]).len(), 2);
    assert!(list(&["--author", "nobody"]).is_empty());
}

#[test]
fn review_show_rejects_table_format_and_supports_no_body() {
    let plan = init_plan_repo();
    create_task_with_body(&plan, "shown", "Body with a notable phrase inside.");
    let id = start_review(&plan, "task/shown");
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "comment",
            &id,
            "--quote",
            "notable phrase",
            "--body",
            "Secret comment body.",
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
            "review",
            "show",
            &id,
            "--project",
            "demo",
            "--format",
            "table",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not supported for 'review show'"));

    // --no-body suppresses comment bodies but keeps metadata + anchors.
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args(["review", "show", &id, "--no-body", "--project", "demo"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Secret comment body.").not())
        .stdout(predicate::str::contains("Comment 1"))
        .stdout(predicate::str::contains("notable phrase"));
}

#[test]
fn review_delete_draft_ok_submitted_requires_force() {
    let plan = init_plan_repo();
    let draft = start_review(&plan, "task/item-x");
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args(["review", "delete", &draft, "--project", "demo"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Deleted review"));

    let submitted = start_review(&plan, "task/item-y");
    add_plain_comment(&plan, &submitted, "A comment.");
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "submit",
            &submitted,
            "--verdict",
            "comment",
            "--no-edit",
            "--project",
            "demo",
        ])
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args(["review", "delete", &submitted, "--project", "demo"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--force"));
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "delete",
            &submitted,
            "--force",
            "--project",
            "demo",
        ])
        .assert()
        .success();
}

#[test]
fn review_help_groups_the_two_families() {
    rdm()
        .args(["review", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Author document reviews"))
        .stdout(predicate::str::contains("awaiting implementation review"))
        .stdout(predicate::str::contains("pending, restamp, blocked"))
        .stdout(predicate::str::contains(
            "start, comment, submit, list, show, update, delete, requests",
        ));
}

#[test]
fn review_start_defers_commit_and_commit_includes_review_file() {
    let plan = init_plan_repo();
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "start",
            "--on",
            "task/item-x",
            "--author",
            "tester",
            "--no-edit",
            "--project",
            "demo",
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains("staged"));

    // The review file exists on disk but is not yet committed.
    let status = git(plan.path(), &["status", "--porcelain"]);
    let porcelain = String::from_utf8_lossy(&status.stdout).to_string();
    assert!(
        porcelain.contains("reviews/"),
        "review file must be uncommitted under staging: {porcelain}"
    );

    // rdm commit persists it.
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args(["commit", "-m", "batch: add review"])
        .assert()
        .success();
    let log = git(plan.path(), &["log", "--name-only", "-1"]);
    let logged = String::from_utf8_lossy(&log.stdout).to_string();
    assert!(
        logged.contains("reviews/"),
        "commit must include the review file: {logged}"
    );
}

#[test]
fn review_pending_unaffected_by_document_reviews() {
    // Regression: the authoring family must not disturb the needs-review
    // queue commands sharing the namespace.
    let plan = init_plan_repo();
    let id = start_review(&plan, "task/item-x");
    add_plain_comment(&plan, &id, "A comment.");
    let src = TempDir::new().unwrap();
    git(src.path(), &["init", "-b", "main"]);
    fs::write(src.path().join("README.md"), "# project").unwrap();
    git(src.path(), &["add", "."]);
    git(src.path(), &["commit", "-m", "initial"]);
    rdm()
        .arg("--root")
        .arg(plan.path())
        .current_dir(src.path())
        .args(["review", "pending", "--project", "demo"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No items pending review."));
}

#[test]
fn review_submit_whitespace_body_without_comments_names_body_flag() {
    // A whitespace-only --body is treated as no summary; with no comments
    // either, the error must say the passed --body was blank rather than
    // claim no summary was given.
    let plan = init_plan_repo();
    let id = start_review(&plan, "task/item-x");
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "submit",
            &id,
            "--verdict",
            "comment",
            "--body",
            "   ",
            "--no-edit",
            "--project",
            "demo",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--body you passed is blank"));
}

#[test]
fn review_submit_whitespace_body_with_comments_does_not_persist_whitespace() {
    let plan = init_plan_repo();
    let id = start_review(&plan, "task/item-x");
    add_plain_comment(&plan, &id, "A comment.");
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "submit",
            &id,
            "--verdict",
            "comment",
            "--body",
            "  \t ",
            "--no-edit",
            "--project",
            "demo",
        ])
        .assert()
        .success();
    let json = show_review_json(&plan, &id);
    assert_eq!(json["state"], "submitted");
    assert_eq!(
        json["body"], "",
        "whitespace-only --body must not persist as the summary"
    );
}

#[test]
fn review_list_format_table_renders_columns() {
    let plan = init_plan_repo();
    create_task_with_body(&plan, "tabled", "A body with a marker phrase inside.");
    let id = start_review(&plan, "task/tabled");
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "comment",
            &id,
            "--body",
            "A note.",
            "--no-edit",
            "--project",
            "demo",
        ])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(plan.path())
        .args(["review", "list", "--format", "table", "--project", "demo"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Id"))
        .stdout(predicate::str::contains("Target"))
        .stdout(predicate::str::contains("task/tabled"))
        .stdout(predicate::str::contains("draft"))
        .stdout(predicate::str::contains("1/1 open"));
}

#[test]
fn review_show_format_markdown_renders_comment_details() {
    let plan = init_plan_repo();
    create_task_with_body(&plan, "marked", "A body with a marker phrase inside.");
    let id = start_review(&plan, "task/marked");
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "comment",
            &id,
            "--quote",
            "marker phrase",
            "--body",
            "Sharpen the phrasing.",
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
            "review",
            "show",
            &id,
            "--format",
            "markdown",
            "--project",
            "demo",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(format!("# Review {id}")))
        .stdout(predicate::str::contains("task/marked"))
        .stdout(predicate::str::contains("## Comments"))
        .stdout(predicate::str::contains("marker phrase"))
        .stdout(predicate::str::contains("Sharpen the phrasing."))
        .stdout(predicate::str::contains("resolved"));
}

/// An anchor whose `anchor_type` this build does not model (written by a
/// newer rdm) must round-trip verbatim through `review show --format json`
/// and resolve as `unresolved` — never error.
#[test]
fn review_show_unknown_anchor_round_trips() {
    let plan = init_plan_repo();

    // Hand-write a review fixture carrying a `heading-path` anchor, the way a
    // newer rdm would have stored it.
    let review_path = plan
        .path()
        .join("projects/demo/reviews/2026-07-01-1200-ffff.md");
    fs::create_dir_all(review_path.parent().unwrap()).unwrap();
    fs::write(
        &review_path,
        r#"---
id: 2026-07-01-1200-ffff
author: fixture
target:
  kind: task
  slug: item-x
state: submitted
verdict: request-changes
created: 2026-07-01T12:00:00Z
submitted: 2026-07-01T12:30:00Z
comments:
- id: 1
  status: open
  anchor:
    anchor_type: heading-path
    start: 3
    end: 7
  body: Uses a heading-path anchor from a newer rdm.
---
Fixture review with an unknown anchor type.
"#,
    )
    .unwrap();

    let output = rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "show",
            "2026-07-01-1200-ffff",
            "--format",
            "json",
            "--project",
            "demo",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: Value = serde_json::from_slice(&output).unwrap();

    let comment = &v["comments"][0];
    // The unknown anchor is echoed verbatim (tagged union preserved)...
    assert_eq!(comment["anchor"]["anchor_type"], "heading-path");
    assert_eq!(comment["anchor"]["start"], 3);
    assert_eq!(comment["anchor"]["end"], 7);
    // ...and resolves as unresolved (whole-document treatment).
    assert_eq!(comment["resolution"]["state"], "unresolved");
    assert!(comment["resolution"].get("quote").is_none());
}

// --- Regression: `review start`/`comment`/`submit` must never block reading
// stdin. Before the fix, `resolve_body` unconditionally read stdin to EOF
// whenever it was not a TTY, *before* `--no-edit` was even consulted — under
// an agent's Bash tool stdin is a non-TTY pipe that is never closed, so the
// read never returned. These mirror `task_create_body_flag_no_hang_with_open_stdin_pipe`
// in `cli_task.rs`: spawn the real binary, hold its stdin pipe open (never
// written, never closed), and assert the process exits within a timeout
// rather than hanging forever. ---

/// Spawns `rdm --root <plan> <args...>` with stdin held open (piped, never
/// written, never closed) and returns its exit status, panicking if it does
/// not exit within 5 seconds.
fn run_with_open_stdin_pipe(plan: &TempDir, args: &[&str]) -> std::process::ExitStatus {
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_rdm"))
        .env("XDG_CONFIG_HOME", "/dev/null/nonexistent")
        .arg("--root")
        .arg(plan.path())
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();

    // Keep the child's stdin pipe open (never written, never closed) —
    // dropping it would deliver EOF and defeat the point of this test.
    let _stdin = child.stdin.take();

    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let status = child.wait();
        let _ = tx.send(status);
    });

    rx.recv_timeout(std::time::Duration::from_secs(5))
        .expect("rdm must not hang with stdin held open")
        .unwrap()
}

#[test]
fn review_start_no_body_no_hang_with_open_stdin_pipe() {
    let plan = init_plan_repo();

    let status = run_with_open_stdin_pipe(
        &plan,
        &[
            "review",
            "start",
            "--on",
            "task/item-x",
            "--author",
            "tester",
            "--no-edit",
            "--project",
            "demo",
        ],
    );

    assert!(status.success());

    // stdin, if it had been read, must not have been consulted for the body:
    // the created review's body must be empty (no --body was passed).
    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "list",
            "--on",
            "task/item-x",
            "--project",
            "demo",
            "--format",
            "json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let list: Value = serde_json::from_slice(&out).unwrap();
    let id = list[0]["id"].as_str().unwrap();
    let review = show_review_json(&plan, id);
    assert_eq!(review["body"], "");
}

#[test]
fn review_comment_no_body_no_hang_with_open_stdin_pipe() {
    let plan = init_plan_repo();
    let id = start_review(&plan, "task/item-x");

    // The fast-fail (empty-body bail) path must not be gated behind a
    // blocking stdin read either — it must exit promptly with a failure.
    let status = run_with_open_stdin_pipe(
        &plan,
        &["review", "comment", &id, "--no-edit", "--project", "demo"],
    );

    assert!(!status.success());
}

#[test]
fn review_submit_no_body_no_hang_with_open_stdin_pipe() {
    let plan = init_plan_repo();
    let id = start_review(&plan, "task/item-x");
    add_plain_comment(&plan, &id, "First comment.");

    let status = run_with_open_stdin_pipe(
        &plan,
        &[
            "review",
            "submit",
            &id,
            "--verdict",
            "approve",
            "--no-edit",
            "--project",
            "demo",
        ],
    );

    assert!(status.success());
}

/// Same as [`run_with_open_stdin_pipe`], but with additional environment
/// variables layered over the child's inherited environment — used below to
/// prove that `resolve_review_body`'s non-interactive path is gated by the
/// **TTY check**, not merely by `--no-edit`. `run_with_open_stdin_pipe` never
/// needed this: every one of its callers already passes `--no-edit`.
fn run_with_open_stdin_pipe_env(
    plan: &TempDir,
    args: &[&str],
    extra_env: &[(&str, &str)],
) -> std::process::ExitStatus {
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_rdm"));
    cmd.env("XDG_CONFIG_HOME", "/dev/null/nonexistent")
        .arg("--root")
        .arg(plan.path())
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    for (key, value) in extra_env {
        cmd.env(key, value);
    }
    let mut child = cmd.spawn().unwrap();

    // Keep the child's stdin pipe open (never written, never closed) — the
    // same non-TTY shape as an agent's Bash tool, and the same shape
    // `run_with_open_stdin_pipe` above holds open.
    let _stdin = child.stdin.take();

    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let status = child.wait();
        let _ = tx.send(status);
    });

    rx.recv_timeout(std::time::Duration::from_secs(5))
        .expect("rdm must not hang with stdin held open")
        .unwrap()
}

/// Regression coverage gap (tests-2): no test previously drove
/// `resolve_review_body` with `--no-edit` **absent** against a non-TTY
/// stdin. `resolve_review_body` (rdm-cli/src/commands/mod.rs) only opens
/// `$EDITOR`/`$VISUAL` when `!no_edit && io::stdin().is_terminal()` — the TTY
/// check, not `no_edit` alone, is what keeps a non-interactive caller (an
/// agent's Bash tool, stdin a pipe that is never closed) from ever launching
/// an editor it cannot interact with, even when the caller forgets
/// `--no-edit`.
///
/// `EDITOR`/`VISUAL` are pointed at `false`, a command that always exits
/// non-zero — loud and immediate if ever actually invoked, in contrast to a
/// nonexistent binary (which fails the same way, but less clearly signals
/// "the editor really launched"). If the TTY guard were dropped (e.g.
/// `if !no_edit { open_editor() }`, discarding the `is_terminal()` half),
/// `open_editor()` would run `false`, its `status()` would come back
/// non-zero, and `resolve_review_body` would bail with "editor exited with
/// non-zero status" — so this test would start failing immediately, well
/// within the 5-second hang timeout `run_with_open_stdin_pipe_env` enforces.
///
/// Confirmed by hand while writing this test: temporarily rewriting
/// `resolve_review_body` to `if !no_edit { open_editor() } else { Ok(None) }`
/// (dropping the `is_terminal()` conjunct) turns this red with exactly that
/// message; reverted immediately after.
#[test]
fn review_submit_no_edit_absent_stays_non_interactive_under_open_stdin_pipe() {
    let plan = init_plan_repo();
    let id = start_review(&plan, "task/item-x");
    add_plain_comment(&plan, &id, "First comment.");

    let status = run_with_open_stdin_pipe_env(
        &plan,
        &[
            "review",
            "submit",
            &id,
            "--verdict",
            "approve",
            "--project",
            "demo",
        ],
        &[("EDITOR", "false"), ("VISUAL", "false")],
    );

    assert!(
        status.success(),
        "review submit with --no-edit omitted must still exit promptly and succeed under a \
         non-TTY stdin, without ever invoking EDITOR/VISUAL"
    );

    // No --body was passed and the editor never ran (it would have failed
    // the command above if it had), so the review's body stays empty.
    let review = show_review_json(&plan, &id);
    assert_eq!(review["body"], "");
}

#[test]
fn review_start_ignores_piped_stdin_body() {
    let plan = init_plan_repo();

    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "start",
            "--on",
            "task/item-x",
            "--author",
            "tester",
            "--no-edit",
            "--project",
            "demo",
            "--format",
            "json",
        ])
        .write_stdin("SNEAKY STDIN BODY")
        .assert()
        .success();

    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "list",
            "--on",
            "task/item-x",
            "--project",
            "demo",
            "--format",
            "json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let list: Value = serde_json::from_slice(&out).unwrap();
    let id = list[0]["id"].as_str().unwrap();
    let review = show_review_json(&plan, id);
    assert_eq!(review["body"], "");
    assert!(
        !review["body"]
            .as_str()
            .unwrap()
            .contains("SNEAKY STDIN BODY")
    );
}

// ---------------------------------------------------------------------------
// Line-range anchors: `review comment --line` and `show --numbered`.
// ---------------------------------------------------------------------------

/// Runs `rdm --root <plan> <args> --project demo` and returns the assertion.
fn rdm_demo(plan: &TempDir, args: &[&str]) -> assert_cmd::assert::Assert {
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args(args)
        .args(["--project", "demo"])
        .assert()
}

/// Lands every staged plan-repo change as one commit.
fn commit_plan(plan: &TempDir, message: &str) {
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args(["commit", "-m", message])
        .assert()
        .success();
}

/// A multi-line plan body: a heading, a blank line, a paragraph wrapped over
/// two lines, and no trailing newline on the last line.
const LINED_BODY: &str =
    "# Plan\n\nIntro line.\nStep one wraps\nonto a second line.\n\nStep two, the last line.";

/// Creates plan `slug` (implementing `task/item-x`) with `body` and commits.
fn create_plan_with_body(plan: &TempDir, slug: &str, body: &str) {
    rdm_demo(
        plan,
        &[
            "plan",
            "create",
            slug,
            "--title",
            slug,
            "--implements",
            "task/item-x",
            "--body",
            body,
            "--no-edit",
        ],
    )
    .success();
    commit_plan(plan, "seed: plan with a lined body");
}

/// Adds a `--line <lines>` comment (plus `extra` args) to review `id`.
fn add_line_comment(plan: &TempDir, id: &str, lines: &str, extra: &[&str]) {
    let mut args = vec![
        "review",
        "comment",
        id,
        "--line",
        lines,
        "--body",
        "About these lines.",
        "--no-edit",
    ];
    args.extend_from_slice(extra);
    rdm_demo(plan, &args).success();
}

/// The text of 1-based line `n` of a `show --numbered` rendering.
fn numbered_line(rendering: &str, n: usize) -> String {
    let line = rendering
        .lines()
        .nth(n - 1)
        .unwrap_or_else(|| panic!("no line {n} in {rendering:?}"));
    let (number, text) = line
        .split_once('\t')
        .unwrap_or_else(|| panic!("no tab in numbered line {line:?}"));
    assert_eq!(number.trim(), n.to_string(), "{rendering}");
    text.to_string()
}

/// Runs `<show_args> --numbered` and returns stdout.
fn show_numbered(plan: &TempDir, show_args: &[&str]) -> String {
    let mut args = show_args.to_vec();
    args.push("--numbered");
    let out = rdm_demo(plan, &args).success().get_output().stdout.clone();
    String::from_utf8(out).unwrap()
}

#[test]
fn review_comment_line_on_a_plan_stores_the_range_and_snippet() {
    let plan = init_plan_repo();
    create_plan_with_body(&plan, "lined", LINED_BODY);
    let id = start_review(&plan, "plan/lined");
    add_line_comment(&plan, &id, "3-4", &[]);

    let json = show_review_json(&plan, &id);
    let c = &json["comments"][0];
    assert_eq!(c["anchor"]["anchor_type"], "line-range");
    assert_eq!(c["anchor"]["start_line"], 3);
    assert_eq!(c["anchor"]["end_line"], 4);
    let expected: Vec<&str> = LINED_BODY.lines().collect();
    let snippet = format!("{}\n{}", expected[2], expected[3]);
    assert_eq!(c["anchor"]["snippet"], snippet.as_str());
    assert_eq!(c["resolution"]["state"], "resolved");
    assert_eq!(c["resolution"]["quote"], snippet.as_str());
    assert_eq!(c["resolution"]["body"], "original");
    assert_eq!(c["target"]["level"], "plan-item");
}

#[test]
fn a_line_anchor_keeps_rendering_its_created_commit_span_after_an_edit() {
    let plan = init_plan_repo();
    create_plan_with_body(&plan, "lined", LINED_BODY);
    let id = start_review(&plan, "plan/lined");
    add_line_comment(&plan, &id, "3-4", &[]);

    // Insert a line above line 3 and rewrite (what was) line 4.
    rdm_demo(
        &plan,
        &[
            "plan",
            "update",
            "lined",
            "--body",
            "# Plan\n\nA brand-new line.\nIntro line.\nStep one, rewritten.\nonto a second line.\n",
            "--no-edit",
        ],
    )
    .success();
    commit_plan(&plan, "edit the plan after the review started");

    let json = show_review_json(&plan, &id);
    let c = &json["comments"][0];
    let snippet = "Intro line.\nStep one wraps";
    assert_eq!(c["anchor"]["snippet"], snippet);
    assert_eq!(c["anchor"]["start_line"], 3);
    assert_eq!(c["resolution"]["state"], "drifted");
    assert_eq!(c["resolution"]["quote"], snippet);
    assert_eq!(c["resolution"]["body"], "original");

    let out = rdm_demo(&plan, &["review", "show", &id])
        .success()
        .get_output()
        .stdout
        .clone();
    let human = String::from_utf8(out).unwrap();
    assert!(human.contains("(drifted) [lines 3-4]"), "{human}");
    assert!(
        human.contains("  > Intro line.\n  > Step one wraps"),
        "{human}"
    );
}

#[test]
fn an_out_of_range_line_is_refused_or_stored_whole_document_with_its_reason() {
    let plan = init_plan_repo();
    create_plan_with_body(&plan, "lined", LINED_BODY);
    let id = start_review(&plan, "plan/lined");

    rdm_demo(
        &plan,
        &[
            "review",
            "comment",
            &id,
            "--line",
            "50",
            "--body",
            "Too far.",
            "--no-edit",
        ],
    )
    .failure()
    .stderr(predicate::str::contains("line 50 out of range"));
    assert!(
        show_review_json(&plan, &id)["comments"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let out = rdm_demo(
        &plan,
        &[
            "review",
            "comment",
            &id,
            "--line",
            "50",
            "--on-anchor-failure",
            "whole-document",
            "--body",
            "Too far.",
            "--no-edit",
        ],
    )
    .success()
    .stderr(predicate::str::contains("line 50 out of range"))
    .get_output()
    .stdout
    .clone();
    let line: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(line["anchored"], false);
    assert_eq!(line["anchor_state"], "line-out-of-range");

    let c = &show_review_json(&plan, &id)["comments"][0];
    assert!(c.get("anchor").is_none(), "{c}");
    assert_eq!(c["anchor_state"], "line-out-of-range");
}

#[test]
fn line_and_quote_together_are_refused_by_the_parser() {
    let plan = init_plan_repo();
    create_task_with_body(&plan, "quoted", "alpha\nbeta\n");
    let id = start_review(&plan, "task/quoted");
    rdm_demo(
        &plan,
        &[
            "review",
            "comment",
            &id,
            "--line",
            "2",
            "--quote",
            "x",
            "--body",
            "b",
            "--no-edit",
        ],
    )
    .failure()
    .code(2)
    .stderr(predicate::str::contains("cannot be used with"));
}

#[test]
fn line_with_anchor_state_or_intended_path_is_refused_by_the_parser() {
    let plan = init_plan_repo();
    create_task_with_body(&plan, "quoted", "alpha\nbeta\n");
    let id = start_review(&plan, "task/quoted");
    for extra in [
        ["--anchor-state", "whole-document"],
        ["--intended-path", "src/lib.rs"],
    ] {
        rdm_demo(
            &plan,
            &[
                "review",
                "comment",
                &id,
                "--line",
                "1",
                extra[0],
                extra[1],
                "--body",
                "b",
                "--no-edit",
            ],
        )
        .failure()
        .code(2)
        .stderr(predicate::str::contains("cannot be used with"));
    }
    assert!(
        show_review_json(&plan, &id)["comments"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn on_anchor_failure_without_quote_or_line_is_refused_by_the_parser() {
    let plan = init_plan_repo();
    create_task_with_body(&plan, "quoted", "alpha\nbeta\n");
    let id = start_review(&plan, "task/quoted");
    rdm_demo(
        &plan,
        &[
            "review",
            "comment",
            &id,
            "--on-anchor-failure",
            "whole-document",
            "--body",
            "b",
            "--no-edit",
        ],
    )
    .failure()
    .code(2)
    .stderr(predicate::str::contains(
        "the following required arguments were not provided",
    ));
    assert!(
        show_review_json(&plan, &id)["comments"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_malformed_line_range_is_refused_by_the_parser() {
    let plan = init_plan_repo();
    create_task_with_body(&plan, "quoted", "alpha\nbeta\n");
    let id = start_review(&plan, "task/quoted");
    for bad in ["0", "5-3", "two"] {
        rdm_demo(
            &plan,
            &[
                "review",
                "comment",
                &id,
                "--line",
                bad,
                "--body",
                "b",
                "--no-edit",
            ],
        )
        .failure()
        .code(2)
        .stderr(predicate::str::contains("invalid line range"));
    }
}

#[test]
fn a_text_quote_comment_is_unchanged_beside_a_line_comment() {
    let plan = init_plan_repo();
    create_task_with_body(&plan, "mixed", "alpha line\nbeta line\ngamma line\n");
    commit_plan(&plan, "seed: mixed task");

    let quote_comment = |id: &str| {
        rdm_demo(
            &plan,
            &[
                "review",
                "comment",
                id,
                "--quote",
                "beta line",
                "--body",
                "Quoted.",
                "--no-edit",
            ],
        )
        .success();
    };
    let alone = start_review(&plan, "task/mixed");
    quote_comment(&alone);
    let together = start_review(&plan, "task/mixed");
    quote_comment(&together);
    add_line_comment(&plan, &together, "3", &[]);

    let a = &show_review_json(&plan, &alone)["comments"][0];
    let both = show_review_json(&plan, &together);
    let t = &both["comments"][0];
    for key in ["anchor", "resolution", "target"] {
        assert_eq!(a[key], t[key], "{key}");
    }
    assert_eq!(t["anchor"]["anchor_type"], "text-quote");
    assert_eq!(both["comments"][1]["anchor"]["snippet"], "gamma line");
}

#[test]
fn numbered_show_line_n_is_what_line_n_captures_for_every_document_kind() {
    let plan = init_plan_repo();
    create_task_with_body(
        &plan,
        "numbered-task",
        "Task intro.\n\nA wrapped\nparagraph.\n",
    );
    rdm_demo(
        &plan,
        &[
            "roadmap",
            "update",
            "roadmap-z",
            "--body",
            "Roadmap summary.\n\nSecond paragraph,\nwrapped.",
            "--no-edit",
        ],
    )
    .success();
    rdm_demo(
        &plan,
        &[
            "phase",
            "update",
            "1",
            "--roadmap",
            "roadmap-z",
            "--body",
            "## Steps\n\n1. First step\n   continues here.\n",
            "--no-edit",
        ],
    )
    .success();
    create_plan_with_body(&plan, "lined", LINED_BODY);

    let cases: [(&str, Vec<&str>, usize); 4] = [
        ("plan/lined", vec!["plan", "show", "lined"], 7),
        (
            "task/numbered-task",
            vec!["task", "show", "numbered-task"],
            3,
        ),
        ("roadmap/roadmap-z", vec!["roadmap", "show", "roadmap-z"], 4),
        (
            "phase/roadmap-z/1",
            vec!["phase", "show", "1", "--roadmap", "roadmap-z"],
            4,
        ),
    ];
    for (target, show_args, n) in cases {
        let rendering = show_numbered(&plan, &show_args);
        let text = numbered_line(&rendering, n);
        assert!(!text.is_empty(), "{target}: line {n} of {rendering:?}");
        let id = start_review(&plan, target);
        add_line_comment(&plan, &id, &n.to_string(), &[]);
        let c = &show_review_json(&plan, &id)["comments"][0];
        assert_eq!(c["anchor"]["snippet"], text.as_str(), "{target}");
        // A blank line is line N too.
        let blank = numbered_line(&rendering, 2);
        assert_eq!(blank, "", "{target}: line 2 of {rendering:?}");
    }
}

#[test]
fn numbered_show_at_the_created_commit_matches_line_capture_after_an_edit() {
    let plan = init_plan_repo();
    create_plan_with_body(&plan, "lined", LINED_BODY);
    let id = start_review(&plan, "plan/lined");
    let created = show_review_json(&plan, &id)["created_commit"]
        .as_str()
        .unwrap()
        .to_string();

    rdm_demo(
        &plan,
        &[
            "plan",
            "update",
            "lined",
            "--body",
            "Completely\ndifferent\ntext\nnow\n",
            "--no-edit",
        ],
    )
    .success();
    commit_plan(&plan, "rewrite the plan");

    let rendering = show_numbered(&plan, &["plan", "show", "lined", "--at", &created]);
    let text = numbered_line(&rendering, 5);
    assert_eq!(text, "onto a second line.");
    // The current rendering disagrees, but `--line` captures at
    // `created_commit`, exactly as `--at <created_commit>` numbers it.
    assert_eq!(
        numbered_line(&show_numbered(&plan, &["plan", "show", "lined"]), 4),
        "now"
    );
    add_line_comment(&plan, &id, "5", &[]);
    let c = &show_review_json(&plan, &id)["comments"][0];
    assert_eq!(c["anchor"]["snippet"], text.as_str());
}

#[test]
fn a_roadmap_review_line_comment_scoped_to_a_phase_numbers_the_phase_body() {
    let plan = init_plan_repo();
    rdm_demo(
        &plan,
        &[
            "phase",
            "update",
            "1",
            "--roadmap",
            "roadmap-z",
            "--body",
            "Phase line one.\nPhase line two.\n",
            "--no-edit",
        ],
    )
    .success();
    commit_plan(&plan, "seed: phase body");

    let rendering = show_numbered(&plan, &["phase", "show", "1", "--roadmap", "roadmap-z"]);
    let text = numbered_line(&rendering, 2);
    let id = start_review(&plan, "roadmap/roadmap-z");
    add_line_comment(&plan, &id, "2", &["--doc", "phase/1"]);
    let c = &show_review_json(&plan, &id)["comments"][0];
    assert_eq!(c["anchor"]["snippet"], text.as_str());
    assert_eq!(c["target"]["level"], "plan-item");
    assert_eq!(c["target"]["link"], "rdm:phase/roadmap-z/phase-1-build");
}

#[test]
fn numbered_show_refuses_structured_formats_and_no_body() {
    let plan = init_plan_repo();
    create_task_with_body(&plan, "numbered-task", "one\ntwo\n");
    for format in ["json", "table"] {
        rdm_demo(
            &plan,
            &[
                "task",
                "show",
                "numbered-task",
                "--numbered",
                "--format",
                format,
            ],
        )
        .failure()
        .stderr(predicate::str::contains("--numbered"));
    }
    rdm_demo(
        &plan,
        &["task", "show", "numbered-task", "--numbered", "--no-body"],
    )
    .failure()
    .code(2);
    let out = rdm_demo(&plan, &["task", "show", "numbered-task", "--numbered"])
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "     1\tone\n     2\ttwo\n"
    );
}

/// Slices `body` at a resolution's `current` placement; `None` when null.
fn slice_current<'a>(body: &'a str, resolution: &Value) -> Option<&'a str> {
    let current = &resolution["current"];
    if current.is_null() {
        return None;
    }
    let start = usize::try_from(current["range_start"].as_u64().unwrap()).unwrap();
    let end = usize::try_from(current["range_end"].as_u64().unwrap()).unwrap();
    Some(&body[start..end])
}

#[test]
fn review_json_reports_each_comment_current_body_placement() {
    let plan = init_plan_repo();
    create_task_with_body(
        &plan,
        "placed",
        "Alpha. The stable phrase stays put. Then the original wording sits here. Omega.",
    );
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args(["commit", "-m", "seed: add placed task"])
        .assert()
        .success();
    let id = start_review(&plan, "task/placed");
    for quote in ["stable phrase", "original wording"] {
        rdm()
            .arg("--root")
            .arg(plan.path())
            .args([
                "review",
                "comment",
                &id,
                "--quote",
                quote,
                "--body",
                "Look here.",
                "--no-edit",
                "--project",
                "demo",
            ])
            .assert()
            .success();
    }
    add_plain_comment(&plan, &id, "Overall thoughts.");

    // Edit the second span (its context survives) and shift everything by
    // prepending text, so current offsets differ from original ones.
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "task",
            "update",
            "placed",
            "--no-edit",
            "--project",
            "demo",
            "--body",
            "Preface. Alpha. The stable phrase stays put. Then the reworded text sits here. Omega.",
        ])
        .assert()
        .success();

    let task_out = rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "task",
            "show",
            "placed",
            "--project",
            "demo",
            "--format",
            "json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let task: Value = serde_json::from_slice(&task_out).unwrap();
    let body = task["body"].as_str().unwrap().to_string();

    let list_out = rdm()
        .arg("--root")
        .arg(plan.path())
        .args(["review", "list", "--project", "demo", "--format", "json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let list: Value = serde_json::from_slice(&list_out).unwrap();
    let listed = list
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == id.as_str())
        .unwrap_or_else(|| panic!("review {id} missing from list: {list}"))
        .clone();

    for review in [show_review_json(&plan, &id), listed] {
        let comments = review["comments"].as_array().unwrap();
        let resolved = &comments[0]["resolution"];
        assert_eq!(resolved["state"], "resolved", "{review}");
        assert_eq!(slice_current(&body, resolved), Some("stable phrase"));

        let drifted = &comments[1]["resolution"];
        assert_eq!(drifted["state"], "drifted", "{review}");
        assert_eq!(drifted["quote"], "original wording");
        assert_eq!(slice_current(&body, drifted), Some("reworded text"));

        let unresolved = &comments[2]["resolution"];
        assert_eq!(unresolved["state"], "unresolved", "{review}");
        assert!(
            unresolved.get("current").is_some_and(Value::is_null),
            "{review}"
        );
    }
}

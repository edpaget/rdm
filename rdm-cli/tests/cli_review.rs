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
            "abc1234",
            "--reply",
            "Clarified in abc1234.",
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
    assert_eq!(json["comments"][0]["applied_commit"], "abc1234");
    assert_eq!(json["comments"][0]["reply"], "Clarified in abc1234.");
    assert_eq!(json["comments"][1]["status"], "wont-fix");
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
fn review_start_staged_defers_commit_and_commit_includes_review_file() {
    let plan = init_plan_repo();
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "--stage",
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
        .stdout(predicate::str::contains("staged"));

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

    // Hand-write a review fixture carrying a `line-range` anchor, the way a
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
    anchor_type: line-range
    start: 3
    end: 7
  body: Uses a line-range anchor from a newer rdm.
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
    assert_eq!(comment["anchor"]["anchor_type"], "line-range");
    assert_eq!(comment["anchor"]["start"], 3);
    assert_eq!(comment["anchor"]["end"], 7);
    // ...and resolves as unresolved (whole-document treatment).
    assert_eq!(comment["resolution"]["state"], "unresolved");
    assert!(comment["resolution"].get("quote").is_none());
}

//! Cross-consumer agreement on what counts as a *user* change, plus the
//! no-user-action backfill of the `INDEX.md` merge-driver mapping.
//!
//! Every assertion here goes through the real `rdm` binary, so it exercises
//! the same `git_status_report()` consumption boundary the CLI ships.

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn rdm() -> Command {
    let mut cmd = Command::cargo_bin("rdm").unwrap();
    // Isolate from host global config (e.g. default_format = "json").
    cmd.env("XDG_CONFIG_HOME", "/dev/null/nonexistent");
    cmd
}

/// Initialize a plan repo with a project and an initial git commit, so the
/// `.gitattributes` written by `rdm init` is already tracked.
fn init_repo(dir: &TempDir) {
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("init")
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "create", "test", "--title", "Test Project"])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["commit", "-m", "seed: init plan repo and project"])
        .assert()
        .success();
}

fn git(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    std::process::Command::new("git")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap()
}

fn last_commit_files(dir: &std::path::Path) -> Vec<String> {
    let output = git(dir, &["log", "--name-only", "-1", "--pretty=format:"]);
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

/// Rewrites HEAD so it has no `.gitattributes` at all, simulating a plan repo
/// created before the merge driver shipped.
///
/// Uses raw git deliberately: every `rdm` command re-ensures the mapping on
/// open, so the removal cannot be committed through `rdm commit`.
fn drop_gitattributes_from_head(dir: &std::path::Path) {
    let out = git(dir, &["rm", "--cached", "--quiet", ".gitattributes"]);
    assert!(out.status.success(), "git rm failed: {out:?}");
    let out = git(dir, &["commit", "--quiet", "-m", "chore: pre-driver repo"]);
    assert!(out.status.success(), "git commit failed: {out:?}");
    std::fs::remove_file(dir.join(".gitattributes")).unwrap();
    let out = git(dir, &["ls-tree", "--name-only", "HEAD"]);
    assert!(
        !String::from_utf8_lossy(&out.stdout).contains(".gitattributes"),
        "HEAD must no longer carry .gitattributes"
    );
}

/// The `added:`/`modified:`/`deleted:` listing lines of `rdm status` output.
fn listed_changes(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .filter(|l| {
            l.starts_with("  added:") || l.starts_with("  modified:") || l.starts_with("  deleted:")
        })
        .map(str::to_string)
        .collect()
}

fn create_one_roadmap(dir: &TempDir, slug: &str) {
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "roadmap",
            "create",
            slug,
            "--title",
            "A Roadmap",
            "--no-edit",
            "--project",
            "test",
        ])
        .assert()
        .success();
}

// ---------------------------------------------------------------------------
// AC2 — all seven `git_status()` consumers agree on what counts as a change
// ---------------------------------------------------------------------------

#[test]
fn status_commit_hint_and_discard_agree_on_the_user_change_count() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    // Exactly one user edit — which regenerates both index files.
    create_one_roadmap(&dir, "only-roadmap");

    // 1. `rdm status`
    let out = rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("status")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let out = String::from_utf8(out).unwrap();
    let listed = listed_changes(&out);
    assert_eq!(
        listed.len(),
        1,
        "status should list exactly the one user change, got: {listed:?}"
    );
    assert!(
        !listed.iter().any(|l| l.contains("INDEX.md")),
        "status must not list generated indexes as changes, got: {listed:?}"
    );
    assert!(
        out.contains("1 file(s) changed"),
        "status count must be the user count, got: {out}"
    );
    assert!(
        out.contains("2 generated index file(s) will be included in the next commit")
            && out.contains("projects/test/INDEX.md"),
        "the generated files must be named, not merely counted, got: {out}"
    );

    // 2. The post-command hint on a read-only command (stderr).
    let hint = rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["roadmap", "list", "--project", "test"])
        .assert()
        .success()
        .get_output()
        .stderr
        .clone();
    let hint = String::from_utf8(hint).unwrap();
    assert!(
        hint.contains("1 uncommitted change(s)"),
        "the hint must agree with status, got: {hint}"
    );

    // 3. `rdm commit` — counts the user change, but lands the indexes too.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["commit", "-m", "feat: add only-roadmap"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Committed 1 file(s) (plus 2 regenerated index file(s)).",
        ));

    let files = last_commit_files(dir.path());
    assert!(
        files.iter().any(|f| f == "INDEX.md"),
        "the root index must be in the commit, got: {files:?}"
    );
    assert!(
        files.iter().any(|f| f == "projects/test/INDEX.md"),
        "the project index must be in the commit, got: {files:?}"
    );
}

#[test]
fn discard_reports_the_user_count_but_restores_the_indexes_too() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);
    create_one_roadmap(&dir, "doomed-roadmap");

    let index_before = std::fs::read_to_string(dir.path().join("projects/test/INDEX.md")).unwrap();
    assert!(index_before.contains("doomed-roadmap"));

    let out = rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["discard", "--force"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Discarded 1 file(s) (plus 2 regenerated index file(s)).",
        ))
        .get_output()
        .stdout
        .clone();
    let out = String::from_utf8(out).unwrap();
    let listed: Vec<&str> = out
        .lines()
        .filter(|l| l.starts_with("  removed:") || l.starts_with("  restored:"))
        .collect();
    assert_eq!(
        listed.len(),
        1,
        "discard should list exactly the one user change, got: {listed:?}"
    );
    assert!(
        !listed.iter().any(|l| l.contains("INDEX.md")),
        "discard must not list generated indexes, got: {listed:?}"
    );

    // But the index file is nevertheless restored on disk.
    let index_after = std::fs::read_to_string(dir.path().join("projects/test/INDEX.md")).unwrap();
    assert!(
        !index_after.contains("doomed-roadmap"),
        "the regenerated index must still be restored, got: {index_after}"
    );
}

#[test]
fn commit_lands_the_regenerated_index_when_it_is_the_only_change() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    // Corrupt only the generated indexes; no user file differs from HEAD.
    // These are raw `fs::write`s outside rdm, so they belong to no changeset
    // — the scoped views must report them rather than sweep them, and the
    // whole-tree views must still cover them.
    std::fs::write(dir.path().join("INDEX.md"), "# stale\n").unwrap();
    std::fs::write(dir.path().join("projects/test/INDEX.md"), "# stale\n").unwrap();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("No uncommitted changes."))
        .stdout(predicate::str::contains(
            "2 file(s) belong to other changesets and were left untouched",
        ));

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["status", "--all"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "2 generated index file(s) will be included in the next commit",
        ));

    let head_before = String::from_utf8_lossy(&git(dir.path(), &["rev-parse", "HEAD"]).stdout)
        .trim()
        .to_string();

    // Gated on the raw truth: a derived-only tree must still be committable
    // through the whole-tree opt-in, which is the documented recovery for
    // out-of-band index corruption.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["commit", "--all", "-m", "chore: regenerate indexes"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Committed 2 regenerated index file(s).",
        ));

    let head_after = String::from_utf8_lossy(&git(dir.path(), &["rev-parse", "HEAD"]).stdout)
        .trim()
        .to_string();
    assert_ne!(
        head_before, head_after,
        "the commit must have advanced HEAD"
    );

    let files = last_commit_files(dir.path());
    assert!(files.iter().any(|f| f == "INDEX.md"), "got: {files:?}");
    assert!(
        files.iter().any(|f| f == "projects/test/INDEX.md"),
        "got: {files:?}"
    );
}

// ---------------------------------------------------------------------------
// AC1 — the mapping is backfilled with no user action, and survives a discard
// ---------------------------------------------------------------------------

#[test]
fn read_only_command_backfills_the_mapping_with_no_user_action() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    // Simulate a repo that predates the merge driver.
    drop_gitattributes_from_head(dir.path());

    // A plain read-only command — no opt-in, no setup command.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["list", "--project", "test"])
        .assert()
        .success();

    let attrs = std::fs::read_to_string(dir.path().join(".gitattributes")).unwrap();
    assert!(
        attrs.contains("INDEX.md merge=rdm-index") && attrs.contains("**/INDEX.md merge=rdm-index"),
        "a read-only command must restore both mapping lines, got: {attrs}"
    );
}

#[test]
fn discard_force_leaves_the_mapping_installed() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    // Make the mapping uncommitted — the `Added` shape a discard would
    // otherwise delete outright: drop it from HEAD, then let the next open
    // re-add it into the worktree.
    drop_gitattributes_from_head(dir.path());
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["list", "--project", "test"])
        .assert()
        .success();
    assert!(dir.path().join(".gitattributes").exists());

    let out = rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["discard", "--force"])
        .assert()
        .success();

    let attrs = std::fs::read_to_string(dir.path().join(".gitattributes")).unwrap_or_default();
    assert!(
        attrs.contains("merge=rdm-index"),
        "a discard must never silently un-map the repo, got: {attrs}"
    );

    // ...and it must not claim it removed a file that is sitting right there.
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    assert!(
        stdout.contains("reinstalled: .gitattributes"),
        "the mapping rdm put straight back must be reported as reinstalled, got: {stdout}"
    );
    assert!(
        !stdout.contains("removed:  .gitattributes"),
        "reporting the still-present mapping as removed is a false statement, got: {stdout}"
    );
}

// ---------------------------------------------------------------------------
// AC3 — the operative claim of the rewritten docs paragraph, checked
// behaviorally (no test asserts on documentation prose)
// ---------------------------------------------------------------------------

#[test]
fn every_command_that_opens_the_repo_installs_both_merge_driver_halves() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    // Strip BOTH halves.
    drop_gitattributes_from_head(dir.path());
    let config_path = dir.path().join(".git").join("config");
    let config = std::fs::read_to_string(&config_path).unwrap();
    let stripped: String = {
        let mut out = String::new();
        let mut skipping = false;
        for line in config.lines() {
            if line.trim_start().starts_with('[') {
                skipping = line.contains("[merge \"rdm-index\"]");
            }
            if !skipping {
                out.push_str(line);
                out.push('\n');
            }
        }
        out
    };
    std::fs::write(&config_path, &stripped).unwrap();
    assert!(!stripped.contains("[merge \"rdm-index\"]"));
    assert!(
        stripped.contains("[core]"),
        "the strip must keep the rest of the config, got: {stripped}"
    );

    // A plain read-only command.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["list", "--project", "test"])
        .assert()
        .success();

    let attrs = std::fs::read_to_string(dir.path().join(".gitattributes")).unwrap();
    assert!(
        attrs.contains("INDEX.md merge=rdm-index") && attrs.contains("**/INDEX.md merge=rdm-index"),
        "worktree half missing after open, got: {attrs}"
    );
    let config = std::fs::read_to_string(&config_path).unwrap();
    assert!(
        config.contains("[merge \"rdm-index\"]") && config.contains("driver = rdm --root . index"),
        "config half missing after open, got: {config}"
    );

    // The doc's second claim: the written `.gitattributes` is an ordinary
    // working-tree change — visible in status, landed by the next commit.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains(".gitattributes"));

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["commit", "-m", "chore: install merge mapping"])
        .assert()
        .success();

    let files = last_commit_files(dir.path());
    assert!(
        files.iter().any(|f| f == ".gitattributes"),
        "the mapping must be committable so it travels with clones, got: {files:?}"
    );
}

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

    // Exactly one user edit. A mutation regenerates no index, so this is the
    // only path in the changeset.
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
        !out.contains("generated index file(s)"),
        "a mutation stages no generated index, so status must not name one, got: {out}"
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

    // 3. `rdm commit` — the commit contains exactly the authored file.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["commit", "-m", "feat: add only-roadmap"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Committed 1 file(s)."))
        .stdout(predicate::str::contains("regenerated index file(s)").not());

    let files = last_commit_files(dir.path());
    assert_eq!(
        files,
        vec!["projects/test/roadmaps/only-roadmap/roadmap.md".to_string()],
        "the commit must contain exactly the authored path, got: {files:?}"
    );
}

#[test]
fn single_mutation_stages_exactly_one_path() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

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
            "test",
        ])
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["commit", "-m", "seed: add fix-bug"])
        .assert()
        .success();

    // One status-only mutation.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "task",
            "update",
            "fix-bug",
            "--status",
            "in-progress",
            "--no-edit",
            "--project",
            "test",
        ])
        .assert()
        .success();

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
        "a single mutation must stage exactly one path, got: {listed:?}"
    );
    assert!(
        listed[0].contains("projects/test/tasks/fix-bug.md"),
        "the staged path must be the task file, got: {listed:?}"
    );
    assert!(
        out.contains("1 file(s) changed"),
        "status count must be 1, got: {out}"
    );
    assert!(
        !out.contains("generated index file(s)"),
        "no generated index may be staged by a mutation, got: {out}"
    );

    // The session journal names exactly that one path.
    let journal = rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["session", "journal", "--format", "json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let journal: serde_json::Value = serde_json::from_slice(&journal).unwrap();
    let paths: Vec<String> = journal["paths"]
        .as_array()
        .expect("journal paths array")
        .iter()
        .map(|e| e["path"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        paths,
        vec!["projects/test/tasks/fix-bug.md".to_string()],
        "the journal must name exactly the authored path, got: {paths:?}"
    );

    // And the commit contains exactly that file.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["commit", "-m", "chore: start fix-bug"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Committed 1 file(s)."))
        .stdout(predicate::str::contains("regenerated index file(s)").not());

    let files = last_commit_files(dir.path());
    assert_eq!(
        files,
        vec!["projects/test/tasks/fix-bug.md".to_string()],
        "the commit tree must name exactly the one task file, got: {files:?}"
    );
}

#[test]
fn discard_reports_the_user_count_and_leaves_the_index_untouched() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    // Produce a committed index explicitly, so there is an on-disk derived
    // file a discard could wrongly rewrite.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("index")
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["commit", "-m", "chore: generate indexes"])
        .assert()
        .success();

    create_one_roadmap(&dir, "doomed-roadmap");

    let index_before = std::fs::read_to_string(dir.path().join("projects/test/INDEX.md")).unwrap();
    assert!(
        !index_before.contains("doomed-roadmap"),
        "the mutation must not have written the new roadmap into the index"
    );

    let out = rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["discard", "--force"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Discarded 1 file(s)."))
        .stdout(predicate::str::contains("regenerated index file(s)").not())
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

    // The on-disk index is byte-identical: a discard rewrites no derived path.
    let index_after = std::fs::read_to_string(dir.path().join("projects/test/INDEX.md")).unwrap();
    assert_eq!(
        index_before, index_after,
        "a discard must leave the generated index exactly as it found it"
    );
}

#[test]
fn commit_lands_the_regenerated_index_when_it_is_the_only_change() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    // Corrupt only the generated indexes; no user file differs from HEAD.
    // These are raw `fs::write`s outside rdm, so they belong to NO
    // changeset at all — the scoped views must report them as unattributed
    // rather than sweep them or misname them as another changeset's, and
    // the whole-tree views must still cover them.
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
            "2 file(s) are not attributed to any changeset",
        ))
        .stdout(predicate::str::contains("belong to other changesets").not());

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

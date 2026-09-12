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
/// tests below start from a committed HEAD and a clean tree.
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

/// Seeds a repo into the shape a plan repo created by an OLD rdm is in:
/// HEAD tracks a `.gitattributes` carrying the `merge=rdm-index` lines, and
/// `.git/config` carries the matching `[merge "rdm-index"]` driver section.
///
/// Uses raw git deliberately: rdm no longer writes either half, so neither can
/// be produced by driving the CLI.
fn seed_legacy_merge_driver(dir: &std::path::Path) {
    std::fs::write(
        dir.join(".gitattributes"),
        "INDEX.md merge=rdm-index\n**/INDEX.md merge=rdm-index\n",
    )
    .unwrap();
    let out = git(dir, &["add", ".gitattributes"]);
    assert!(out.status.success(), "git add failed: {out:?}");
    let out = git(
        dir,
        &["commit", "--quiet", "-m", "chore: legacy merge driver"],
    );
    assert!(out.status.success(), "git commit failed: {out:?}");

    let config_path = dir.join(".git").join("config");
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&config_path)
        .unwrap();
    use std::io::Write;
    writeln!(
        file,
        "\n[merge \"rdm-index\"]\n\tname = rdm INDEX.md merge driver\n\tdriver = rdm --root . index --merge-output %A --merge-path %P"
    )
    .unwrap();
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
        "status must not list an INDEX.md as a change: a mutation writes none, got: {listed:?}"
    );
    assert!(
        out.contains("1 file(s) changed"),
        "status count must be the user count, got: {out}"
    );
    assert!(
        !out.contains("generated index file(s)"),
        "a mutation stages no INDEX.md, and the retired generated-index line must \
         never come back, got: {out}"
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

/// A derived index this session regenerated *itself* is part of its own
/// changeset, and a discard must actually discard it.
///
/// The sibling of the test above, and the case that separates "another
/// session's dirty index is left alone" (correct) from "any dirty index is
/// left alone" (an orphan). Since mutations stopped regenerating the index,
/// the only way a changeset holds a derived path is an explicit `rdm index`
/// — the very workflow the CHANGELOG points users at. If the discard skipped
/// it while still retiring its journal claim, the file would be left on disk
/// holding the reverted mutation's content, attributed to nobody, and
/// `rdm status` would report it as unattributed dirt needing `--all`.
#[test]
fn discard_reverts_an_index_this_session_regenerated_itself() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    // A committed baseline index, so the regeneration below is a modification
    // of a tracked file rather than an add.
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

    let index_file = dir.path().join("projects/test/INDEX.md");
    let committed_index = std::fs::read_to_string(&index_file).unwrap();

    // One mutation, then an explicit regeneration that folds it into the
    // index — both now belong to this one changeset.
    create_one_roadmap(&dir, "doomed-roadmap");
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("index")
        .assert()
        .success();

    let dirty_index = std::fs::read_to_string(&index_file).unwrap();
    assert!(
        dirty_index.contains("doomed-roadmap"),
        "the explicit regeneration must have folded the mutation into the index, got: \
         {dirty_index}"
    );

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["discard", "--force"])
        .assert()
        .success()
        // One count over every restored path: the two indexes this session
        // regenerated are ordinary claimed paths, not a separate class. The
        // negatives are standing guards that the retired wording never
        // returns.
        .stdout(predicate::str::contains("Discarded 3 file(s)."))
        .stdout(predicate::str::contains("regenerated index file(s)").not())
        .stdout(predicate::str::contains("generated index file(s)").not());

    // The index this session dirtied is back at its committed content — not
    // left holding a roadmap that no longer exists.
    let index_after = std::fs::read_to_string(&index_file).unwrap();
    assert_eq!(
        committed_index, index_after,
        "a discard must revert the index this same session regenerated"
    );

    // Nothing is stranded: the tree is clean and nothing is unattributed.
    let porcelain = git(dir.path(), &["status", "--porcelain"]);
    let porcelain = String::from_utf8(porcelain.stdout).unwrap();
    assert!(
        porcelain.trim().is_empty(),
        "the tree must be clean after the discard, got: {porcelain}"
    );

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
    assert!(
        !out.contains("not attributed"),
        "the discard must not strand unattributed dirt, got: {out}"
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
        // Two ordinary changes in the whole-tree view, listed by name rather
        // than summarized on a generated-index line that no longer exists.
        .stdout(predicate::str::contains("2 file(s) changed."))
        .stdout(predicate::str::contains("INDEX.md"))
        .stdout(predicate::str::contains("projects/test/INDEX.md"))
        .stdout(predicate::str::contains("generated index file(s)").not());

    let head_before = String::from_utf8_lossy(&git(dir.path(), &["rev-parse", "HEAD"]).stdout)
        .trim()
        .to_string();

    // Gated on the raw truth: an index-only dirty tree must still be
    // committable through the whole-tree opt-in, which is the documented
    // recovery for out-of-band index corruption.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["commit", "--all", "-m", "chore: regenerate indexes"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Committed 2 file(s)."))
        .stdout(predicate::str::contains("regenerated index file(s)").not());

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
// The merge driver is retired: rdm installs neither half, authors no dirt,
// and reinstates nothing on discard. These are the inversions of the three
// tests that used to pin the opposite.
// ---------------------------------------------------------------------------

#[test]
fn a_read_only_command_creates_no_gitattributes_and_leaves_status_empty() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    // A plain read-only command on a repo that never carried the file.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["list", "--project", "test"])
        .assert()
        .success();

    assert!(
        !dir.path().join(".gitattributes").exists(),
        "a read-only command must author nothing into the worktree"
    );

    let out = rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("status")
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    assert!(
        listed_changes(&stdout).is_empty(),
        "rdm status must name zero paths after a read-only command, got: {stdout}"
    );
}

#[test]
fn no_command_installs_a_merge_driver_and_the_stale_section_is_swept() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);
    seed_legacy_merge_driver(dir.path());

    let config_path = dir.path().join(".git").join("config");
    assert!(
        std::fs::read_to_string(&config_path)
            .unwrap()
            .contains("[merge \"rdm-index\"]"),
        "the fixture must start with the stale section, or this proves nothing"
    );

    // A plain read-only command.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["list", "--project", "test"])
        .assert()
        .success();

    let config = std::fs::read_to_string(&config_path).unwrap();
    assert!(
        !config.contains("[merge \"rdm-index\"]"),
        "the stale driver section must be swept on open, got: {config}"
    );
    assert!(
        config.contains("[core]"),
        "the sweep must remove only that section, got: {config}"
    );

    // The tracked `.gitattributes` is a user file and is deliberately left
    // alone: inert without a configured driver, and possibly hand-edited.
    assert_eq!(
        std::fs::read_to_string(dir.path().join(".gitattributes")).unwrap(),
        "INDEX.md merge=rdm-index\n**/INDEX.md merge=rdm-index\n",
        "the user's tracked .gitattributes must be left byte-for-byte alone"
    );

    // ...and touching neither leaves the tree clean.
    let out = rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("status")
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    assert!(
        listed_changes(&stdout).is_empty(),
        "the sweep touches only .git/config, which is invisible to status, got: {stdout}"
    );
}

#[test]
fn discard_force_reinstates_nothing() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);
    seed_legacy_merge_driver(dir.path());

    // Some real dirt for the discard to act on. Raw `fs::write` belongs to no
    // changeset, so `--all` is what reaches it — and `--all` is the whole-tree
    // path, which is where the `reinstalled:` line used to come from.
    std::fs::write(dir.path().join(".gitattributes"), "*.bin binary\n").unwrap();

    let out = rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["discard", "--force", "--all"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();

    assert!(
        !stdout.contains("reinstalled:"),
        "a discard puts nothing back any more, so it must never say reinstalled, got: {stdout}"
    );

    // The tree matches HEAD exactly — including the file rdm used to rewrite.
    assert_eq!(
        std::fs::read_to_string(dir.path().join(".gitattributes")).unwrap(),
        "INDEX.md merge=rdm-index\n**/INDEX.md merge=rdm-index\n",
        "the discard must restore HEAD's content, not rdm's own"
    );
    let porcelain = git(dir.path(), &["status", "--porcelain"]);
    assert_eq!(
        String::from_utf8_lossy(&porcelain.stdout).trim(),
        "",
        "the post-discard tree must be HEAD-exact"
    );
}

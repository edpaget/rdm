use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn rdm() -> Command {
    let mut cmd = Command::cargo_bin("rdm").unwrap();
    // Isolate from host global config (e.g. default_format = "json").
    cmd.env("XDG_CONFIG_HOME", "/dev/null/nonexistent");
    cmd
}

/// Initialize a plan repo with a project and an initial git commit.
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
        .arg("project")
        .arg("create")
        .arg("test")
        .arg("--title")
        .arg("Test Project")
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["commit", "-m", "seed: init plan repo and project"])
        .assert()
        .success();
}

/// Count git commits using gitoxide.
fn count_git_commits(dir: &std::path::Path) -> usize {
    let repo = gix::open(dir).unwrap();
    let mut count = 0;
    if let Ok(mut head) = repo.head()
        && let Ok(commit) = head.peel_to_commit()
    {
        count = 1;
        let mut ancestors = commit.ancestors().all().unwrap();
        while ancestors.next().is_some() {
            count += 1;
        }
    }
    count
}

/// Get the latest commit message using gitoxide.
fn last_commit_message(dir: &std::path::Path) -> String {
    let repo = gix::open(dir).unwrap();
    let mut head = repo.head().unwrap();
    let commit = head.peel_to_commit().unwrap();
    String::from_utf8_lossy(commit.message_raw_sloppy()).to_string()
}

/// Names of files touched by the most recent commit.
fn last_commit_files(dir: &std::path::Path) -> Vec<String> {
    let output = std::process::Command::new("git")
        // Clear GIT_DIR/GIT_WORK_TREE/GIT_INDEX_FILE so this doesn't inherit
        // the outer repo's git env when run from inside a git hook (e.g. the
        // pre-commit hook that runs this very test suite).
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .args(["log", "--name-only", "-1", "--pretty=format:"])
        .current_dir(dir)
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

/// Every path in HEAD's tree, not just the ones the tip commit changed.
///
/// Distinct from [`last_commit_files`] on purpose: assertions about what a
/// commit *contains* (including paths inherited from HEAD) would pass
/// vacuously against a changed-files listing. Clears the same git env vars,
/// for the same reason.
fn committed_tree_paths(dir: &std::path::Path) -> Vec<String> {
    let output = std::process::Command::new("git")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .args(["ls-tree", "-r", "--name-only", "HEAD"])
        .current_dir(dir)
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

/// The contents of a path as of HEAD.
fn show_at_head(dir: &std::path::Path, path: &str) -> String {
    let output = std::process::Command::new("git")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .args(["show", &format!("HEAD:{path}")])
        .current_dir(dir)
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn status_shows_uncommitted_changes() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("roadmap")
        .arg("create")
        .arg("st-test")
        .arg("--title")
        .arg("Status Test")
        .arg("--no-edit")
        .arg("--project")
        .arg("test")
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("Uncommitted changes"))
        .stdout(predicate::str::contains("roadmap.md"));
}

#[test]
fn commit_creates_git_commit() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("roadmap")
        .arg("create")
        .arg("commit-test")
        .arg("--title")
        .arg("Commit Test")
        .arg("--no-edit")
        .arg("--project")
        .arg("test")
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("commit")
        .arg("-m")
        .arg("test commit message")
        .assert()
        .success()
        .stdout(predicate::str::contains("Committed"));

    let msg = last_commit_message(dir.path());
    assert!(
        msg.contains("test commit message"),
        "expected 'test commit message' in commit message:\n{msg}"
    );
}

#[test]
fn discard_requires_force() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("discard")
        .assert()
        .failure()
        .stderr(predicate::str::contains("--force"));
}

#[test]
fn discard_restores_head() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    // Create (write + stage) a roadmap.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("roadmap")
        .arg("create")
        .arg("discard-test")
        .arg("--title")
        .arg("Discard Test")
        .arg("--no-edit")
        .arg("--project")
        .arg("test")
        .assert()
        .success();

    // Verify file exists
    assert!(
        dir.path()
            .join("projects/test/roadmaps/discard-test/roadmap.md")
            .exists()
    );

    // Discard
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("discard")
        .arg("--force")
        .assert()
        .success()
        .stdout(predicate::str::contains("Discarded"));

    // File should be gone
    assert!(
        !dir.path()
            .join("projects/test/roadmaps/discard-test/roadmap.md")
            .exists()
    );
}

/// End-to-end: creating a roadmap and phase only stages changes; `rdm status`
/// reports them and `rdm commit` lands exactly one new commit containing both
/// entity files.
#[test]
fn end_to_end_stage_then_commit_lands_one_commit() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    let commits_before = count_git_commits(dir.path());

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "roadmap",
            "create",
            "e2e-roadmap",
            "--title",
            "E2E Roadmap",
            "--no-edit",
            "--project",
            "test",
        ])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "create",
            "e2e-phase",
            "--title",
            "E2E Phase",
            "--number",
            "1",
            "--no-edit",
            "--roadmap",
            "e2e-roadmap",
            "--project",
            "test",
        ])
        .assert()
        .success();

    // Status reports exactly the two entity files as user changes. INDEX
    // regeneration also rewrites the top-level INDEX.md and
    // projects/test/INDEX.md, but those are generated output and are reported
    // on their own line rather than counted.
    let status_out = rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("Uncommitted changes"))
        .stdout(predicate::str::contains("roadmap.md"))
        .stdout(predicate::str::contains("phase-1-e2e-phase.md"))
        .stdout(predicate::str::contains("2 file(s) changed"))
        .get_output()
        .stdout
        .clone();
    let status_out = String::from_utf8(status_out).unwrap();
    let listed: Vec<&str> = status_out
        .lines()
        .filter(|l| {
            l.starts_with("  added:") || l.starts_with("  modified:") || l.starts_with("  deleted:")
        })
        .collect();
    assert_eq!(
        listed.len(),
        2,
        "expected exactly the two entity files listed, got: {listed:?}"
    );
    assert!(
        !listed.iter().any(|l| l.contains("INDEX.md")),
        "generated indexes must not appear in the change listing, got: {listed:?}"
    );
    assert!(
        status_out.contains("2 generated index file(s) will be included in the next commit"),
        "generated indexes must still be named, got: {status_out}"
    );

    // Commit lands exactly one new commit.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["commit", "-m", "feat: add e2e roadmap and phase"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Committed 2 file(s) (plus 2 regenerated index file(s)).",
        ));

    let commits_after = count_git_commits(dir.path());
    assert_eq!(
        commits_after,
        commits_before + 1,
        "exactly one new commit should have landed"
    );

    // Both entity files appear in that one commit.
    let files = last_commit_files(dir.path());
    assert!(
        files.iter().any(|f| f.ends_with("roadmap.md")),
        "commit should include roadmap.md, got: {files:?}"
    );
    assert!(
        files.iter().any(|f| f.ends_with("phase-1-e2e-phase.md")),
        "commit should include phase-1-e2e-phase.md, got: {files:?}"
    );
    // The generated indexes are excluded from the count but NOT from the
    // commit — that is the whole contract.
    assert!(
        files.iter().any(|f| f == "INDEX.md"),
        "commit should include the regenerated root INDEX.md, got: {files:?}"
    );
    assert!(
        files.iter().any(|f| f == "projects/test/INDEX.md"),
        "commit should include the regenerated project INDEX.md, got: {files:?}"
    );
}

// ---------------------------------------------------------------------------
// Session-scoped commit / status / discard
//
// Each test drives the real binary with two distinct explicit `RDM_SESSION`
// values, so a single working tree carries two concurrent changesets — the
// arrangement every assertion below is about.
// ---------------------------------------------------------------------------

fn rdm_as(session: &str, dir: &TempDir) -> Command {
    let mut cmd = rdm();
    cmd.env("RDM_SESSION", session);
    cmd.arg("--root").arg(dir.path());
    cmd
}

fn seed_two_changesets(dir: &TempDir) {
    init_repo(dir);
    rdm_as("cs-a", dir)
        .args([
            "task",
            "create",
            "a-task",
            "--title",
            "A",
            "--no-edit",
            "--project",
            "test",
        ])
        .assert()
        .success();
    rdm_as("cs-b", dir)
        .args([
            "task",
            "create",
            "b-task",
            "--title",
            "B",
            "--no-edit",
            "--project",
            "test",
        ])
        .assert()
        .success();
}

/// Session A creates a project and does not commit; session B creates a task
/// under it and commits first. Before the seed-side prune this aborted with
/// exit 1 and a misleading `project not found: alt`.
#[test]
fn commit_lands_under_a_project_another_changeset_has_not_committed() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    rdm_as("cs-a", &dir)
        .args(["project", "create", "alt", "--title", "Alt"])
        .assert()
        .success();
    // Not committed: A's manifest is on disk but in no commit.
    assert!(
        dir.path().join("projects/alt/project.md").exists(),
        "fixture: A's project.md was never written"
    );
    assert!(
        !committed_tree_paths(dir.path())
            .iter()
            .any(|p| p == "projects/alt/project.md"),
        "fixture: A's project already landed, the scenario is vacuous"
    );

    rdm_as("cs-b", &dir)
        .args([
            "task",
            "create",
            "b-task",
            "--title",
            "B",
            "--no-edit",
            "--project",
            "alt",
        ])
        .assert()
        .success();
    rdm_as("cs-b", &dir)
        .args(["commit", "-m", "land b"])
        .assert()
        .success()
        .stderr(predicate::str::contains("project not found").not());

    let tree = committed_tree_paths(dir.path());
    assert!(
        tree.iter().any(|p| p == "projects/alt/tasks/b-task.md"),
        "B's own document did not land: {tree:?}"
    );
    assert!(
        !tree.iter().any(|p| p == "projects/alt/project.md"),
        "A's uncommitted manifest was swept in: {tree:?}"
    );
    assert!(
        !tree.iter().any(|p| p == "projects/alt/INDEX.md"),
        "an orphan project index landed: {tree:?}"
    );
    let index = show_at_head(dir.path(), "INDEX.md");
    assert!(
        !index.contains("projects/alt/INDEX.md"),
        "the root index links a path the tree does not contain: {index}"
    );
}

/// The heal: the deferred rows return the moment the owning session commits.
#[test]
fn a_deferred_index_row_returns_when_the_owning_changeset_commits() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);
    rdm_as("cs-a", &dir)
        .args(["project", "create", "alt", "--title", "Alt"])
        .assert()
        .success();
    rdm_as("cs-b", &dir)
        .args([
            "task",
            "create",
            "b-task",
            "--title",
            "B",
            "--no-edit",
            "--project",
            "alt",
        ])
        .assert()
        .success();
    rdm_as("cs-b", &dir)
        .args(["commit", "-m", "land b"])
        .assert()
        .success();

    rdm_as("cs-a", &dir)
        .args(["commit", "-m", "land alt"])
        .assert()
        .success();

    let tree = committed_tree_paths(dir.path());
    for want in ["projects/alt/project.md", "projects/alt/INDEX.md"] {
        assert!(
            tree.iter().any(|p| p == want),
            "{want} missing after the owning session committed: {tree:?}"
        );
    }
    let index = show_at_head(dir.path(), "INDEX.md");
    assert!(
        index.contains("projects/alt/INDEX.md"),
        "the deferred root-index row did not return: {index}"
    );
    let project_index = show_at_head(dir.path(), "projects/alt/INDEX.md");
    assert!(
        project_index.contains("b-task"),
        "B's task did not reappear in the reconciled project index: {project_index}"
    );
}

#[test]
fn commit_lands_only_the_callers_changeset() {
    let dir = TempDir::new().unwrap();
    seed_two_changesets(&dir);

    rdm_as("cs-a", &dir)
        .args(["commit", "-m", "land a"])
        .assert()
        .success();

    let files = last_commit_files(dir.path());
    assert!(
        files.iter().any(|f| f == "projects/test/tasks/a-task.md"),
        "own file missing: {files:?}"
    );
    assert!(
        !files.iter().any(|f| f == "projects/test/tasks/b-task.md"),
        "the other changeset's file was swept in: {files:?}"
    );
    assert!(
        dir.path().join("projects/test/tasks/b-task.md").exists(),
        "the other changeset's file must survive on disk"
    );
}

#[test]
fn commit_all_is_the_whole_tree_opt_in() {
    let dir = TempDir::new().unwrap();
    seed_two_changesets(&dir);

    rdm_as("cs-a", &dir)
        .args(["commit", "--all", "-m", "land everything"])
        .assert()
        .success();

    let files = last_commit_files(dir.path());
    assert!(
        files.iter().any(|f| f == "projects/test/tasks/b-task.md"),
        "--all did not include the other changeset: {files:?}"
    );
}

/// AC2 + AC4: `rdm commit --all` must clear every changeset's journal, not
/// just the acting session's own — `rdm session list` reports nothing
/// outstanding once everything has landed.
#[test]
fn commit_all_clears_every_changesets_journal() {
    let dir = TempDir::new().unwrap();
    seed_two_changesets(&dir);

    rdm_as("cs-a", &dir)
        .args(["commit", "--all", "-m", "land everything"])
        .assert()
        .success();

    let out = rdm_as("cs-a", &dir)
        .args(["session", "list", "--format", "json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(
        String::from_utf8(out).unwrap().trim(),
        "[]",
        "rdm session list must report nothing outstanding after `commit --all`"
    );
}

/// The degenerate "already clean" half of AC4: a tree that already matches
/// HEAD (nothing for git to commit) but still carries a stale journal entry
/// left over from an earlier no-op write — the literal "no unlanded work"
/// case from the phase's Problem section.
#[test]
fn commit_all_clears_a_stale_journal_even_when_the_tree_is_already_clean() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    rdm_as("cs-a", &dir)
        .args([
            "task",
            "create",
            "t1",
            "--title",
            "T1",
            "--no-edit",
            "--project",
            "test",
        ])
        .assert()
        .success();
    rdm_as("cs-a", &dir)
        .args(["commit", "-m", "land t1"])
        .assert()
        .success();

    // An idempotent update: re-setting the title to its own existing value
    // re-serializes to byte-identical content, journaling a write whose blob
    // already equals HEAD — but the working tree is already clean.
    rdm_as("cs-a", &dir)
        .args([
            "task",
            "update",
            "t1",
            "--title",
            "T1",
            "--no-edit",
            "--project",
            "test",
        ])
        .assert()
        .success();

    rdm_as("cs-a", &dir)
        .args(["commit", "--all", "-m", "noop --all"])
        .assert()
        .success();

    let out = rdm_as("cs-a", &dir)
        .args(["session", "list", "--format", "json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(
        String::from_utf8(out).unwrap().trim(),
        "[]",
        "a stale journal entry from a no-op write must not survive `commit --all` \
         on an already-clean tree"
    );
}

#[test]
fn commit_by_changeset_id_is_the_orphan_recovery_path() {
    let dir = TempDir::new().unwrap();
    seed_two_changesets(&dir);

    // A third session lands B's orphaned changeset by name.
    rdm_as("cs-c", &dir)
        .args(["commit", "--changeset", "cs-b", "-m", "recover b"])
        .assert()
        .success();

    let files = last_commit_files(dir.path());
    assert!(
        files.iter().any(|f| f == "projects/test/tasks/b-task.md"),
        "the named changeset did not land: {files:?}"
    );
    assert!(
        !files.iter().any(|f| f == "projects/test/tasks/a-task.md"),
        "committing one changeset by name swept another: {files:?}"
    );
}

#[test]
fn commit_reports_unattributed_dirt_instead_of_sweeping_or_going_quiet() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);
    // A raw write outside rdm: it can belong to no changeset.
    std::fs::write(dir.path().join("projects/test/stray.md"), "stray\n").unwrap();

    rdm_as("cs-empty", &dir)
        .args(["commit", "-m", "nothing of mine"])
        .assert()
        .success()
        .stdout(predicate::str::contains("stray.md"))
        .stdout(predicate::str::contains("rdm session list"))
        .stdout(predicate::str::contains("rdm commit --changeset"))
        .stdout(predicate::str::contains("rdm commit --all"));

    let files = last_commit_files(dir.path());
    assert!(
        !files.iter().any(|f| f == "projects/test/stray.md"),
        "an unattributed path was swept into a commit: {files:?}"
    );
}

/// A journaled path whose file has vanished must be reported even when the
/// commit lands nothing.
///
/// This is the branch that used to go quiet. Removing the only file a
/// changeset owns leaves nothing to skip *around*: the regenerated indexes
/// reconcile straight back to HEAD, the scoped tree equals HEAD, and the commit
/// correctly returns no SHA. Printing a bare `Nothing to commit.` there tells
/// the session its work was a no-op when in fact the work is gone — so the skip
/// note has to survive onto this branch too.
#[test]
fn commit_reports_a_vanished_journaled_path_even_when_it_lands_nothing() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);
    rdm_as("cs-vanish", &dir)
        .args([
            "task",
            "create",
            "vanishing",
            "--title",
            "V",
            "--no-edit",
            "--project",
            "test",
        ])
        .assert()
        .success();

    let path = dir.path().join("projects/test/tasks/vanishing.md");
    assert!(path.exists(), "precondition: the task file was created");
    std::fs::remove_file(&path).unwrap();

    let before = count_git_commits(dir.path());
    rdm_as("cs-vanish", &dir)
        .args(["commit", "-m", "land the vanished task"])
        .assert()
        .success()
        .stdout(predicate::str::contains("no longer on disk"))
        .stdout(predicate::str::contains("projects/test/tasks/vanishing.md"));

    assert_eq!(
        count_git_commits(dir.path()),
        before,
        "a changeset whose files all vanished must not create an empty commit"
    );
}

/// The same note must also ride along on the branch that *does* land a commit,
/// so a partially-vanished changeset is not reported as a clean win.
#[test]
fn commit_reports_a_vanished_journaled_path_alongside_the_paths_it_landed() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);
    for slug in ["survivor", "casualty"] {
        rdm_as("cs-partial", &dir)
            .args([
                "task",
                "create",
                slug,
                "--title",
                "T",
                "--no-edit",
                "--project",
                "test",
            ])
            .assert()
            .success();
    }
    std::fs::remove_file(dir.path().join("projects/test/tasks/casualty.md")).unwrap();

    rdm_as("cs-partial", &dir)
        .args(["commit", "-m", "land what survived"])
        .assert()
        .success()
        .stdout(predicate::str::contains("no longer on disk"))
        .stdout(predicate::str::contains("projects/test/tasks/casualty.md"));

    let files = last_commit_files(dir.path());
    assert!(
        files.iter().any(|f| f == "projects/test/tasks/survivor.md"),
        "the surviving path must still land: {files:?}"
    );
    assert!(
        !files.iter().any(|f| f == "projects/test/tasks/casualty.md"),
        "a vanished path must never be committed: {files:?}"
    );
}

#[test]
fn status_defaults_to_the_callers_changeset_and_all_shows_everything() {
    let dir = TempDir::new().unwrap();
    seed_two_changesets(&dir);

    rdm_as("cs-a", &dir)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("a-task.md"))
        .stdout(predicate::str::contains("b-task.md").not())
        .stdout(predicate::str::contains("belong to other changesets"));

    rdm_as("cs-a", &dir)
        .args(["status", "--all"])
        .assert()
        .success()
        .stdout(predicate::str::contains("a-task.md"))
        .stdout(predicate::str::contains("b-task.md"));
}

#[test]
fn discard_defaults_to_the_callers_changeset() {
    let dir = TempDir::new().unwrap();
    seed_two_changesets(&dir);

    rdm_as("cs-a", &dir)
        .args(["discard", "--force"])
        .assert()
        .success();

    assert!(
        !dir.path().join("projects/test/tasks/a-task.md").exists(),
        "the caller's own file survived its discard"
    );
    assert!(
        dir.path().join("projects/test/tasks/b-task.md").exists(),
        "a scoped discard destroyed another changeset's file"
    );
    let index = std::fs::read_to_string(dir.path().join("projects/test/INDEX.md")).unwrap();
    assert!(
        index.contains("b-task"),
        "the regenerated index dropped the other changeset's row: {index}"
    );
}

/// A discards only its own edits — never one it merely shares a path with.
/// A creates and commits a task, then edits it twice (uncommitted); B edits
/// the same never-committed path once more, after A's last write. A's
/// `discard --force` must exit 0, report the path as skipped, leave B's
/// content byte-identical on disk, and B's later `commit` must still land
/// with B's content.
#[test]
fn discard_leaves_a_path_another_changeset_overwrote_since() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    rdm_as("cs-a", &dir)
        .args([
            "task",
            "create",
            "shared",
            "--title",
            "Shared",
            "--no-edit",
            "--project",
            "test",
        ])
        .assert()
        .success();
    rdm_as("cs-a", &dir)
        .args(["commit", "-m", "seed shared task"])
        .assert()
        .success();

    rdm_as("cs-a", &dir)
        .args([
            "task",
            "update",
            "shared",
            "--body",
            "A's first edit",
            "--no-edit",
            "--project",
            "test",
        ])
        .assert()
        .success();
    rdm_as("cs-a", &dir)
        .args([
            "task",
            "update",
            "shared",
            "--body",
            "A's second edit",
            "--no-edit",
            "--project",
            "test",
        ])
        .assert()
        .success();

    rdm_as("cs-b", &dir)
        .args([
            "task",
            "update",
            "shared",
            "--body",
            "B's edit",
            "--no-edit",
            "--project",
            "test",
        ])
        .assert()
        .success();

    let path = dir.path().join("projects/test/tasks/shared.md");
    let before_discard = std::fs::read_to_string(&path).unwrap();
    assert!(
        before_discard.contains("B's edit"),
        "fixture: B's overwrite did not land on disk: {before_discard}"
    );

    rdm_as("cs-a", &dir)
        .args(["discard", "--force"])
        .assert()
        .success()
        .stdout(predicate::str::contains("skipped"))
        .stdout(predicate::str::contains("shared.md"));

    let after_discard = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        after_discard, before_discard,
        "A's discard did not leave B's content byte-identical on disk"
    );

    rdm_as("cs-b", &dir)
        .args(["commit", "-m", "land B's edit"])
        .assert()
        .success();
    let after_commit = std::fs::read_to_string(&path).unwrap();
    assert!(
        after_commit.contains("B's edit"),
        "B's commit did not land B's content: {after_commit}"
    );
}

/// A creates+commits a roadmap, then deletes it (uncommitted); B recreates a
/// roadmap at the same slug/path. A's `discard --force` must leave B's
/// roadmap.md content on disk — not reverted to A's original HEAD content —
/// and report it skipped.
#[test]
fn discard_leaves_a_path_another_changeset_recreated_after_a_delete() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    rdm_as("cs-a", &dir)
        .args([
            "roadmap",
            "create",
            "shared-map",
            "--title",
            "A's roadmap",
            "--no-edit",
            "--project",
            "test",
        ])
        .assert()
        .success();
    rdm_as("cs-a", &dir)
        .args(["commit", "-m", "seed shared-map roadmap"])
        .assert()
        .success();

    rdm_as("cs-a", &dir)
        .args([
            "roadmap",
            "delete",
            "shared-map",
            "--force",
            "--project",
            "test",
        ])
        .assert()
        .success();

    rdm_as("cs-b", &dir)
        .args([
            "roadmap",
            "create",
            "shared-map",
            "--title",
            "B's roadmap",
            "--no-edit",
            "--project",
            "test",
        ])
        .assert()
        .success();

    let path = dir
        .path()
        .join("projects/test/roadmaps/shared-map/roadmap.md");
    let before_discard = std::fs::read_to_string(&path).unwrap();
    assert!(
        before_discard.contains("B's roadmap"),
        "fixture: B's recreate did not land on disk: {before_discard}"
    );

    rdm_as("cs-a", &dir)
        .args(["discard", "--force"])
        .assert()
        .success()
        .stdout(predicate::str::contains("skipped"))
        .stdout(predicate::str::contains("shared-map"));

    assert!(
        path.exists(),
        "A's discard destroyed B's recreated roadmap.md"
    );
    let after_discard = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        after_discard, before_discard,
        "A's discard did not leave B's recreated content byte-identical on disk"
    );
}

#[test]
fn discard_all_is_the_whole_tree_opt_in_and_still_needs_force() {
    let dir = TempDir::new().unwrap();
    seed_two_changesets(&dir);

    rdm_as("cs-a", &dir)
        .args(["discard", "--all"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--force"));

    rdm_as("cs-a", &dir)
        .args(["discard", "--force", "--all"])
        .assert()
        .success()
        // Names what it is about to destroy, before destroying it.
        .stderr(predicate::str::contains("cs-b"));

    assert!(
        !dir.path().join("projects/test/tasks/b-task.md").exists(),
        "--all did not destroy the other changeset's work"
    );
}

#[test]
fn init_remote_still_lands_its_config_commit() {
    let src = TempDir::new().unwrap();
    init_repo(&src);
    let bare = TempDir::new().unwrap();
    let bare_path = bare.path().join("plan.git");
    let out = std::process::Command::new("git")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .args(["clone", "--quiet", "--bare"])
        .arg(src.path())
        .arg(&bare_path)
        .output()
        .unwrap();
    assert!(out.status.success(), "bare clone failed: {out:?}");

    let clone = TempDir::new().unwrap();
    let target = clone.path().join("plan");
    rdm()
        .arg("--root")
        .arg(&target)
        .args([
            "init",
            "--remote",
            &format!("file://{}", bare_path.display()),
        ])
        .assert()
        .success();

    let files = last_commit_files(&target);
    assert!(
        files.iter().any(|f| f == "rdm.toml"),
        "rdm init --remote did not land its rdm.toml commit: {files:?}"
    );
}

#[test]
fn a_backfilled_gitattributes_reaches_a_scoped_commit() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    // Rewrite HEAD so it predates the merge mapping. Raw git deliberately:
    // every rdm command re-ensures the mapping on open.
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            // Clear the whole inherited git env, not just GIT_DIR: this suite
            // runs from inside the pre-commit hook, where GIT_INDEX_FILE also
            // points at the outer repo.
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .args(args)
            .current_dir(dir.path())
            .output()
            .unwrap()
    };
    assert!(
        git(&["rm", "--cached", "--quiet", ".gitattributes"])
            .status
            .success()
    );
    assert!(
        git(&["commit", "--quiet", "-m", "pre-driver"])
            .status
            .success()
    );
    std::fs::remove_file(dir.path().join(".gitattributes")).unwrap();

    rdm_as("cs-attrs", &dir)
        .args([
            "task",
            "create",
            "x",
            "--title",
            "X",
            "--no-edit",
            "--project",
            "test",
        ])
        .assert()
        .success();
    rdm_as("cs-attrs", &dir)
        .args(["commit", "-m", "land x"])
        .assert()
        .success();

    let files = last_commit_files(dir.path());
    assert!(
        files.iter().any(|f| f == ".gitattributes"),
        "the backfilled mapping never reached a commit: {files:?}"
    );
}

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

    // Status should report uncommitted changes including both entity files.
    // INDEX regeneration also stages top-level INDEX.md and
    // projects/test/INDEX.md, so don't assert an exact count of 2.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("Uncommitted changes"))
        .stdout(predicate::str::contains("roadmap.md"))
        .stdout(predicate::str::contains("phase-1-e2e-phase.md"));

    // Commit lands exactly one new commit.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["commit", "-m", "feat: add e2e roadmap and phase"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Committed"));

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
}

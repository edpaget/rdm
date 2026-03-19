use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn rdm() -> Command {
    Command::cargo_bin("rdm").unwrap()
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
}

/// Count git commits using gitoxide-compatible approach.
fn count_git_commits(dir: &std::path::Path) -> usize {
    let repo = gix::open(dir).unwrap();
    let mut count = 0;
    if let Ok(mut head) = repo.head() {
        if let Ok(commit) = head.peel_to_commit() {
            count = 1;
            let mut ancestors = commit.ancestors().all().unwrap();
            while ancestors.next().is_some() {
                count += 1;
            }
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

#[test]
fn stage_flag_writes_to_disk_no_git_commit() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    let commits_before = count_git_commits(dir.path());

    // Create a roadmap with --stage
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("--stage")
        .arg("roadmap")
        .arg("create")
        .arg("staging-test")
        .arg("--title")
        .arg("Staging Test")
        .arg("--no-edit")
        .arg("--project")
        .arg("test")
        .assert()
        .success()
        .stdout(predicate::str::contains("staged"));

    // File should exist on disk
    assert!(
        dir.path()
            .join("projects/test/roadmaps/staging-test/roadmap.md")
            .exists()
    );

    // But no new git commit
    let commits_after = count_git_commits(dir.path());
    assert_eq!(commits_before, commits_after);
}

#[test]
fn status_shows_staged_changes() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    // Create a roadmap with --stage
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("--stage")
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

    // Run status
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

    // Stage a roadmap
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("--stage")
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

    // Commit
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("commit")
        .arg("-m")
        .arg("test commit message")
        .assert()
        .success()
        .stdout(predicate::str::contains("Committed"));

    // Verify the latest commit has our message
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

    // Stage a roadmap
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("--stage")
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

#[test]
fn env_var_enables_staging() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    let commits_before = count_git_commits(dir.path());

    // Create with RDM_STAGE=true instead of --stage flag
    rdm()
        .arg("--root")
        .arg(dir.path())
        .env("RDM_STAGE", "true")
        .arg("roadmap")
        .arg("create")
        .arg("env-test")
        .arg("--title")
        .arg("Env Test")
        .arg("--no-edit")
        .arg("--project")
        .arg("test")
        .assert()
        .success()
        .stdout(predicate::str::contains("staged"));

    // No new git commit
    let commits_after = count_git_commits(dir.path());
    assert_eq!(commits_before, commits_after);
}

#[test]
fn config_enables_staging() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    // Write stage = true to rdm.toml
    let config_path = dir.path().join("rdm.toml");
    std::fs::write(&config_path, "stage = true\n").unwrap();

    let commits_before = count_git_commits(dir.path());

    // Create without --stage flag (config should enable it)
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("roadmap")
        .arg("create")
        .arg("config-test")
        .arg("--title")
        .arg("Config Test")
        .arg("--no-edit")
        .arg("--project")
        .arg("test")
        .assert()
        .success()
        .stdout(predicate::str::contains("staged"));

    // No new git commit
    let commits_after = count_git_commits(dir.path());
    assert_eq!(commits_before, commits_after);
}

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn rdm() -> Command {
    Command::cargo_bin("rdm").unwrap()
}

/// Initialize a plan repo with an initial git commit.
fn init_repo(dir: &TempDir) {
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("init")
        .assert()
        .success();
}

/// Runs a git command with GIT_DIR/GIT_WORK_TREE/GIT_INDEX_FILE cleared
/// to avoid inheriting env vars from parent git hooks.
fn git_cmd() -> std::process::Command {
    let mut cmd = std::process::Command::new("git");
    cmd.env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE");
    cmd
}

/// Creates a bare clone of the repo and adds it as a remote.
fn setup_bare_remote(dir: &TempDir, remote_name: &str) -> TempDir {
    let bare_dir = TempDir::new().unwrap();
    git_cmd()
        .args(["clone", "--bare"])
        .arg(dir.path())
        .arg(bare_dir.path())
        .output()
        .unwrap();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("remote")
        .arg("add")
        .arg(remote_name)
        .arg(bare_dir.path().to_str().unwrap())
        .assert()
        .success();
    bare_dir
}

/// Sets the default remote in rdm.toml.
fn set_default_remote(dir: &TempDir, remote_name: &str) {
    let config_path = dir.path().join("rdm.toml");
    let mut content = std::fs::read_to_string(&config_path).unwrap_or_default();
    content.push_str(&format!("\n[remote]\ndefault = \"{remote_name}\"\n"));
    std::fs::write(&config_path, content).unwrap();
}

#[test]
fn remote_list_empty() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("remote")
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains("No remotes configured."));
}

#[test]
fn remote_add_and_list() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("remote")
        .arg("add")
        .arg("origin")
        .arg("https://example.com/repo.git")
        .assert()
        .success()
        .stdout(predicate::str::contains("Added remote 'origin'"))
        .stdout(predicate::str::contains("https://example.com/repo.git"));

    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("remote")
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains("origin"))
        .stdout(predicate::str::contains("https://example.com/repo.git"));
}

#[test]
fn remote_add_duplicate_fails() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("remote")
        .arg("add")
        .arg("origin")
        .arg("https://example.com/repo.git")
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("remote")
        .arg("add")
        .arg("origin")
        .arg("https://other.com/repo.git")
        .assert()
        .failure()
        .stderr(predicate::str::contains("already exists"));
}

#[test]
fn remote_remove_and_list() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("remote")
        .arg("add")
        .arg("origin")
        .arg("https://example.com/repo.git")
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("remote")
        .arg("remove")
        .arg("origin")
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed remote 'origin'"));

    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("remote")
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains("No remotes configured."));
}

#[test]
fn remote_remove_nonexistent_fails() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("remote")
        .arg("remove")
        .arg("nope")
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));
}

#[test]
fn remote_fetch_success() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    let bare_dir = setup_bare_remote(&dir, "origin");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("remote")
        .arg("fetch")
        .arg("origin")
        .assert()
        .success()
        .stdout(predicate::str::contains("Fetched from 'origin'"));

    let _ = bare_dir; // keep alive
}

#[test]
fn remote_fetch_unknown_remote() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("remote")
        .arg("fetch")
        .arg("nonexistent")
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));
}

#[test]
fn status_no_remote_no_sync_info() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("No uncommitted changes"))
        // Should NOT contain any sync info
        .stdout(predicate::str::contains("Up to date").not())
        .stdout(predicate::str::contains("ahead").not())
        .stdout(predicate::str::contains("behind").not());
}

#[test]
fn status_shows_sync_info() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    // Set default remote in rdm.toml before cloning to bare
    // so the bare has it and local matches after fetch.
    set_default_remote(&dir, "origin");
    // Commit the rdm.toml change so it's part of HEAD
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("commit")
        .arg("-m")
        .arg("set default remote")
        .assert()
        .success()
        .stdout(predicate::str::contains("Committed"));

    // Now clone to bare and add as remote — bare has same commits
    let bare_dir = setup_bare_remote(&dir, "origin");

    // Fetch to populate tracking refs
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("remote")
        .arg("fetch")
        .arg("origin")
        .assert()
        .success();

    // Local and remote should be in sync — verify "Up to date" appears
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("Up to date"));

    let _ = bare_dir;
}

#[test]
fn remote_push_success() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    let bare_dir = setup_bare_remote(&dir, "origin");

    // Create a task to generate a local commit
    // Create a local commit by writing a file and committing via git
    std::fs::write(dir.path().join("local-change.md"), "content").unwrap();
    git_cmd()
        .args(["add", "."])
        .current_dir(dir.path())
        .output()
        .unwrap();
    git_cmd()
        .args(["commit", "-m", "local change"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("remote")
        .arg("push")
        .arg("origin")
        .assert()
        .success()
        .stdout(predicate::str::contains("Pushed"))
        .stdout(predicate::str::contains("origin"));

    let _ = bare_dir;
}

#[test]
fn remote_push_rejected() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    let bare_dir = setup_bare_remote(&dir, "origin");

    // Fetch to establish tracking refs
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("remote")
        .arg("fetch")
        .arg("origin")
        .assert()
        .success();

    // Push a commit to bare from a separate clone
    let clone_dir = tempfile::TempDir::new().unwrap();
    git_cmd()
        .args(["clone"])
        .arg(bare_dir.path())
        .arg(clone_dir.path())
        .output()
        .unwrap();
    std::fs::write(clone_dir.path().join("remote.md"), "remote").unwrap();
    git_cmd()
        .args(["add", "."])
        .current_dir(clone_dir.path())
        .output()
        .unwrap();
    git_cmd()
        .args(["commit", "-m", "remote commit"])
        .current_dir(clone_dir.path())
        .output()
        .unwrap();
    git_cmd()
        .args(["push"])
        .current_dir(clone_dir.path())
        .output()
        .unwrap();

    // Make a local commit
    // Create a local commit by writing a file and committing via git
    std::fs::write(dir.path().join("local-change.md"), "content").unwrap();
    git_cmd()
        .args(["add", "."])
        .current_dir(dir.path())
        .output()
        .unwrap();
    git_cmd()
        .args(["commit", "-m", "local change"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    // Push should be rejected
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("remote")
        .arg("push")
        .arg("origin")
        .assert()
        .failure()
        .stderr(predicate::str::contains("push rejected").or(predicate::str::contains("rejected")));

    let _ = bare_dir;
}

#[test]
fn remote_pull_success() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    let bare_dir = setup_bare_remote(&dir, "origin");

    // Push a commit to bare from a separate clone
    let clone_dir = tempfile::TempDir::new().unwrap();
    git_cmd()
        .args(["clone"])
        .arg(bare_dir.path())
        .arg(clone_dir.path())
        .output()
        .unwrap();
    std::fs::write(clone_dir.path().join("new-file.md"), "content").unwrap();
    git_cmd()
        .args(["add", "."])
        .current_dir(clone_dir.path())
        .output()
        .unwrap();
    git_cmd()
        .args(["commit", "-m", "add new file"])
        .current_dir(clone_dir.path())
        .output()
        .unwrap();
    git_cmd()
        .args(["push"])
        .current_dir(clone_dir.path())
        .output()
        .unwrap();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("remote")
        .arg("pull")
        .arg("origin")
        .assert()
        .success()
        .stdout(predicate::str::contains("Pulled"))
        .stdout(predicate::str::contains("origin"));

    // File should now exist locally
    assert!(dir.path().join("new-file.md").exists());

    let _ = bare_dir;
}

#[test]
fn remote_pull_diverged() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    let bare_dir = setup_bare_remote(&dir, "origin");

    // Fetch to establish tracking refs
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("remote")
        .arg("fetch")
        .arg("origin")
        .assert()
        .success();

    // Push a commit to bare from a separate clone
    let clone_dir = tempfile::TempDir::new().unwrap();
    git_cmd()
        .args(["clone"])
        .arg(bare_dir.path())
        .arg(clone_dir.path())
        .output()
        .unwrap();
    std::fs::write(clone_dir.path().join("remote.md"), "remote").unwrap();
    git_cmd()
        .args(["add", "."])
        .current_dir(clone_dir.path())
        .output()
        .unwrap();
    git_cmd()
        .args(["commit", "-m", "remote commit"])
        .current_dir(clone_dir.path())
        .output()
        .unwrap();
    git_cmd()
        .args(["push"])
        .current_dir(clone_dir.path())
        .output()
        .unwrap();

    // Make a local commit
    // Create a local commit by writing a file and committing via git
    std::fs::write(dir.path().join("local-change.md"), "content").unwrap();
    git_cmd()
        .args(["add", "."])
        .current_dir(dir.path())
        .output()
        .unwrap();
    git_cmd()
        .args(["commit", "-m", "local change"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    // Pull should fail with diverged
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("remote")
        .arg("pull")
        .arg("origin")
        .assert()
        .failure()
        .stderr(predicate::str::contains("diverged"));

    let _ = bare_dir;
}

#[test]
fn remote_pull_regenerates_index() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    let bare_dir = setup_bare_remote(&dir, "origin");

    // Push a new file from a separate clone
    let clone_dir = tempfile::TempDir::new().unwrap();
    git_cmd()
        .args(["clone"])
        .arg(bare_dir.path())
        .arg(clone_dir.path())
        .output()
        .unwrap();
    std::fs::write(clone_dir.path().join("extra.md"), "extra content").unwrap();
    git_cmd()
        .args(["add", "."])
        .current_dir(clone_dir.path())
        .output()
        .unwrap();
    git_cmd()
        .args(["commit", "-m", "add extra file"])
        .current_dir(clone_dir.path())
        .output()
        .unwrap();
    git_cmd()
        .args(["push"])
        .current_dir(clone_dir.path())
        .output()
        .unwrap();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("remote")
        .arg("pull")
        .arg("origin")
        .assert()
        .success()
        .stdout(predicate::str::contains("Pulled"));

    // The pulled file should exist
    assert!(
        dir.path().join("extra.md").exists(),
        "extra.md should exist after pull"
    );

    // INDEX.md should exist (regenerated after pull)
    let index_path = dir.path().join("INDEX.md");
    assert!(index_path.exists(), "INDEX.md should exist after pull");

    let _ = bare_dir;
}

#[test]
fn status_with_fetch_flag() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    // Set default remote before cloning
    set_default_remote(&dir, "origin");
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("commit")
        .arg("-m")
        .arg("set default remote")
        .assert()
        .success()
        .stdout(predicate::str::contains("Committed"));

    let bare_dir = setup_bare_remote(&dir, "origin");

    // status --fetch should fetch and show sync info
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("status")
        .arg("--fetch")
        .assert()
        .success()
        .stdout(predicate::str::contains("Up to date"));

    let _ = bare_dir;
}

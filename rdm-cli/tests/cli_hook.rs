use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

fn rdm() -> Command {
    Command::cargo_bin("rdm").unwrap()
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

/// Initialize a plan repo (also creates a git repo via `rdm init`).
fn init_repo(dir: &TempDir) {
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("init")
        .assert()
        .success();
}

/// Set up a plan repo with a project, roadmap, and phase for hook testing.
fn init_with_phase(dir: &TempDir) {
    init_repo(dir);
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "create", "test-proj"])
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "roadmap",
            "create",
            "my-roadmap",
            "--title",
            "My Roadmap",
            "--project",
            "test-proj",
        ])
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "create",
            "my-phase",
            "--title",
            "My Phase",
            "--roadmap",
            "my-roadmap",
            "--project",
            "test-proj",
        ])
        .assert()
        .success();
}

// -- hook install tests --

#[test]
fn hook_install_creates_post_merge() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["hook", "install"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Installed post-merge hook"));

    let hook_path = dir.path().join(".git/hooks/post-merge");
    assert!(hook_path.exists());
    let contents = fs::read_to_string(&hook_path).unwrap();
    assert!(contents.contains("rdm hook post-merge"));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&hook_path).unwrap().permissions().mode();
        assert!(mode & 0o111 != 0, "hook should be executable");
    }
}

#[test]
fn hook_install_fails_if_exists() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    // First install succeeds.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["hook", "install"])
        .assert()
        .success();

    // Second install without --force fails.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["hook", "install"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("already exists"));
}

#[test]
fn hook_install_force_overwrites() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["hook", "install"])
        .assert()
        .success();

    // Force install should succeed.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["hook", "install", "--force"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Installed post-merge hook"));
}

// -- hook uninstall tests --

#[test]
fn hook_uninstall_removes_hook() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["hook", "install"])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["hook", "uninstall"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed post-merge hook"));

    let hook_path = dir.path().join(".git/hooks/post-merge");
    assert!(!hook_path.exists());
}

#[test]
fn hook_uninstall_refuses_foreign_hook() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    // Write a foreign hook that doesn't contain "rdm hook post-merge".
    let hooks_dir = dir.path().join(".git/hooks");
    fs::create_dir_all(&hooks_dir).unwrap();
    fs::write(hooks_dir.join("post-merge"), "#!/bin/sh\necho custom\n").unwrap();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["hook", "uninstall"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not installed by rdm"));
}

// -- hook post-merge tests --

#[test]
fn hook_post_merge_marks_phase_done() {
    let dir = TempDir::new().unwrap();
    init_with_phase(&dir);

    // Create a git commit with a Done: directive via raw git.
    let dummy_path = dir.path().join("dummy.txt");
    fs::write(&dummy_path, "trigger commit").unwrap();
    git_cmd()
        .args(["add", "dummy.txt"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    git_cmd()
        .args([
            "commit",
            "-m",
            "feat: merge stuff\n\nDone: my-roadmap/phase-1-my-phase",
        ])
        .current_dir(dir.path())
        .output()
        .unwrap();

    // Get the commit SHA for verification.
    let sha_output = git_cmd()
        .args(["log", "-1", "--format=%H"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    let sha = String::from_utf8_lossy(&sha_output.stdout)
        .trim()
        .to_string();

    // Run the hook.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .env("RDM_PROJECT", "test-proj")
        .args(["hook", "post-merge"])
        .assert()
        .success();

    // Verify the phase is now done.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "show",
            "phase-1-my-phase",
            "--roadmap",
            "my-roadmap",
            "--project",
            "test-proj",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Status: done"))
        .stdout(predicate::str::contains(&sha));
}

#[test]
fn hook_post_merge_silent_on_no_directives() {
    let dir = TempDir::new().unwrap();
    init_with_phase(&dir);

    // Normal commit without Done: directives.
    let dummy_path = dir.path().join("dummy.txt");
    fs::write(&dummy_path, "no directives here").unwrap();
    git_cmd()
        .args(["add", "dummy.txt"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    git_cmd()
        .args(["commit", "-m", "chore: just a regular commit"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .env("RDM_PROJECT", "test-proj")
        .args(["hook", "post-merge"])
        .assert()
        .success();
}

#[test]
fn hook_post_merge_silent_on_missing_phase() {
    let dir = TempDir::new().unwrap();
    init_with_phase(&dir);

    // Commit with Done: referencing a nonexistent roadmap/phase.
    let dummy_path = dir.path().join("dummy.txt");
    fs::write(&dummy_path, "bad directive").unwrap();
    git_cmd()
        .args(["add", "dummy.txt"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    git_cmd()
        .args([
            "commit",
            "-m",
            "feat: merge\n\nDone: nonexistent-roadmap/nonexistent-phase",
        ])
        .current_dir(dir.path())
        .output()
        .unwrap();

    // Should exit 0 even though the phase doesn't exist.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .env("RDM_PROJECT", "test-proj")
        .args(["hook", "post-merge"])
        .assert()
        .success();
}

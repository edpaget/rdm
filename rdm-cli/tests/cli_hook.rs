use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

fn rdm() -> Command {
    let mut cmd = Command::cargo_bin("rdm").unwrap();
    // Isolate from host global config (e.g. default_format = "json").
    cmd.env("XDG_CONFIG_HOME", "/dev/null/nonexistent");
    cmd
}

/// Runs a git command with GIT_DIR/GIT_WORK_TREE/GIT_INDEX_FILE cleared
/// to avoid inheriting env vars from parent git hooks. Sets author/committer
/// identity so commits work on CI without global gitconfig.
fn git_cmd() -> std::process::Command {
    let mut cmd = std::process::Command::new("git");
    cmd.env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@test.com")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@test.com");
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

/// Create a separate git repo to act as the project (code) repo.
fn init_project_repo(dir: &TempDir) {
    let out = git_cmd()
        .args(["init"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "git init failed");
    // Need an initial commit so HEAD exists.
    fs::write(dir.path().join("README.md"), "# project").unwrap();
    let out = git_cmd()
        .args(["add", "."])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "git add failed");
    let out = git_cmd()
        .args(["commit", "-m", "initial commit"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git commit failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
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
    let project_dir = TempDir::new().unwrap();
    init_project_repo(&project_dir);

    rdm()
        .args(["hook", "install"])
        .current_dir(project_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Installed post-merge hook"));

    let hook_path = project_dir.path().join(".git/hooks/post-merge");
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
    let project_dir = TempDir::new().unwrap();
    init_project_repo(&project_dir);

    // First install succeeds.
    rdm()
        .args(["hook", "install"])
        .current_dir(project_dir.path())
        .assert()
        .success();

    // Second install without --force fails.
    rdm()
        .args(["hook", "install"])
        .current_dir(project_dir.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("already exists"));
}

#[test]
fn hook_install_force_overwrites() {
    let project_dir = TempDir::new().unwrap();
    init_project_repo(&project_dir);

    rdm()
        .args(["hook", "install"])
        .current_dir(project_dir.path())
        .assert()
        .success();

    // Force install should succeed.
    rdm()
        .args(["hook", "install", "--force"])
        .current_dir(project_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Installed post-merge hook"));
}

// -- hook uninstall tests --

#[test]
fn hook_uninstall_removes_hook() {
    let project_dir = TempDir::new().unwrap();
    init_project_repo(&project_dir);

    rdm()
        .args(["hook", "install"])
        .current_dir(project_dir.path())
        .assert()
        .success();

    rdm()
        .args(["hook", "uninstall"])
        .current_dir(project_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed post-merge hook"));

    let hook_path = project_dir.path().join(".git/hooks/post-merge");
    assert!(!hook_path.exists());
}

#[test]
fn hook_uninstall_refuses_foreign_hook() {
    let project_dir = TempDir::new().unwrap();
    init_project_repo(&project_dir);

    // Write a foreign hook that doesn't contain "rdm hook post-merge".
    let hooks_dir = project_dir.path().join(".git/hooks");
    fs::create_dir_all(&hooks_dir).unwrap();
    fs::write(hooks_dir.join("post-merge"), "#!/bin/sh\necho custom\n").unwrap();

    rdm()
        .args(["hook", "uninstall"])
        .current_dir(project_dir.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("not installed by rdm"));
}

// -- hook post-merge tests --

#[test]
fn hook_post_merge_marks_phase_done() {
    let plan_dir = TempDir::new().unwrap();
    let project_dir = TempDir::new().unwrap();
    init_with_phase(&plan_dir);
    init_project_repo(&project_dir);

    // Create a git commit in the project repo with a Done: directive.
    let dummy_path = project_dir.path().join("dummy.txt");
    fs::write(&dummy_path, "trigger commit").unwrap();
    git_cmd()
        .args(["add", "dummy.txt"])
        .current_dir(project_dir.path())
        .output()
        .unwrap();
    git_cmd()
        .args([
            "commit",
            "-m",
            "feat: merge stuff\n\nDone: my-roadmap/phase-1-my-phase",
        ])
        .current_dir(project_dir.path())
        .output()
        .unwrap();

    // Get the commit SHA for verification.
    let sha_output = git_cmd()
        .args(["log", "-1", "--format=%H"])
        .current_dir(project_dir.path())
        .output()
        .unwrap();
    let sha = String::from_utf8_lossy(&sha_output.stdout)
        .trim()
        .to_string();

    // Run the hook from the project dir, pointing --root at the plan repo.
    rdm()
        .arg("--root")
        .arg(plan_dir.path())
        .env("RDM_PROJECT", "test-proj")
        .args(["hook", "post-merge"])
        .current_dir(project_dir.path())
        .assert()
        .success();

    // Verify the phase is now done in the plan repo.
    rdm()
        .arg("--root")
        .arg(plan_dir.path())
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
    let plan_dir = TempDir::new().unwrap();
    let project_dir = TempDir::new().unwrap();
    init_with_phase(&plan_dir);
    init_project_repo(&project_dir);

    // Normal commit without Done: directives.
    let dummy_path = project_dir.path().join("dummy.txt");
    fs::write(&dummy_path, "no directives here").unwrap();
    git_cmd()
        .args(["add", "dummy.txt"])
        .current_dir(project_dir.path())
        .output()
        .unwrap();
    git_cmd()
        .args(["commit", "-m", "chore: just a regular commit"])
        .current_dir(project_dir.path())
        .output()
        .unwrap();

    rdm()
        .arg("--root")
        .arg(plan_dir.path())
        .env("RDM_PROJECT", "test-proj")
        .args(["hook", "post-merge"])
        .current_dir(project_dir.path())
        .assert()
        .success();
}

#[test]
fn hook_post_merge_silent_on_missing_phase() {
    let plan_dir = TempDir::new().unwrap();
    let project_dir = TempDir::new().unwrap();
    init_with_phase(&plan_dir);
    init_project_repo(&project_dir);

    // Commit with Done: referencing a nonexistent roadmap/phase.
    let dummy_path = project_dir.path().join("dummy.txt");
    fs::write(&dummy_path, "bad directive").unwrap();
    git_cmd()
        .args(["add", "dummy.txt"])
        .current_dir(project_dir.path())
        .output()
        .unwrap();
    git_cmd()
        .args([
            "commit",
            "-m",
            "feat: merge\n\nDone: nonexistent-roadmap/nonexistent-phase",
        ])
        .current_dir(project_dir.path())
        .output()
        .unwrap();

    // Should exit 0 even though the phase doesn't exist.
    rdm()
        .arg("--root")
        .arg(plan_dir.path())
        .env("RDM_PROJECT", "test-proj")
        .args(["hook", "post-merge"])
        .current_dir(project_dir.path())
        .assert()
        .success();
}

// -- hook post-merge multi-commit scanning tests --

/// Helper: create a project with a roadmap and multiple phases.
fn init_with_phases(dir: &TempDir, phases: &[&str]) {
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
    for phase in phases {
        rdm()
            .arg("--root")
            .arg(dir.path())
            .args([
                "phase",
                "create",
                phase,
                "--title",
                phase,
                "--roadmap",
                "my-roadmap",
                "--project",
                "test-proj",
            ])
            .assert()
            .success();
    }
}

#[test]
fn hook_post_merge_scans_multiple_commits() {
    let plan_dir = TempDir::new().unwrap();
    let project_dir = TempDir::new().unwrap();
    init_with_phases(&plan_dir, &["alpha", "beta", "gamma"]);
    init_project_repo(&project_dir);

    // Tag the current HEAD as our anchor before adding Done: commits.
    git_cmd()
        .args(["tag", "before-merge"])
        .current_dir(project_dir.path())
        .output()
        .unwrap();

    // Create three separate commits, each with a Done: directive.
    for (i, phase) in ["phase-1-alpha", "phase-2-beta", "phase-3-gamma"]
        .iter()
        .enumerate()
    {
        let filename = format!("file{i}.txt");
        fs::write(project_dir.path().join(&filename), format!("content {i}")).unwrap();
        git_cmd()
            .args(["add", &filename])
            .current_dir(project_dir.path())
            .output()
            .unwrap();
        git_cmd()
            .args([
                "commit",
                "-m",
                &format!("feat: implement {phase}\n\nDone: my-roadmap/{phase}"),
            ])
            .current_dir(project_dir.path())
            .output()
            .unwrap();
    }

    // Run hook with --since to scan all commits since the anchor.
    rdm()
        .arg("--root")
        .arg(plan_dir.path())
        .env("RDM_PROJECT", "test-proj")
        .args(["hook", "post-merge", "--since", "before-merge"])
        .current_dir(project_dir.path())
        .assert()
        .success();

    // Verify all three phases are now done.
    for phase in ["phase-1-alpha", "phase-2-beta", "phase-3-gamma"] {
        rdm()
            .arg("--root")
            .arg(plan_dir.path())
            .args([
                "phase",
                "show",
                phase,
                "--roadmap",
                "my-roadmap",
                "--project",
                "test-proj",
            ])
            .assert()
            .success()
            .stdout(predicate::str::contains("Status: done"));
    }
}

#[test]
fn hook_post_merge_since_flag_limits_range() {
    let plan_dir = TempDir::new().unwrap();
    let project_dir = TempDir::new().unwrap();
    init_with_phases(&plan_dir, &["alpha", "beta"]);
    init_project_repo(&project_dir);

    // Create a commit with Done: for alpha.
    fs::write(project_dir.path().join("file1.txt"), "content 1").unwrap();
    git_cmd()
        .args(["add", "file1.txt"])
        .current_dir(project_dir.path())
        .output()
        .unwrap();
    git_cmd()
        .args([
            "commit",
            "-m",
            "feat: alpha\n\nDone: my-roadmap/phase-1-alpha",
        ])
        .current_dir(project_dir.path())
        .output()
        .unwrap();

    // Tag after alpha — only commits after this should be scanned.
    git_cmd()
        .args(["tag", "after-alpha"])
        .current_dir(project_dir.path())
        .output()
        .unwrap();

    // Create a commit with Done: for beta.
    fs::write(project_dir.path().join("file2.txt"), "content 2").unwrap();
    git_cmd()
        .args(["add", "file2.txt"])
        .current_dir(project_dir.path())
        .output()
        .unwrap();
    git_cmd()
        .args([
            "commit",
            "-m",
            "feat: beta\n\nDone: my-roadmap/phase-2-beta",
        ])
        .current_dir(project_dir.path())
        .output()
        .unwrap();

    // Run hook with --since after-alpha: should only pick up beta.
    rdm()
        .arg("--root")
        .arg(plan_dir.path())
        .env("RDM_PROJECT", "test-proj")
        .args(["hook", "post-merge", "--since", "after-alpha"])
        .current_dir(project_dir.path())
        .assert()
        .success();

    // Alpha should still be not-started.
    rdm()
        .arg("--root")
        .arg(plan_dir.path())
        .args([
            "phase",
            "show",
            "phase-1-alpha",
            "--roadmap",
            "my-roadmap",
            "--project",
            "test-proj",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Status: not-started"));

    // Beta should be done.
    rdm()
        .arg("--root")
        .arg(plan_dir.path())
        .args([
            "phase",
            "show",
            "phase-2-beta",
            "--roadmap",
            "my-roadmap",
            "--project",
            "test-proj",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Status: done"));
}

#[test]
fn hook_post_merge_deduplicates_same_phase_across_commits() {
    let plan_dir = TempDir::new().unwrap();
    let project_dir = TempDir::new().unwrap();
    init_with_phases(&plan_dir, &["alpha"]);
    init_project_repo(&project_dir);

    git_cmd()
        .args(["tag", "anchor"])
        .current_dir(project_dir.path())
        .output()
        .unwrap();

    // Two commits both reference the same phase.
    for i in 0..2 {
        let filename = format!("dup{i}.txt");
        fs::write(project_dir.path().join(&filename), format!("dup {i}")).unwrap();
        git_cmd()
            .args(["add", &filename])
            .current_dir(project_dir.path())
            .output()
            .unwrap();
        git_cmd()
            .args([
                "commit",
                "-m",
                "feat: work\n\nDone: my-roadmap/phase-1-alpha",
            ])
            .current_dir(project_dir.path())
            .output()
            .unwrap();
    }

    // Get the SHA of the latest commit (should be used for the phase).
    let sha_output = git_cmd()
        .args(["log", "-1", "--format=%H"])
        .current_dir(project_dir.path())
        .output()
        .unwrap();
    let latest_sha = String::from_utf8_lossy(&sha_output.stdout)
        .trim()
        .to_string();

    rdm()
        .arg("--root")
        .arg(plan_dir.path())
        .env("RDM_PROJECT", "test-proj")
        .args(["hook", "post-merge", "--since", "anchor"])
        .current_dir(project_dir.path())
        .assert()
        .success();

    // Phase should be done with the latest commit's SHA.
    rdm()
        .arg("--root")
        .arg(plan_dir.path())
        .args([
            "phase",
            "show",
            "phase-1-alpha",
            "--roadmap",
            "my-roadmap",
            "--project",
            "test-proj",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Status: done"))
        .stdout(predicate::str::contains(&latest_sha));
}

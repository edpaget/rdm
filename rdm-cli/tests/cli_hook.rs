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

/// Returns the number of commits reachable from HEAD in the plan repo.
fn plan_repo_commit_count(plan_dir: &TempDir) -> u64 {
    let out = git_cmd()
        .args(["rev-list", "--count", "HEAD"])
        .current_dir(plan_dir.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git rev-list --count HEAD failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().parse().unwrap()
}

/// Returns the full commit message of the plan repo's current HEAD.
fn plan_repo_head_message(plan_dir: &TempDir) -> String {
    let out = git_cmd()
        .args(["log", "-1", "--format=%B"])
        .current_dir(plan_dir.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git log -1 --format=%B failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
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
        .args(["init", "-b", "main"])
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
    // Staging is the only workflow now: commit the seeded project/roadmap/phase
    // so the plan repo has real git history for the hook to build on.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["commit", "-m", "seed: create phase"])
        .assert()
        .success();
}

// -- hook install tests --

#[test]
fn hook_install_creates_post_merge_and_post_commit() {
    let project_dir = TempDir::new().unwrap();
    init_project_repo(&project_dir);

    rdm()
        .args(["hook", "install"])
        .current_dir(project_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Installed post-merge hook"))
        .stdout(predicate::str::contains("Installed post-commit hook"));

    for (name, marker) in &[
        ("post-merge", "rdm hook post-merge"),
        ("post-commit", "rdm hook post-commit"),
    ] {
        let hook_path = project_dir.path().join(format!(".git/hooks/{name}"));
        assert!(hook_path.exists(), "{name} hook should exist");
        let contents = fs::read_to_string(&hook_path).unwrap();
        assert!(
            contents.contains(marker),
            "{name} hook should contain {marker}"
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&hook_path).unwrap().permissions().mode();
            assert!(mode & 0o111 != 0, "{name} hook should be executable");
        }
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
        .stdout(predicate::str::contains("Installed post-merge hook"))
        .stdout(predicate::str::contains("Installed post-commit hook"));
}

#[test]
fn hook_install_respects_hooks_path() {
    let project_dir = TempDir::new().unwrap();
    init_project_repo(&project_dir);

    // Set core.hooksPath to a custom directory.
    let out = git_cmd()
        .args(["config", "core.hooksPath", ".githooks"])
        .current_dir(project_dir.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "git config failed");

    rdm()
        .args(["hook", "install"])
        .current_dir(project_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Installed post-merge hook"))
        .stdout(predicate::str::contains("Installed post-commit hook"));

    // Hooks should be in .githooks/, not .git/hooks/.
    let custom_dir = project_dir.path().join(".githooks");
    for name in &["post-merge", "post-commit"] {
        let hook_path = custom_dir.join(name);
        assert!(hook_path.exists(), "{name} hook should exist in .githooks/");
        let contents = fs::read_to_string(&hook_path).unwrap();
        let marker = format!("rdm hook {name}");
        assert!(
            contents.contains(&marker),
            "{name} hook should contain {marker}"
        );
    }

    // .git/hooks/ should NOT have the hooks.
    assert!(!project_dir.path().join(".git/hooks/post-merge").exists());
    assert!(!project_dir.path().join(".git/hooks/post-commit").exists());
}

#[test]
fn hook_uninstall_respects_hooks_path() {
    let project_dir = TempDir::new().unwrap();
    init_project_repo(&project_dir);

    // Set core.hooksPath and install hooks there.
    let out = git_cmd()
        .args(["config", "core.hooksPath", ".githooks"])
        .current_dir(project_dir.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "git config failed");

    rdm()
        .args(["hook", "install"])
        .current_dir(project_dir.path())
        .assert()
        .success();

    // Uninstall should remove from .githooks/.
    rdm()
        .args(["hook", "uninstall"])
        .current_dir(project_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed post-merge hook"))
        .stdout(predicate::str::contains("Removed post-commit hook"));

    let custom_dir = project_dir.path().join(".githooks");
    assert!(!custom_dir.join("post-merge").exists());
    assert!(!custom_dir.join("post-commit").exists());
}

// -- hook uninstall tests --

#[test]
fn hook_uninstall_removes_hooks() {
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
        .stdout(predicate::str::contains("Removed post-merge hook"))
        .stdout(predicate::str::contains("Removed post-commit hook"));

    assert!(!project_dir.path().join(".git/hooks/post-merge").exists());
    assert!(!project_dir.path().join(".git/hooks/post-commit").exists());
}

#[test]
fn hook_uninstall_refuses_foreign_hook() {
    let project_dir = TempDir::new().unwrap();
    init_project_repo(&project_dir);

    // Write a foreign post-merge hook that doesn't contain "rdm hook post-merge".
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
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["commit", "-m", "seed: create phases"])
        .assert()
        .success();
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

// -- hook post-commit tests --

#[test]
fn hook_post_commit_marks_phase_done_on_default_branch() {
    let plan_dir = TempDir::new().unwrap();
    let project_dir = TempDir::new().unwrap();
    init_with_phase(&plan_dir);
    init_project_repo(&project_dir);

    // We're on `main` (the default branch). Create a commit with Done: directive.
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
            "feat: land it\n\nDone: my-roadmap/phase-1-my-phase",
        ])
        .current_dir(project_dir.path())
        .output()
        .unwrap();

    let sha_output = git_cmd()
        .args(["log", "-1", "--format=%H"])
        .current_dir(project_dir.path())
        .output()
        .unwrap();
    let sha = String::from_utf8_lossy(&sha_output.stdout)
        .trim()
        .to_string();

    rdm()
        .arg("--root")
        .arg(plan_dir.path())
        .env("RDM_PROJECT", "test-proj")
        .args(["hook", "post-commit"])
        .current_dir(project_dir.path())
        .assert()
        .success();

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
fn hook_post_commit_leaves_working_tree_clean() {
    let plan_dir = TempDir::new().unwrap();
    let project_dir = TempDir::new().unwrap();
    init_with_phase(&plan_dir);
    init_project_repo(&project_dir);

    // We're on `main`. Create a commit with a Done: directive.
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
            "feat: land it\n\nDone: my-roadmap/phase-1-my-phase",
        ])
        .current_dir(project_dir.path())
        .output()
        .unwrap();

    // Capture the plan repo's HEAD before running the hook.
    let head_before = git_cmd()
        .args(["rev-parse", "HEAD"])
        .current_dir(plan_dir.path())
        .output()
        .unwrap();
    let head_before = String::from_utf8_lossy(&head_before.stdout)
        .trim()
        .to_string();

    // Run the hook. It must always create a real commit, otherwise the
    // Done: update is silently lost.
    rdm()
        .arg("--root")
        .arg(plan_dir.path())
        .env("RDM_PROJECT", "test-proj")
        .args(["hook", "post-commit"])
        .current_dir(project_dir.path())
        .assert()
        .success();

    // Phase is marked done.
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
        .stdout(predicate::str::contains("Status: done"));

    // A real commit landed: HEAD changed.
    let head_after = git_cmd()
        .args(["rev-parse", "HEAD"])
        .current_dir(plan_dir.path())
        .output()
        .unwrap();
    let head_after = String::from_utf8_lossy(&head_after.stdout)
        .trim()
        .to_string();
    assert_ne!(
        head_before, head_after,
        "hook should have created a commit in the plan repo"
    );

    // Nothing left staged-but-uncommitted: the tree is clean.
    let status = git_cmd()
        .args(["status", "--porcelain"])
        .current_dir(plan_dir.path())
        .output()
        .unwrap();
    let status = String::from_utf8_lossy(&status.stdout);
    assert!(
        status.trim().is_empty(),
        "plan repo working tree should be clean after hook, got: {status:?}"
    );
}

#[test]
fn hook_post_merge_leaves_working_tree_clean() {
    let plan_dir = TempDir::new().unwrap();
    let project_dir = TempDir::new().unwrap();
    init_with_phase(&plan_dir);
    init_project_repo(&project_dir);

    // Create a project commit with a Done: directive.
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

    // Capture the plan repo's HEAD before running the hook.
    let head_before = git_cmd()
        .args(["rev-parse", "HEAD"])
        .current_dir(plan_dir.path())
        .output()
        .unwrap();
    let head_before = String::from_utf8_lossy(&head_before.stdout)
        .trim()
        .to_string();

    // Run the hook. As with post-commit, it must always create a real commit.
    rdm()
        .arg("--root")
        .arg(plan_dir.path())
        .env("RDM_PROJECT", "test-proj")
        .args(["hook", "post-merge"])
        .current_dir(project_dir.path())
        .assert()
        .success();

    // Phase is marked done.
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
        .stdout(predicate::str::contains("Status: done"));

    // A real commit landed: HEAD changed, and the tree is left clean.
    let head_after = git_cmd()
        .args(["rev-parse", "HEAD"])
        .current_dir(plan_dir.path())
        .output()
        .unwrap();
    let head_after = String::from_utf8_lossy(&head_after.stdout)
        .trim()
        .to_string();
    assert_ne!(
        head_before, head_after,
        "hook should have created a commit in the plan repo"
    );

    let status = git_cmd()
        .args(["status", "--porcelain"])
        .current_dir(plan_dir.path())
        .output()
        .unwrap();
    let status = String::from_utf8_lossy(&status.stdout);
    assert!(
        status.trim().is_empty(),
        "plan repo working tree should be clean after hook, got: {status:?}"
    );
}

#[test]
fn hook_post_commit_skips_feature_branch() {
    let plan_dir = TempDir::new().unwrap();
    let project_dir = TempDir::new().unwrap();
    init_with_phase(&plan_dir);
    init_project_repo(&project_dir);

    // Create and switch to a feature branch.
    git_cmd()
        .args(["checkout", "-b", "feature-x"])
        .current_dir(project_dir.path())
        .output()
        .unwrap();

    let dummy_path = project_dir.path().join("dummy.txt");
    fs::write(&dummy_path, "feature work").unwrap();
    git_cmd()
        .args(["add", "dummy.txt"])
        .current_dir(project_dir.path())
        .output()
        .unwrap();
    git_cmd()
        .args([
            "commit",
            "-m",
            "feat: feature work\n\nDone: my-roadmap/phase-1-my-phase",
        ])
        .current_dir(project_dir.path())
        .output()
        .unwrap();

    // Post-commit should be a no-op on a feature branch.
    rdm()
        .arg("--root")
        .arg(plan_dir.path())
        .env("RDM_PROJECT", "test-proj")
        .args(["hook", "post-commit"])
        .current_dir(project_dir.path())
        .assert()
        .success();

    // Phase should still be not-started.
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
        .stdout(predicate::str::contains("Status: not-started"));
}

#[test]
fn hook_post_commit_silent_on_no_directives() {
    let plan_dir = TempDir::new().unwrap();
    let project_dir = TempDir::new().unwrap();
    init_with_phase(&plan_dir);
    init_project_repo(&project_dir);

    let dummy_path = project_dir.path().join("dummy.txt");
    fs::write(&dummy_path, "no directives").unwrap();
    git_cmd()
        .args(["add", "dummy.txt"])
        .current_dir(project_dir.path())
        .output()
        .unwrap();
    git_cmd()
        .args(["commit", "-m", "chore: regular commit"])
        .current_dir(project_dir.path())
        .output()
        .unwrap();

    rdm()
        .arg("--root")
        .arg(plan_dir.path())
        .env("RDM_PROJECT", "test-proj")
        .args(["hook", "post-commit"])
        .current_dir(project_dir.path())
        .assert()
        .success();
}

#[test]
fn hook_post_commit_respects_custom_default_branch() {
    let plan_dir = TempDir::new().unwrap();
    let project_dir = TempDir::new().unwrap();
    init_with_phase(&plan_dir);
    init_project_repo(&project_dir);

    // Configure default_branch = "develop" in the plan repo's rdm.toml.
    let config_path = plan_dir.path().join("rdm.toml");
    let existing = fs::read_to_string(&config_path).unwrap_or_default();
    fs::write(
        &config_path,
        format!("{existing}\ndefault_branch = \"develop\"\n"),
    )
    .unwrap();

    // We're on `main`, but default_branch is `develop` — should skip.
    let dummy_path = project_dir.path().join("dummy.txt");
    fs::write(&dummy_path, "on main").unwrap();
    git_cmd()
        .args(["add", "dummy.txt"])
        .current_dir(project_dir.path())
        .output()
        .unwrap();
    git_cmd()
        .args([
            "commit",
            "-m",
            "feat: on main\n\nDone: my-roadmap/phase-1-my-phase",
        ])
        .current_dir(project_dir.path())
        .output()
        .unwrap();

    rdm()
        .arg("--root")
        .arg(plan_dir.path())
        .env("RDM_PROJECT", "test-proj")
        .args(["hook", "post-commit"])
        .current_dir(project_dir.path())
        .assert()
        .success();

    // Phase should still be not-started because we're on main, not develop.
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
        .stdout(predicate::str::contains("Status: not-started"));

    // Now switch to `develop` and commit there.
    git_cmd()
        .args(["checkout", "-b", "develop"])
        .current_dir(project_dir.path())
        .output()
        .unwrap();
    let dummy2 = project_dir.path().join("dummy2.txt");
    fs::write(&dummy2, "on develop").unwrap();
    git_cmd()
        .args(["add", "dummy2.txt"])
        .current_dir(project_dir.path())
        .output()
        .unwrap();
    git_cmd()
        .args([
            "commit",
            "-m",
            "feat: on develop\n\nDone: my-roadmap/phase-1-my-phase",
        ])
        .current_dir(project_dir.path())
        .output()
        .unwrap();

    let sha_output = git_cmd()
        .args(["log", "-1", "--format=%H"])
        .current_dir(project_dir.path())
        .output()
        .unwrap();
    let sha = String::from_utf8_lossy(&sha_output.stdout)
        .trim()
        .to_string();

    rdm()
        .arg("--root")
        .arg(plan_dir.path())
        .env("RDM_PROJECT", "test-proj")
        .args(["hook", "post-commit"])
        .current_dir(project_dir.path())
        .assert()
        .success();

    // Now it should be done — we're on develop, which matches default_branch.
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
fn hook_post_commit_idempotent_with_post_merge() {
    let plan_dir = TempDir::new().unwrap();
    let project_dir = TempDir::new().unwrap();
    init_with_phase(&plan_dir);
    init_project_repo(&project_dir);

    let dummy_path = project_dir.path().join("dummy.txt");
    fs::write(&dummy_path, "trigger").unwrap();
    git_cmd()
        .args(["add", "dummy.txt"])
        .current_dir(project_dir.path())
        .output()
        .unwrap();
    git_cmd()
        .args([
            "commit",
            "-m",
            "feat: done\n\nDone: my-roadmap/phase-1-my-phase",
        ])
        .current_dir(project_dir.path())
        .output()
        .unwrap();

    let sha_output = git_cmd()
        .args(["log", "-1", "--format=%H"])
        .current_dir(project_dir.path())
        .output()
        .unwrap();
    let sha = String::from_utf8_lossy(&sha_output.stdout)
        .trim()
        .to_string();

    // Run post-commit first.
    rdm()
        .arg("--root")
        .arg(plan_dir.path())
        .env("RDM_PROJECT", "test-proj")
        .args(["hook", "post-commit"])
        .current_dir(project_dir.path())
        .assert()
        .success();

    // Run post-merge second — should be a no-op (already done).
    rdm()
        .arg("--root")
        .arg(plan_dir.path())
        .env("RDM_PROJECT", "test-proj")
        .args(["hook", "post-merge"])
        .current_dir(project_dir.path())
        .assert()
        .success();

    // Phase should still be done with correct SHA.
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

// -- hook log tests --

fn read_log(project_dir: &TempDir) -> String {
    let path = project_dir.path().join(".git/rdm-hook.log");
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()))
}

#[test]
fn hook_post_commit_writes_log_on_success() {
    let plan_dir = TempDir::new().unwrap();
    let project_dir = TempDir::new().unwrap();
    init_with_phase(&plan_dir);
    init_project_repo(&project_dir);

    fs::write(project_dir.path().join("dummy.txt"), "x").unwrap();
    git_cmd()
        .args(["add", "dummy.txt"])
        .current_dir(project_dir.path())
        .output()
        .unwrap();
    git_cmd()
        .args([
            "commit",
            "-m",
            "feat: x\n\nDone: my-roadmap/phase-1-my-phase",
        ])
        .current_dir(project_dir.path())
        .output()
        .unwrap();

    rdm()
        .arg("--root")
        .arg(plan_dir.path())
        .env("RDM_PROJECT", "test-proj")
        .args(["hook", "post-commit"])
        .current_dir(project_dir.path())
        .assert()
        .success();

    let log = read_log(&project_dir);
    assert!(
        log.contains("post-commit entry"),
        "log missing entry: {log}"
    );
    assert!(
        log.contains("post-commit apply-phase status=ok"),
        "log missing apply-phase: {log}"
    );
    assert!(log.contains("post-commit exit"), "log missing exit: {log}");
}

#[test]
fn hook_post_commit_logs_skip_on_feature_branch() {
    let plan_dir = TempDir::new().unwrap();
    let project_dir = TempDir::new().unwrap();
    init_with_phase(&plan_dir);
    init_project_repo(&project_dir);

    git_cmd()
        .args(["checkout", "-b", "feature-x"])
        .current_dir(project_dir.path())
        .output()
        .unwrap();
    fs::write(project_dir.path().join("dummy.txt"), "feat").unwrap();
    git_cmd()
        .args(["add", "dummy.txt"])
        .current_dir(project_dir.path())
        .output()
        .unwrap();
    git_cmd()
        .args([
            "commit",
            "-m",
            "feat: x\n\nDone: my-roadmap/phase-1-my-phase",
        ])
        .current_dir(project_dir.path())
        .output()
        .unwrap();

    rdm()
        .arg("--root")
        .arg(plan_dir.path())
        .env("RDM_PROJECT", "test-proj")
        .args(["hook", "post-commit"])
        .current_dir(project_dir.path())
        .assert()
        .success();

    let log = read_log(&project_dir);
    assert!(
        log.contains("skip-branch branch=feature-x default=main"),
        "log missing skip-branch: {log}"
    );
}

#[test]
fn hook_post_merge_logs_skip_on_unknown_phase() {
    let plan_dir = TempDir::new().unwrap();
    let project_dir = TempDir::new().unwrap();
    init_with_phase(&plan_dir);
    init_project_repo(&project_dir);

    fs::write(project_dir.path().join("dummy.txt"), "bad").unwrap();
    git_cmd()
        .args(["add", "dummy.txt"])
        .current_dir(project_dir.path())
        .output()
        .unwrap();
    git_cmd()
        .args([
            "commit",
            "-m",
            // Numeric phase identifier forces resolve_phase_stem to query the
            // store, which fails because phase 99 does not exist.
            "feat: merge\n\nDone: my-roadmap/99",
        ])
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

    let log = read_log(&project_dir);
    assert!(
        log.contains("skip-unknown-phase"),
        "log missing skip-unknown-phase: {log}"
    );
    assert!(
        log.contains("roadmap=my-roadmap"),
        "log missing roadmap kv: {log}"
    );
}

#[test]
fn hook_install_shim_redirects_to_log_file() {
    let project_dir = TempDir::new().unwrap();
    init_project_repo(&project_dir);

    rdm()
        .args(["hook", "install"])
        .current_dir(project_dir.path())
        .assert()
        .success();

    for name in &["post-merge", "post-commit"] {
        let hook_path = project_dir.path().join(format!(".git/hooks/{name}"));
        let contents = fs::read_to_string(&hook_path).unwrap();
        assert!(
            contents.contains("rdm-hook.log"),
            "{name} shim should reference rdm-hook.log: {contents}"
        );
    }

    rdm()
        .args(["hook", "uninstall"])
        .current_dir(project_dir.path())
        .assert()
        .success();
    assert!(!project_dir.path().join(".git/hooks/post-merge").exists());
    assert!(!project_dir.path().join(".git/hooks/post-commit").exists());
}

#[test]
fn hook_log_truncates_when_oversize() {
    use std::io::Write;

    let plan_dir = TempDir::new().unwrap();
    let project_dir = TempDir::new().unwrap();
    init_with_phase(&plan_dir);
    init_project_repo(&project_dir);

    // Pre-seed log with > 256 KB of junk lines.
    let log_path = project_dir.path().join(".git/rdm-hook.log");
    fs::create_dir_all(log_path.parent().unwrap()).unwrap();
    let mut f = fs::File::create(&log_path).unwrap();
    let line = format!("{}\n", "x".repeat(1023));
    for _ in 0..400 {
        f.write_all(line.as_bytes()).unwrap();
    }
    drop(f);
    let pre_len = fs::metadata(&log_path).unwrap().len();
    assert!(
        pre_len > 256 * 1024,
        "pre-seed should be > cap, was {pre_len}"
    );

    fs::write(project_dir.path().join("dummy.txt"), "x").unwrap();
    git_cmd()
        .args(["add", "dummy.txt"])
        .current_dir(project_dir.path())
        .output()
        .unwrap();
    git_cmd()
        .args(["commit", "-m", "chore: noop"])
        .current_dir(project_dir.path())
        .output()
        .unwrap();

    rdm()
        .arg("--root")
        .arg(plan_dir.path())
        .env("RDM_PROJECT", "test-proj")
        .args(["hook", "post-commit"])
        .current_dir(project_dir.path())
        .assert()
        .success();

    let post_len = fs::metadata(&log_path).unwrap().len();
    assert!(
        post_len < 256 * 1024,
        "log should be under cap after truncate, was {post_len}"
    );
    let log = fs::read_to_string(&log_path).unwrap();
    assert!(
        log.contains("post-commit entry"),
        "latest entry should be present after truncate: tail={}",
        &log[log.len().saturating_sub(500)..]
    );
}

// -- AC4: git-subprocess re-entrancy guard --

#[test]
fn hook_post_commit_skips_when_marker_env_present() {
    let plan_dir = TempDir::new().unwrap();
    let project_dir = TempDir::new().unwrap();
    init_with_phase(&plan_dir);
    init_project_repo(&project_dir);

    fs::write(project_dir.path().join("dummy.txt"), "trigger commit").unwrap();
    git_cmd()
        .args(["add", "dummy.txt"])
        .current_dir(project_dir.path())
        .output()
        .unwrap();
    git_cmd()
        .args([
            "commit",
            "-m",
            "feat: land it\n\nDone: my-roadmap/phase-1-my-phase",
        ])
        .current_dir(project_dir.path())
        .output()
        .unwrap();

    // Simulate this hook invocation itself being a subprocess spawned by
    // rdm's own git_command — the marker every rdm-invoked git subprocess
    // carries (e.g. the completing `git commit --no-edit` in
    // `git_resolve_conflict`, whose own hook runner could invoke this same
    // `rdm hook post-commit` as a nested child — see §1.2(C)).
    rdm()
        .arg("--root")
        .arg(plan_dir.path())
        .env("RDM_PROJECT", "test-proj")
        .env("RDM_GIT_SUBPROCESS", "1")
        .args(["hook", "post-commit"])
        .current_dir(project_dir.path())
        .assert()
        .success();

    // The phase must NOT be marked done — the guard must short-circuit
    // before ever touching the store.
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
        .stdout(predicate::str::contains("Status: not-started"));

    let log = read_log(&project_dir);
    assert!(
        log.contains("post-commit skip-reentrant"),
        "log missing skip-reentrant event: {log}"
    );
}

#[test]
fn hook_post_merge_skips_when_marker_env_present() {
    let plan_dir = TempDir::new().unwrap();
    let project_dir = TempDir::new().unwrap();
    init_with_phase(&plan_dir);
    init_project_repo(&project_dir);

    fs::write(project_dir.path().join("dummy.txt"), "trigger commit").unwrap();
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

    rdm()
        .arg("--root")
        .arg(plan_dir.path())
        .env("RDM_PROJECT", "test-proj")
        .env("RDM_GIT_SUBPROCESS", "1")
        .args(["hook", "post-merge"])
        .current_dir(project_dir.path())
        .assert()
        .success();

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
        .stdout(predicate::str::contains("Status: not-started"));

    let log = read_log(&project_dir);
    assert!(
        log.contains("post-merge skip-reentrant"),
        "log missing skip-reentrant event: {log}"
    );
}

// -- AC1: hook execution deadline --

#[test]
fn hook_post_commit_never_blocks_past_configured_timeout() {
    let plan_dir = TempDir::new().unwrap();
    let project_dir = TempDir::new().unwrap();
    init_with_phase(&plan_dir);
    init_project_repo(&project_dir);

    // Configure a very low hook_timeout_secs in the plan repo.
    let config_path = plan_dir.path().join("rdm.toml");
    let existing = fs::read_to_string(&config_path).unwrap_or_default();
    fs::write(&config_path, format!("{existing}\nhook_timeout_secs = 1\n")).unwrap();

    fs::write(project_dir.path().join("dummy.txt"), "trigger commit").unwrap();
    git_cmd()
        .args(["add", "dummy.txt"])
        .current_dir(project_dir.path())
        .output()
        .unwrap();
    git_cmd()
        .args([
            "commit",
            "-m",
            "feat: land it\n\nDone: my-roadmap/phase-1-my-phase",
        ])
        .current_dir(project_dir.path())
        .output()
        .unwrap();

    let start = std::time::Instant::now();
    rdm()
        .arg("--root")
        .arg(plan_dir.path())
        .env("RDM_PROJECT", "test-proj")
        // Make the hook body stall far longer than the configured 1s
        // timeout — simulating the kind of stuck-subprocess hang AC2/AC3
        // fix, without needing a real stuck editor/credential prompt here.
        .env("RDM_TEST_STALL_HOOK_MS", "5000")
        .args(["hook", "post-commit"])
        .current_dir(project_dir.path())
        .assert()
        .success();
    let elapsed = start.elapsed();

    assert!(
        elapsed < std::time::Duration::from_secs(4),
        "hook post-commit should return within a small multiple of the \
         configured 1s timeout even though its body stalls for 5s, took {elapsed:?}"
    );

    let log = read_log(&project_dir);
    assert!(
        log.contains("post-commit timeout"),
        "log missing timeout event: {log}"
    );

    // Because the body was abandoned mid-stall, the phase was never
    // actually marked done.
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
        .stdout(predicate::str::contains("Status: not-started"));
}

#[test]
fn hook_post_merge_never_blocks_past_configured_timeout() {
    let plan_dir = TempDir::new().unwrap();
    let project_dir = TempDir::new().unwrap();
    init_with_phase(&plan_dir);
    init_project_repo(&project_dir);

    // Configure a very low hook_timeout_secs in the plan repo.
    let config_path = plan_dir.path().join("rdm.toml");
    let existing = fs::read_to_string(&config_path).unwrap_or_default();
    fs::write(&config_path, format!("{existing}\nhook_timeout_secs = 1\n")).unwrap();

    fs::write(project_dir.path().join("dummy.txt"), "trigger commit").unwrap();
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

    let start = std::time::Instant::now();
    rdm()
        .arg("--root")
        .arg(plan_dir.path())
        .env("RDM_PROJECT", "test-proj")
        // Same stall injection as the post-commit test above — each hook
        // arm gets its own fresh timeout budget, so this pins the
        // post-merge wiring independently.
        .env("RDM_TEST_STALL_HOOK_MS", "5000")
        .args(["hook", "post-merge"])
        .current_dir(project_dir.path())
        .assert()
        .success();
    let elapsed = start.elapsed();

    assert!(
        elapsed < std::time::Duration::from_secs(4),
        "hook post-merge should return within a small multiple of the \
         configured 1s timeout even though its body stalls for 5s, took {elapsed:?}"
    );

    let log = read_log(&project_dir);
    assert!(
        log.contains("post-merge timeout"),
        "log missing timeout event: {log}"
    );

    // Because the body was abandoned mid-stall, the phase was never
    // actually marked done.
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
        .stdout(predicate::str::contains("Status: not-started"));
}

// -- batching tests (Done: directives collapse into one commit) --

#[test]
fn hook_post_merge_batches_multiple_directives_into_one_commit() {
    let plan_dir = TempDir::new().unwrap();
    let project_dir = TempDir::new().unwrap();
    init_with_phases(&plan_dir, &["alpha", "beta", "gamma"]);
    init_project_repo(&project_dir);

    git_cmd()
        .args(["tag", "before-merge"])
        .current_dir(project_dir.path())
        .output()
        .unwrap();

    // Create three separate commits, each with a Done: directive, and
    // remember each one's own SHA.
    let phases = ["phase-1-alpha", "phase-2-beta", "phase-3-gamma"];
    let mut shas = Vec::new();
    for (i, phase) in phases.iter().enumerate() {
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
        let sha_output = git_cmd()
            .args(["log", "-1", "--format=%H"])
            .current_dir(project_dir.path())
            .output()
            .unwrap();
        shas.push(
            String::from_utf8_lossy(&sha_output.stdout)
                .trim()
                .to_string(),
        );
    }

    let commits_before = plan_repo_commit_count(&plan_dir);

    rdm()
        .arg("--root")
        .arg(plan_dir.path())
        .env("RDM_PROJECT", "test-proj")
        .args(["hook", "post-merge", "--since", "before-merge"])
        .current_dir(project_dir.path())
        .assert()
        .success();

    let commits_after = plan_repo_commit_count(&plan_dir);
    assert_eq!(
        commits_after,
        commits_before + 1,
        "3 Done: directives across 3 source commits must land as exactly one plan-repo commit"
    );

    let head_message = plan_repo_head_message(&plan_dir);
    for (phase, sha) in phases.iter().zip(shas.iter()) {
        // Assert the target is paired with its OWN source SHA on one line —
        // not merely that both strings appear somewhere in the message.
        assert!(
            head_message.contains(&format!("Done: my-roadmap/{phase} ({})", &sha[..7])),
            "batch commit message must pair {phase} with its own source sha \
             {}: {head_message}",
            &sha[..7]
        );
    }

    // Every phase is done, each carrying its own source commit's SHA (not a
    // sibling directive's SHA, and not the plan repo's own new batch SHA).
    for (phase, sha) in phases.iter().zip(shas.iter()) {
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
            .stdout(predicate::str::contains("Status: done"))
            .stdout(predicate::str::contains(sha.as_str()));
    }
}

#[test]
fn hook_post_commit_batches_multiple_directives_in_one_message_into_one_commit() {
    let plan_dir = TempDir::new().unwrap();
    let project_dir = TempDir::new().unwrap();
    init_with_phase(&plan_dir);
    init_project_repo(&project_dir);

    rdm()
        .arg("--root")
        .arg(plan_dir.path())
        .args([
            "task",
            "create",
            "fix-bug",
            "--title",
            "Fix the bug",
            "--project",
            "test-proj",
        ])
        .assert()
        .success();

    let commits_before = plan_repo_commit_count(&plan_dir);

    fs::write(project_dir.path().join("dummy.txt"), "trigger").unwrap();
    git_cmd()
        .args(["add", "dummy.txt"])
        .current_dir(project_dir.path())
        .output()
        .unwrap();
    git_cmd()
        .args([
            "commit",
            "-m",
            "feat: land two things\n\nDone: my-roadmap/phase-1-my-phase\nDone: task/fix-bug",
        ])
        .current_dir(project_dir.path())
        .output()
        .unwrap();

    let sha_output = git_cmd()
        .args(["log", "-1", "--format=%H"])
        .current_dir(project_dir.path())
        .output()
        .unwrap();
    let sha = String::from_utf8_lossy(&sha_output.stdout)
        .trim()
        .to_string();

    rdm()
        .arg("--root")
        .arg(plan_dir.path())
        .env("RDM_PROJECT", "test-proj")
        .args(["hook", "post-commit"])
        .current_dir(project_dir.path())
        .assert()
        .success();

    let commits_after = plan_repo_commit_count(&plan_dir);
    assert_eq!(
        commits_after,
        commits_before + 1,
        "one commit message with two Done: directives must land as exactly one plan-repo commit"
    );

    let head_message = plan_repo_head_message(&plan_dir);
    assert!(head_message.contains("Done: my-roadmap/phase-1-my-phase"));
    assert!(head_message.contains("Done: task/fix-bug"));
    assert!(head_message.contains(&sha[..7]));

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
        .stdout(predicate::str::contains(sha.as_str()));

    rdm()
        .arg("--root")
        .arg(plan_dir.path())
        .args(["task", "show", "fix-bug", "--project", "test-proj"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Status: done"))
        .stdout(predicate::str::contains(sha.as_str()));
}

#[test]
fn hook_post_commit_then_post_merge_batch_idempotent() {
    let plan_dir = TempDir::new().unwrap();
    let project_dir = TempDir::new().unwrap();
    init_with_phases(&plan_dir, &["alpha", "beta"]);
    init_project_repo(&project_dir);

    fs::write(project_dir.path().join("dummy.txt"), "trigger").unwrap();
    git_cmd()
        .args(["add", "dummy.txt"])
        .current_dir(project_dir.path())
        .output()
        .unwrap();
    git_cmd()
        .args([
            "commit",
            "-m",
            "feat: done both\n\nDone: my-roadmap/phase-1-alpha\nDone: my-roadmap/phase-2-beta",
        ])
        .current_dir(project_dir.path())
        .output()
        .unwrap();

    let sha_output = git_cmd()
        .args(["log", "-1", "--format=%H"])
        .current_dir(project_dir.path())
        .output()
        .unwrap();
    let sha = String::from_utf8_lossy(&sha_output.stdout)
        .trim()
        .to_string();

    // Run post-commit first (the multi-directive batched path).
    rdm()
        .arg("--root")
        .arg(plan_dir.path())
        .env("RDM_PROJECT", "test-proj")
        .args(["hook", "post-commit"])
        .current_dir(project_dir.path())
        .assert()
        .success();

    // Run post-merge second — should be a no-op re-application (already done).
    rdm()
        .arg("--root")
        .arg(plan_dir.path())
        .env("RDM_PROJECT", "test-proj")
        .args(["hook", "post-merge"])
        .current_dir(project_dir.path())
        .assert()
        .success();

    for phase in ["phase-1-alpha", "phase-2-beta"] {
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
            .stdout(predicate::str::contains("Status: done"))
            .stdout(predicate::str::contains(sha.as_str()));
    }
}

#[test]
fn hook_post_commit_logs_directives_even_when_index_regen_fails() {
    let plan_dir = TempDir::new().unwrap();
    let project_dir = TempDir::new().unwrap();
    init_with_phase(&plan_dir);
    init_project_repo(&project_dir);

    // Corrupt an unrelated roadmap directly on disk (bypassing `rdm`, since
    // this is a hermetic fixture, not the dogfood plan repo) so that index
    // regeneration fails for the whole project.
    let broken_dir = plan_dir.path().join("projects/test-proj/roadmaps/broken");
    fs::create_dir_all(&broken_dir).unwrap();
    fs::write(
        broken_dir.join("roadmap.md"),
        "---\nnot_a_valid_roadmap_field: true\n---\nBroken.\n",
    )
    .unwrap();
    git_cmd()
        .args(["add", "-A"])
        .current_dir(plan_dir.path())
        .output()
        .unwrap();
    git_cmd()
        .args(["commit", "-m", "test: seed corrupt roadmap fixture"])
        .current_dir(plan_dir.path())
        .output()
        .unwrap();

    fs::write(project_dir.path().join("dummy.txt"), "trigger").unwrap();
    git_cmd()
        .args(["add", "dummy.txt"])
        .current_dir(project_dir.path())
        .output()
        .unwrap();
    git_cmd()
        .args([
            "commit",
            "-m",
            "feat: land it\n\nDone: my-roadmap/phase-1-my-phase",
        ])
        .current_dir(project_dir.path())
        .output()
        .unwrap();

    let commits_before = plan_repo_commit_count(&plan_dir);

    // The hook wrapper always exits 0 — errors are logged, never propagated
    // to the invoking `git commit`.
    rdm()
        .arg("--root")
        .arg(plan_dir.path())
        .env("RDM_PROJECT", "test-proj")
        .args(["hook", "post-commit"])
        .current_dir(project_dir.path())
        .assert()
        .success();

    let log = read_log(&project_dir);
    assert!(
        log.contains("post-commit apply-phase status=ok"),
        "per-directive log fidelity must survive a shared index-regen failure: {log}"
    );
    assert!(
        log.contains("batch-commit-error"),
        "log missing batch-commit-error event: {log}"
    );

    // The batch is all-or-nothing: a finalize failure must leave the plan
    // repo's history untouched — no partial or index-less commit lands.
    let commits_after = plan_repo_commit_count(&plan_dir);
    assert_eq!(
        commits_after, commits_before,
        "a finalize failure must not create a plan-repo commit"
    );
}

#[test]
fn hook_post_commit_one_bad_directive_does_not_abort_others() {
    let plan_dir = TempDir::new().unwrap();
    let project_dir = TempDir::new().unwrap();
    init_with_phases(&plan_dir, &["alpha", "beta", "gamma"]);
    init_project_repo(&project_dir);

    let commits_before = plan_repo_commit_count(&plan_dir);

    fs::write(project_dir.path().join("dummy.txt"), "trigger").unwrap();
    git_cmd()
        .args(["add", "dummy.txt"])
        .current_dir(project_dir.path())
        .output()
        .unwrap();
    git_cmd()
        .args([
            "commit",
            "-m",
            "feat: mixed batch\n\nDone: my-roadmap/phase-1-alpha\nDone: my-roadmap/99\nDone: my-roadmap/phase-3-gamma",
        ])
        .current_dir(project_dir.path())
        .output()
        .unwrap();

    rdm()
        .arg("--root")
        .arg(plan_dir.path())
        .env("RDM_PROJECT", "test-proj")
        .args(["hook", "post-commit"])
        .current_dir(project_dir.path())
        .assert()
        .success();

    let log = read_log(&project_dir);
    assert!(
        log.contains("skip-unknown-phase"),
        "log missing skip-unknown-phase: {log}"
    );
    assert!(
        log.contains("post-commit apply-phase status=ok"),
        "log missing apply-phase status=ok for the good directives: {log}"
    );

    for phase in ["phase-1-alpha", "phase-3-gamma"] {
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
        .stdout(predicate::str::contains("Status: not-started"));

    // Both good directives still landed together in exactly one commit.
    let commits_after = plan_repo_commit_count(&plan_dir);
    assert_eq!(
        commits_after,
        commits_before + 1,
        "the two good directives must still batch into exactly one commit \
         even though a sibling directive in the same message was unknown"
    );
}

#[test]
fn hook_post_commit_duplicate_directive_in_one_message_not_deduped() {
    let plan_dir = TempDir::new().unwrap();
    let project_dir = TempDir::new().unwrap();
    init_with_phase(&plan_dir);
    init_project_repo(&project_dir);

    fs::write(project_dir.path().join("dummy.txt"), "trigger").unwrap();
    git_cmd()
        .args(["add", "dummy.txt"])
        .current_dir(project_dir.path())
        .output()
        .unwrap();
    git_cmd()
        .args([
            "commit",
            "-m",
            "feat: duplicate directive\n\nDone: my-roadmap/phase-1-my-phase\nDone: my-roadmap/phase-1-my-phase",
        ])
        .current_dir(project_dir.path())
        .output()
        .unwrap();

    // post-commit does not dedupe within one message (unlike post-merge's
    // cross-commit `seen`-set dedup) — batching must not silently change
    // that. This must not error or double-apply incorrectly.
    rdm()
        .arg("--root")
        .arg(plan_dir.path())
        .env("RDM_PROJECT", "test-proj")
        .args(["hook", "post-commit"])
        .current_dir(project_dir.path())
        .assert()
        .success();

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
        .stdout(predicate::str::contains("Status: done"));

    // The batch commit message is the observable proof of non-dedup: the
    // duplicate directive must be applied — and enumerated — twice.
    let message = plan_repo_head_message(&plan_dir);
    assert!(
        message.contains("rdm: apply 2 Done: directive(s)"),
        "expected non-deduped count of 2 in batch commit message, got:\n{message}"
    );
    assert_eq!(
        message.matches("Done: my-roadmap/phase-1-my-phase").count(),
        2,
        "expected the duplicate directive enumerated twice, got:\n{message}"
    );
}

#[test]
fn hook_post_commit_failed_step_is_excluded_from_batch_commit_message() {
    let plan_dir = TempDir::new().unwrap();
    let project_dir = TempDir::new().unwrap();
    init_with_phase(&plan_dir);
    init_project_repo(&project_dir);

    let commits_before = plan_repo_commit_count(&plan_dir);

    // `task/<slug>` directives have no pre-mutate existence check (unlike
    // phases), so a nonexistent slug becomes a real batch step whose
    // mutation fails — producing a mixed Ok/Err result slice.
    fs::write(project_dir.path().join("dummy.txt"), "trigger").unwrap();
    git_cmd()
        .args(["add", "dummy.txt"])
        .current_dir(project_dir.path())
        .output()
        .unwrap();
    git_cmd()
        .args([
            "commit",
            "-m",
            "feat: mixed outcome batch\n\nDone: my-roadmap/phase-1-my-phase\nDone: task/no-such-task",
        ])
        .current_dir(project_dir.path())
        .output()
        .unwrap();

    rdm()
        .arg("--root")
        .arg(plan_dir.path())
        .env("RDM_PROJECT", "test-proj")
        .args(["hook", "post-commit"])
        .current_dir(project_dir.path())
        .assert()
        .success();

    let log = read_log(&project_dir);
    assert!(
        log.contains("post-commit apply-phase status=ok"),
        "log missing apply-phase status=ok: {log}"
    );
    assert!(
        log.contains("apply-task status=error"),
        "log missing apply-task status=error for the unknown task: {log}"
    );

    // The good directive still lands in exactly one commit.
    let commits_after = plan_repo_commit_count(&plan_dir);
    assert_eq!(commits_after, commits_before + 1);

    // The commit message enumerates only the SUCCESSFUL directive: the failed
    // step must not appear, and the count must reflect successes only.
    let message = plan_repo_head_message(&plan_dir);
    assert!(
        message.contains("rdm: apply 1 Done: directive(s)"),
        "count must reflect only successful steps, got:\n{message}"
    );
    assert!(
        message.contains("Done: my-roadmap/phase-1-my-phase"),
        "successful directive missing from message:\n{message}"
    );
    assert!(
        !message.contains("no-such-task"),
        "failed directive must not be enumerated as applied:\n{message}"
    );
}

#[test]
fn hook_post_commit_all_directives_skipped_creates_no_commit() {
    let plan_dir = TempDir::new().unwrap();
    let project_dir = TempDir::new().unwrap();
    init_with_phase(&plan_dir);
    init_project_repo(&project_dir);

    let commits_before = plan_repo_commit_count(&plan_dir);

    fs::write(project_dir.path().join("dummy.txt"), "trigger").unwrap();
    git_cmd()
        .args(["add", "dummy.txt"])
        .current_dir(project_dir.path())
        .output()
        .unwrap();
    git_cmd()
        .args(["commit", "-m", "feat: nothing valid\n\nDone: my-roadmap/99"])
        .current_dir(project_dir.path())
        .output()
        .unwrap();

    rdm()
        .arg("--root")
        .arg(plan_dir.path())
        .env("RDM_PROJECT", "test-proj")
        .args(["hook", "post-commit"])
        .current_dir(project_dir.path())
        .assert()
        .success();

    let log = read_log(&project_dir);
    assert!(
        log.contains("skip-unknown-phase"),
        "log missing skip-unknown-phase: {log}"
    );

    // Every directive was skipped pre-batch: no spurious empty batch commit.
    let commits_after = plan_repo_commit_count(&plan_dir);
    assert_eq!(
        commits_after, commits_before,
        "an all-skipped batch must not create a plan-repo commit"
    );
}

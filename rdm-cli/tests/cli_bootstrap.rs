use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn rdm() -> Command {
    let mut cmd = Command::cargo_bin("rdm").unwrap();
    cmd.env("XDG_CONFIG_HOME", "/dev/null/nonexistent");
    cmd
}

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

/// Creates a source plan repo and a bare clone of it.
/// Returns (source_dir, bare_dir) — use `bare_dir.path()` as the `--plan-repo` url.
fn make_plan_repo_with_bare() -> (TempDir, TempDir) {
    let source = TempDir::new().unwrap();
    rdm()
        .arg("--root")
        .arg(source.path())
        .arg("init")
        .assert()
        .success();

    let bare = TempDir::new().unwrap();
    let status = git_cmd()
        .args(["clone", "--bare"])
        .arg(source.path())
        .arg(bare.path())
        .status()
        .unwrap();
    assert!(status.success());

    (source, bare)
}

/// Creates a bare git repo with no rdm.toml (so it can't be recognized as a plan repo).
fn make_empty_bare_repo() -> TempDir {
    let source = TempDir::new().unwrap();
    let bare = TempDir::new().unwrap();
    // Init the source and add a single commit so bare clone has a branch.
    git_cmd()
        .args(["init"])
        .arg(source.path())
        .status()
        .unwrap();
    std::fs::write(source.path().join("README.md"), "hi").unwrap();
    git_cmd()
        .args(["-C"])
        .arg(source.path())
        .args(["add", "README.md"])
        .status()
        .unwrap();
    git_cmd()
        .args(["-C"])
        .arg(source.path())
        .args(["commit", "-m", "init"])
        .status()
        .unwrap();
    git_cmd()
        .args(["clone", "--bare"])
        .arg(source.path())
        .arg(bare.path())
        .status()
        .unwrap();
    bare
}

#[test]
fn bootstrap_clones_into_custom_path() {
    let (_source, bare) = make_plan_repo_with_bare();
    let target_parent = TempDir::new().unwrap();
    let target = target_parent.path().join("plan");

    rdm()
        .arg("bootstrap")
        .arg("--plan-repo")
        .arg(bare.path())
        .arg("--path")
        .arg(&target)
        .assert()
        .success()
        .stdout(predicate::str::contains("Plan repo ready at"))
        .stdout(predicate::str::contains("export RDM_ROOT="));

    assert!(target.join(".git").exists());
    assert!(target.join("rdm.toml").exists());
}

#[test]
fn bootstrap_is_idempotent() {
    let (_source, bare) = make_plan_repo_with_bare();
    let target_parent = TempDir::new().unwrap();
    let target = target_parent.path().join("plan");

    rdm()
        .arg("bootstrap")
        .arg("--plan-repo")
        .arg(bare.path())
        .arg("--path")
        .arg(&target)
        .assert()
        .success();

    // Re-run: should fast-forward no-op.
    rdm()
        .arg("bootstrap")
        .arg("--plan-repo")
        .arg(bare.path())
        .arg("--path")
        .arg(&target)
        .assert()
        .success()
        .stdout(predicate::str::contains("Already up to date"));
}

#[test]
fn bootstrap_fast_forwards_new_commits() {
    let (source, bare) = make_plan_repo_with_bare();
    let target_parent = TempDir::new().unwrap();
    let target = target_parent.path().join("plan");

    rdm()
        .arg("bootstrap")
        .arg("--plan-repo")
        .arg(bare.path())
        .arg("--path")
        .arg(&target)
        .assert()
        .success();

    // Add a new commit in the source and push it to the bare.
    rdm()
        .arg("--root")
        .arg(source.path())
        .arg("project")
        .arg("create")
        .arg("demo")
        .assert()
        .success();
    let status = git_cmd()
        .args(["-C"])
        .arg(source.path())
        .args(["push", "origin", "HEAD"])
        .status();
    // Source repo from `rdm init` has no "origin" by default; push via the bare url directly.
    if !status.map(|s| s.success()).unwrap_or(false) {
        let s = git_cmd()
            .args(["-C"])
            .arg(source.path())
            .args(["push"])
            .arg(bare.path())
            .arg("HEAD:refs/heads/main")
            .status()
            .unwrap();
        assert!(s.success(), "failed to push new commit to bare repo");
    }

    // Re-run bootstrap: should pull the new commit.
    rdm()
        .arg("bootstrap")
        .arg("--plan-repo")
        .arg(bare.path())
        .arg("--path")
        .arg(&target)
        .assert()
        .success()
        .stdout(predicate::str::contains("Fast-forwarded"));
}

#[test]
fn bootstrap_without_init_fails_on_non_plan_repo() {
    let bare = make_empty_bare_repo();
    let target_parent = TempDir::new().unwrap();
    let target = target_parent.path().join("plan");

    rdm()
        .arg("bootstrap")
        .arg("--plan-repo")
        .arg(bare.path())
        .arg("--path")
        .arg(&target)
        .assert()
        .failure()
        .stderr(predicate::str::contains("not a plan repo"));
}

#[test]
fn bootstrap_with_init_initializes_empty_remote() {
    let bare = make_empty_bare_repo();
    let target_parent = TempDir::new().unwrap();
    let target = target_parent.path().join("plan");

    rdm()
        .arg("bootstrap")
        .arg("--plan-repo")
        .arg(bare.path())
        .arg("--path")
        .arg(&target)
        .arg("--init")
        .assert()
        .success()
        .stdout(predicate::str::contains("Plan repo ready at"));

    assert!(target.join("rdm.toml").exists());
}

#[test]
fn bootstrap_rejects_non_empty_non_git_target() {
    let (_source, bare) = make_plan_repo_with_bare();
    let target_parent = TempDir::new().unwrap();
    let target = target_parent.path().join("plan");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join("blocker.txt"), "hi").unwrap();

    rdm()
        .arg("bootstrap")
        .arg("--plan-repo")
        .arg(bare.path())
        .arg("--path")
        .arg(&target)
        .assert()
        .failure()
        .stderr(predicate::str::contains("not empty"));
}

#[test]
fn bootstrap_with_branch_checks_out_that_branch() {
    let (source, bare) = make_plan_repo_with_bare();

    // Add a feature branch to the source and push it to the bare.
    let s = git_cmd()
        .args(["-C"])
        .arg(source.path())
        .args(["checkout", "-b", "feature-x"])
        .status()
        .unwrap();
    assert!(s.success());
    std::fs::write(source.path().join("feature.txt"), "x").unwrap();
    let s = git_cmd()
        .args(["-C"])
        .arg(source.path())
        .args(["add", "feature.txt"])
        .status()
        .unwrap();
    assert!(s.success());
    let s = git_cmd()
        .args(["-C"])
        .arg(source.path())
        .args(["commit", "-m", "feature"])
        .status()
        .unwrap();
    assert!(s.success());
    let s = git_cmd()
        .args(["-C"])
        .arg(source.path())
        .args(["push"])
        .arg(bare.path())
        .arg("feature-x:refs/heads/feature-x")
        .status()
        .unwrap();
    assert!(s.success());

    let target_parent = TempDir::new().unwrap();
    let target = target_parent.path().join("plan");

    rdm()
        .arg("bootstrap")
        .arg("--plan-repo")
        .arg(bare.path())
        .arg("--path")
        .arg(&target)
        .arg("--branch")
        .arg("feature-x")
        .assert()
        .success();

    assert!(target.join("feature.txt").exists());
}

// ----------------------------------------------------------------------------
// Token handling
// ----------------------------------------------------------------------------

const SECRET: &str = "HUNTER2SECRETTOKENVALUE";

#[test]
fn bootstrap_token_flag_is_not_echoed_on_failure() {
    let target_parent = TempDir::new().unwrap();
    let target = target_parent.path().join("plan");

    let output = rdm()
        .arg("bootstrap")
        .arg("--plan-repo")
        .arg("https://127.0.0.1:1/foo.git")
        .arg("--token")
        .arg(SECRET)
        .arg("--path")
        .arg(&target)
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "expected failure");
    assert!(!stdout.contains(SECRET), "token leaked to stdout: {stdout}");
    assert!(!stderr.contains(SECRET), "token leaked to stderr: {stderr}");
}

#[test]
fn bootstrap_token_env_is_not_echoed_on_failure() {
    let target_parent = TempDir::new().unwrap();
    let target = target_parent.path().join("plan");

    let output = rdm()
        .env("RDM_PLAN_REPO_TOKEN", SECRET)
        .arg("bootstrap")
        .arg("--plan-repo")
        .arg("https://127.0.0.1:1/foo.git")
        .arg("--path")
        .arg(&target)
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "expected failure");
    assert!(!stdout.contains(SECRET), "token leaked to stdout: {stdout}");
    assert!(!stderr.contains(SECRET), "token leaked to stderr: {stderr}");
}

#[test]
fn bootstrap_rejects_token_over_plain_http() {
    let target_parent = TempDir::new().unwrap();
    let target = target_parent.path().join("plan");

    rdm()
        .arg("bootstrap")
        .arg("--plan-repo")
        .arg("http://example.com/foo.git")
        .arg("--token")
        .arg(SECRET)
        .arg("--path")
        .arg(&target)
        .assert()
        .failure()
        .stderr(predicate::str::contains("http://"));
}

#[test]
fn bootstrap_ssh_url_with_token_warns_and_proceeds() {
    let target_parent = TempDir::new().unwrap();
    let target = target_parent.path().join("plan");

    let output = rdm()
        .arg("bootstrap")
        .arg("--plan-repo")
        .arg("git@example.invalid:foo/bar.git")
        .arg("--token")
        .arg(SECRET)
        .arg("--path")
        .arg(&target)
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("ignored for SSH URLs"),
        "expected SSH warning, got: {stderr}"
    );
    assert!(!stderr.contains(SECRET), "token leaked to stderr: {stderr}");
}

// ----------------------------------------------------------------------------
// Doctor
// ----------------------------------------------------------------------------

#[test]
fn doctor_reports_missing_plan_repo_url() {
    let xdg = TempDir::new().unwrap();
    let output = Command::cargo_bin("rdm")
        .unwrap()
        .env("XDG_CONFIG_HOME", xdg.path())
        .env("HOME", xdg.path()) // avoid picking up developer's real config
        .env_remove("RDM_PLAN_REPO")
        .env_remove("RDM_PLAN_REPO_TOKEN")
        .arg("bootstrap")
        .arg("doctor")
        .output()
        .unwrap();

    assert!(!output.status.success());
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("no plan repo URL"),
        "expected missing-URL message, got: {combined}"
    );
}

#[test]
fn doctor_reports_missing_token_for_github_https_url() {
    let xdg = TempDir::new().unwrap();
    let output = Command::cargo_bin("rdm")
        .unwrap()
        .env("XDG_CONFIG_HOME", xdg.path())
        .env("HOME", xdg.path())
        .env("RDM_PLAN_REPO", "https://github.com/example/private.git")
        .env_remove("RDM_PLAN_REPO_TOKEN")
        .arg("bootstrap")
        .arg("doctor")
        .output()
        .unwrap();

    assert!(!output.status.success());
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("no token"),
        "expected missing-token message, got: {combined}"
    );
    assert!(
        combined.contains("RDM_PLAN_REPO_TOKEN"),
        "expected fix referencing env var, got: {combined}"
    );
}

#[test]
fn doctor_ssh_url_reports_no_token_needed() {
    let xdg = TempDir::new().unwrap();
    let output = Command::cargo_bin("rdm")
        .unwrap()
        .env("XDG_CONFIG_HOME", xdg.path())
        .env("HOME", xdg.path())
        .env("RDM_PLAN_REPO", "git@github.com:example/private.git")
        .env_remove("RDM_PLAN_REPO_TOKEN")
        .arg("bootstrap")
        .arg("doctor")
        .output()
        .unwrap();

    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("SSH URL"),
        "expected SSH-no-token message, got: {combined}"
    );
}

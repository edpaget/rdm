use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn rdm() -> Command {
    let mut cmd = Command::cargo_bin("rdm").unwrap();
    // Isolate from host global config (e.g. default_format = "json").
    cmd.env("XDG_CONFIG_HOME", "/dev/null/nonexistent");
    cmd
}

/// Initialize a plan repo with an initial git commit.
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
        .args(["commit", "-m", "seed: init plan repo"])
        .assert()
        .success();
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
fn pull_non_conflicting_concurrent_edits() {
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

    // Push a commit to bare from a separate clone (different file)
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

    // Make a local commit (different file from remote)
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

    // Pull should succeed with a clean merge (different files)
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("remote")
        .arg("pull")
        .arg("origin")
        .assert()
        .success()
        .stdout(predicate::str::contains("Pulled"));

    // Both files should exist
    assert!(dir.path().join("local-change.md").exists());
    assert!(dir.path().join("remote.md").exists());

    let _ = bare_dir;
}

#[test]
fn pull_conflicting_shows_items() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    // Create initial file and commit it
    std::fs::write(dir.path().join("shared.md"), "original").unwrap();
    git_cmd()
        .args(["add", "."])
        .current_dir(dir.path())
        .output()
        .unwrap();
    git_cmd()
        .args(["commit", "-m", "add shared file"])
        .current_dir(dir.path())
        .output()
        .unwrap();

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

    // Push a conflicting change from a clone
    let clone_dir = tempfile::TempDir::new().unwrap();
    git_cmd()
        .args(["clone"])
        .arg(bare_dir.path())
        .arg(clone_dir.path())
        .output()
        .unwrap();
    std::fs::write(clone_dir.path().join("shared.md"), "remote change").unwrap();
    git_cmd()
        .args(["add", "."])
        .current_dir(clone_dir.path())
        .output()
        .unwrap();
    git_cmd()
        .args(["commit", "-m", "remote conflict"])
        .current_dir(clone_dir.path())
        .output()
        .unwrap();
    git_cmd()
        .args(["push"])
        .current_dir(clone_dir.path())
        .output()
        .unwrap();

    // Make a local conflicting change
    std::fs::write(dir.path().join("shared.md"), "local change").unwrap();
    git_cmd()
        .args(["add", "."])
        .current_dir(dir.path())
        .output()
        .unwrap();
    git_cmd()
        .args(["commit", "-m", "local conflict"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    // Pull should fail with conflict info
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("remote")
        .arg("pull")
        .arg("origin")
        .assert()
        .failure()
        .stderr(predicate::str::contains("conflict"))
        .stderr(predicate::str::contains("shared.md"));

    let _ = bare_dir;
}

#[test]
fn conflicts_command_lists_unresolved() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    // Create initial file
    std::fs::write(dir.path().join("shared.md"), "original").unwrap();
    git_cmd()
        .args(["add", "."])
        .current_dir(dir.path())
        .output()
        .unwrap();
    git_cmd()
        .args(["commit", "-m", "add shared file"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    let bare_dir = setup_bare_remote(&dir, "origin");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("remote")
        .arg("fetch")
        .arg("origin")
        .assert()
        .success();

    // Create conflicting changes
    let clone_dir = tempfile::TempDir::new().unwrap();
    git_cmd()
        .args(["clone"])
        .arg(bare_dir.path())
        .arg(clone_dir.path())
        .output()
        .unwrap();
    std::fs::write(clone_dir.path().join("shared.md"), "remote change").unwrap();
    git_cmd()
        .args(["add", "."])
        .current_dir(clone_dir.path())
        .output()
        .unwrap();
    git_cmd()
        .args(["commit", "-m", "remote conflict"])
        .current_dir(clone_dir.path())
        .output()
        .unwrap();
    git_cmd()
        .args(["push"])
        .current_dir(clone_dir.path())
        .output()
        .unwrap();

    std::fs::write(dir.path().join("shared.md"), "local change").unwrap();
    git_cmd()
        .args(["add", "."])
        .current_dir(dir.path())
        .output()
        .unwrap();
    git_cmd()
        .args(["commit", "-m", "local conflict"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    // Pull to create conflict
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("remote")
        .arg("pull")
        .arg("origin")
        .assert()
        .failure();

    // rdm conflicts should list the conflict
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("conflicts")
        .assert()
        .success()
        .stdout(predicate::str::contains("conflict"))
        .stdout(predicate::str::contains("shared.md"));

    let _ = bare_dir;
}

#[test]
fn resolve_completes_merge() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    // Create initial file
    std::fs::write(dir.path().join("shared.md"), "original").unwrap();
    git_cmd()
        .args(["add", "."])
        .current_dir(dir.path())
        .output()
        .unwrap();
    git_cmd()
        .args(["commit", "-m", "add shared file"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    let bare_dir = setup_bare_remote(&dir, "origin");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("remote")
        .arg("fetch")
        .arg("origin")
        .assert()
        .success();

    // Create conflicting changes
    let clone_dir = tempfile::TempDir::new().unwrap();
    git_cmd()
        .args(["clone"])
        .arg(bare_dir.path())
        .arg(clone_dir.path())
        .output()
        .unwrap();
    std::fs::write(clone_dir.path().join("shared.md"), "remote change").unwrap();
    git_cmd()
        .args(["add", "."])
        .current_dir(clone_dir.path())
        .output()
        .unwrap();
    git_cmd()
        .args(["commit", "-m", "remote conflict"])
        .current_dir(clone_dir.path())
        .output()
        .unwrap();
    git_cmd()
        .args(["push"])
        .current_dir(clone_dir.path())
        .output()
        .unwrap();

    std::fs::write(dir.path().join("shared.md"), "local change").unwrap();
    git_cmd()
        .args(["add", "."])
        .current_dir(dir.path())
        .output()
        .unwrap();
    git_cmd()
        .args(["commit", "-m", "local conflict"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    // Pull to create conflict
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("remote")
        .arg("pull")
        .arg("origin")
        .assert()
        .failure();

    // Resolve the conflict
    std::fs::write(dir.path().join("shared.md"), "resolved content").unwrap();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("resolve")
        .arg("shared.md")
        .assert()
        .success()
        .stdout(predicate::str::contains("Resolved"))
        .stdout(predicate::str::contains("merge complete"));

    // Conflicts should show no merge
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("conflicts")
        .assert()
        .success()
        .stdout(predicate::str::contains("No merge in progress"));

    let _ = bare_dir;
}

#[test]
fn conflicts_no_merge_in_progress() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("conflicts")
        .assert()
        .success()
        .stdout(predicate::str::contains("No merge in progress"));
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
fn pull_with_conflicting_index_md_auto_resolves_via_merge_driver() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    // Seed a shared project before diverging so both sides touch the same
    // project-level INDEX.md as well as the root one.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "create", "demo"])
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["commit", "-m", "add demo project"])
        .assert()
        .success();

    let bare_dir = setup_bare_remote(&dir, "origin");

    // Fetch to establish tracking refs.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("remote")
        .arg("fetch")
        .arg("origin")
        .assert()
        .success();

    // Push a divergent roadmap from a separate clone.
    let clone_dir = tempfile::TempDir::new().unwrap();
    git_cmd()
        .args(["clone"])
        .arg(bare_dir.path())
        .arg(clone_dir.path())
        .output()
        .unwrap();
    rdm()
        .arg("--root")
        .arg(clone_dir.path())
        .args(["roadmap", "create", "clone-roadmap", "--project", "demo"])
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(clone_dir.path())
        .args(["commit", "-m", "add clone-roadmap"])
        .assert()
        .success();
    git_cmd()
        .args(["push"])
        .current_dir(clone_dir.path())
        .output()
        .unwrap();

    // Make a locally-divergent roadmap that also touches INDEX.md and
    // projects/demo/INDEX.md.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["roadmap", "create", "local-roadmap", "--project", "demo"])
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["commit", "-m", "add local-roadmap"])
        .assert()
        .success();

    // The merge driver is configured as a bare `rdm index ...` command, so
    // the `rdm` binary must be resolvable on PATH for git to invoke it.
    let bin_dir = std::path::Path::new(env!("CARGO_BIN_EXE_rdm"))
        .parent()
        .unwrap();
    let path = format!(
        "{}:{}",
        bin_dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );

    // Re-fetch so the tracking ref sees the just-pushed clone-roadmap commit,
    // then merge directly via `git merge` (bypassing `rdm remote pull`'s own
    // porcelain, which — independent of the merge driver, and unrelated to
    // this phase — always regenerates INDEX.md again after a successful pull
    // and flushes it straight to disk without re-staging into git; that step
    // would otherwise mask what we're isolating here). This exercises AC1-3
    // directly: the auto-installed driver must resolve the INDEX.md/
    // projects/demo/INDEX.md conflict without leaving the merge blocked.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .env("PATH", &path)
        .arg("remote")
        .arg("fetch")
        .arg("origin")
        .assert()
        .success();

    let merge_output = git_cmd()
        .env("PATH", &path)
        .args(["merge", "--no-edit", "origin/main"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(
        merge_output.status.success(),
        "expected the merge driver to auto-resolve the INDEX.md conflict, got: {}",
        String::from_utf8_lossy(&merge_output.stderr)
    );

    // Consistency check: with the driver installed, the working tree is
    // self-consistent with what git just committed — no leftover conflict
    // markers, no diff between the merge commit and disk. Note this
    // assertion is NOT by itself a regression guard for the %A/%P design:
    // in this scenario (both sides create brand-new roadmaps) the driver's
    // regeneration coincides with the stale "ours" blob because the sibling
    // roadmap file isn't materialized when the driver runs, so a bare
    // `driver = rdm index` would produce the same clean status here. The
    // %A/%P wiring is guarded by `init_writes_merge_driver_git_config`
    // (rdm-store-git) asserting the placeholders are present, and by the
    // `index_merge_output_*` CLI tests exercising the flags behaviorally.
    let status = git_cmd()
        .args(["status", "--porcelain"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    let status_out = String::from_utf8_lossy(&status.stdout);
    assert!(
        status_out.trim().is_empty(),
        "expected a clean working tree immediately after the merge driver ran \
         (no stale %A content), got: {status_out}"
    );

    // The merge driver regenerates from whatever's on disk at the moment it
    // runs; because git's merge machinery doesn't guarantee every
    // concurrently-merged sibling file is materialized in the working tree
    // before drivers run for other conflicting paths, the driver's own
    // regeneration can be transiently stale immediately post-merge (a known,
    // accepted limitation — see the plan's "mid-merge sibling-content
    // caveat"). A follow-up `rdm index` (exactly what `rdm remote pull`
    // already does after every successful pull) always converges to the
    // fully correct state.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("index")
        .assert()
        .success();

    let project_index = std::fs::read_to_string(dir.path().join("projects/demo/INDEX.md")).unwrap();
    assert!(
        project_index.contains("clone-roadmap"),
        "expected clone-roadmap in the fully-converged project index, got: {project_index}"
    );
    assert!(
        project_index.contains("local-roadmap"),
        "expected local-roadmap in the fully-converged project index, got: {project_index}"
    );

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

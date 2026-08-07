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

/// The sentinel line committed into the local side's `projects/demo/INDEX.md`,
/// making the committed `ours` blob deliberately STALE relative to its own
/// source markdown.
///
/// This is what makes the merge-driver tests below discriminating: git
/// pre-loads `%A` with the `ours` content, so a driver that regenerates the
/// file on disk but never writes `%A` (i.e. one missing `--merge-output %A
/// --merge-path %P`) leaves this sentinel in the merge result, while the real
/// driver's regeneration cannot contain it.
const STALE_OURS_SENTINEL: &str = "<!-- STALE-OURS-SENTINEL -->";

/// Seeds a two-sided `projects/demo/INDEX.md` conflict in which the local
/// side's committed index is deliberately stale.
///
/// Returns the local plan repo, the bare remote (kept alive by the caller),
/// and a `PATH` with the test `rdm` binary prepended — git spawns the merge
/// driver as a plain subprocess and must resolve `rdm` from `PATH`.
fn seed_stale_ours_index_conflict() -> (TempDir, TempDir, String) {
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

    // Now make the local side's COMMITTED index stale relative to its own
    // source markdown, by appending a sentinel line and committing that
    // tampered file. `rdm commit` commits the working tree verbatim and
    // performs no regeneration, so the sentinel survives into HEAD.
    //
    // Both sides still modify the same index, so the conflict and the driver
    // invocation are unchanged — but `ours` is no longer what a regeneration
    // would produce, which is exactly what the assertions below rely on.
    let index_path = dir.path().join("projects/demo/INDEX.md");
    let mut index = std::fs::read_to_string(&index_path).unwrap();
    index.push_str(&format!("\n{STALE_OURS_SENTINEL}\n"));
    std::fs::write(&index_path, index).unwrap();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["commit", "-m", "chore: tamper with the committed index"])
        .assert()
        .success();
    let committed = git_cmd()
        .args(["show", "HEAD:projects/demo/INDEX.md"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&committed.stdout).contains(STALE_OURS_SENTINEL),
        "the stale-ours fixture must actually be committed"
    );

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

    // Re-fetch so the tracking ref sees the just-pushed clone-roadmap commit.
    // The merge itself is left to the caller, which merges directly via
    // `git merge` (bypassing `rdm remote pull`'s own porcelain, which —
    // independent of the merge driver — always regenerates INDEX.md again
    // after a successful pull and flushes it straight to disk without
    // re-staging into git; that step would otherwise mask what is isolated
    // here).
    rdm()
        .arg("--root")
        .arg(dir.path())
        .env("PATH", &path)
        .arg("remote")
        .arg("fetch")
        .arg("origin")
        .assert()
        .success();

    (dir, bare_dir, path)
}

/// Runs `git merge origin/main` with the driver resolvable on `PATH` and
/// returns the merge commit's `projects/demo/INDEX.md` blob.
fn merge_and_read_merged_index(dir: &TempDir, path: &str) -> String {
    let merge_output = git_cmd()
        .env("PATH", path)
        .args(["merge", "--no-edit", "origin/main"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(
        merge_output.status.success(),
        "expected the merge driver to auto-resolve the INDEX.md conflict, got: {}",
        String::from_utf8_lossy(&merge_output.stderr)
    );

    let blob = git_cmd()
        .args(["show", "HEAD:projects/demo/INDEX.md"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(blob.status.success());
    String::from_utf8_lossy(&blob.stdout).to_string()
}

#[test]
fn pull_with_conflicting_index_md_auto_resolves_via_merge_driver() {
    let (dir, bare_dir, path) = seed_stale_ours_index_conflict();

    let merged = merge_and_read_merged_index(&dir, &path);

    // THE discriminating assertion. This blob is exactly the `%A` content git
    // copied back into the merge result, so it can only be sentinel-free if
    // the driver wrote `%A` via `--merge-output %A --merge-path %P`. A bare
    // `driver = rdm index` regenerates on disk but leaves `%A` holding the
    // stale `ours` blob — see the negative control below.
    assert!(
        !merged.contains(STALE_OURS_SENTINEL),
        "the merge commit must hold the driver's regeneration, not the stale \
         `ours` blob, got: {merged}"
    );
    assert!(
        merged.contains("local-roadmap"),
        "the driver's regeneration must still carry the local roadmap, got: {merged}"
    );

    // With the driver installed, the working tree is self-consistent with what
    // git just committed — no leftover conflict markers, no diff between the
    // merge commit and disk. Under the stale-ours fixture this is
    // discriminating too: with a bare driver the committed blob is the stale
    // `ours` while disk holds the regenerated content, so the tree is dirty.
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

/// Negative control for the test above: proves its sentinel assertion is not
/// vacuous by removing ONLY the `%A`/`%P` wiring from the driver command.
///
/// `--root .` is deliberately kept — dropping it too would make this test pass
/// for the wrong reason (root resolution rather than the merge-output wiring).
#[test]
fn bare_index_merge_driver_leaves_the_stale_ours_blob_in_the_merge_result() {
    let (dir, bare_dir, path) = seed_stale_ours_index_conflict();

    // Strip `--merge-output %A --merge-path %P` from the installed driver.
    let config_path = dir.path().join(".git").join("config");
    let config = std::fs::read_to_string(&config_path).unwrap();
    assert!(
        config.contains("driver = rdm --root . index --merge-output %A --merge-path %P"),
        "expected the shipped driver command, got: {config}"
    );
    let bare = config.replace(
        "driver = rdm --root . index --merge-output %A --merge-path %P",
        "driver = rdm --root . index",
    );
    std::fs::write(&config_path, &bare).unwrap();

    let merged = merge_and_read_merged_index(&dir, &path);

    assert!(
        merged.contains(STALE_OURS_SENTINEL),
        "without --merge-output %A --merge-path %P the merge result must be \
         the stale `ours` blob — if this ever stops holding, the positive \
         test's assertion has stopped discriminating, got: {merged}"
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

/// End-to-end guard for the pull carve-out: rdm's own backfilled
/// `.gitattributes` must never wedge a diverged `rdm remote pull`.
///
/// Before the carve-out this refused with "cannot pull with uncommitted
/// changes — commit or discard first", and the instruction was unfollowable:
/// `rdm discard --force` re-ensures the mapping, so the tree could never come
/// clean again.
#[test]
fn diverged_pull_is_not_wedged_by_the_backfilled_merge_mapping() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    // Make this a repo that predates the mapping: drop it from HEAD entirely.
    git_cmd()
        .args(["rm", "--cached", "--quiet", ".gitattributes"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    std::fs::remove_file(dir.path().join(".gitattributes")).unwrap();
    git_cmd()
        .args(["commit", "-q", "-m", "legacy: drop .gitattributes"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    let bare_dir = setup_bare_remote(&dir, "origin");
    git_cmd()
        .args(["push", "-q", "origin", "HEAD:refs/heads/main"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    // Remote moves ahead on its own file.
    let clone_dir = TempDir::new().unwrap();
    git_cmd()
        .args(["clone", "-q"])
        .arg(bare_dir.path())
        .arg(clone_dir.path())
        .output()
        .unwrap();
    std::fs::write(clone_dir.path().join("remote.md"), "remote").unwrap();
    for args in [
        vec!["add", "."],
        vec!["commit", "-q", "-m", "remote work"],
        vec!["push", "-q"],
    ] {
        git_cmd()
            .args(&args)
            .current_dir(clone_dir.path())
            .output()
            .unwrap();
    }

    // Local moves ahead too, so the pull takes the diverged merge path.
    std::fs::write(dir.path().join("local.md"), "local").unwrap();
    git_cmd()
        .args(["add", "."])
        .current_dir(dir.path())
        .output()
        .unwrap();
    git_cmd()
        .args(["commit", "-q", "-m", "local work"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    // Any rdm command reopens the store and backfills the mapping, dirtying a
    // tree the user never touched.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("status")
        .assert()
        .success();
    assert!(dir.path().join(".gitattributes").exists());

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["remote", "pull", "origin"])
        .assert()
        .success();

    assert!(dir.path().join("remote.md").exists());
    let attrs = std::fs::read_to_string(dir.path().join(".gitattributes")).unwrap();
    assert!(
        attrs.contains("merge=rdm-index"),
        "the pull must leave the mapping installed, got: {attrs}"
    );

    let _ = bare_dir;
}

/// The fast-forward-only sibling of the guard above, and the commoner case: a
/// legacy repo that is merely *behind* a peer which has already committed the
/// mapping.
///
/// This path used to skip the working-tree guard entirely, so the backfilled
/// *untracked* `.gitattributes` collided with the incoming committed one and
/// git refused with "the following untracked working tree files would be
/// overwritten by merge". Like the diverged case, it was unrecoverable through
/// the CLI: `rdm discard --force` reinstalls the file.
#[test]
fn fast_forward_pull_is_not_wedged_by_the_backfilled_merge_mapping() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    // Make this a repo that predates the mapping: drop it from HEAD entirely.
    git_cmd()
        .args(["rm", "--cached", "--quiet", ".gitattributes"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    std::fs::remove_file(dir.path().join(".gitattributes")).unwrap();
    git_cmd()
        .args(["commit", "-q", "-m", "legacy: drop .gitattributes"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    let bare_dir = setup_bare_remote(&dir, "origin");
    git_cmd()
        .args(["push", "-q", "origin", "HEAD:refs/heads/main"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    // The remote moves ahead and commits its own mapping, exactly as any peer's
    // `rdm commit` does once the backfill has shipped.
    let clone_dir = TempDir::new().unwrap();
    git_cmd()
        .args(["clone", "-q"])
        .arg(bare_dir.path())
        .arg(clone_dir.path())
        .output()
        .unwrap();
    std::fs::write(
        clone_dir.path().join(".gitattributes"),
        "INDEX.md merge=rdm-index\n**/INDEX.md merge=rdm-index\n",
    )
    .unwrap();
    std::fs::write(clone_dir.path().join("remote.md"), "remote").unwrap();
    for args in [
        vec!["add", "."],
        vec!["commit", "-q", "-m", "remote work"],
        vec!["push", "-q"],
    ] {
        git_cmd()
            .args(&args)
            .current_dir(clone_dir.path())
            .output()
            .unwrap();
    }

    // The local side stays put, so the pull takes the fast-forward-only path.
    // Any rdm command reopens the store and backfills the mapping as an
    // untracked file, right where the incoming commit wants to write.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("status")
        .assert()
        .success();
    let porcelain = git_cmd()
        .args(["status", "--porcelain"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&porcelain.stdout).contains("?? .gitattributes"),
        "the backfill must leave an untracked .gitattributes, or this proves nothing"
    );

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["remote", "pull", "origin"])
        .assert()
        .success();

    assert!(dir.path().join("remote.md").exists());
    let attrs = std::fs::read_to_string(dir.path().join(".gitattributes")).unwrap();
    assert!(
        attrs.contains("merge=rdm-index"),
        "the pull must leave the mapping installed, got: {attrs}"
    );

    let _ = bare_dir;
}

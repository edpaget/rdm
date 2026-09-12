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
    // Commit the rdm.toml change so it's part of HEAD. `--all` is
    // load-bearing: `set_default_remote` is a raw `fs::write` outside rdm, so
    // the write belongs to no changeset and the scoped default deliberately
    // refuses to sweep it.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("commit")
        .arg("--all")
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
fn remote_pull_writes_no_index() {
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

    // Nothing in rdm generates an index any more, so a pull that never had
    // one on either side must not conjure one either.
    assert!(
        !dir.path().join("INDEX.md").exists(),
        "pull must not write an INDEX.md that neither side had"
    );

    let _ = bare_dir;
}

/// Writes literal `INDEX.md` + `projects/<project>/INDEX.md` content naming
/// `roadmap`, standing in for the shape the retired `rdm index` command used
/// to produce. Written directly to disk rather than through rdm, since
/// nothing in rdm writes this content any more — the caller is responsible
/// for sweeping these untracked/modified paths into a commit (e.g. via
/// `rdm commit --all`).
fn write_fake_index(dir: &std::path::Path, project: &str, roadmap: &str) {
    std::fs::write(
        dir.join("INDEX.md"),
        format!("# Plan Index\n\n- [{project}](projects/{project}/INDEX.md)\n"),
    )
    .unwrap();
    std::fs::write(
        dir.join(format!("projects/{project}/INDEX.md")),
        format!("# Project: {project}\n\n- {roadmap}\n"),
    )
    .unwrap();
}

/// Seeds a two-sided `projects/demo/INDEX.md` conflict in a LEGACY repo: HEAD
/// tracks a `.gitattributes` carrying both `merge=rdm-index` lines (as every
/// plan repo an older rdm touched does), and no merge driver is configured.
///
/// Returns the local plan repo and the bare remote (kept alive by the caller).
/// The caller runs the merge itself, directly via `git merge` rather than
/// `rdm remote pull`, which no longer touches `INDEX.md` at all.
fn seed_legacy_index_conflict() -> (TempDir, TempDir) {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    // rdm no longer writes `.gitattributes`, so the legacy shape has to be
    // seeded by hand. This is the file that outlives the change on every
    // existing plan repo, and the whole point of the tests below.
    std::fs::write(
        dir.path().join(".gitattributes"),
        "INDEX.md merge=rdm-index\n**/INDEX.md merge=rdm-index\n",
    )
    .unwrap();

    // Seed a shared project before diverging so both sides touch the same
    // project-level INDEX.md as well as the root one.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "create", "demo"])
        .assert()
        .success();
    // Nothing in rdm writes an index any more, so the conflicting indexes
    // have to be produced explicitly — otherwise no `INDEX.md` ever diverges
    // and the merge assertions below would pass vacuously.
    write_fake_index(dir.path(), "demo", "(no roadmaps yet)");
    // `--all` sweeps the hand-written `.gitattributes` and INDEX.md files,
    // which belong to no changeset.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["commit", "--all", "-m", "add demo project"])
        .assert()
        .success();
    let tracked = git_cmd()
        .args(["ls-tree", "--name-only", "HEAD"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&tracked.stdout).contains(".gitattributes"),
        "the legacy fixture must track .gitattributes, or it proves nothing"
    );

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
    write_fake_index(clone_dir.path(), "demo", "clone-roadmap");
    rdm()
        .arg("--root")
        .arg(clone_dir.path())
        .args(["commit", "--all", "-m", "add clone-roadmap"])
        .assert()
        .success();
    git_cmd()
        .args(["push"])
        .current_dir(clone_dir.path())
        .output()
        .unwrap();

    // Make a locally-divergent roadmap that also touches INDEX.md and
    // projects/demo/INDEX.md — the same rows, so the merge really conflicts.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["roadmap", "create", "local-roadmap", "--project", "demo"])
        .assert()
        .success();
    write_fake_index(dir.path(), "demo", "local-roadmap");
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["commit", "--all", "-m", "add local-roadmap"])
        .assert()
        .success();

    // Re-fetch so the tracking ref sees the just-pushed clone-roadmap commit.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("remote")
        .arg("fetch")
        .arg("origin")
        .assert()
        .success();

    (dir, bare_dir)
}

/// Appends the `[merge "rdm-index"]` section an older rdm installed, whose
/// driver command names flags `rdm index` no longer accepts.
fn install_stale_driver_section(dir: &std::path::Path) {
    let config_path = dir.join(".git").join("config");
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&config_path)
        .unwrap();
    use std::io::Write;
    writeln!(
        file,
        "\n[merge \"rdm-index\"]\n\tname = rdm INDEX.md merge driver\n\tdriver = rdm --root . index --merge-output %A --merge-path %P"
    )
    .unwrap();
}

/// Runs `git merge --no-edit origin/main` with the test `rdm` binary on
/// `PATH` (so a configured driver, if any, really resolves) and returns the
/// exit status plus captured stderr.
fn merge_origin(dir: &TempDir) -> (std::process::ExitStatus, String) {
    let bin_dir = std::path::Path::new(env!("CARGO_BIN_EXE_rdm"))
        .parent()
        .unwrap();
    let path = format!(
        "{}:{}",
        bin_dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let out = git_cmd()
        .env("PATH", &path)
        .args(["merge", "--no-edit", "origin/main"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    (out.status, String::from_utf8_lossy(&out.stderr).to_string())
}

/// THE migration evidence for retiring the `rdm-index` merge driver.
///
/// Every existing plan repo will keep a tracked `.gitattributes` carrying
/// `merge=rdm-index` long after rdm stops configuring a driver for that name,
/// and this records — against a real `git merge`, not by reasoning — what git
/// then does: it silently falls back to its built-in three-way merge.
/// Measured on git 2.55.0: `CONFLICT (content)`, exit 1, ordinary
/// `<<<<<<< HEAD` / `>>>>>>>` markers in the file, `UU` in porcelain, and
/// NOTHING on stderr about the unknown driver.
///
/// That is why the stale `.gitattributes` line is left alone (it is a user
/// file, and it is inert), while the stale `.git/config` section is not — see
/// the control below.
#[test]
fn a_legacy_repo_merges_index_md_via_gits_builtin_three_way() {
    let (dir, bare_dir) = seed_legacy_index_conflict();
    let index_file = dir.path().join("projects/demo/INDEX.md");

    let (status, stderr) = merge_origin(&dir);

    assert!(
        !status.success(),
        "a genuine two-sided INDEX.md conflict must fail the merge"
    );
    let merged = std::fs::read_to_string(&index_file).unwrap();
    assert!(
        merged.contains("<<<<<<< HEAD") && merged.contains(">>>>>>>"),
        "git's built-in three-way merge must leave ordinary conflict markers, got: {merged}"
    );
    assert!(
        merged.contains("local-roadmap") && merged.contains("clone-roadmap"),
        "both sides' rows must be present in the conflicted file, got: {merged}"
    );
    let porcelain = git_cmd()
        .args(["status", "--porcelain", "--", "projects/demo/INDEX.md"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&porcelain.stdout).starts_with("UU"),
        "the conflicted index must be UU, got: {}",
        String::from_utf8_lossy(&porcelain.stdout)
    );
    assert!(
        !stderr.to_lowercase().contains("driver"),
        "git must say nothing about the absent driver, got: {stderr}"
    );

    // The recovery path is real and one command long: `rdm resolve` marks
    // the file resolved and completes the merge, taking the content as the
    // user (or their merge tool) left it — nothing regenerates it afterward.
    std::fs::write(
        &index_file,
        "# Project: demo\n\n- local-roadmap\n- clone-roadmap\n",
    )
    .unwrap();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["resolve", "projects/demo/INDEX.md"])
        .assert()
        .success();

    let converged = std::fs::read_to_string(&index_file).unwrap();
    assert!(
        !converged.contains("<<<<<<<"),
        "the resolved index must carry no conflict markers, got: {converged}"
    );
    assert!(
        converged.contains("local-roadmap") && converged.contains("clone-roadmap"),
        "the resolved index must carry both sides' roadmaps exactly as the user \
         wrote them, got: {converged}"
    );

    let _ = bare_dir;
}

/// The non-vacuity control, and the reason the migration sweep in
/// `GitRepo::remove_rdm_index_driver_section` is load-bearing rather than
/// cosmetic.
///
/// With the stale `[merge "rdm-index"]` section left in `.git/config`, its
/// driver command now fails (`rdm index` no longer accepts
/// `--merge-output`/`--merge-path`), and git's response is far worse than the
/// fallback above: the merge result is the unmodified `ours` blob with NO
/// conflict markers, so the local side wins silently and the incoming rows
/// vanish. Measured on git 2.55.0 — and it fires on non-conflicting merges
/// too, which is why the section must be removed rather than tolerated.
#[test]
fn a_stale_driver_section_resolves_silently_to_ours() {
    let (dir, bare_dir) = seed_legacy_index_conflict();
    let index_file = dir.path().join("projects/demo/INDEX.md");
    let ours = std::fs::read_to_string(&index_file).unwrap();

    install_stale_driver_section(dir.path());

    let (status, _stderr) = merge_origin(&dir);

    assert!(!status.success(), "a failing driver reports a conflict");
    let merged = std::fs::read_to_string(&index_file).unwrap();
    assert!(
        !merged.contains("<<<<<<<"),
        "the damning part: no conflict markers at all, got: {merged}"
    );
    assert_eq!(
        merged, ours,
        "the merge result is the untouched `ours` blob — the incoming side is \
         silently dropped, which is why the sweep removes this section"
    );
    assert!(
        !merged.contains("clone-roadmap"),
        "the incoming roadmap must be absent, proving the silent loss"
    );

    let _ = bare_dir;
}

#[test]
fn status_with_fetch_flag() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    // Set default remote before cloning. `--all` is load-bearing:
    // `set_default_remote` is a raw `fs::write` outside rdm, so the write
    // belongs to no changeset and the scoped default deliberately refuses to
    // sweep it.
    set_default_remote(&dir, "origin");
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("commit")
        .arg("--all")
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

/// The retargeted form of the two deleted "not wedged by the backfilled merge
/// mapping" tests, covering BOTH pull paths in one place.
///
/// Their premise — rdm dirtying a legacy repo's tree on open, then needing a
/// carve-out to get past its own dirt — is gone with the writer. What must
/// stay true in its place is stronger and simpler: opening a legacy repo
/// authors nothing, so neither the diverged merge nor the fast-forward has
/// anything to trip over. If a writer ever comes back, the wedge comes back
/// with it and this goes red.
#[test]
fn a_legacy_repo_pulls_cleanly_because_rdm_authors_no_dirt() {
    for fast_forward in [false, true] {
        let dir = TempDir::new().unwrap();
        init_repo(&dir);

        let bare_dir = setup_bare_remote(&dir, "origin");
        git_cmd()
            .args(["push", "-q", "origin", "HEAD:refs/heads/main"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        // The remote moves ahead and commits a `.gitattributes` of its own,
        // exactly as a peer that hand-maintains the file does — landing right
        // where rdm's back-fill used to sit on the fast-forward path.
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

        if !fast_forward {
            // Local moves ahead too, so the pull takes the diverged path.
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
        }

        // Any rdm command reopens the store. It must author nothing.
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
        assert_eq!(
            String::from_utf8_lossy(&porcelain.stdout).trim(),
            "",
            "opening a legacy repo must author no dirt (fast_forward={fast_forward})"
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
            "the peer's committed .gitattributes must arrive by the pull, got: {attrs}"
        );

        let _ = bare_dir;
    }
}

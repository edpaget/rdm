//! Integration tests for `rdm_git::worktree` against real temp git repos.

use std::path::Path;
use std::process::Command;

use rdm_git::worktree::{self, ItemRef, RemoveOptions};
use tempfile::TempDir;

/// Run a git command in `dir` with isolated identity/env, asserting success.
fn git(dir: &Path, args: &[&str]) -> std::process::Output {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@test.com")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@test.com")
        .output()
        .expect("failed to run git");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

/// Create a project git repo with one initial commit on `main`.
fn init_project_repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    git(dir.path(), &["init", "-b", "main"]);
    std::fs::write(dir.path().join("README.md"), "# project").unwrap();
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["commit", "-m", "initial commit"]);
    dir
}

/// Create a *bare* canonical repo with one linked worktree beside it, modeling
/// the setup where the project's canonical repo has no working tree of its own.
///
/// Returns `(tempdir, bare_dir, linked_worktree)`; keep the tempdir alive for
/// the duration of the test.
fn init_bare_canonical() -> (TempDir, std::path::PathBuf, std::path::PathBuf) {
    let dir = TempDir::new().unwrap();
    // Seed a normal repo with one commit so the bare clone has a HEAD.
    let seed = dir.path().join("seed");
    std::fs::create_dir(&seed).unwrap();
    git(&seed, &["init", "-b", "main"]);
    std::fs::write(seed.join("README.md"), "# project").unwrap();
    git(&seed, &["add", "."]);
    git(&seed, &["commit", "-m", "initial commit"]);

    let bare = dir.path().join("canonical.git");
    git(
        dir.path(),
        &[
            "clone",
            "--bare",
            seed.to_str().unwrap(),
            bare.to_str().unwrap(),
        ],
    );

    let linked = dir.path().join("wt-feature");
    git(
        &bare,
        &["worktree", "add", linked.to_str().unwrap(), "-b", "feature"],
    );

    (dir, bare, linked)
}

fn task_item() -> ItemRef {
    ItemRef::parse("task/fix-bug").unwrap()
}

#[test]
fn current_reports_rdm_worktree_via_marker() {
    let repo = init_project_repo();
    let item = task_item();
    let info = worktree::add(repo.path(), &item, &item.branch_name(), None).unwrap();

    let cur = worktree::current(&info.path)
        .unwrap()
        .expect("inside the rdm worktree");
    assert_eq!(cur.item, "task/fix-bug");
    assert_eq!(cur.branch, "task/fix-bug");
    assert!(cur.rdm_managed, "marker present → rdm-managed");
    assert_eq!(
        cur.path.canonicalize().unwrap(),
        info.path.canonicalize().unwrap()
    );
}

#[test]
fn current_infers_item_from_branch_without_marker() {
    let repo = init_project_repo();
    // Check out an item-convention branch in the MAIN checkout (no marker).
    git(
        repo.path(),
        &["checkout", "-b", "phase/my-roadmap/phase-1-build"],
    );

    let cur = worktree::current(repo.path())
        .unwrap()
        .expect("on an item branch");
    assert_eq!(cur.item, "my-roadmap/phase-1-build");
    assert_eq!(cur.branch, "phase/my-roadmap/phase-1-build");
    assert!(!cur.rdm_managed, "no marker → inferred from branch");
}

#[test]
fn current_resolves_from_worktree_subdirectory() {
    let repo = init_project_repo();
    let item = task_item();
    let info = worktree::add(repo.path(), &item, &item.branch_name(), None).unwrap();

    // A nested subdirectory inside the worktree must still resolve the item —
    // `current` reads the marker from the toplevel, not from `cwd`.
    let subdir = info.path.join("src/nested");
    std::fs::create_dir_all(&subdir).unwrap();

    let cur = worktree::current(&subdir)
        .unwrap()
        .expect("a subdir of the worktree still resolves the item");
    assert_eq!(cur.item, "task/fix-bug");
    assert!(cur.rdm_managed);
    assert_eq!(
        cur.path.canonicalize().unwrap(),
        info.path.canonicalize().unwrap(),
        "path is the worktree toplevel, not the subdir"
    );
}

#[test]
fn current_returns_none_on_main_checkout() {
    let repo = init_project_repo();
    assert!(
        worktree::current(repo.path()).unwrap().is_none(),
        "main checkout on `main` is not an item context"
    );
}

#[test]
fn add_creates_worktree_and_marker() {
    let repo = init_project_repo();
    let item = task_item();
    let info = worktree::add(repo.path(), &item, &item.branch_name(), None).unwrap();

    assert!(info.created);
    assert_eq!(info.item, "task/fix-bug");
    assert_eq!(info.branch, "task/fix-bug");
    assert!(info.path.exists(), "worktree dir should exist");
    // Branch was created.
    let out = git(repo.path(), &["branch", "--list", "task/fix-bug"]);
    assert!(!out.stdout.is_empty());
}

#[test]
fn add_and_current_round_trip_a_roadmap_worktree() {
    let repo = init_project_repo();
    let item = ItemRef::parse("fix-worktree-review-firing").unwrap();
    assert_eq!(
        item,
        ItemRef::Roadmap {
            roadmap: "fix-worktree-review-firing".to_string()
        }
    );

    let info = worktree::add(repo.path(), &item, &item.branch_name(), None).unwrap();
    assert!(info.created);
    assert_eq!(info.item, "fix-worktree-review-firing");
    assert_eq!(info.branch, "roadmap/fix-worktree-review-firing");
    // Directory follows the `roadmap-<slug>` convention.
    assert!(
        info.path.ends_with("roadmap-fix-worktree-review-firing"),
        "unexpected worktree dir: {}",
        info.path.display()
    );

    // `current` reports the roadmap via the marker.
    let cur = worktree::current(&info.path)
        .unwrap()
        .expect("inside the roadmap worktree");
    assert_eq!(cur.item, "fix-worktree-review-firing");
    assert_eq!(cur.branch, "roadmap/fix-worktree-review-firing");
    assert!(cur.rdm_managed, "marker present → rdm-managed");
}

#[test]
fn add_roadmap_is_idempotent() {
    let repo = init_project_repo();
    let item = ItemRef::parse("fix-worktree-review-firing").unwrap();
    let first = worktree::add(repo.path(), &item, &item.branch_name(), None).unwrap();
    let second = worktree::add(repo.path(), &item, &item.branch_name(), None).unwrap();
    assert!(first.created, "first add creates the worktree");
    assert!(!second.created, "second add reuses it");
    assert_eq!(
        first.path.canonicalize().unwrap(),
        second.path.canonicalize().unwrap()
    );
    assert_eq!(second.item, "fix-worktree-review-firing");
    assert_eq!(second.branch, "roadmap/fix-worktree-review-firing");
}

#[test]
fn current_infers_roadmap_from_branch_without_marker() {
    let repo = init_project_repo();
    // Main checkout sitting on a `roadmap/<slug>` branch, no marker.
    git(repo.path(), &["checkout", "-b", "roadmap/my-roadmap"]);
    let cur = worktree::current(repo.path())
        .unwrap()
        .expect("roadmap branch is recognized");
    assert_eq!(cur.item, "my-roadmap");
    assert_eq!(cur.branch, "roadmap/my-roadmap");
    assert!(!cur.rdm_managed, "no marker → inferred from branch");
}

#[test]
fn add_is_idempotent() {
    let repo = init_project_repo();
    let item = task_item();
    let first = worktree::add(repo.path(), &item, &item.branch_name(), None).unwrap();
    let second = worktree::add(repo.path(), &item, &item.branch_name(), None).unwrap();

    assert!(first.created);
    assert!(!second.created, "second add must be a no-op hit");
    // Canonicalize: `add` computes the path itself, but an idempotent hit reads
    // it back from `git worktree list` (which resolves macOS /private symlinks).
    assert_eq!(
        first.path.canonicalize().unwrap(),
        second.path.canonicalize().unwrap()
    );
}

#[test]
fn list_returns_only_rdm_worktrees() {
    let repo = init_project_repo();
    let item = task_item();
    worktree::add(repo.path(), &item, &item.branch_name(), None).unwrap();

    // A non-rdm worktree created directly with git must be ignored. Use a
    // dedicated temp dir so parallel test runs never collide on the path.
    let other_parent = TempDir::new().unwrap();
    let other = other_parent.path().join("manual-wt");
    git(
        repo.path(),
        &["worktree", "add", other.to_str().unwrap(), "-b", "manual"],
    );

    let listed = worktree::list(repo.path()).unwrap();
    assert_eq!(listed.len(), 1, "only the rdm worktree should be listed");
    assert_eq!(listed[0].item, "task/fix-bug");
    assert!(!listed[0].dirty);
}

#[test]
fn list_reports_dirty() {
    let repo = init_project_repo();
    let item = task_item();
    let info = worktree::add(repo.path(), &item, &item.branch_name(), None).unwrap();

    std::fs::write(info.path.join("scratch.txt"), "wip").unwrap();
    let listed = worktree::list(repo.path()).unwrap();
    assert_eq!(listed.len(), 1);
    assert!(listed[0].dirty, "untracked file should mark worktree dirty");
}

#[test]
fn add_reuses_existing_branch() {
    let repo = init_project_repo();
    let item = task_item();
    let info = worktree::add(repo.path(), &item, &item.branch_name(), None).unwrap();

    // Manually remove the worktree but keep the branch.
    git(
        repo.path(),
        &["worktree", "remove", info.path.to_str().unwrap()],
    );
    assert!(!info.path.exists());

    // add must reuse the surviving branch instead of erroring.
    let again = worktree::add(repo.path(), &item, &item.branch_name(), None).unwrap();
    assert!(again.created);
    assert!(again.path.exists());
}

#[test]
fn remove_refuses_dirty_without_force() {
    let repo = init_project_repo();
    let item = task_item();
    let info = worktree::add(repo.path(), &item, &item.branch_name(), None).unwrap();
    std::fs::write(info.path.join("scratch.txt"), "wip").unwrap();

    let err = worktree::remove(repo.path(), "task/fix-bug", RemoveOptions::default()).unwrap_err();
    assert!(matches!(err, worktree::WorktreeError::Dirty(_)));
    assert!(info.path.exists(), "worktree must survive a refused remove");
}

#[test]
fn remove_force_clears_dirty_worktree() {
    let repo = init_project_repo();
    let item = task_item();
    let info = worktree::add(repo.path(), &item, &item.branch_name(), None).unwrap();
    std::fs::write(info.path.join("scratch.txt"), "wip").unwrap();

    worktree::remove(
        repo.path(),
        "task/fix-bug",
        RemoveOptions {
            force: true,
            delete_branch: false,
        },
    )
    .unwrap();
    assert!(!info.path.exists());
    assert!(worktree::list(repo.path()).unwrap().is_empty());
}

#[test]
fn remove_marker_disappears() {
    let repo = init_project_repo();
    let item = task_item();
    worktree::add(repo.path(), &item, &item.branch_name(), None).unwrap();
    assert_eq!(worktree::list(repo.path()).unwrap().len(), 1);

    worktree::remove(repo.path(), "task/fix-bug", RemoveOptions::default()).unwrap();
    assert!(worktree::list(repo.path()).unwrap().is_empty());
}

#[test]
fn remove_delete_branch_refuses_unmerged() {
    let repo = init_project_repo();
    let item = task_item();
    let info = worktree::add(repo.path(), &item, &item.branch_name(), None).unwrap();

    // Make a commit on the worktree's branch so it is unmerged into main.
    std::fs::write(info.path.join("feature.txt"), "work").unwrap();
    git(&info.path, &["add", "."]);
    git(&info.path, &["commit", "-m", "feature work"]);

    let err = worktree::remove(
        repo.path(),
        "task/fix-bug",
        RemoveOptions {
            force: false,
            delete_branch: true,
        },
    )
    .unwrap_err();
    assert!(matches!(err, worktree::WorktreeError::UnmergedBranch(_)));
}

#[test]
fn remove_delete_branch_force_removes_unmerged() {
    let repo = init_project_repo();
    let item = task_item();
    let info = worktree::add(repo.path(), &item, &item.branch_name(), None).unwrap();
    std::fs::write(info.path.join("feature.txt"), "work").unwrap();
    git(&info.path, &["add", "."]);
    git(&info.path, &["commit", "-m", "feature work"]);

    worktree::remove(
        repo.path(),
        "task/fix-bug",
        RemoveOptions {
            force: true,
            delete_branch: true,
        },
    )
    .unwrap();
    let out = git(repo.path(), &["branch", "--list", "task/fix-bug"]);
    assert!(out.stdout.is_empty(), "branch should be force-deleted");
}

#[test]
fn remove_unknown_item_errors() {
    let repo = init_project_repo();
    let err = worktree::remove(repo.path(), "task/nope", RemoveOptions::default()).unwrap_err();
    assert!(matches!(err, worktree::WorktreeError::NotFound(_)));
}

#[test]
fn discover_project_repo_finds_toplevel() {
    let repo = init_project_repo();
    let sub = repo.path().join("subdir");
    std::fs::create_dir(&sub).unwrap();
    let found = worktree::discover_project_repo(&sub).unwrap();
    // Canonicalize to sidestep macOS /private symlink differences.
    assert_eq!(
        found.canonicalize().unwrap(),
        repo.path().canonicalize().unwrap()
    );
}

#[test]
fn add_from_inside_linked_worktree_anchors_on_main_repo() {
    let repo = init_project_repo();
    // Create a linked worktree directly with git (not via rdm), checked out on a
    // fresh branch, so `cwd` inside it would otherwise resolve to its own root.
    let linked = repo.path().join("linked-wt");
    git(
        repo.path(),
        &[
            "worktree",
            "add",
            linked.to_str().unwrap(),
            "-b",
            "linked-branch",
        ],
    );

    // The crux of the fix: discovery from inside the linked worktree must resolve
    // the *main* working tree, not the linked worktree's own toplevel.
    let discovered = worktree::discover_project_repo(&linked).unwrap();
    assert_eq!(
        discovered.canonicalize().unwrap(),
        repo.path().canonicalize().unwrap(),
        "discovery from a linked worktree must anchor on the main repo"
    );

    // And `add` against that root must place the worktree as a sibling of the
    // main repo, never nested under the linked worktree.
    let item = task_item();
    let info = worktree::add(&discovered, &item, &item.branch_name(), None).unwrap();
    let parent = info.path.parent().unwrap();
    let expected_parent = repo.path().parent().unwrap().join(format!(
        "{}__worktrees",
        repo.path().file_name().unwrap().to_string_lossy()
    ));
    assert_eq!(
        parent.canonicalize().unwrap(),
        expected_parent.canonicalize().unwrap(),
        "worktree must be a sibling of the main repo"
    );
    assert!(
        !info
            .path
            .canonicalize()
            .unwrap()
            .starts_with(linked.canonicalize().unwrap()),
        "worktree must not be nested under the linked worktree"
    );
}

#[test]
fn add_list_remove_from_linked_worktree_of_bare_canonical() {
    let (_dir, bare, linked) = init_bare_canonical();

    // Discovery from inside a linked worktree of a bare canonical repo must
    // anchor on the bare dir (the directory we can run `git worktree` in).
    let repo = worktree::discover_project_repo(&linked).unwrap();
    assert_eq!(
        repo.canonicalize().unwrap(),
        bare.canonicalize().unwrap(),
        "discovery must anchor on the bare canonical repo"
    );

    let item = task_item();
    let info = worktree::add(&repo, &item, &item.branch_name(), None).unwrap();
    assert!(info.created);
    assert!(info.path.exists(), "worktree dir should exist");
    // The bare dir's `.git` suffix must not leak into the sibling worktree path.
    assert!(
        !info.path.to_string_lossy().contains(".git__worktrees"),
        "sibling worktree path must not contain a .git segment: {}",
        info.path.display()
    );
    let parent = info.path.parent().unwrap();
    assert_eq!(
        parent.file_name().unwrap().to_string_lossy(),
        "canonical__worktrees",
        "anchor name must strip the .git suffix"
    );

    let listed = worktree::list(&repo).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].item, "task/fix-bug");

    worktree::remove(&repo, "task/fix-bug", RemoveOptions::default()).unwrap();
    assert!(worktree::list(&repo).unwrap().is_empty());
}

#[test]
fn add_list_remove_from_inside_bare_dir() {
    let (_dir, bare, _linked) = init_bare_canonical();

    // Discovery invoked from inside the bare dir itself (no working tree) must
    // still resolve the repo rather than failing with "not a git repository".
    let repo = worktree::discover_project_repo(&bare).unwrap();
    assert_eq!(
        repo.canonicalize().unwrap(),
        bare.canonicalize().unwrap(),
        "discovery from inside the bare dir must return the bare dir"
    );

    let item = task_item();
    let info = worktree::add(&repo, &item, &item.branch_name(), None).unwrap();
    assert!(info.created);
    assert!(info.path.exists());

    let listed = worktree::list(&repo).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].item, "task/fix-bug");

    worktree::remove(&repo, "task/fix-bug", RemoveOptions::default()).unwrap();
    assert!(worktree::list(&repo).unwrap().is_empty());
}

#[test]
fn nonexistent_cwd_reports_actionable_error_not_git_missing() {
    let dir = TempDir::new().unwrap();
    let missing = dir.path().join("no-such-subdir");
    // Running a git command against a directory that does not exist yields ENOENT
    // from the spawn — the same error kind as a missing `git` binary. It must be
    // reported actionably, not misclassified as "git is not installed".
    let err = worktree::list(&missing).unwrap_err();
    assert!(
        !matches!(err, worktree::WorktreeError::GitMissing),
        "a missing cwd must not be misreported as a missing git install"
    );
    assert!(
        matches!(err, worktree::WorktreeError::NoSuchDirectory(_)),
        "expected NoSuchDirectory, got {err:?}"
    );
    assert!(err.to_string().contains("does not exist"));
}

#[test]
fn file_as_cwd_reports_no_such_directory() {
    let dir = TempDir::new().unwrap();
    // A path that exists but is a FILE, not a directory. Spawning git with it as
    // the cwd yields ENOENT — the same error kind as a missing `git` binary and a
    // missing directory. This pins the `is_dir()` discriminator: `exists()` alone
    // would misclassify a file-as-cwd as GitMissing.
    let file = dir.path().join("not-a-dir");
    std::fs::write(&file, "regular file").unwrap();
    let err = worktree::list(&file).unwrap_err();
    assert!(
        !matches!(err, worktree::WorktreeError::GitMissing),
        "a file used as cwd must not be misreported as a missing git install"
    );
    assert!(
        matches!(err, worktree::WorktreeError::NoSuchDirectory(_)),
        "expected NoSuchDirectory, got {err:?}"
    );
    assert!(err.to_string().contains("does not exist"));
}

#[test]
fn discover_project_repo_errors_outside_git() {
    let dir = TempDir::new().unwrap();
    let err = worktree::discover_project_repo(dir.path()).unwrap_err();
    assert!(matches!(err, worktree::WorktreeError::NotAGitRepo(_)));
}

#[test]
fn add_branches_from_explicit_base() {
    let repo = init_project_repo();
    // Create a second branch with an extra commit, then branch the worktree
    // from `main` explicitly while HEAD is on the other branch.
    git(repo.path(), &["checkout", "-b", "other"]);
    std::fs::write(repo.path().join("other.txt"), "x").unwrap();
    git(repo.path(), &["add", "."]);
    git(repo.path(), &["commit", "-m", "other-only commit"]);

    let item = task_item();
    let info = worktree::add(repo.path(), &item, &item.branch_name(), Some("main")).unwrap();

    // The worktree was based on main, so the other-branch file must be absent.
    assert!(
        !info.path.join("other.txt").exists(),
        "worktree based on main must not contain the other branch's file"
    );
}

#[test]
fn remove_by_path_works() {
    let repo = init_project_repo();
    let item = task_item();
    let info = worktree::add(repo.path(), &item, &item.branch_name(), None).unwrap();

    let path_str = info.path.to_string_lossy().to_string();
    worktree::remove(repo.path(), &path_str, RemoveOptions::default()).unwrap();
    assert!(!info.path.exists());
    assert!(worktree::list(repo.path()).unwrap().is_empty());
}

#[test]
fn remove_non_rdm_worktree_by_path_errors() {
    let repo = init_project_repo();
    // A worktree created directly with git carries no rdm marker.
    let other_parent = TempDir::new().unwrap();
    let other = other_parent.path().join("manual-wt");
    git(
        repo.path(),
        &["worktree", "add", other.to_str().unwrap(), "-b", "manual"],
    );

    let err = worktree::remove(
        repo.path(),
        other.to_str().unwrap(),
        RemoveOptions::default(),
    )
    .unwrap_err();
    assert!(matches!(err, worktree::WorktreeError::NotRdmWorktree(_)));
}

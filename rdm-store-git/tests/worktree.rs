//! Integration tests for `rdm_store_git::worktree` against real temp git repos.

use std::path::Path;
use std::process::Command;

use rdm_store_git::worktree::{self, ItemRef, RemoveOptions};
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

fn task_item() -> ItemRef {
    ItemRef::parse("task/fix-bug").unwrap()
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

//! Repo-agnostic git primitives shared across rdm.
//!
//! Everything here operates on a git repository identified by a filesystem
//! path — it neither knows nor cares whether that path is the **plan repo**
//! (`RDM_ROOT`, backed by the `GitStore` in `rdm-store-git`) or the
//! **project/code repo** rdm is invoked in. That neutrality is the whole point
//! of this crate: the plan-store backend in `rdm-store-git` delegates its
//! HEAD/log/branch reads here, while the project-repo concerns — review-history
//! reachability ([`is_ancestor_of_head_at`], [`is_ancestor_of_branch_at`]) and
//! the [`worktree`] lifecycle — live here too, so neither set leaks into the
//! other crate's API.
//!
//! All process spawning routes through a single private `git_command` spawner;
//! the path-taking [`run_git`]/[`run_git_at`] wrappers map the raw
//! [`std::io::Result`] into [`rdm_core::error::Error`].

#![warn(missing_docs)]

use std::path::{Path, PathBuf};

use rdm_core::error::{Error, Result};

mod process;
pub mod worktree;

/// Information about a single commit (its SHA and raw message).
///
/// Returned by [`head_commit_info_at`] and [`commit_messages_since_at`], and
/// re-exported from `rdm-store-git` as the return type of the corresponding
/// `GitRepo` methods.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadCommitInfo {
    /// The full commit SHA.
    pub sha: String,
    /// The raw commit message.
    pub message: String,
}

/// Run a git command without a working directory (e.g. for `git clone`).
///
/// # Errors
///
/// Returns [`Error::Git`] if git is not installed or the spawn fails.
pub fn run_git(args: &[&str]) -> Result<std::process::Output> {
    match process::git_command(None, args) {
        Ok(o) => Ok(o),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(Error::Git(
            "git is not installed — install git to use remote features".to_string(),
        )),
        Err(e) => Err(Error::Git(format!("failed to run git: {e}"))),
    }
}

/// Run a git command in the working directory of the repository containing
/// `path`.
///
/// # Errors
///
/// Returns [`Error::Git`] if git is not installed or the spawn fails.
pub fn run_git_at(path: &Path, args: &[&str]) -> Result<std::process::Output> {
    match process::git_command(Some(path), args) {
        Ok(o) => Ok(o),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(Error::Git(
            "git is not installed — install git to use remote features".to_string(),
        )),
        Err(e) => Err(Error::Git(format!("failed to run git: {e}"))),
    }
}

/// Discover the git directory for the repository containing `path`.
///
/// Uses `gix::discover` to walk up from `path` until a `.git` directory is
/// found.
///
/// # Errors
///
/// Returns [`Error::Git`] if no git repository is found at or above `path`.
pub fn discover_git_dir(path: &Path) -> Result<PathBuf> {
    let repo = gix::discover(path).map_err(|e| Error::Git(e.to_string()))?;
    Ok(repo.git_dir().to_owned())
}

/// Discover the effective hooks directory for the repository containing `path`.
///
/// Checks `core.hooksPath` in the merged git config first. If set, returns
/// that path (resolved against the working tree root when relative). Falls
/// back to `<git_dir>/hooks` when unset.
///
/// # Errors
///
/// Returns [`Error::Git`] if no git repository is found at or above `path`.
pub fn discover_hooks_dir(path: &Path) -> Result<PathBuf> {
    let repo = gix::discover(path).map_err(|e| Error::Git(e.to_string()))?;
    let config = repo.config_snapshot();
    if let Some(hooks_path) = config.string("core.hooksPath") {
        let p = PathBuf::from(hooks_path.to_string());
        if p.is_absolute() {
            return Ok(p);
        }
        // Relative paths are resolved against the working tree root.
        if let Some(work_dir) = repo.workdir() {
            return Ok(work_dir.join(p));
        }
    }
    Ok(repo.git_dir().join("hooks"))
}

/// Read HEAD commit info from the repository containing `path`.
///
/// Uses `gix::discover` to find the repo, then reads the HEAD commit.
/// Returns `Ok(None)` if the repository has no commits (unborn HEAD).
///
/// # Errors
///
/// Returns [`Error::Git`] if no git repository is found at or above `path`.
pub fn head_commit_info_at(path: &Path) -> Result<Option<HeadCommitInfo>> {
    let repo = gix::discover(path).map_err(|e| Error::Git(e.to_string()))?;
    let commit = match repo.head().ok().and_then(|mut h| h.peel_to_commit().ok()) {
        Some(c) => c,
        None => return Ok(None),
    };
    let sha = commit.id().to_string();
    let message = commit.message_raw_sloppy().to_string();
    Ok(Some(HeadCommitInfo { sha, message }))
}

/// Return commit messages from the repository at `path` in the range
/// `since_ref..HEAD`.
///
/// When `since_ref` is `None`, uses `HEAD@{1}` (the reflog anchor from
/// before the most recent merge). Commits are returned newest-first.
///
/// Returns an empty `Vec` if the anchor ref does not exist (e.g. shallow
/// clone or missing reflog entry) rather than failing.
///
/// # Errors
///
/// Returns [`Error::Git`] if `path` is not inside a git repository or git is
/// not installed.
pub fn commit_messages_since_at(
    path: &Path,
    since_ref: Option<&str>,
) -> Result<Vec<HeadCommitInfo>> {
    let anchor = since_ref.unwrap_or("HEAD@{1}");
    let output = run_git_at(
        path,
        &["log", "--format=%H%n%B%n<END>", "HEAD", "--not", anchor],
    )?;
    if !output.status.success() {
        return Ok(Vec::new());
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut commits = Vec::new();
    for block in stdout.split("<END>") {
        let block = block.trim();
        if block.is_empty() {
            continue;
        }
        if let Some((sha, message)) = block.split_once('\n') {
            commits.push(HeadCommitInfo {
                sha: sha.trim().to_string(),
                message: message.trim().to_string(),
            });
        }
    }
    Ok(commits)
}

/// Returns the current branch name for the repository containing `path`,
/// or `None` if HEAD is detached or unborn.
///
/// # Errors
///
/// Returns [`Error::Git`] if `path` is not inside a git repository or git is
/// not installed.
pub fn current_branch_at(path: &Path) -> Result<Option<String>> {
    let output = run_git_at(path, &["symbolic-ref", "--quiet", "HEAD"])?;
    if !output.status.success() {
        return Ok(None);
    }
    let full_ref = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(full_ref.strip_prefix("refs/heads/").map(|s| s.to_string()))
}

/// Whether `sha` is an ancestor of (or equal to) HEAD in the repo at `path`.
///
/// Shells out to `git merge-base --is-ancestor <sha> HEAD`: exit code 0 means
/// `sha` is reachable from HEAD (`Ok(true)`), exit code 1 means it is not
/// (`Ok(false)`). Any other exit code (e.g. an unknown SHA, or HEAD being
/// unborn) is treated as an error.
///
/// # Errors
///
/// Returns [`Error::Git`] if `path` is not inside a git repository, git is not
/// installed, or the command fails for a reason other than a clean
/// ancestor/not-ancestor determination (such as an invalid `sha`).
pub fn is_ancestor_of_head_at(path: &Path, sha: &str) -> Result<bool> {
    let output = run_git_at(path, &["merge-base", "--is-ancestor", sha, "HEAD"])?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(Error::Git(format!(
                "git merge-base --is-ancestor failed: {}",
                stderr.trim()
            )))
        }
    }
}

/// Whether `sha` is an ancestor of (or equal to) `branch` in the repo at `path`.
///
/// Like [`is_ancestor_of_head_at`], but resolves reachability against an
/// explicitly named branch rather than the checkout's current `HEAD`. This is
/// the execution-side primitive for reviewing a stamped branch from a different
/// checkout: given an item's recorded `review_branch`, decide whether a commit
/// is reachable from that branch's tip even when the reviewer is standing
/// somewhere else.
///
/// Shells out to `git merge-base --is-ancestor <sha> <branch>`: exit code 0
/// means `sha` is reachable from `branch` (`Ok(true)`), exit code 1 means it is
/// not (`Ok(false)`). Any other exit code (e.g. an unknown SHA or an unknown
/// branch) is treated as an error.
///
/// # Errors
///
/// Returns [`Error::Git`] if `path` is not inside a git repository, git is not
/// installed, or the command fails for a reason other than a clean
/// ancestor/not-ancestor determination (such as an invalid `sha` or `branch`).
pub fn is_ancestor_of_branch_at(path: &Path, branch: &str, sha: &str) -> Result<bool> {
    let output = run_git_at(path, &["merge-base", "--is-ancestor", sha, branch])?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(Error::Git(format!(
                "git merge-base --is-ancestor failed: {}",
                stderr.trim()
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::TempDir;

    /// A `git` command in `dir` with author/committer identity configured so
    /// commits succeed in a clean environment.
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
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        out
    }

    /// Initialize a repo on `main` at `dir`.
    fn init_repo() -> TempDir {
        let dir = TempDir::new().unwrap();
        git(dir.path(), &["init", "-b", "main"]);
        dir
    }

    /// Write `name` with `content`, commit it with `name` as the message, and
    /// return the new commit SHA.
    fn commit_file(dir: &Path, name: &str, content: &str) -> String {
        std::fs::write(dir.join(name), content).unwrap();
        git(dir, &["add", "."]);
        git(dir, &["commit", "-m", name]);
        let out = git(dir, &["rev-parse", "HEAD"]);
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    #[test]
    fn is_ancestor_of_head_distinguishes_branches() {
        let dir = init_repo();
        let base_sha = commit_file(dir.path(), "base.md", "base");

        // Branch A: a commit reachable from the branch-A tip.
        git(dir.path(), &["checkout", "-b", "branch-a"]);
        let a_sha = commit_file(dir.path(), "a.md", "a");

        // Branch B (off base): its tip is NOT reachable from branch A's HEAD.
        git(dir.path(), &["checkout", "-b", "branch-b", &base_sha]);
        let b_sha = commit_file(dir.path(), "b.md", "b");

        // Back on branch A.
        git(dir.path(), &["checkout", "branch-a"]);

        // From branch A's HEAD: base and A's own tip are ancestors; B is not.
        assert!(is_ancestor_of_head_at(dir.path(), &base_sha).unwrap());
        assert!(is_ancestor_of_head_at(dir.path(), &a_sha).unwrap());
        assert!(!is_ancestor_of_head_at(dir.path(), &b_sha).unwrap());
    }

    #[test]
    fn is_ancestor_of_head_errors_on_unknown_sha() {
        let dir = init_repo();
        commit_file(dir.path(), "init.md", "init");

        let result = is_ancestor_of_head_at(dir.path(), "0000000000000000000000000000000000000000");
        assert!(result.is_err());
    }

    #[test]
    fn is_ancestor_of_branch_distinguishes_branches() {
        let dir = init_repo();
        let base_sha = commit_file(dir.path(), "base.md", "base");

        // Branch A: a commit reachable from the branch-A tip.
        git(dir.path(), &["checkout", "-b", "branch-a"]);
        let a_sha = commit_file(dir.path(), "a.md", "a");

        // Branch B (off base): its tip is NOT reachable from branch A's tip.
        git(dir.path(), &["checkout", "-b", "branch-b", &base_sha]);
        let b_sha = commit_file(dir.path(), "b.md", "b");

        // Standing on branch B, query reachability against branch-a *by name*:
        // base and A's own tip are ancestors of branch-a; B's tip is not.
        assert!(is_ancestor_of_branch_at(dir.path(), "branch-a", &base_sha).unwrap());
        assert!(is_ancestor_of_branch_at(dir.path(), "branch-a", &a_sha).unwrap());
        assert!(!is_ancestor_of_branch_at(dir.path(), "branch-a", &b_sha).unwrap());
    }

    #[test]
    fn is_ancestor_of_branch_errors_on_unknown_ref() {
        let dir = init_repo();
        let sha = commit_file(dir.path(), "init.md", "init");

        let result = is_ancestor_of_branch_at(dir.path(), "no-such-branch", &sha);
        assert!(result.is_err());
    }

    #[test]
    fn current_branch_at_reports_branch_and_none_when_detached() {
        let dir = init_repo();
        let sha = commit_file(dir.path(), "init.md", "init");
        assert_eq!(
            current_branch_at(dir.path()).unwrap().as_deref(),
            Some("main")
        );

        git(dir.path(), &["checkout", "--detach", &sha]);
        assert_eq!(current_branch_at(dir.path()).unwrap(), None);
    }

    #[test]
    fn discover_git_dir_finds_repo() {
        let dir = TempDir::new().unwrap();
        gix::init(dir.path()).unwrap();
        let git_dir = discover_git_dir(dir.path()).unwrap();
        assert!(git_dir.ends_with(".git"));
    }

    #[test]
    fn discover_git_dir_from_subdir() {
        let dir = TempDir::new().unwrap();
        gix::init(dir.path()).unwrap();
        let sub = dir.path().join("a/b");
        std::fs::create_dir_all(&sub).unwrap();
        let git_dir = discover_git_dir(&sub).unwrap();
        assert!(git_dir.ends_with(".git"));
    }

    #[test]
    fn discover_git_dir_errors_for_non_repo() {
        let dir = TempDir::new().unwrap();
        assert!(discover_git_dir(dir.path()).is_err());
    }

    #[test]
    fn head_commit_info_at_returns_none_for_empty_repo() {
        let dir = TempDir::new().unwrap();
        gix::init(dir.path()).unwrap();
        let info = head_commit_info_at(dir.path()).unwrap();
        assert!(info.is_none());
    }

    #[test]
    fn head_commit_info_at_reads_commit() {
        let dir = init_repo();
        commit_file(dir.path(), "file.md", "content");

        let info = head_commit_info_at(dir.path()).unwrap().unwrap();
        assert!(!info.sha.is_empty());
        assert!(!info.message.is_empty());
    }

    #[test]
    fn commit_messages_since_at_returns_commits() {
        let dir = init_repo();
        commit_file(dir.path(), "init.md", "init");
        git(dir.path(), &["tag", "anchor"]);
        commit_file(dir.path(), "a.md", "a");
        commit_file(dir.path(), "b.md", "b");

        let commits = commit_messages_since_at(dir.path(), Some("anchor")).unwrap();
        assert_eq!(commits.len(), 2);
        assert!(commits[0].message.contains("b.md"));
        assert!(commits[1].message.contains("a.md"));
    }

    #[test]
    fn commit_messages_since_at_empty_range() {
        let dir = init_repo();
        commit_file(dir.path(), "init.md", "init");

        let commits = commit_messages_since_at(dir.path(), Some("HEAD")).unwrap();
        assert!(commits.is_empty());
    }
}

//! The [`GitRepo`] collaborator: owns the gix handle plus repository root and
//! provides all git logic (plumbing in [`commit`](crate::commit), remote
//! porcelain in [`remote`](crate::remote), merge porcelain in
//! [`merge`](crate::merge)). [`GitStore`](crate::GitStore) composes a
//! `GitRepo` with an [`FsStore`](rdm_store_fs::FsStore) and delegates every
//! git operation to it.
//!
//! This module also holds the path-taking free functions that the inherent
//! `GitRepo` methods delegate to ([`head_commit_info_at`],
//! [`commit_messages_since_at`], [`current_branch_at`]) and the repository
//! discovery helpers ([`discover_git_dir`], [`discover_hooks_dir`]).

use std::path::{Path, PathBuf};

use rdm_core::error::{Error, Result};

use crate::HeadCommitInfo;

/// The git capability: a gix handle paired with the repository root.
///
/// Owns all git logic — low-level plumbing and high-level porcelain — so that
/// [`GitStore`](crate::GitStore) can remain a thin storage adapter. Methods are
/// split across sibling modules but operate on the same `root`/`repo` fields.
pub struct GitRepo {
    pub(crate) root: PathBuf,
    pub(crate) repo: gix::ThreadSafeRepository,
}

impl GitRepo {
    /// Creates a `GitRepo` from a repository root and an opened gix handle.
    pub(crate) fn new(root: PathBuf, repo: gix::ThreadSafeRepository) -> Self {
        Self { root, repo }
    }

    /// Returns the root path of the repository.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns the path to the `.git` directory (or the git dir for worktrees).
    pub fn git_dir(&self) -> &Path {
        self.repo.git_dir()
    }

    /// Reopens the gix handle from disk to refresh cached refs/config.
    ///
    /// Used after CLI git operations (fetch, push, merge, remote edits) that
    /// mutate refs or config behind gix's back.
    pub(crate) fn reopen(&mut self) -> Result<()> {
        self.repo = gix::open(&self.root)
            .map_err(|e| Error::Git(e.to_string()))?
            .into_sync();
        Ok(())
    }

    /// Runs a git command in the repository's working directory.
    pub(crate) fn run_git(&self, args: &[&str]) -> Result<std::process::Output> {
        run_git_at(&self.root, args)
    }
}

/// Run a git command without a working directory (e.g. for `git clone`).
pub(crate) fn run_git(args: &[&str]) -> Result<std::process::Output> {
    match crate::process::git_command(None, args) {
        Ok(o) => Ok(o),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(Error::Git(
            "git is not installed — install git to use remote features".to_string(),
        )),
        Err(e) => Err(Error::Git(format!("failed to run git: {e}"))),
    }
}

/// Run a git command in the working directory of the repository containing
/// `path`.
pub(crate) fn run_git_at(path: &Path, args: &[&str]) -> Result<std::process::Output> {
    match crate::process::git_command(Some(path), args) {
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
/// Returns `Error::Git` if no git repository is found at or above `path`.
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
/// Returns `Error::Git` if no git repository is found at or above `path`.
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
/// Returns `Error::Git` if no git repository is found at or above `path`.
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
/// Returns `Error::Git` if `path` is not inside a git repository or git is
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
/// Returns `Error::Git` if `path` is not inside a git repository or git is
/// not installed.
pub fn current_branch_at(path: &Path) -> Result<Option<String>> {
    let output = run_git_at(path, &["symbolic-ref", "--quiet", "HEAD"])?;
    if !output.status.success() {
        return Ok(None);
    }
    let full_ref = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(full_ref.strip_prefix("refs/heads/").map(|s| s.to_string()))
}

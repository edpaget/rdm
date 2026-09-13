//! The [`GitRepo`] collaborator: owns the gix handle plus repository root and
//! provides all git logic (plumbing in [`commit`](crate::commit), remote
//! porcelain in [`remote`](crate::remote), merge porcelain in
//! [`merge`](crate::merge)). [`GitStore`](crate::GitStore) composes a
//! `GitRepo` with an [`FsStore`](rdm_store_fs::FsStore) and delegates every
//! git operation to it.
//!
//! The repo-agnostic path-taking helpers these methods delegate to
//! (`head_commit_info_at`, `commit_messages_since_at`, `current_branch_at`, the
//! discovery helpers, and the `run_git`/`run_git_at` spawners) live in the
//! [`rdm_git`] crate.

use std::path::{Path, PathBuf};

use rdm_core::error::{Error, Result};

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
        rdm_git::run_git_at(&self.root, args)
    }
}

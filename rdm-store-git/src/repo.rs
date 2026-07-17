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

    /// Ensures `.gitattributes` routes `INDEX.md` (root and per-project) to
    /// the `rdm-index` merge driver.
    ///
    /// Idempotent and non-destructive: if the marker `merge=rdm-index` is
    /// already present anywhere in the file, this is a no-op. Otherwise the
    /// two entries are appended, preserving any existing content and
    /// inserting a newline separator first if the file doesn't already end
    /// with one.
    pub(crate) fn ensure_gitattributes(&self) -> Result<()> {
        let path = self.root.join(".gitattributes");
        let existing = std::fs::read_to_string(&path).unwrap_or_default();
        if existing.contains("merge=rdm-index") {
            return Ok(());
        }

        let mut content = existing;
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str("INDEX.md merge=rdm-index\n**/INDEX.md merge=rdm-index\n");

        std::fs::write(&path, content)
            .map_err(|e| Error::Git(format!("failed to write .gitattributes: {e}")))?;
        Ok(())
    }

    /// Ensures the repository-local `.git/config` defines the `rdm-index`
    /// merge driver.
    ///
    /// Idempotent and non-destructive: if a `[merge "rdm-index"]` section is
    /// already present (whether installed by rdm or hand-customized), this is
    /// a no-op — the short-circuit check runs before any write I/O since this
    /// runs on every `GitStore` open. The driver command uses git's `%A`/`%P`
    /// placeholders: `%A` is the temp file whose content git copies back into
    /// the merge result, `%P` is the repo-relative path being merged.
    ///
    /// `--root .` is required, not cosmetic: `rdm`'s root resolution never
    /// discovers the plan repo from the current directory (only
    /// `RDM_ROOT`/`--root`, global config, or the XDG default) — see
    /// `rdm_core::root::resolve_root`. Git spawns the driver as a plain
    /// subprocess with cwd set to the worktree toplevel and no `RDM_ROOT`
    /// guarantee, so without an explicit `--root .` the driver can silently
    /// regenerate/read the *wrong* plan repo (whatever the ambient default
    /// resolves to) instead of the one actually being merged.
    pub(crate) fn ensure_merge_driver_config(&self) -> Result<()> {
        let config_path = self.repo.git_dir().join("config");
        let existing = std::fs::read_to_string(&config_path).unwrap_or_default();
        if existing.contains("[merge \"rdm-index\"]") {
            return Ok(());
        }

        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&config_path)
            .map_err(|e| Error::Git(format!("failed to write git config: {e}")))?;
        use std::io::Write;
        writeln!(
            file,
            "\n[merge \"rdm-index\"]\n\tname = rdm INDEX.md merge driver\n\tdriver = rdm --root . index --merge-output %A --merge-path %P"
        )
        .map_err(|e| Error::Git(format!("failed to write git config: {e}")))?;
        Ok(())
    }
}

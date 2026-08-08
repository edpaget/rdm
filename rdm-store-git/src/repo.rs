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

/// The relative path of the file carrying the merge-driver mapping.
pub(crate) const GITATTRIBUTES_PATH: &str = ".gitattributes";

/// The substring whose presence anywhere in `.gitattributes` means the mapping
/// is already installed. Also the idempotence short-circuit.
const GITATTRIBUTES_MARKER: &str = "merge=rdm-index";

/// The two entries [`GitRepo::ensure_gitattributes`] appends.
const GITATTRIBUTES_MAPPING: &str = "INDEX.md merge=rdm-index\n**/INDEX.md merge=rdm-index\n";

/// Returns what [`GitRepo::ensure_gitattributes`] would leave on disk given
/// `existing` as the file's current content (`""` when the file is absent).
///
/// Pure, so the same rule can be *recognized* as well as applied: the pull
/// guard in [`remote`](crate::remote) uses it to decide whether a dirty
/// `.gitattributes` is rdm's own mapping write and nothing else. Keeping one
/// definition means the guard can never drift from the writer.
pub(crate) fn gitattributes_with_mapping(existing: &str) -> String {
    if existing.contains(GITATTRIBUTES_MARKER) {
        return existing.to_string();
    }
    let mut content = existing.to_string();
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str(GITATTRIBUTES_MAPPING);
    content
}

/// The git capability: a gix handle paired with the repository root.
///
/// Owns all git logic — low-level plumbing and high-level porcelain — so that
/// [`GitStore`](crate::GitStore) can remain a thin storage adapter. Methods are
/// split across sibling modules but operate on the same `root`/`repo` fields.
pub struct GitRepo {
    pub(crate) root: PathBuf,
    pub(crate) repo: gix::ThreadSafeRepository,
    /// Set whenever [`ensure_gitattributes`](Self::ensure_gitattributes)
    /// actually wrote the file.
    ///
    /// This is the "report upward" channel: `ensure_gitattributes` runs at
    /// sites with no `Store` in reach (the pull re-ensure, the whole-tree
    /// discard) whose callers historically swallowed its result. Rather than
    /// let the write go unjournaled — and therefore, under session scoping,
    /// become permanently uncommittable — the fact is latched here and
    /// drained by `GitStore` before it commits.
    pub(crate) gitattributes_written: std::sync::atomic::AtomicBool,
}

impl GitRepo {
    /// Creates a `GitRepo` from a repository root and an opened gix handle.
    pub(crate) fn new(root: PathBuf, repo: gix::ThreadSafeRepository) -> Self {
        Self {
            root,
            repo,
            gitattributes_written: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Takes and clears the latched `.gitattributes`-was-written flag.
    pub(crate) fn take_gitattributes_written(&self) -> bool {
        self.gitattributes_written
            .swap(false, std::sync::atomic::Ordering::SeqCst)
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
    ///
    /// # Return value — the journaling responsibility
    ///
    /// Returns `true` when this call actually wrote the file, `false` when it
    /// was already mapped. **Every caller that can reach a `GitStore` must
    /// journal a `true` result** (`GitStore::journal_side_write`). This write
    /// has no `Store` batch behind it, so under session-scoped committing an
    /// unjournaled `.gitattributes` belongs to no changeset and is therefore
    /// permanently uncommittable — silently cancelling the merge-driver
    /// back-fill this method exists to perform.
    ///
    /// # Call sites
    ///
    /// - [`GitStore::init`](crate::GitStore::init) — hard-fails, which is
    ///   correct for an explicit `rdm init`; journals on write.
    /// - [`GitStore::new`](crate::GitStore::new) and
    ///   [`GitStore::clone_remote`](crate::GitStore::clone_remote) —
    ///   **best-effort**: a failure warns and the open/clone still succeeds,
    ///   so a read-only mount still opens for reads. This is the backfill that
    ///   maps repos created or cloned before the merge driver shipped, with no
    ///   user action; both journal on write.
    /// - [`git_discard`](Self::git_discard) and
    ///   [`GitStore::discard_changeset`](crate::GitStore::discard_changeset) —
    ///   best-effort, after the restore loop, because discarding an
    ///   as-yet-uncommitted `.gitattributes` would otherwise silently un-map
    ///   the repo. The scoped discard journals it; the whole-tree one need
    ///   not, since a whole-tree commit takes it from disk regardless.
    /// - `git_pull`'s diverged branch — best-effort, immediately after the
    ///   merge subprocess, putting back the mapping the pull guard restored
    ///   to HEAD so `git merge` would accept the tree. Reports upward so the
    ///   store layer journals it instead of swallowing the write.
    ///
    /// The write lands in the worktree, so it appears in `rdm status` as an
    /// ordinary change until the next `rdm commit` tracks it — which it must
    /// be, for the mapping to travel with clones.
    pub(crate) fn ensure_gitattributes(&self) -> Result<bool> {
        let path = self.root.join(GITATTRIBUTES_PATH);
        let existing = std::fs::read_to_string(&path).unwrap_or_default();
        let content = gitattributes_with_mapping(&existing);
        if content == existing {
            return Ok(false);
        }

        std::fs::write(&path, content)
            .map_err(|e| Error::Git(format!("failed to write .gitattributes: {e}")))?;
        self.gitattributes_written
            .store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(true)
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

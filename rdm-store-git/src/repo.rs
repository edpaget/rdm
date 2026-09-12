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

    /// Removes the stale `[merge "rdm-index"]` section rdm used to install in
    /// the repository-local `.git/config`, returning whether it removed one.
    ///
    /// # Why this is required, not cosmetic
    ///
    /// rdm no longer ships an `rdm-index` merge driver, and `rdm index` no
    /// longer accepts the `--merge-output`/`--merge-path` flags the installed
    /// driver command passed. Left in place, that section names a command that
    /// now fails, and git's response to a failing merge driver is measured, not
    /// theoretical (git 2.55.0): it reports `CONFLICT (content)`, exits
    /// non-zero, and leaves the merge result as the unmodified `ours` blob with
    /// no conflict markers — on **every** `INDEX.md` merge, including ones that
    /// would have merged cleanly. Every existing plan repo would silently
    /// resolve `INDEX.md` to its own side on every `git merge` /
    /// `rdm remote pull` until a human noticed.
    ///
    /// The tracked `.gitattributes` line is deliberately **not** touched: it is
    /// a hand-customizable user file, and a `merge=rdm-index` attribute naming
    /// a driver that is absent from config is inert — git falls back to its
    /// built-in three-way merge, with ordinary conflict markers and nothing on
    /// stderr.
    ///
    /// # Scope of the removal
    ///
    /// Only rdm's *own* section is removed: the `driver =` value must contain
    /// `index --merge-output`, or be one of the historical bare spellings
    /// (`rdm --root . index`, `rdm index`). A hand-customized driver, or a
    /// section carrying no `driver =` line at all, is preserved untouched and
    /// yields `Ok(false)`.
    ///
    /// Removal goes through `git config --remove-section` rather than
    /// hand-rolled string surgery on the file: git takes `.git/config.lock`,
    /// so a concurrent `rdm init --remote` writing `remote.origin.url` cannot
    /// be clobbered, and git resolves the right config file in a linked
    /// worktree. A substring fast path over the file means an already-swept
    /// repo — the overwhelmingly common case — spawns no subprocess at all.
    ///
    /// This is a one-release migration sweep; its retirement belongs to
    /// `retire-generated-index/phase-6-one-command-exit`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Git`] if a `git config` invocation cannot be spawned.
    /// Callers treat this as best-effort — a read-only `.git/config` must warn
    /// and continue, never fail an open, an init or a clone.
    pub(crate) fn remove_rdm_index_driver_section(&self) -> Result<bool> {
        let config_path = self.repo.git_dir().join("config");
        let existing = std::fs::read_to_string(&config_path).unwrap_or_default();
        if !existing.contains("[merge \"rdm-index\"]") {
            return Ok(false);
        }

        let out = self.run_git(&["config", "--get", "merge.rdm-index.driver"])?;
        if !out.status.success() {
            // No `driver =` line (or it is unreadable): inert either way, and
            // not provably rdm's. Leave it alone.
            return Ok(false);
        }
        let driver = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !is_rdm_index_driver(&driver) {
            return Ok(false);
        }

        let out = self.run_git(&["config", "--remove-section", "merge.rdm-index"])?;
        if !out.status.success() {
            return Err(Error::Git(format!(
                "failed to remove the stale [merge \"rdm-index\"] section from {}: {}",
                config_path.display(),
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        Ok(true)
    }
}

/// Returns whether `driver` is a merge-driver command rdm itself installed.
///
/// Matches every shipped spelling: the current
/// `rdm --root . index --merge-output %A --merge-path %P`, the pre-`--root`
/// form `rdm index --merge-output %A --merge-path %P`, and the two bare
/// historical forms. Anything else is a user's own driver and is preserved.
fn is_rdm_index_driver(driver: &str) -> bool {
    let driver = driver.trim();
    driver.contains("index --merge-output")
        || driver == "rdm --root . index"
        || driver == "rdm index"
}

#[cfg(test)]
mod tests {
    use super::is_rdm_index_driver;

    #[test]
    fn recognizes_every_shipped_driver_spelling() {
        assert!(is_rdm_index_driver(
            "rdm --root . index --merge-output %A --merge-path %P"
        ));
        assert!(is_rdm_index_driver(
            "rdm index --merge-output %A --merge-path %P"
        ));
        assert!(is_rdm_index_driver("rdm --root . index"));
        assert!(is_rdm_index_driver("rdm index"));
    }

    #[test]
    fn leaves_a_hand_customized_driver_alone() {
        assert!(!is_rdm_index_driver("custom-driver %A"));
        assert!(!is_rdm_index_driver("my-rdm-wrapper index %A %P"));
        assert!(!is_rdm_index_driver(""));
    }
}

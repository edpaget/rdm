//! Error type for git-porcelain operations on the plan repo.
//!
//! These errors arise from the remote/merge porcelain methods on
//! [`GitStore`](crate::GitStore) (`git_remote_add`, `git_push`, `git_pull`,
//! `git_resolve_conflict`, …). They are deliberately self-contained in
//! `rdm-store-git`, mirroring `rdm-git`'s `WorktreeError`:
//! the porcelain methods are not part of the [`Store`](rdm_core::store::Store)
//! trait, so they do not belong in `rdm-core`'s error enum, which a
//! filesystem-only build would otherwise compile in but never produce.

/// Errors that can occur during git-porcelain operations.
///
/// Variants are matchable so callers can map each to an actionable message.
#[derive(Debug)]
pub enum GitError {
    /// The specified git remote was not found.
    RemoteNotFound(String),
    /// A git remote with the given name already exists.
    DuplicateRemote(String),
    /// A git push was rejected (non-fast-forward).
    PushRejected(String),
    /// Local and remote branches have diverged.
    ///
    /// Relocated from core for parity with the porcelain error vocabulary;
    /// not currently constructed (divergent pulls attempt a real merge and
    /// surface conflicts as [`PullOutcome::Conflict`](crate::PullOutcome)).
    BranchesDiverged(String),
    /// A merge conflict occurred during pull.
    ///
    /// Relocated from core for parity; not currently constructed — pull
    /// conflicts surface as the successful
    /// [`PullOutcome::Conflict`](crate::PullOutcome), not as an error.
    MergeConflict(String),
    /// No merge is in progress.
    NoMergeInProgress,
    /// A file is not in the unmerged list.
    NotConflicted(String),
    /// A git operation failed; carries a human-readable message.
    Git(String),
}

impl std::fmt::Display for GitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GitError::RemoteNotFound(name) => {
                write!(
                    f,
                    "remote not found: {name} — use `rdm remote add` to create one"
                )
            }
            GitError::DuplicateRemote(name) => {
                write!(f, "remote '{name}' already exists")
            }
            GitError::PushRejected(msg) => {
                write!(
                    f,
                    "push rejected: {msg} — pull first with `rdm remote pull`, then push again"
                )
            }
            GitError::BranchesDiverged(msg) => {
                write!(
                    f,
                    "branches have diverged: {msg} — resolve manually with `git rebase` or `git merge`"
                )
            }
            GitError::MergeConflict(msg) => {
                write!(
                    f,
                    "merge conflict: {msg} — run `rdm conflicts` to see details, then `rdm resolve <file>`"
                )
            }
            GitError::NoMergeInProgress => {
                write!(f, "no merge in progress — nothing to resolve")
            }
            GitError::NotConflicted(path) => {
                write!(f, "file '{path}' is not in the unmerged list")
            }
            GitError::Git(msg) => write!(f, "git error: {msg}"),
        }
    }
}

impl std::error::Error for GitError {}

impl From<rdm_core::error::Error> for GitError {
    /// Bridges the generic `rdm-core` errors raised by internal git plumbing
    /// (e.g. `run_git`) into [`GitError::Git`], preserving the message of a
    /// core [`Error::Git`](rdm_core::error::Error::Git) without double-prefixing.
    fn from(err: rdm_core::error::Error) -> Self {
        match err {
            rdm_core::error::Error::Git(msg) => GitError::Git(msg),
            other => GitError::Git(other.to_string()),
        }
    }
}

/// A convenience `Result` type for git-porcelain operations.
pub type Result<T> = std::result::Result<T, GitError>;

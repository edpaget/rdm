//! The read-only **source repository** port.
//!
//! A `change/<sha>` review ([`ReviewTarget::Change`](crate::model::ReviewTarget))
//! lives in the plan repo, but its comment anchors point into the project's
//! *source* repository. rdm-core must be able to derive and resolve those
//! anchors without depending on git — exactly the separation
//! [`VersionedStore`](crate::store::VersionedStore) /
//! [`MemoryStore`](crate::store::MemoryStore) already keep for plan data.
//!
//! [`SourceRepo`] is therefore a narrow trait of **reads only**: revision
//! resolution, merge-base computation, file content at a revision, and a
//! unified diff. There is no write method, and there never should be — the
//! invariant that reviewing a change never mutates the source repository
//! requires implementations to protect subprocess argument boundaries as well
//! as expose only read operations.
//! `rdm-git`'s `GitSourceRepo` is the production implementation;
//! [`MemorySourceRepo`] is the in-memory double the pure logic in
//! [`crate::change`] is unit-tested against.

use std::collections::HashMap;

use crate::error::Result;

/// A read-only view of a git-like source repository.
///
/// Every method returns `Ok(None)` for a *benign* miss — an unknown
/// revision, a path absent at that revision, two histories with no common
/// ancestor — and reserves [`Err`] for a genuine tool failure (git missing,
/// the path not being a repository, non-UTF-8 content) or unsafe revision input.
/// Production adapters reject option-shaped operands before spawning and peel
/// revisions to commits for file/diff reads; missing/noncommit objects are benign
/// misses. Identity syntax alone does not prove that an object is a commit.
/// Callers turn the
/// `None` into an actionable message with the context they have; they never
/// have to distinguish "git broke" from "not there" themselves.
pub trait SourceRepo {
    /// Resolves `rev` (a sha, abbreviated sha, branch, tag, or `HEAD`) to a
    /// full 40-character commit SHA.
    ///
    /// # Errors
    ///
    /// Returns an error when the repository cannot be queried at all.
    /// Also returns [`crate::error::Error::InvalidChangeRevisionInput`]
    /// for option-shaped input, before any subprocess.
    fn rev_parse(&self, rev: &str) -> Result<Option<String>>;

    /// The best common ancestor of `a` and `b`, as a full commit SHA.
    ///
    /// # Errors
    ///
    /// Returns an error when the repository cannot be queried at all.
    /// Also returns [`crate::error::Error::InvalidChangeRevisionInput`]
    /// for option-shaped input, before any subprocess.
    fn merge_base(&self, a: &str, b: &str) -> Result<Option<String>>;

    /// The complete content of `path` as of `rev`.
    ///
    /// Read from the object database (`git show <rev>:<path>`), never from
    /// the working tree, so a dirty or differently line-ending-normalized
    /// checkout can never change what an anchor resolves against.
    ///
    /// # Errors
    ///
    /// Returns an error when the repository cannot be queried, or when the
    /// content is not valid UTF-8 (a binary file cannot be quoted), or
    /// [`crate::error::Error::InvalidChangeRevisionInput`] for option-shaped input
    /// before any subprocess. Missing or noncommit revisions return `Ok(None)`.
    fn file_at(&self, rev: &str, path: &str) -> Result<Option<String>>;

    /// A zero-context unified diff of `path` between `base` and `head`.
    ///
    /// `Ok(None)` means the change does not touch `path` at all, which is
    /// distinct from `Ok(Some(""))`.
    ///
    /// # Errors
    ///
    /// Returns an error when the repository cannot be queried at all.
    /// Also returns [`crate::error::Error::InvalidChangeRevisionInput`]
    /// for option-shaped input, before any subprocess.
    fn unified_diff(&self, base: &str, head: &str, path: &str) -> Result<Option<String>>;

    /// The full SHA currently at `HEAD`, or `None` on an unborn HEAD.
    ///
    /// # Errors
    ///
    /// Returns an error when the repository cannot be queried at all.
    fn head(&self) -> Result<Option<String>>;

    /// The currently checked-out branch name, or `None` on a detached HEAD.
    ///
    /// # Errors
    ///
    /// Returns an error when the repository cannot be queried at all.
    fn current_branch(&self) -> Result<Option<String>>;
}

/// An in-memory [`SourceRepo`] for tests, with no git dependency.
///
/// Revisions, file contents and diffs are all seeded explicitly, so a test
/// can construct exactly the repository shape it needs (including ones that
/// are awkward to produce with real git, like two revisions with no merge
/// base) without spawning a process.
#[derive(Debug, Default, Clone)]
pub struct MemorySourceRepo {
    /// `rev` (as typed) → full SHA.
    revs: HashMap<String, String>,
    /// `(rev, path)` → file content at that revision.
    files: HashMap<(String, String), String>,
    /// `(base, head, path)` → unified diff.
    diffs: HashMap<(String, String, String), String>,
    /// `(a, b)` → merge base, in both orders as seeded.
    merge_bases: HashMap<(String, String), String>,
    /// What [`SourceRepo::head`] returns.
    head: Option<String>,
    /// What [`SourceRepo::current_branch`] returns.
    branch: Option<String>,
}

impl MemorySourceRepo {
    /// An empty repository: every lookup misses.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Seeds `rev` as resolving to `sha` (and `sha` to itself).
    #[must_use]
    pub fn with_rev(mut self, rev: &str, sha: &str) -> Self {
        self.revs.insert(rev.to_string(), sha.to_string());
        self.revs.insert(sha.to_string(), sha.to_string());
        self
    }

    /// Seeds the content of `path` at `rev`.
    #[must_use]
    pub fn with_file(mut self, rev: &str, path: &str, content: &str) -> Self {
        self.files
            .insert((rev.to_string(), path.to_string()), content.to_string());
        self
    }

    /// Seeds the unified diff of `path` between `base` and `head`.
    #[must_use]
    pub fn with_diff(mut self, base: &str, head: &str, path: &str, diff: &str) -> Self {
        self.diffs.insert(
            (base.to_string(), head.to_string(), path.to_string()),
            diff.to_string(),
        );
        self
    }

    /// Seeds the merge base of `a` and `b` (symmetrically).
    #[must_use]
    pub fn with_merge_base(mut self, a: &str, b: &str, base: &str) -> Self {
        self.merge_bases
            .insert((a.to_string(), b.to_string()), base.to_string());
        self.merge_bases
            .insert((b.to_string(), a.to_string()), base.to_string());
        self
    }

    /// Seeds what [`SourceRepo::head`] reports.
    #[must_use]
    pub fn with_head(mut self, sha: &str) -> Self {
        self.head = Some(sha.to_string());
        self
    }

    /// Seeds what [`SourceRepo::current_branch`] reports.
    #[must_use]
    pub fn with_branch(mut self, branch: &str) -> Self {
        self.branch = Some(branch.to_string());
        self
    }
}

impl SourceRepo for MemorySourceRepo {
    fn rev_parse(&self, rev: &str) -> Result<Option<String>> {
        Ok(self.revs.get(rev).cloned())
    }

    fn merge_base(&self, a: &str, b: &str) -> Result<Option<String>> {
        Ok(self
            .merge_bases
            .get(&(a.to_string(), b.to_string()))
            .cloned())
    }

    fn file_at(&self, rev: &str, path: &str) -> Result<Option<String>> {
        Ok(self
            .files
            .get(&(rev.to_string(), path.to_string()))
            .cloned())
    }

    fn unified_diff(&self, base: &str, head: &str, path: &str) -> Result<Option<String>> {
        Ok(self
            .diffs
            .get(&(base.to_string(), head.to_string(), path.to_string()))
            .cloned())
    }

    fn head(&self) -> Result<Option<String>> {
        Ok(self.head.clone())
    }

    fn current_branch(&self) -> Result<Option<String>> {
        Ok(self.branch.clone())
    }
}

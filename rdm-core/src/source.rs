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

use std::collections::{HashMap, HashSet};

use crate::error::Result;

/// What kind of Git object a tree entry names, as reported by `git cat-file
/// -t`.
///
/// A `--path` anchor is only ever eligible when this is [`Self::Blob`] — a
/// [`Self::Tree`] (directory) or [`Self::Gitlink`] (submodule) cannot be
/// quoted, and [`SourceRepo::object_kind_at`] is how
/// [`crate::change::derive_change_anchor`] and
/// [`crate::change::resolve_change_comment`] tell the three apart without
/// guessing from content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceObjectKind {
    /// A regular file (or symlink) — the only kind an anchor may quote.
    Blob,
    /// A directory entry.
    Tree,
    /// A submodule entry (a commit object referenced by a tree).
    Gitlink,
}

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

    /// The kind of Git object `path` names at `rev` — a blob, a tree
    /// (directory), or a gitlink (submodule).
    ///
    /// Answers via git's own object typing rather than by inspecting
    /// content, so a text blob whose bytes happen to resemble a directory
    /// listing is still correctly reported as [`SourceObjectKind::Blob`].
    /// `Ok(None)` covers both an unresolvable `rev` and a `path` absent at
    /// it — callers that need to distinguish those already call
    /// [`SourceRepo::rev_parse`] or [`SourceRepo::file_at`] first.
    ///
    /// # Errors
    ///
    /// Returns an error when the repository cannot be queried at all, or
    /// [`crate::error::Error::InvalidChangeRevisionInput`] for option-shaped
    /// input, before any subprocess.
    fn object_kind_at(&self, rev: &str, path: &str) -> Result<Option<SourceObjectKind>>;
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
    /// `(rev, path)` → explicitly seeded object kind, for
    /// [`SourceRepo::object_kind_at`].
    object_kinds: HashMap<(String, String), SourceObjectKind>,
    /// Revisions [`SourceRepo::rev_parse`] must fail on rather than miss.
    failing_rev_parses: HashSet<String>,
    /// Whether [`SourceRepo::head`] must fail rather than answer.
    failing_head: bool,
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

    /// Makes [`SourceRepo::rev_parse`] return [`Err`] for `rev`, rather
    /// than the benign `Ok(None)` miss every other seed produces.
    ///
    /// Every `MemorySourceRepo` method is otherwise infallible, which makes
    /// the *genuine tool failure* arm of the [`SourceRepo`] contract —
    /// distinct from a miss, and the arm
    /// [`crate::change::resolve_drift_tip`] must propagate rather than
    /// swallow — untestable without spawning real git. This is the seam.
    #[must_use]
    pub fn with_failing_rev_parse(mut self, rev: &str) -> Self {
        self.failing_rev_parses.insert(rev.to_string());
        self
    }

    /// Makes [`SourceRepo::head`] return [`Err`] rather than answering.
    ///
    /// The companion to [`Self::with_failing_rev_parse`]; see its note on
    /// why failure injection is needed at all.
    #[must_use]
    pub fn with_failing_head(mut self) -> Self {
        self.failing_head = true;
        self
    }

    /// Seeds the [`SourceObjectKind`] of `path` at `rev`, for
    /// [`SourceRepo::object_kind_at`].
    ///
    /// Tests that only need a directory or submodule at a path (never its
    /// content) use this alone. A path seeded via [`Self::with_file`] does
    /// not need this too — [`SourceRepo::object_kind_at`] infers
    /// [`SourceObjectKind::Blob`] for any `(rev, path)` with seeded content
    /// and no explicit kind, so the ~dozen pre-existing `with_file`-only call
    /// sites keep compiling and passing unmodified.
    #[must_use]
    pub fn with_object_kind(mut self, rev: &str, path: &str, kind: SourceObjectKind) -> Self {
        self.object_kinds
            .insert((rev.to_string(), path.to_string()), kind);
        self
    }
}

impl SourceRepo for MemorySourceRepo {
    fn rev_parse(&self, rev: &str) -> Result<Option<String>> {
        if self.failing_rev_parses.contains(rev) {
            return Err(crate::error::Error::Git(format!(
                "seeded rev-parse failure for '{rev}'"
            )));
        }
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
        if self.failing_head {
            return Err(crate::error::Error::Git(
                "seeded HEAD lookup failure".to_string(),
            ));
        }
        Ok(self.head.clone())
    }

    fn current_branch(&self) -> Result<Option<String>> {
        Ok(self.branch.clone())
    }

    fn object_kind_at(&self, rev: &str, path: &str) -> Result<Option<SourceObjectKind>> {
        let key = (rev.to_string(), path.to_string());
        if let Some(kind) = self.object_kinds.get(&key) {
            return Ok(Some(*kind));
        }
        // Inference fallback: any pre-existing `with_file`-only seed reads as
        // a Blob, so tests written before this method existed keep passing.
        if self.files.contains_key(&key) {
            return Ok(Some(SourceObjectKind::Blob));
        }
        Ok(None)
    }
}

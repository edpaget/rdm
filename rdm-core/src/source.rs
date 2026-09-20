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

/// The outcome of a [`SourceRepo::object_kind_at`] lookup, as a **tri-state**.
///
/// The three outcomes are genuinely different situations and callers act
/// differently on each, so none of them is folded into another:
///
/// - [`Self::RevMissing`] — the revision itself is not in this checkout (an
///   unfetched branch, a commit that only exists on another machine). Nothing
///   about the path is known, and the remedy is environmental.
/// - [`Self::PathMissing`] — the revision resolves, but names no entry at
///   `path`. The remedy is to fix the path.
/// - [`Self::Kind`] — the entry exists, and this is what kind it is.
///
/// Before this existed, the first two both returned `Ok(None)` and every
/// caller reported the *path* as missing, so a checkout lacking the reviewed
/// commit claimed each anchored file "no longer exists" — a confidently false
/// statement about a file that is present in every checkout that has the
/// commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceObjectLookup {
    /// The revision does not resolve in this repository at all.
    RevMissing,
    /// The revision resolves, but holds no entry at the path.
    PathMissing,
    /// The path names an entry of this kind at the revision.
    Kind(SourceObjectKind),
}

/// A read-only view of a git-like source repository.
///
/// Every method distinguishes a *benign* miss — an unknown revision, a path
/// absent at that revision, two histories with no common ancestor — from a
/// genuine failure: the miss is `Ok(None)` (or, for
/// [`Self::object_kind_at`], whichever [`SourceObjectLookup`] miss occurred),
/// and [`Err`] is reserved for a genuine tool failure (git missing,
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
    /// (directory), or a gitlink (submodule) — or which of the two *misses*
    /// occurred.
    ///
    /// Answers via git's own object typing rather than by inspecting
    /// content, so a text blob whose bytes happen to resemble a directory
    /// listing is still correctly reported as [`SourceObjectKind::Blob`].
    ///
    /// The result is a [`SourceObjectLookup`] tri-state precisely because an
    /// unresolvable `rev` and a `path` absent at a resolvable one demand
    /// different messages and different remedies: the first is an
    /// environmental skip (fetch the branch, or point `source.repo` at the
    /// right checkout), the second is a genuine statement about the path. An
    /// implementation must never report the one as the other.
    ///
    /// An entry whose recorded type is unrecognizable is reported as
    /// [`SourceObjectLookup::PathMissing`], deliberately: git records only
    /// `blob`/`tree`/`commit`, so nothing can produce it, and folding it into
    /// the ineligible-anchor arm is the fail-safe choice.
    ///
    /// # Errors
    ///
    /// Returns an error when the repository cannot be queried at all, or
    /// [`crate::error::Error::InvalidChangeRevisionInput`] for option-shaped
    /// input, before any subprocess.
    fn object_kind_at(&self, rev: &str, path: &str) -> Result<SourceObjectLookup>;

    /// The human-readable place this repository was read from, for an
    /// operator-facing message.
    ///
    /// `None` when the implementation has no filesystem identity to name (an
    /// in-memory double). Core needs this so
    /// [`crate::error::Error::ChangeHeadNotInSource`] can name the checkout
    /// it consulted without the CLI re-composing the message.
    fn location(&self) -> Option<String> {
        None
    }
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
    /// What [`SourceRepo::location`] reports.
    location: Option<String>,
    /// Revisions [`SourceRepo::object_kind_at`] must report as
    /// [`SourceObjectLookup::RevMissing`] even though content is seeded for
    /// them — see [`Self::with_missing_rev`].
    missing_revs: HashSet<String>,
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

    /// Seeds what [`SourceRepo::location`] reports.
    ///
    /// Defaults to `None` (an in-memory repository has no filesystem
    /// identity), so a test that wants to see
    /// [`crate::error::Error::ChangeHeadNotInSource`]'s *named*-location
    /// rendering opts in explicitly and a test that wants the degraded
    /// rendering simply leaves it alone.
    #[must_use]
    pub fn with_location(mut self, location: &str) -> Self {
        self.location = Some(location.to_string());
        self
    }

    /// Makes [`SourceRepo::object_kind_at`] report `rev` as
    /// [`SourceObjectLookup::RevMissing`], the way a real checkout that does
    /// not contain the commit does.
    ///
    /// A rev with no [`Self::with_rev`] seed is already `RevMissing`; this
    /// exists for the harder shape the tri-state branches need — a review
    /// whose head has seeded *content* (so a test can assert that the content
    /// is deliberately not reached) while the lookup reports the commit as
    /// absent. Without it, "the checkout does not have this commit" and
    /// "nothing is seeded at all" are indistinguishable.
    ///
    /// [`SourceRepo::rev_parse`] misses on it too, so the double stays
    /// coherent: a repository cannot resolve a commit it does not have.
    #[must_use]
    pub fn with_missing_rev(mut self, rev: &str) -> Self {
        self.missing_revs.insert(rev.to_string());
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
        // A rev seeded as absent misses here too — see `with_missing_rev`.
        if self.missing_revs.contains(rev) {
            return Ok(None);
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

    fn object_kind_at(&self, rev: &str, path: &str) -> Result<SourceObjectLookup> {
        // An explicitly missing rev answers before anything else, so a test
        // can hold a fully-seeded review and still express "this checkout
        // does not contain the reviewed commit".
        if self.missing_revs.contains(rev) {
            return Ok(SourceObjectLookup::RevMissing);
        }
        let key = (rev.to_string(), path.to_string());
        if let Some(kind) = self.object_kinds.get(&key) {
            return Ok(SourceObjectLookup::Kind(*kind));
        }
        // Inference fallback: any pre-existing `with_file`-only seed reads as
        // a Blob, so tests written before this method existed keep passing.
        if self.files.contains_key(&key) {
            return Ok(SourceObjectLookup::Kind(SourceObjectKind::Blob));
        }
        // Otherwise: a rev this repository knows anything about at all holds
        // nothing at this path; a rev it has never heard of is absent. "Knows
        // anything about" deliberately includes a rev that only appears as a
        // `with_file`/`with_object_kind` key, so the ~dozen seeds that name a
        // revision only through its content keep reporting a missing PATH
        // (the pre-tri-state meaning of their `Ok(None)`) rather than
        // silently becoming missing COMMITS.
        let rev_known = self.revs.contains_key(rev)
            || self.files.keys().any(|(r, _)| r == rev)
            || self.object_kinds.keys().any(|(r, _)| r == rev)
            || self.diffs.keys().any(|(b, h, _)| b == rev || h == rev)
            || self.head.as_deref() == Some(rev);
        if rev_known {
            Ok(SourceObjectLookup::PathMissing)
        } else {
            Ok(SourceObjectLookup::RevMissing)
        }
    }

    fn location(&self) -> Option<String> {
        self.location.clone()
    }
}

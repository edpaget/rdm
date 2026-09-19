//! The read-only **worktree** port.
//!
//! The `reviewed` transition gate ([`crate::ops::gate`]) has to answer one
//! question about the *source* repository — "is the worktree this item is
//! being implemented in clean?" — without rdm-core depending on git. This is
//! exactly the separation [`SourceRepo`](crate::source::SourceRepo) /
//! [`MemorySourceRepo`](crate::source::MemorySourceRepo) already keep for a
//! `change/<sha>` review's anchors, and this port is shaped verbatim on it.
//!
//! [`WorktreeProbe`] is therefore a trait of **reads only**: given an item,
//! report the worktree rdm knows for it and whether that worktree has
//! uncommitted changes. There is no write method, and there never should be —
//! the invariant that evaluating a gate never mutates the source repository is
//! enforced by this port's shape, not by discipline at each call site.
//! `rdm-git`'s `GitWorktreeProbe` is the production implementation;
//! [`MemoryWorktreeProbe`] is the in-memory double [`crate::ops::gate`] is
//! unit-tested against.
//!
//! [`parse_porcelain`] is the pure `git status --porcelain` reader both
//! implementations share. It is deliberately **fail-closed**, mirroring
//! `parseWorktreeStatus` in `.claude/workflows/lib/dispatch-phase.mjs` — the
//! JS enforcement this gate moves into Rust — so the two can never disagree
//! about what "clean" means.

use std::collections::HashMap;

use crate::error::Result;
use crate::link::ItemRef;

/// How many uncommitted paths a refusal names before it truncates.
///
/// Matches `WORKTREE_PATH_CAP` in `.claude/workflows/lib/dispatch-phase.mjs`:
/// enough to identify what is dirty, few enough that a forgotten `target/`
/// does not bury the message.
pub const WORKTREE_PATH_CAP: usize = 20;

/// What a [`WorktreeProbe`] reports about one item's worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeCheck {
    /// Absolute path of the worktree, as the refusal should name it.
    pub path: String,
    /// Observed full source HEAD, when available.
    pub head: Option<String>,
    /// Observed source branch, when available.
    pub branch: Option<String>,
    /// Uncommitted paths, already capped at [`WORKTREE_PATH_CAP`]. Empty
    /// **and** `observable` means clean.
    pub dirty: Vec<String>,
    /// How many further dirty paths were dropped by the cap.
    pub truncated: usize,
    /// Whether the status output could be read at all.
    ///
    /// `false` is the fail-closed branch: `git status` produced text that
    /// yielded no parsable path, so rdm cannot tell whether the worktree is
    /// clean. The gate reports that as *unobservable*, never as clean —
    /// matching `parseWorktreeStatus`'s `{ ran: false, clean: false }` in
    /// `.claude/workflows/lib/dispatch-phase.mjs`.
    pub observable: bool,
}

impl WorktreeCheck {
    /// Builds a check for `path` from raw `git status --porcelain` output.
    ///
    /// Fail-closed: output that carries a non-empty line but yields no
    /// parsable path sets [`observable`](WorktreeCheck::observable) to
    /// `false` rather than reading as clean.
    #[must_use]
    pub fn from_porcelain(path: &str, porcelain: &str) -> Self {
        let (dirty, truncated) = parse_porcelain(porcelain);
        let had_text = porcelain.lines().any(|l| !l.trim().is_empty());
        Self {
            path: path.to_string(),
            head: None,
            branch: None,
            observable: !had_text || !dirty.is_empty(),
            dirty,
            truncated,
        }
    }

    /// Whether the worktree was observed to have no uncommitted changes.
    ///
    /// An *unobservable* worktree is never clean.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.observable && self.dirty.is_empty()
    }
}

/// Explicit inputs for a source-bound review. Resolution never creates a checkout.
#[derive(Debug, Clone, Default)]
pub struct ReviewSourceRequest {
    /// Optional registered checkout; tasks require an explicit base with this binding.
    pub path: Option<String>,
    /// Base revision; otherwise use the configured default branch's merge base.
    pub base: Option<String>,
    /// Expected full HEAD; refusal on drift.
    pub expected_head: Option<String>,
    /// Expected branch; refusal on drift.
    pub expected_branch: Option<String>,
    /// Configured source default branch.
    pub default_branch: String,
    /// Deliberate declaration that an empty committed diff is reviewable.
    pub no_code: bool,
}

/// Canonical committed source identity shared by every review stage.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewSource {
    /// Canonical reviewed item.
    pub item: String,
    /// Canonical common git directory identifying the source repository.
    pub repository: String,
    /// Canonical registered source checkout.
    pub path: String,
    /// Observed branch.
    pub branch: String,
    /// Full resolved base SHA.
    pub base: String,
    /// Full observed HEAD SHA.
    pub head: String,
    /// Committed changed paths relative to the repository root.
    pub changed_files: Vec<String>,
    /// Exact committed diff, with external diff drivers disabled.
    pub diff_text: String,
    /// Explicit empty-diff declaration.
    pub no_code: bool,
}

/// The single source-selection policy for probes and standalone reviews.
/// Phases always use the shared roadmap checkout, never an obsolete phase branch.
#[must_use]
pub fn review_worktree_item(item: &ItemRef) -> Option<String> {
    match item {
        ItemRef::Phase { roadmap, .. } | ItemRef::Roadmap { roadmap } => Some(roadmap.clone()),
        ItemRef::Task { slug } => Some(format!("task/{slug}")),
        _ => None,
    }
}

/// Resolves and validates a source identity through the read-only port.
///
/// # Errors
/// Returns `Error::Git` for unavailable, mismatched, moved or unexpectedly empty source.
pub fn resolve_review_source(
    probe: &dyn WorktreeProbe,
    item: &ItemRef,
    request: &ReviewSourceRequest,
) -> Result<ReviewSource> {
    let fail = |message: &str| crate::error::Error::Git(format!("review source: {message}"));
    if !matches!(item, ItemRef::Phase { .. } | ItemRef::Task { .. }) {
        return Err(fail("requires a phase or task"));
    }
    if matches!(item, ItemRef::Task { .. }) && request.path.is_some() && request.base.is_none() {
        return Err(fail("explicit task checkout requires --base"));
    }
    let source = probe.review_source(item, request)?;
    if source.item != item.label() {
        return Err(fail("returned item differs from intended item"));
    }
    if request
        .expected_head
        .as_ref()
        .is_some_and(|h| h != &source.head)
    {
        return Err(fail("HEAD moved; restart review at the new commit"));
    }
    if request
        .expected_branch
        .as_ref()
        .is_some_and(|b| b != &source.branch)
    {
        return Err(fail("branch changed; restart review"));
    }
    if source.changed_files.is_empty() && !request.no_code {
        return Err(fail(
            "empty committed range; declare --no-code only for intentional no-code review",
        ));
    }
    Ok(source)
}

/// A read-only view of the worktrees rdm manages for plan items.
pub trait WorktreeProbe {
    /// The worktree rdm knows for `item`, and whether it is clean.
    ///
    /// `Ok(None)` is a *benign* miss — rdm manages no worktree for this item,
    /// which is the "if `rdm worktree` knows one" escape clause the gate's
    /// worktree precondition carries. [`Err`] never means "no worktree"; the
    /// gate treats every error as a refusal, never as a clean worktree.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::Error::Git`] when the repository cannot be
    /// queried at all (git missing, the repo unreadable, a failing
    /// `git status`) — that variant is reserved for exactly that condition.
    ///
    /// Returns [`crate::error::Error::ReviewSourceItemMismatch`] when the
    /// probe is bound to one item and asked about another (a caller bug),
    /// and [`crate::error::Error::ReviewSourceBranchChanged`] when the
    /// registered checkout has been switched off the branch it was
    /// registered on. Both are caller- or operator-visible conditions, not
    /// query failures, which is why they are matchable apart from
    /// [`crate::error::Error::Git`].
    fn worktree_for(&self, item: &ItemRef) -> Result<Option<WorktreeCheck>>;

    /// Reads a registered source checkout and its committed range.
    ///
    /// # Errors
    /// Returns `Error::Git` when source inspection is unavailable or fails.
    fn review_source(
        &self,
        _item: &ItemRef,
        _request: &ReviewSourceRequest,
    ) -> Result<ReviewSource> {
        Err(crate::error::Error::Git(
            "review source inspection unavailable".to_string(),
        ))
    }
}

/// Parses `git status --porcelain` (v1) output into the uncommitted paths it
/// names, capped at [`WORKTREE_PATH_CAP`].
///
/// Returns `(paths, truncated)` where `truncated` is how many further paths
/// the cap dropped. An empty return means genuinely clean.
///
/// Behavior, matching `parseWorktreeStatus` in
/// `.claude/workflows/lib/dispatch-phase.mjs` exactly:
///
/// - Porcelain v1 lines are `XY <path>`: two status characters and one space.
///   The path is taken by *offset*, never by splitting on whitespace, so a
///   path containing spaces survives intact.
/// - A rename/copy entry reads `old -> new`; the **destination** is reported,
///   because that is the path that exists now.
/// - Untracked (`??`) entries count as dirty.
/// - A trailing newline must not manufacture a phantom path: empty lines are
///   dropped before parsing.
///
/// # Examples
///
/// ```
/// use rdm_core::worktree::parse_porcelain;
///
/// let (paths, truncated) = parse_porcelain(" M src/a b.rs\nR  old.rs -> new.rs\n?? junk\n");
/// assert_eq!(paths, vec!["src/a b.rs", "new.rs", "junk"]);
/// assert_eq!(truncated, 0);
/// assert_eq!(parse_porcelain("\n\n"), (Vec::new(), 0));
/// ```
#[must_use]
pub fn parse_porcelain(text: &str) -> (Vec<String>, usize) {
    let mut paths: Vec<String> = Vec::new();
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            continue;
        }
        // `XY <path>`: take by offset, never by whitespace splitting — a path
        // may contain spaces (and may be quoted).
        // `get` (not a bare index) so a garbage line that happens to split a
        // multi-byte character at byte 3 yields nothing instead of panicking.
        let mut rest = line.get(3..).unwrap_or("");
        // `old -> new`: the destination is the path that exists now.
        if let Some(idx) = rest.find(" -> ") {
            rest = &rest[idx + 4..];
        }
        let rest = rest.trim();
        if !rest.is_empty() {
            paths.push(rest.to_string());
        }
    }
    let truncated = paths.len().saturating_sub(WORKTREE_PATH_CAP);
    paths.truncate(WORKTREE_PATH_CAP);
    (paths, truncated)
}

/// An in-memory [`WorktreeProbe`] for tests, with no git dependency.
///
/// Every item's answer is seeded explicitly, so a test can construct exactly
/// the shape it needs — including an *unobservable* worktree, which is awkward
/// to produce with real git but is precisely the fail-closed branch the gate
/// must get right.
#[derive(Debug, Default, Clone)]
pub struct MemoryWorktreeProbe {
    /// `item.label()` → the check to report.
    known: HashMap<String, WorktreeCheck>,
    /// `item.label()`s whose probe fails outright.
    errors: Vec<String>,
}

impl MemoryWorktreeProbe {
    /// A probe that knows no worktrees: every lookup is a benign miss.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Seeds `item` as having a worktree at `path` with these `dirty` paths
    /// (an empty slice = clean).
    #[must_use]
    pub fn with_worktree(mut self, item: &ItemRef, path: &str, dirty: &[&str]) -> Self {
        self.known.insert(
            item.label(),
            WorktreeCheck {
                path: path.to_string(),
                head: None,
                branch: None,
                dirty: dirty.iter().map(|s| (*s).to_string()).collect(),
                truncated: 0,
                observable: true,
            },
        );
        self
    }

    /// Sets the observed HEAD of a seeded worktree.
    #[must_use]
    pub fn with_head(mut self, item: &ItemRef, head: &str) -> Self {
        if let Some(check) = self.known.get_mut(&item.label()) {
            check.head = Some(head.to_string());
        }
        self
    }

    /// Seeds `item` as unobservable — the probe returns [`Err`].
    #[must_use]
    pub fn with_error(mut self, item: &ItemRef) -> Self {
        self.errors.push(item.label());
        self
    }
}

impl WorktreeProbe for MemoryWorktreeProbe {
    fn worktree_for(&self, item: &ItemRef) -> Result<Option<WorktreeCheck>> {
        let label = item.label();
        if self.errors.contains(&label) {
            return Err(crate::error::Error::Git(format!(
                "simulated probe failure for {label}"
            )));
        }
        Ok(self.known.get(&label).cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_and_newline_only_porcelain_is_clean() {
        assert_eq!(parse_porcelain(""), (Vec::new(), 0));
        assert_eq!(parse_porcelain("\n"), (Vec::new(), 0));
        assert_eq!(parse_porcelain("\n\n\n"), (Vec::new(), 0));
    }

    #[test]
    fn a_path_with_spaces_is_never_split() {
        let (paths, _) = parse_porcelain(" M docs/my notes.md\n");
        assert_eq!(paths, vec!["docs/my notes.md"]);
    }

    #[test]
    fn a_rename_reports_the_destination() {
        let (paths, _) = parse_porcelain("R  old/name.rs -> new/name.rs\n");
        assert_eq!(paths, vec!["new/name.rs"]);
    }

    #[test]
    fn untracked_entries_count_as_dirty() {
        let (paths, _) = parse_porcelain("?? scratch.txt\n");
        assert_eq!(paths, vec!["scratch.txt"]);
    }

    #[test]
    fn crlf_line_endings_do_not_leak_into_paths() {
        let (paths, _) = parse_porcelain(" M a.rs\r\n M b.rs\r\n");
        assert_eq!(paths, vec!["a.rs", "b.rs"]);
    }

    #[test]
    fn the_path_cap_truncates_and_reports_the_remainder() {
        let mut raw = String::new();
        for i in 0..25 {
            raw.push_str(&format!(" M f{i}.rs\n"));
        }
        let (paths, truncated) = parse_porcelain(&raw);
        assert_eq!(paths.len(), WORKTREE_PATH_CAP);
        assert_eq!(truncated, 5);
        assert_eq!(paths[0], "f0.rs");
        assert_eq!(paths[19], "f19.rs");
    }

    #[test]
    fn memory_probe_reports_miss_hit_and_error() {
        let clean = ItemRef::Task {
            slug: "clean".to_string(),
        };
        let dirty = ItemRef::Task {
            slug: "dirty".to_string(),
        };
        let broken = ItemRef::Task {
            slug: "broken".to_string(),
        };
        let unknown = ItemRef::Task {
            slug: "unknown".to_string(),
        };
        let probe = MemoryWorktreeProbe::new()
            .with_worktree(&clean, "/wt/clean", &[])
            .with_worktree(&dirty, "/wt/dirty", &["src/a.rs"])
            .with_error(&broken);

        assert!(probe.worktree_for(&unknown).unwrap().is_none());
        assert!(probe.worktree_for(&clean).unwrap().unwrap().is_clean());
        let d = probe.worktree_for(&dirty).unwrap().unwrap();
        assert_eq!(d.path, "/wt/dirty");
        assert_eq!(d.dirty, vec!["src/a.rs"]);
        assert!(probe.worktree_for(&broken).is_err());
    }

    #[test]
    fn from_porcelain_builds_a_check() {
        let c = WorktreeCheck::from_porcelain("/wt/x", " M a.rs\n");
        assert_eq!(c.path, "/wt/x");
        assert!(!c.is_clean());
        assert_eq!(c.dirty, vec!["a.rs"]);
        assert!(WorktreeCheck::from_porcelain("/wt/x", "").is_clean());
        assert!(WorktreeCheck::from_porcelain("/wt/x", "   \n").is_clean());
    }

    #[test]
    fn text_that_yields_no_path_is_unobservable_not_clean() {
        // Fail-closed: `git status` said *something* rdm could not read, so
        // the worktree's state is unknown rather than clean.
        let c = WorktreeCheck::from_porcelain("/wt/x", "??\n");
        assert!(!c.observable);
        assert!(!c.is_clean());
        assert!(c.dirty.is_empty());
    }

    #[test]
    fn a_line_whose_byte_3_splits_a_character_does_not_panic() {
        // "ab" is 2 bytes, so the following multi-byte character spans bytes
        // 2..5 and byte 3 lands *inside* it. A bare `&line[3..]` would panic
        // here; the parser must yield nothing instead.
        let (paths, _) = parse_porcelain("ab\u{65e5}\u{672c}\n");
        assert!(paths.is_empty(), "got {paths:?}");
        // A non-ASCII path whose status prefix is well-formed still parses.
        let (paths, _) = parse_porcelain(" M \u{65e5}\u{672c}.md\n");
        assert_eq!(paths, vec!["\u{65e5}\u{672c}.md"]);
    }
}

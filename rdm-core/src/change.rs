//! Cross-repository change review: anchoring a review comment to a quoted
//! span of a **source-repository** file, and resolving it again later.
//!
//! This is the correctness-critical half of the `change/<head>` review
//! target ([`ReviewTarget::Change`]). Everything here is pure with respect
//! to git: it consumes a [`SourceRepo`] port and unified-diff *text*, so
//! every rule below is unit-testable against
//! [`MemorySourceRepo`](crate::source::MemorySourceRepo) with no process
//! spawned and no temp repository on disk.
//!
//! # The anchoring model
//!
//! A `--path`/`--quote` comment is derived at review time against the file
//! content **at the review target's `head`** and is restricted to the hunks
//! the change touches: a quote that lands entirely outside every touched
//! hunk is an error naming the nearest one ([`derive_file_quote`]). The
//! resulting [`Anchor::FileQuote`] records the path, the quote, its 1-based
//! occurrence, and the line range it occupied at `head` — and deliberately
//! nothing else, so a plan-repo review file never embeds source content
//! beyond the quote the reviewer chose.
//!
//! Later, [`resolve_change_comment`] compares that recorded quote against
//! the same path at a **tip** revision, chosen by [`resolve_drift_tip`]:
//!
//! - the path is gone at the tip → [`Resolution::Unresolved`]
//! - the tip holds at least as many occurrences of the quote as `head`
//!   did, **and** one of those occurrences still sits between the same
//!   neighboring lines the anchored one sat between at `head` → `Original
//!   { drifted: false }`
//! - otherwise → `Original { drifted: true }`
//!
//! The test is occurrence *identity*, not a bare substring search and not a
//! bare count. Two clauses, in that order:
//!
//! 1. **Count.** When a file holds the same text twice and the author edits
//!    exactly the occurrence the reviewer anchored to, the surviving
//!    *other* copy would satisfy a `contains` check, and the comment would
//!    read "still true" although the line it named is gone. Requiring the
//!    tip to retain at least as many occurrences as `head` had reports that
//!    as drift.
//! 2. **Line window.** A count alone is still fooled by a single commit
//!    that edits the anchored occurrence away *and* introduces a fresh copy
//!    of the same text elsewhere: the count is unchanged, yet the anchor is
//!    genuinely drifted. So the tip must also hold an occurrence whose
//!    immediately neighboring lines are the ones the anchored occurrence
//!    sat between at `head`.
//!
//! The window is deliberately neighbor-line-based rather than positional,
//! so a pure relocation — a block that moves *with* its neighbors — still
//! resolves: the reviewer's words are still true of it. Neighbors that do
//! not exist head-side (the quote is at the start or end of the file) are
//! not compared, and a quote whose line-extended span is the whole file
//! degenerates to the count clause alone rather than reporting permanent
//! drift. The anchored occurrence's *own* line is not compared byte for
//! byte either: it already carries the quote, and an edit beside the quote
//! on the same line — a trailing comment, say — leaves the reviewer's words
//! true of it.
//!
//! Because clause 1 is retained unchanged as the first term of the
//! disjunction, the window clause can only ever *add* drift detections; no
//! anchor that reports drifted under a pure count can start reporting
//! resolved.
//!
//! The reported byte range always indexes the **head-side** content, matching
//! [`Resolution::Original`]'s "the body the reviewer saw" contract.
//!
//! # Line endings and content source
//!
//! Both derivation and resolution read through [`SourceRepo::file_at`],
//! which is contractually the object-database content (`git show
//! <rev>:<path>`), never the working tree. A checkout with
//! `core.autocrlf`, a dirty file, or a different trailing newline therefore
//! cannot flip a resolved anchor to drifted.

use std::ops::Range;

use crate::anchor::{QuoteOccurrence, Resolution, ResolvedComment};
use crate::error::{Error, Result};
use crate::link::Link;
use crate::model::{Anchor, Review, ReviewComment, ReviewTarget};
use crate::source::SourceRepo;

/// A contiguous run of head-side lines a change touches, 1-based and
/// inclusive on both ends.
///
/// An insertion-only hunk whose head-side count is `0` (`@@ -3,2 +2,0 @@`, a
/// pure deletion) is normalized to the single line it abuts, so "the
/// nearest touched hunk" is always a real, quotable position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HunkRange {
    /// First head-side line the hunk covers (1-based).
    pub start: u32,
    /// Last head-side line the hunk covers (1-based, inclusive).
    pub end: u32,
}

/// Parses the head-side (`+`) line ranges out of a unified diff.
///
/// Reads only `@@ -a[,b] +c[,d] @@` headers, so it is independent of the
/// context width the diff was produced with (rdm asks for `--unified=0`).
/// Both the omitted-count form (`+c`, meaning one line) and the
/// zero-count form (`+c,0`, a pure deletion) are handled; the latter is
/// normalized to the line it abuts.
///
/// A new file's diff is a single `@@ -0,0 +1,N @@` hunk covering the whole
/// file, which needs no special case.
///
/// # Examples
///
/// ```
/// use rdm_core::change::parse_hunks;
///
/// let hunks = parse_hunks("@@ -1,0 +2,3 @@\n+a\n+b\n+c\n");
/// assert_eq!(hunks.len(), 1);
/// assert_eq!((hunks[0].start, hunks[0].end), (2, 4));
/// ```
#[must_use]
pub fn parse_hunks(unified_diff: &str) -> Vec<HunkRange> {
    let mut out = Vec::new();
    for line in unified_diff.lines() {
        let Some(rest) = line.strip_prefix("@@ ") else {
            continue;
        };
        // `-a,b +c,d @@ …` — take the `+`-prefixed field.
        let Some(plus) = rest.split_whitespace().find(|f| f.starts_with('+')) else {
            continue;
        };
        let plus = &plus[1..];
        let (start_str, count_str) = match plus.split_once(',') {
            Some((s, c)) => (s, Some(c)),
            None => (plus, None),
        };
        let Ok(start) = start_str.parse::<u32>() else {
            continue;
        };
        let count: u32 = match count_str {
            None => 1,
            Some(c) => match c.parse() {
                Ok(n) => n,
                Err(_) => continue,
            },
        };
        if count == 0 {
            // Pure deletion: no head-side line is part of the hunk. Report
            // the line it abuts so `nearest_hunk` still has somewhere to
            // point. `+0,0` (deleting the whole file) clamps to line 1.
            let at = start.max(1);
            out.push(HunkRange { start: at, end: at });
        } else {
            out.push(HunkRange {
                start,
                end: start + count - 1,
            });
        }
    }
    out
}

/// The 1-based, inclusive line range a byte range occupies within
/// `content`.
///
/// The start line counts the newlines before `range.start`; the end line
/// counts them before the last byte of the span. An empty range reports the
/// single line it sits on.
///
/// Offsets are floored to the enclosing character boundary before slicing,
/// so a span whose last byte sits inside a multi-byte character — any quote
/// ending in an accented letter, an em-dash or a curly quote — reports its
/// line rather than panicking. Flooring cannot change the answer: `\n` is
/// ASCII, so it never occurs inside a multi-byte character.
///
/// # Examples
///
/// ```
/// use rdm_core::change::line_range_of;
///
/// let content = "one\ntwo\nthree\n";
/// assert_eq!(line_range_of(content, 4..7), (2, 2));
/// assert_eq!(line_range_of(content, 0..12), (1, 3));
///
/// // A span whose final byte is inside a multi-byte character.
/// let content = "a\ncafé\n";
/// let start = content.find("café").unwrap();
/// assert_eq!(line_range_of(content, start..start + "café".len()), (2, 2));
/// ```
///
/// # Panics
///
/// Never panics.
#[must_use]
pub fn line_range_of(content: &str, range: Range<usize>) -> (u32, u32) {
    let start = range.start.min(content.len());
    let end_inclusive = range.end.saturating_sub(1).max(start).min(content.len());
    let line_of = |offset: usize| -> u32 {
        // Floor to a character boundary: `range.end - 1` lands inside a
        // multi-byte character whenever the span ends in one, and slicing
        // there would panic.
        let mut at = offset.min(content.len());
        while at > 0 && !content.is_char_boundary(at) {
            at -= 1;
        }
        u32::try_from(content[..at].matches('\n').count() + 1).unwrap_or(u32::MAX)
    };
    (line_of(start), line_of(end_inclusive))
}

/// The hunk nearest to the 1-based inclusive line range `(start, end)`,
/// measured by gap; ties go to the earlier hunk.
///
/// Returns `None` only when `hunks` is empty.
#[must_use]
pub fn nearest_hunk(hunks: &[HunkRange], start: u32, end: u32) -> Option<HunkRange> {
    // Gap to the range, 0 when they overlap. Saturating on both sides, so the
    // subtraction order can never underflow and at most one term is non-zero.
    hunks
        .iter()
        .copied()
        .min_by_key(|h| start.saturating_sub(h.end).max(h.start.saturating_sub(end)))
}

/// Whether `(start, end)` overlaps at least one hunk.
///
/// Overlap, not containment: a quote covering an edited line *and* its
/// unchanged neighbour is a legitimate finding and anchors.
fn intersects_any(hunks: &[HunkRange], start: u32, end: u32) -> bool {
    hunks.iter().any(|h| h.start <= end && start <= h.end)
}

/// Builds the display context for one quote occurrence, mirroring
/// [`crate::anchor::derive_text_quote`]'s ambiguity reporting so a change
/// review never invents a second disambiguation vocabulary.
fn occurrence_context(content: &str, start: usize, quote_len: usize) -> String {
    let before = {
        let s = &content[..start];
        match s.char_indices().rev().nth(15) {
            Some((i, _)) => &s[i..],
            None => s,
        }
    };
    let after = {
        let s = &content[start + quote_len..];
        match s.char_indices().nth(16) {
            Some((i, _)) => &s[..i],
            None => s,
        }
    };
    format!(
        "…{}[{}]{}…",
        before.replace('\n', " "),
        content[start..start + quote_len].replace('\n', " "),
        after.replace('\n', " ")
    )
}

/// Derives an [`Anchor::FileQuote`] for `quote` within `content`, requiring
/// the quote to overlap at least one touched hunk.
///
/// Occurrence and ambiguity semantics are exactly
/// [`crate::anchor::derive_text_quote`]'s: a unique match wins, multiple
/// matches require a 1-based `occurrence`, and occurrences are counted
/// without overlap.
///
/// `range_label` names the reviewed `base..head` range and is used only to
/// make the out-of-hunk error actionable.
///
/// # Errors
///
/// Returns [`Error::QuoteNotFound`] when `quote` is empty or absent,
/// [`Error::QuoteAmbiguous`] when it occurs more than once and `occurrence`
/// is `None`, [`Error::QuoteOccurrenceOutOfRange`] when `occurrence`
/// exceeds the match count, or [`Error::QuoteOutsideChangedHunks`] when the
/// located span overlaps no hunk (naming the nearest one, or saying the
/// path is untouched when `hunks` is empty).
pub fn derive_file_quote(
    content: &str,
    path: &str,
    quote: &str,
    occurrence: Option<usize>,
    hunks: &[HunkRange],
    range_label: &str,
) -> Result<Anchor> {
    if quote.is_empty() {
        return Err(Error::QuoteNotFound {
            quote: quote.to_string(),
            commit: None,
        });
    }
    let starts: Vec<usize> = content.match_indices(quote).map(|(i, _)| i).collect();
    let (start, picked) = match (starts.as_slice(), occurrence) {
        ([], _) => {
            return Err(Error::QuoteNotFound {
                quote: quote.to_string(),
                commit: None,
            });
        }
        ([only], None) => (*only, 1usize),
        (many, None) => {
            return Err(Error::QuoteAmbiguous {
                quote: quote.to_string(),
                commit: None,
                occurrences: many
                    .iter()
                    .enumerate()
                    .map(|(i, &s)| QuoteOccurrence {
                        occurrence: i + 1,
                        context: occurrence_context(content, s, quote.len()),
                    })
                    .collect(),
            });
        }
        (many, Some(n)) => {
            if n == 0 || n > many.len() {
                return Err(Error::QuoteOccurrenceOutOfRange {
                    quote: quote.to_string(),
                    occurrence: n,
                    available: many.len(),
                });
            }
            (many[n - 1], n)
        }
    };
    let (start_line, end_line) = line_range_of(content, start..start + quote.len());
    if !intersects_any(hunks, start_line, end_line) {
        return Err(Error::QuoteOutsideChangedHunks {
            path: path.to_string(),
            start_line,
            end_line,
            nearest: nearest_hunk(hunks, start_line, end_line).map(|h| (h.start, h.end)),
            range: range_label.to_string(),
        });
    }
    Ok(Anchor::FileQuote {
        path: path.to_string(),
        quote: quote.to_string(),
        occurrence: u32::try_from(picked).unwrap_or(1),
        start_line,
        end_line,
    })
}

/// Pins a `change/<rev>` review target against the source repository.
///
/// This is the *construction* half of the change-review target, the mirror
/// of [`resolve_change_comment`]'s resolution half, and it lives here for
/// the same reason: every rule below is a domain rule, and every one of
/// them is unit-testable against
/// [`MemorySourceRepo`](crate::source::MemorySourceRepo) with no process
/// spawned. The caller supplies only what is genuinely environmental — which
/// checkout to read (`source`) and which branch the project calls its
/// default (`default_branch`).
///
/// `rev` is rev-parsed to a full 40-character SHA, so `HEAD`, a branch name
/// and an abbreviation all pin to the same target and two reviews of one
/// commit compare equal.
///
/// `base` precedence: an explicit `base_override` (itself rev-parsed), else
/// the merge base of the head with `default_branch`. A missing merge base is
/// an error, never a silent fallback to the root commit or to the head —
/// either would empty the diff and put every quote outside every hunk.
///
/// Also returns the checkout's current branch, which the review stamps so
/// later drift is measured against the branch the change was on rather than
/// whatever `HEAD` happens to be by then. `None` on a detached HEAD, and a
/// repository that cannot answer at all degrades to `None` rather than
/// failing the whole `review start`.
///
/// # Errors
///
/// Returns [`Error::InvalidChangeRevisionInput`] when the source adapter rejects
/// an unresolved revision operand. The Git adapter rejects option-shaped `rev`,
/// `base_override`, or `default_branch` values when that operand is used.
///
/// Returns [`Error::InvalidStoredChangeRevision`] for malformed resolved change
/// identities (head or present base must be 40 lowercase ASCII hex characters).
///
/// Returns [`Error::ChangeRevisionNotFound`] when `rev` names no commit,
/// [`Error::ChangeBaseNotFound`] when `base_override` names no commit,
/// [`Error::ChangeNoMergeBase`] when head and `default_branch` share no
/// ancestor, or whatever error the [`SourceRepo`] itself reports when it
/// cannot be queried.
///
/// # Examples
///
/// ```
/// use rdm_core::change::resolve_change_target;
/// use rdm_core::model::ReviewTarget;
/// use rdm_core::source::MemorySourceRepo;
///
/// let head = "a".repeat(40);
/// let base = "b".repeat(40);
/// let source = MemorySourceRepo::new()
///     .with_rev("HEAD", &head)
///     .with_merge_base(&head, "main", &base)
///     .with_branch("phase/auth/phase-1-design");
///
/// let (target, branch) = resolve_change_target(&source, "HEAD", None, "main").unwrap();
/// assert_eq!(target, ReviewTarget::Change { head, base: Some(base) });
/// assert_eq!(branch.as_deref(), Some("phase/auth/phase-1-design"));
/// ```
pub fn resolve_change_target(
    source: &impl SourceRepo,
    rev: &str,
    base_override: Option<&str>,
    default_branch: &str,
) -> Result<(ReviewTarget, Option<String>)> {
    let head = source
        .rev_parse(rev)?
        .ok_or_else(|| Error::ChangeRevisionNotFound(rev.to_string()))?;
    ReviewTarget::validate_change_identity(&head, None)?;
    let base =
        match base_override {
            Some(explicit) => source
                .rev_parse(explicit)?
                .ok_or_else(|| Error::ChangeBaseNotFound(explicit.to_string()))?,
            None => source.merge_base(&head, default_branch)?.ok_or_else(|| {
                Error::ChangeNoMergeBase {
                    head: head.clone(),
                    branch: default_branch.to_string(),
                }
            })?,
        };
    ReviewTarget::validate_change_identity(&head, Some(&base))?;
    // A repository that cannot name its branch is not a reason to refuse the
    // review: drift then falls back to the repository's HEAD.
    let branch = source.current_branch().ok().flatten();
    Ok((
        ReviewTarget::Change {
            head,
            base: Some(base),
        },
        branch,
    ))
}

/// Derives a `change/` review comment's anchor from an optional
/// `--path`/`--quote` pair.
///
/// The pair is all-or-nothing: neither means a whole-change comment
/// (`Ok(None)`), both anchor into the file, and either alone is an error
/// naming the missing flag. With both present, the file's content is read
/// **at `head`**, the hunks the change touches in it are computed from
/// `base..head`, and both are handed to [`derive_file_quote`], which
/// enforces the inside-a-touched-hunk rule.
///
/// Unlike [`resolve_change_comment`], this path **fails loudly** when the
/// source repository cannot answer: a stored anchor that was never checked
/// against real content would silently mislead every later reader.
///
/// A review with no recorded `base` (only reachable by hand-editing the
/// frontmatter) yields no hunks, so the error names the real problem rather
/// than anchoring blindly.
///
/// # Errors
///
/// Returns [`Error::InvalidStoredChangeRevision`] for malformed resolved change
/// identities (head or present base must be 40 lowercase ASCII hex characters).
///
/// Returns [`Error::ChangePathNeedsQuote`] or [`Error::ChangeQuoteNeedsPath`]
/// when only one of the pair is given, [`Error::ChangePathNotInRevision`]
/// when `path` is not repo-relative or does not exist at `head`,
/// [`Error::ChangePathNotLinkable`] when `path` contains `@` or `#`
/// (reserved by the `rdm:src/` permalink grammar — see
/// [`normalize_source_path`]), [`Error::ChangePathNotAFile`] when `path` names a directory or a
/// submodule at `head` rather than a regular file, anything
/// [`derive_file_quote`] returns (quote not found, ambiguous, occurrence out
/// of range, outside every touched hunk), or whatever error the
/// [`SourceRepo`] reports when it cannot be queried.
pub fn derive_change_anchor(
    source: &impl SourceRepo,
    head: &str,
    base: Option<&str>,
    quote: Option<&str>,
    path: Option<&str>,
    occurrence: Option<usize>,
) -> Result<Option<Anchor>> {
    ReviewTarget::validate_change_identity(head, base)?;
    let Some(quote) = quote else {
        if path.is_some() {
            return Err(Error::ChangePathNeedsQuote);
        }
        return Ok(None);
    };
    let Some(path) = path else {
        return Err(Error::ChangeQuoteNeedsPath);
    };
    let path = normalize_source_path(path)?;
    // Check git's own object type before reading any content: a directory
    // or submodule is rejected loudly here rather than being read (or, for
    // a directory, silently returning `git show`'s tree-listing text) and
    // anchored as if it were a file.
    match source.object_kind_at(head, &path)? {
        None | Some(crate::source::SourceObjectKind::Blob) => {}
        Some(found) => {
            return Err(Error::ChangePathNotAFile {
                path,
                rev: head.to_string(),
                found,
            });
        }
    }
    let content = source
        .file_at(head, &path)?
        .ok_or_else(|| Error::ChangePathNotInRevision {
            path: path.clone(),
            rev: head.to_string(),
        })?;
    let (hunks, range_label) = match base {
        Some(base) => {
            let diff = source.unified_diff(base, head, &path)?.unwrap_or_default();
            (
                parse_hunks(&diff),
                format!("{}..{}", abbreviate(base), abbreviate(head)),
            )
        }
        None => (Vec::new(), "this change".to_string()),
    };
    derive_file_quote(&content, &path, quote, occurrence, &hunks, &range_label).map(Some)
}

/// The first twelve characters of a revision, for an error message.
fn abbreviate(rev: &str) -> &str {
    let mut at = rev.len().min(12);
    while at > 0 && !rev.is_char_boundary(at) {
        at -= 1;
    }
    &rev[..at]
}

/// The head-side byte range an [`Anchor::FileQuote`] occupies in `content`.
///
/// Uses the recorded `occurrence` when it still selects a match, and falls
/// back to the first match otherwise (the anchor was derived against this
/// exact content, so the fallback only matters for a hand-edited review
/// file).
fn head_range(content: &str, quote: &str, occurrence: u32) -> Option<Range<usize>> {
    let starts: Vec<usize> = content.match_indices(quote).map(|(i, _)| i).collect();
    let idx = (occurrence.max(1) as usize) - 1;
    let start = *starts.get(idx).or_else(|| starts.first())?;
    Some(start..start + quote.len())
}

/// How many times `quote` occurs in `content`, counting non-overlapping
/// matches left to right — exactly the population [`head_range`] indexes
/// into and [`derive_file_quote`] disambiguates with `--occurrence`.
fn occurrence_count(content: &str, quote: &str) -> usize {
    content.match_indices(quote).count()
}

/// The line-extended neighborhood a byte range occupies in `content`:
/// `(the whole line before it, the line-extended span itself, the whole
/// line after it)`.
///
/// The middle element runs from the start of the line holding
/// `range.start` through the end of the line holding the span's last byte,
/// newline excluded. The neighbors are `None` at the start or end of the
/// file, which is what lets [`occurrence_survives`] recognize a window it
/// cannot discriminate on.
///
/// Every offset is floored to a character boundary before slicing, exactly
/// like [`line_range_of`], so a quote ending mid-multi-byte-character
/// cannot panic here.
fn line_window<'a>(
    content: &'a str,
    range: &Range<usize>,
) -> (Option<&'a str>, &'a str, Option<&'a str>) {
    let floor = |mut at: usize| {
        at = at.min(content.len());
        while at > 0 && !content.is_char_boundary(at) {
            at -= 1;
        }
        at
    };
    let start = floor(range.start);
    // The span's last byte, floored; an empty range reports the line it
    // sits on, matching `line_range_of`.
    let last = floor(range.end.saturating_sub(1).max(start));

    // Start of the line holding `start`.
    let line_start = content[..start].rfind('\n').map_or(0, |i| i + 1);
    // End of the line holding `last`, newline excluded.
    let line_end = content[last..]
        .find('\n')
        .map_or(content.len(), |i| last + i);

    let before = if line_start == 0 {
        None
    } else {
        let prev_end = line_start - 1; // the '\n' that ends the previous line
        let prev_start = content[..prev_end].rfind('\n').map_or(0, |i| i + 1);
        Some(&content[prev_start..prev_end])
    };
    let after = if line_end >= content.len() {
        None
    } else {
        let next_start = line_end + 1; // skip the '\n'
        if next_start >= content.len() {
            // The file ends with a newline: there is no following line.
            None
        } else {
            let next_end = content[next_start..]
                .find('\n')
                .map_or(content.len(), |i| next_start + i);
            Some(&content[next_start..next_end])
        }
    };
    (before, &content[line_start..line_end], after)
}

/// Whether the *specific* occurrence anchored at `head_range` still exists
/// at the tip, judged by the line window it sat in rather than by how many
/// copies of `quote` the file holds.
///
/// Walks every non-overlapping occurrence of `quote` in `tip_content` —
/// the same population [`occurrence_count`] counts, so the two halves of
/// the drift disjunction can never disagree about what an occurrence is —
/// and returns `true` as soon as one of them sits between the same
/// neighboring lines, with the same line-extended span, the anchored
/// occurrence occupied at `head`.
///
/// Neighbor lines that do not exist head-side (the quote is at the start or
/// end of the file) are not compared, so a block that moves *with* its
/// neighbors still matches. When the head window has no neighbor on either
/// side — the quote's line-extended span is the whole file — the window
/// rule cannot discriminate at all, so this falls back to the count
/// comparison rather than reporting permanent drift.
///
/// What is deliberately **not** compared is the anchored occurrence's own
/// line, byte for byte. The occurrence already carries the quote by
/// construction; requiring the rest of its line to be untouched would call
/// a trailing comment appended beside the quote "drift", which contradicts
/// the contract that only losing the anchored occurrence is drift
/// (`a_derived_anchor_round_trips_through_resolution` pins it). The
/// neighbors are the identity signal; the line itself is the subject.
fn occurrence_survives(
    head_content: &str,
    tip_content: &str,
    quote: &str,
    head_range: &Range<usize>,
) -> bool {
    let (before, _, after) = line_window(head_content, head_range);
    if before.is_none() && after.is_none() {
        // Degenerate window: nothing to compare against, so defer to the
        // count clause the caller already evaluated.
        return occurrence_count(tip_content, quote) >= occurrence_count(head_content, quote);
    }
    tip_content.match_indices(quote).any(|(start, _)| {
        let (tb, _, ta) = line_window(tip_content, &(start..start + quote.len()));
        before.is_none_or(|b| tb == Some(b)) && after.is_none_or(|a| ta == Some(a))
    })
}

/// Where a change review's drift tip came from.
///
/// Recorded alongside the revision by [`resolve_drift_tip`] so a caller can
/// explain *why* drift is being measured against what it is, rather than
/// having to re-derive the ladder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TipOrigin {
    /// The review's stamped `change_branch`, which still resolves.
    StampedBranch,
    /// The review stamped a branch, but it no longer resolves (renamed,
    /// deleted, or never pushed) — the repository's HEAD was used instead.
    BranchGone,
    /// The review stamped no branch at all (it was started on a detached
    /// HEAD) — the repository's HEAD was used.
    NoBranchStamp,
}

/// The revision a change review's drift is measured against, plus the
/// provenance of that choice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriftTip {
    /// The full SHA drift is measured against.
    pub rev: String,
    /// The review's stamped branch, when it stamped one — carried whether
    /// or not it still resolves, so a caller can name it in a message.
    pub branch: Option<String>,
    /// Which rung of the ladder produced [`Self::rev`].
    pub origin: TipOrigin,
}

/// Chooses the revision a change review's drift is measured against.
///
/// This is the single implementation of that policy. It used to live as an
/// `.ok().flatten()` chain in `rdm-cli`; consumers (the CLI today, a server
/// route later) must delegate here rather than re-deriving it, so every
/// surface measures drift against the same revision.
///
/// The ladder, in order:
///
/// 1. `change_branch` is stamped and still resolves → that branch's
///    **current** SHA, [`TipOrigin::StampedBranch`].
/// 2. `change_branch` is stamped but no longer resolves (renamed or
///    deleted) → the repository's `HEAD`, [`TipOrigin::BranchGone`].
/// 3. No branch was stamped — the review was started on a detached HEAD →
///    the repository's `HEAD`, [`TipOrigin::NoBranchStamp`].
///
/// Rung 1 deliberately returns the branch's *current* SHA, not the review's
/// recorded `head`: drift is measured against where the branch is **now**,
/// so a force-moved branch re-points the tip. That is the intent, not a
/// bug — a later reader should not "fix" it.
///
/// A [`SourceRepo`] failure is **propagated, never swallowed**. This is one
/// intentional behavior change from the CLI chain it replaces, which read
/// `.and_then(|b| source.rev_parse(b).ok().flatten()).or_else(|| source.head().ok().flatten())`
/// and so laundered a genuine source failure — git missing, or the
/// adapter's option-shaped-operand guard
/// ([`Error::InvalidChangeRevisionInput`]) firing before any subprocess —
/// into a silent re-point of drift at a *different* revision. A source that
/// cannot answer must not silently change what drift is measured against;
/// the caller degrades explicitly instead.
///
/// # Errors
///
/// - [`Error::ChangeTipUnresolvable`] when no revision can be chosen at
///   all: no branch resolves *and* `HEAD` is unborn (`Ok(None)`), or the
///   path is not a usable repository.
/// - Any error [`SourceRepo::rev_parse`] or [`SourceRepo::head`] returns is
///   propagated unchanged — notably [`Error::Git`] and
///   [`Error::InvalidChangeRevisionInput`].
///
/// # Examples
///
/// A stamped branch that still resolves wins over `HEAD`:
///
/// ```
/// use rdm_core::change::{TipOrigin, resolve_drift_tip};
/// use rdm_core::source::MemorySourceRepo;
///
/// let source = MemorySourceRepo::new()
///     .with_rev("feature/x", "a".repeat(40).as_str())
///     .with_head(&"b".repeat(40));
/// let tip = resolve_drift_tip(&source, Some("feature/x")).unwrap();
/// assert_eq!(tip.rev, "a".repeat(40));
/// assert_eq!(tip.origin, TipOrigin::StampedBranch);
/// ```
///
/// A branch that has since been deleted degrades to `HEAD`:
///
/// ```
/// use rdm_core::change::{TipOrigin, resolve_drift_tip};
/// use rdm_core::source::MemorySourceRepo;
///
/// let source = MemorySourceRepo::new().with_head(&"b".repeat(40));
/// let tip = resolve_drift_tip(&source, Some("gone")).unwrap();
/// assert_eq!(tip.rev, "b".repeat(40));
/// assert_eq!(tip.origin, TipOrigin::BranchGone);
/// assert_eq!(tip.branch.as_deref(), Some("gone"));
/// ```
pub fn resolve_drift_tip(
    source: &impl SourceRepo,
    change_branch: Option<&str>,
) -> Result<DriftTip> {
    let branch = change_branch.map(str::to_string);
    if let Some(name) = change_branch {
        // Propagate, do NOT fall back: a source that cannot answer must not
        // silently re-point drift at a different revision.
        if let Some(rev) = source.rev_parse(name)? {
            return Ok(DriftTip {
                rev,
                branch,
                origin: TipOrigin::StampedBranch,
            });
        }
    }
    let origin = if change_branch.is_some() {
        TipOrigin::BranchGone
    } else {
        TipOrigin::NoBranchStamp
    };
    match source.head()? {
        Some(rev) => Ok(DriftTip {
            rev,
            branch,
            origin,
        }),
        None => Err(Error::ChangeTipUnresolvable { branch }),
    }
}

/// Resolves one change-review comment against the source repository.
///
/// `tip` is the revision drift is measured against; [`resolve_drift_tip`]
/// owns the policy that chooses it. This function is the pure resolution
/// half and takes whatever revision it is handed.
///
/// The ladder, per the [module documentation](self):
///
/// 1. No anchor, a non-[`Anchor::FileQuote`] anchor, a non-change review, or
///    the file missing at `head` → [`Resolution::Unresolved`].
/// 2. The quote located at `head` but the path gone at `tip` →
///    [`Resolution::Unresolved`].
/// 3. `tip` holds at least as many byte-for-byte occurrences of the quote
///    as `head` did **and** some tip occurrence sits between the same
///    neighboring lines the anchored one sat between at `head` →
///    `Original { drifted: false }`.
/// 4. Otherwise → `Original { drifted: true }`. That covers both the case
///    where an unrelated duplicate of the quote survives but the anchored
///    one was edited away (the count shrank), and the case where the
///    anchored occurrence was edited away while a fresh copy of the same
///    text appeared elsewhere (the count is unchanged, and only the window
///    sees it).
///
/// Malformed head or base returns unresolved without querying the source.
/// `tip` remains unresolved caller input, guarded by the source adapter.
///
/// Infallible, exactly like
/// [`crate::anchor::resolve_against_history`]: any source-repository error
/// degrades to [`Resolution::Unresolved`] rather than failing a
/// `rdm review show`.
///
/// # Panics
///
/// Never panics.
#[must_use]
pub fn resolve_change_comment(
    source: &impl SourceRepo,
    review: &Review,
    comment: &ReviewComment,
    tip: &str,
) -> ResolvedComment {
    let unresolved = ResolvedComment {
        resolution: Resolution::Unresolved,
        quote: None,
    };
    if review.target.validate_stored_identity().is_err() {
        return unresolved;
    }
    let ReviewTarget::Change { head, .. } = &review.target else {
        return unresolved;
    };
    let Some(Anchor::FileQuote {
        path,
        quote,
        occurrence,
        ..
    }) = &comment.anchor
    else {
        return unresolved;
    };
    // A path that is not (or no longer) a plain file at head can never
    // resolve — treated exactly like a path missing at head, so a
    // pre-existing bad anchor (from before this check existed, or a
    // hand-edited review file) degrades to unresolved rather than being
    // read as though its content were quotable.
    if !matches!(
        source.object_kind_at(head, path),
        Ok(Some(crate::source::SourceObjectKind::Blob))
    ) {
        return unresolved;
    }
    let Ok(Some(head_content)) = source.file_at(head, path) else {
        return unresolved;
    };
    let Some(range) = head_range(&head_content, quote, *occurrence) else {
        return unresolved;
    };
    // Path gone, or turned into a directory/submodule, at the tip: the
    // anchor has nowhere to live any more.
    if !matches!(
        source.object_kind_at(tip, path),
        Ok(Some(crate::source::SourceObjectKind::Blob))
    ) {
        return unresolved;
    }
    let tip_content = match source.file_at(tip, path) {
        Ok(Some(c)) => c,
        // An unreadable tip is not evidence the anchor survived, but it is
        // also not evidence it was deleted — degrade, never guess.
        Ok(None) | Err(_) => return unresolved,
    };
    // Occurrence *identity*, not a bare count. The count clause comes first
    // and is retained verbatim, so nothing that reports drift today can
    // start reporting resolved — the window clause can only ADD drift
    // detections. It catches the case a count cannot see: one commit that
    // edits the anchored occurrence away and introduces a fresh copy of the
    // same text elsewhere leaves the count unchanged, yet the line the
    // reviewer named is gone.
    let drifted = occurrence_count(&tip_content, quote) < occurrence_count(&head_content, quote)
        || !occurrence_survives(&head_content, &tip_content, quote, &range);
    ResolvedComment {
        resolution: Resolution::Original { range, drifted },
        quote: Some(quote.clone()),
    }
}

/// Resolves every comment in `review` against the source repository, in
/// comment order.
///
/// Malformed stored identities yield unresolved comments without source access.
///
/// The returned slice is parallel to `review.comments`, exactly like
/// [`crate::anchor::resolve_comments`], so the JSON, human and markdown
/// renderers all consume one resolution pass.
///
/// # Panics
///
/// Never panics.
#[must_use]
pub fn resolve_change_comments(
    source: &impl SourceRepo,
    review: &Review,
    tip: &str,
) -> Vec<ResolvedComment> {
    review
        .comments
        .iter()
        .map(|c| resolve_change_comment(source, review, c, tip))
        .collect()
}

/// A human-readable reason an [`Anchor::FileQuote`] cannot resolve because
/// the path it names is not (or no longer) a plain file at `head`.
///
/// Re-derives the object kind fresh via [`SourceRepo::object_kind_at`] rather
/// than trusting anything cached in `anchor`, so it reflects the real reason
/// even for an anchor written before this check existed (a hand-edited
/// review file, or one written by a pre-fix build).
///
/// Returns `None` when the anchor is a [`Anchor::FileQuote`] that IS
/// currently eligible (a blob at `head`), when it is any other anchor kind,
/// or when the source repository cannot answer at all — an environmental
/// failure is not evidence the anchor itself is invalid, so it gets no note
/// here (the caller's own "source verification skipped" note already covers
/// that case).
#[must_use]
pub fn change_anchor_ineligibility(
    source: &impl SourceRepo,
    head: &str,
    anchor: &Anchor,
) -> Option<String> {
    let Anchor::FileQuote { path, .. } = anchor else {
        return None;
    };
    match source.object_kind_at(head, path) {
        Ok(Some(crate::source::SourceObjectKind::Blob)) | Err(_) => None,
        Ok(Some(crate::source::SourceObjectKind::Tree)) => {
            Some(format!("'{path}' is a directory, not a file"))
        }
        Ok(Some(crate::source::SourceObjectKind::Gitlink)) => {
            Some(format!("'{path}' is a submodule, not a file"))
        }
        Ok(None) => Some(format!("'{path}' no longer exists at {}", abbreviate(head))),
    }
}

/// The head-pinned `rdm:src/<path>@<head>#L<start>[-L<end>]` permalink for
/// an [`Anchor::FileQuote`].
///
/// Derived from the anchor alone — the line range was recorded at
/// derivation time — so `rdm review show` emits permalinks with no source
/// checkout present. A single-line span emits `#L7`, not `#L7-L7`.
///
/// Returns `None` for any other anchor kind.
///
/// # Examples
///
/// ```
/// use rdm_core::change::permalink_for;
/// use rdm_core::model::Anchor;
///
/// let anchor = Anchor::FileQuote {
///     path: "src/lib.rs".to_string(),
///     quote: "fn main".to_string(),
///     occurrence: 1,
///     start_line: 7,
///     end_line: 7,
/// };
/// assert_eq!(
///     permalink_for("abc123", &anchor).unwrap().to_string(),
///     "rdm:src/src/lib.rs@abc123#L7"
/// );
/// ```
#[must_use]
pub fn permalink_for(head: &str, anchor: &Anchor) -> Option<Link> {
    let Anchor::FileQuote {
        path,
        start_line,
        end_line,
        ..
    } = anchor
    else {
        return None;
    };
    Some(Link::Code {
        path: path.clone(),
        rev: Some(head.to_string()),
        lines: Some((
            *start_line,
            if end_line > start_line {
                Some(*end_line)
            } else {
                None
            },
        )),
    })
}

/// Normalizes a user-supplied `--path` into a repo-relative, *linkable*
/// path.
///
/// Strips a leading `./`, and rejects an absolute path, a `..` segment, or
/// a backslash separator — the stored path has to be repo-relative for the
/// emitted `rdm:src/` permalink to parse.
///
/// It additionally rejects the two characters the
/// `rdm:src/<path>@<rev>#L<n>` grammar reserves for itself, `@` and `#`.
/// [`crate::link::parse`] ends the
/// path at the *first* of either character, while [`permalink_for`]
/// interpolates the stored path verbatim, so a path carrying one of them
/// renders a permalink that parses back as a shorter path plus a nonsense
/// revision. Refusal at intake is the only defensible fix: nothing in the
/// grammar escapes them. Other punctuation the grammar does **not** reserve
/// (`?`, `%`, spaces, multi-byte characters) is deliberately still
/// accepted, so the rule stays minimal and explicable.
///
/// # Round-trip invariant
///
/// Every path this function returns re-parses identically through
/// [`Link::Code`] → `to_string()` → [`crate::link::parse`]. The guarantee is
/// forward-looking: an anchor *already stored* with a reserved character
/// (written before this check existed, or hand-edited into a review file)
/// is not retracted — [`permalink_for`] still renders it rather than
/// panicking or silently truncating at render time.
///
/// # Errors
///
/// Returns [`Error::ChangePathNotInRevision`] with `rev` set to the literal
/// `"the source repository root"` when the path is not repo-relative; the
/// caller has no better error to map it onto and the message names exactly
/// what to fix.
///
/// Returns [`Error::ChangePathNotLinkable`], naming the offending
/// character, when the normalized path contains `@` or `#`.
pub fn normalize_source_path(path: &str) -> Result<String> {
    let trimmed = path.trim();
    let stripped = trimmed.strip_prefix("./").unwrap_or(trimmed);
    let bad = stripped.is_empty()
        || stripped.starts_with('/')
        || stripped.contains('\\')
        || stripped.split('/').any(|seg| seg == "..");
    if bad {
        return Err(Error::ChangePathNotInRevision {
            path: path.to_string(),
            rev: "the source repository root".to_string(),
        });
    }
    // Checked against the NORMALIZED value, so `./a@b` is reported with the
    // same offending character as `a@b`.
    if let Some(delimiter) = stripped.chars().find(|c| matches!(c, '@' | '#')) {
        return Err(Error::ChangePathNotLinkable {
            path: stripped.to_string(),
            delimiter,
        });
    }
    Ok(stripped.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ReviewCommentStatus, ReviewState};
    use crate::source::{MemorySourceRepo, SourceObjectKind};
    use chrono::Utc;

    /// AC1: the drift-tip ladder lives in core and covers every branch
    /// state — present, gone, never stamped, unborn HEAD, and a genuine
    /// lookup failure that must NOT be laundered into a HEAD fallback.
    #[test]
    fn resolve_drift_tip_covers_every_branch_state() {
        let branch_sha = "1".repeat(40);
        let head_sha = "2".repeat(40);

        // (a) A stamped branch that still resolves wins, at the branch's
        //     current sha — not the review's recorded head.
        let source = MemorySourceRepo::new()
            .with_rev("feature/x", &branch_sha)
            .with_head(&head_sha);
        let tip = resolve_drift_tip(&source, Some("feature/x")).unwrap();
        assert_eq!(tip.rev, branch_sha);
        assert_eq!(tip.branch.as_deref(), Some("feature/x"));
        assert_eq!(tip.origin, TipOrigin::StampedBranch);

        // (b) A stamped branch that was renamed or deleted degrades to HEAD,
        //     and still names the branch so a caller can explain itself.
        let source = MemorySourceRepo::new().with_head(&head_sha);
        let tip = resolve_drift_tip(&source, Some("feature/gone")).unwrap();
        assert_eq!(tip.rev, head_sha);
        assert_eq!(tip.branch.as_deref(), Some("feature/gone"));
        assert_eq!(tip.origin, TipOrigin::BranchGone);

        // (c) No branch was ever stamped (a detached-HEAD review): HEAD,
        //     distinguishable from (b) by origin.
        let tip = resolve_drift_tip(&source, None).unwrap();
        assert_eq!(tip.rev, head_sha);
        assert_eq!(tip.branch, None);
        assert_eq!(tip.origin, TipOrigin::NoBranchStamp);

        // (d) A genuine rev-parse failure propagates. HEAD is deliberately
        //     seeded and reachable here, so this test fails if the error is
        //     laundered into the HEAD fallback the old CLI chain took.
        let source = MemorySourceRepo::new()
            .with_head(&head_sha)
            .with_failing_rev_parse("feature/x");
        let err = resolve_drift_tip(&source, Some("feature/x")).unwrap_err();
        assert!(
            matches!(err, Error::Git(_)),
            "a source failure must propagate, not fall back to HEAD: {err:?}"
        );

        // (e) Nothing resolvable at all — a gone branch over an unborn HEAD.
        let source = MemorySourceRepo::new();
        let err = resolve_drift_tip(&source, Some("feature/gone")).unwrap_err();
        assert!(matches!(
            err,
            Error::ChangeTipUnresolvable { ref branch } if branch.as_deref() == Some("feature/gone")
        ));
        let err = resolve_drift_tip(&source, None).unwrap_err();
        assert!(matches!(err, Error::ChangeTipUnresolvable { branch: None }));

        // A failing HEAD lookup propagates rather than becoming "unresolvable".
        let source = MemorySourceRepo::new().with_failing_head();
        let err = resolve_drift_tip(&source, None).unwrap_err();
        assert!(matches!(err, Error::Git(_)));
    }

    fn review_with(anchor: Option<Anchor>) -> Review {
        Review {
            id: "r1".to_string(),
            author: "tester".to_string(),
            target: ReviewTarget::Change {
                head: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
                base: Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string()),
            },
            state: ReviewState::Draft,
            verdict: None,
            created: Utc::now(),
            submitted: None,
            created_commit: None,
            implements: None,
            change_branch: None,
            comments: vec![ReviewComment {
                id: 1,
                doc: None,
                status: ReviewCommentStatus::Open,
                applied_commit: None,
                anchor,
                body: "a comment".to_string(),
                reply: None,
            }],
        }
    }

    // --- parse_hunks ---

    #[test]
    fn parse_hunks_reads_explicit_counts() {
        let hunks = parse_hunks("@@ -1,3 +1,4 @@\n@@ -20,0 +30,2 @@\n");
        assert_eq!(
            hunks,
            vec![
                HunkRange { start: 1, end: 4 },
                HunkRange { start: 30, end: 31 }
            ]
        );
    }

    #[test]
    fn parse_hunks_handles_omitted_count() {
        // `+7` with no comma means exactly one line.
        assert_eq!(
            parse_hunks("@@ -7 +7 @@\n"),
            vec![HunkRange { start: 7, end: 7 }]
        );
    }

    #[test]
    fn parse_hunks_handles_new_file() {
        assert_eq!(
            parse_hunks("@@ -0,0 +1,5 @@\n"),
            vec![HunkRange { start: 1, end: 5 }]
        );
    }

    #[test]
    fn parse_hunks_normalizes_pure_deletion_to_the_abutting_line() {
        assert_eq!(
            parse_hunks("@@ -3,2 +2,0 @@\n"),
            vec![HunkRange { start: 2, end: 2 }]
        );
        // Deleting the whole file clamps to line 1 rather than line 0.
        assert_eq!(
            parse_hunks("@@ -1,4 +0,0 @@\n"),
            vec![HunkRange { start: 1, end: 1 }]
        );
    }

    #[test]
    fn parse_hunks_ignores_content_lines_and_headers() {
        let diff = "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n+new\n";
        assert_eq!(parse_hunks(diff), vec![HunkRange { start: 1, end: 1 }]);
    }

    #[test]
    fn parse_hunks_on_an_empty_diff_is_empty() {
        assert!(parse_hunks("").is_empty());
    }

    // --- line_range_of ---

    #[test]
    fn line_range_of_counts_newlines() {
        let content = "one\ntwo\nthree\n";
        assert_eq!(line_range_of(content, 0..3), (1, 1));
        assert_eq!(line_range_of(content, 4..7), (2, 2));
        assert_eq!(line_range_of(content, 4..13), (2, 3));
    }

    #[test]
    fn line_range_of_is_char_safe_on_multibyte_content() {
        let content = "héllo\nwörld\n";
        let start = content.find("wörld").unwrap();
        assert_eq!(line_range_of(content, start..start + "wörld".len()), (2, 2));
    }

    #[test]
    fn line_range_of_is_char_safe_when_the_span_ends_mid_character() {
        // `range.end - 1` lands INSIDE the final character here, which is the
        // shape a quote ending in an accented letter, an em-dash or a curly
        // quote produces. Flooring to the boundary keeps it a line number
        // instead of a panic.
        for quote in ["café", "a—", "say ”"] {
            let content = format!("first\nx {quote}\nlast\n");
            let start = content.find(quote).unwrap();
            assert_eq!(
                line_range_of(&content, start..start + quote.len()),
                (2, 2),
                "quote {quote:?} must report line 2"
            );
        }
    }

    #[test]
    fn derive_file_quote_anchors_a_quote_ending_in_a_multibyte_character() {
        // The end-to-end shape of the bug: deriving an anchor for a quote
        // whose last byte is mid-character must yield an anchor, not a panic.
        let content = "fn a() {}\nfn b() {} // café\n";
        let hunks = parse_hunks("@@ -1,1 +1,2 @@\n");
        let anchor = derive_file_quote(content, "src/lib.rs", "// café", None, &hunks, "b..h")
            .expect("a multi-byte quote inside a touched hunk must anchor");
        let Anchor::FileQuote {
            start_line,
            end_line,
            quote,
            ..
        } = anchor
        else {
            panic!("expected a file-quote anchor");
        };
        assert_eq!((start_line, end_line), (2, 2));
        assert_eq!(quote, "// café");
    }

    // --- nearest_hunk ---

    #[test]
    fn nearest_hunk_picks_the_closest_and_breaks_ties_earlier() {
        let hunks = vec![
            HunkRange { start: 1, end: 2 },
            HunkRange { start: 8, end: 9 },
        ];
        assert_eq!(nearest_hunk(&hunks, 5, 5), Some(hunks[0]));
        assert_eq!(nearest_hunk(&hunks, 7, 7), Some(hunks[1]));
        assert_eq!(nearest_hunk(&[], 1, 1), None);
    }

    // --- derive_file_quote ---

    const CONTENT: &str = "alpha\nbeta\ngamma\nbeta\n";

    #[test]
    fn derive_file_quote_anchors_inside_a_hunk() {
        let hunks = vec![HunkRange { start: 3, end: 3 }];
        let anchor =
            derive_file_quote(CONTENT, "src/lib.rs", "gamma", None, &hunks, "base..head").unwrap();
        assert_eq!(
            anchor,
            Anchor::FileQuote {
                path: "src/lib.rs".to_string(),
                quote: "gamma".to_string(),
                occurrence: 1,
                start_line: 3,
                end_line: 3,
            }
        );
    }

    #[test]
    fn derive_file_quote_accepts_a_span_overlapping_a_hunk_boundary() {
        // The quote covers lines 2-3; only line 3 is touched.
        let hunks = vec![HunkRange { start: 3, end: 3 }];
        let anchor =
            derive_file_quote(CONTENT, "src/lib.rs", "beta\ngamma", None, &hunks, "b..h").unwrap();
        let Anchor::FileQuote {
            start_line,
            end_line,
            ..
        } = anchor
        else {
            panic!("expected a file-quote anchor");
        };
        assert_eq!((start_line, end_line), (2, 3));
    }

    #[test]
    fn derive_file_quote_rejects_a_quote_outside_every_hunk() {
        let hunks = vec![HunkRange { start: 3, end: 3 }];
        let err =
            derive_file_quote(CONTENT, "src/lib.rs", "alpha", None, &hunks, "b..h").unwrap_err();
        let Error::QuoteOutsideChangedHunks {
            path,
            start_line,
            end_line,
            nearest,
            ..
        } = err
        else {
            panic!("expected QuoteOutsideChangedHunks, got {err:?}");
        };
        assert_eq!(path, "src/lib.rs");
        assert_eq!((start_line, end_line), (1, 1));
        assert_eq!(nearest, Some((3, 3)));
    }

    #[test]
    fn derive_file_quote_on_an_untouched_path_reports_no_nearest_hunk() {
        let err = derive_file_quote(CONTENT, "src/lib.rs", "alpha", None, &[], "b..h").unwrap_err();
        let Error::QuoteOutsideChangedHunks { nearest, .. } = err else {
            panic!("expected QuoteOutsideChangedHunks, got {err:?}");
        };
        assert_eq!(nearest, None);
        // The two messages are genuinely different, not one template.
        assert!(
            Error::QuoteOutsideChangedHunks {
                path: "p".to_string(),
                start_line: 1,
                end_line: 1,
                nearest: None,
                range: "b..h".to_string(),
            }
            .to_string()
            .contains("is not touched by")
        );
    }

    #[test]
    fn derive_file_quote_reports_ambiguity_with_occurrences() {
        let hunks = vec![HunkRange { start: 1, end: 4 }];
        let err = derive_file_quote(CONTENT, "f", "beta", None, &hunks, "b..h").unwrap_err();
        let Error::QuoteAmbiguous { occurrences, .. } = err else {
            panic!("expected QuoteAmbiguous, got {err:?}");
        };
        assert_eq!(occurrences.len(), 2);
    }

    #[test]
    fn derive_file_quote_selects_an_occurrence() {
        let hunks = vec![HunkRange { start: 1, end: 4 }];
        let anchor = derive_file_quote(CONTENT, "f", "beta", Some(2), &hunks, "b..h").unwrap();
        let Anchor::FileQuote {
            occurrence,
            start_line,
            ..
        } = anchor
        else {
            panic!("expected a file-quote anchor");
        };
        assert_eq!((occurrence, start_line), (2, 4));
    }

    #[test]
    fn derive_file_quote_rejects_an_out_of_range_occurrence() {
        let hunks = vec![HunkRange { start: 1, end: 4 }];
        let err = derive_file_quote(CONTENT, "f", "beta", Some(9), &hunks, "b..h").unwrap_err();
        assert!(matches!(err, Error::QuoteOccurrenceOutOfRange { .. }));
    }

    #[test]
    fn derive_file_quote_rejects_a_missing_or_empty_quote() {
        let hunks = vec![HunkRange { start: 1, end: 4 }];
        assert!(matches!(
            derive_file_quote(CONTENT, "f", "nope", None, &hunks, "b..h").unwrap_err(),
            Error::QuoteNotFound { .. }
        ));
        assert!(matches!(
            derive_file_quote(CONTENT, "f", "", None, &hunks, "b..h").unwrap_err(),
            Error::QuoteNotFound { .. }
        ));
    }

    // --- resolution ---

    fn anchor() -> Anchor {
        Anchor::FileQuote {
            path: "src/lib.rs".to_string(),
            quote: "gamma".to_string(),
            occurrence: 1,
            start_line: 3,
            end_line: 3,
        }
    }

    #[test]
    fn resolve_reports_resolved_when_the_tip_still_has_the_quote() {
        let source = MemorySourceRepo::new()
            .with_file(
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "src/lib.rs",
                CONTENT,
            )
            .with_file("tip", "src/lib.rs", CONTENT);
        let review = review_with(Some(anchor()));
        let resolved = resolve_change_comments(&source, &review, "tip");
        assert_eq!(
            resolved[0].resolution,
            Resolution::Original {
                range: 11..16,
                drifted: false
            }
        );
        assert_eq!(resolved[0].quote.as_deref(), Some("gamma"));
    }

    #[test]
    fn resolve_reports_drifted_when_the_tip_edited_the_quoted_text() {
        let source = MemorySourceRepo::new()
            .with_file(
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "src/lib.rs",
                CONTENT,
            )
            .with_file("tip", "src/lib.rs", "alpha\nbeta\nGAMMA\nbeta\n");
        let review = review_with(Some(anchor()));
        let resolved = resolve_change_comments(&source, &review, "tip");
        let Resolution::Original { drifted, .. } = resolved[0].resolution else {
            panic!("expected an Original resolution");
        };
        assert!(drifted);
    }

    /// The duplicate-quote regression: `CONTENT` holds `beta` twice, the
    /// comment anchors the **second** one, and the tip edits exactly that
    /// occurrence while the unrelated first copy survives untouched. A bare
    /// `tip_content.contains(quote)` reads that as "still true"; counting
    /// occurrences reports the drift it is.
    #[test]
    fn resolve_reports_drifted_when_only_the_anchored_duplicate_was_edited() {
        let source = MemorySourceRepo::new()
            .with_file(
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "src/lib.rs",
                CONTENT,
            )
            .with_file("tip", "src/lib.rs", "alpha\nbeta\ngamma\nBETA\n");
        let mut review = review_with(Some(Anchor::FileQuote {
            path: "src/lib.rs".to_string(),
            quote: "beta".to_string(),
            occurrence: 2,
            start_line: 4,
            end_line: 4,
        }));
        review.comments[0].body = "the second beta".to_string();
        let resolved = resolve_change_comments(&source, &review, "tip");
        let Resolution::Original { drifted, range } = resolved[0].resolution.clone() else {
            panic!("expected an Original resolution");
        };
        assert!(
            drifted,
            "editing the anchored occurrence is drift even when a duplicate survives"
        );
        // The reported range still indexes the head-side content.
        assert_eq!(&CONTENT[range], "beta");
    }

    /// The other half of the same rule: text that merely *moved* keeps its
    /// occurrence count, so it must still resolve.
    #[test]
    fn resolve_reports_resolved_when_a_duplicated_quote_only_moved() {
        let source = MemorySourceRepo::new()
            .with_file(
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "src/lib.rs",
                CONTENT,
            )
            .with_file("tip", "src/lib.rs", "beta\nalpha\ngamma\nbeta\n");
        let review = review_with(Some(Anchor::FileQuote {
            path: "src/lib.rs".to_string(),
            quote: "beta".to_string(),
            occurrence: 2,
            start_line: 4,
            end_line: 4,
        }));
        let Resolution::Original { drifted, .. } =
            resolve_change_comments(&source, &review, "tip")[0].resolution
        else {
            panic!("expected an Original resolution");
        };
        assert!(!drifted, "a pure relocation preserves the count");
    }

    /// AC2: the case a bare occurrence count cannot see. One commit edits
    /// the anchored occurrence away AND introduces a fresh copy of the same
    /// text elsewhere, so the count is unchanged (1 → 1) while the line the
    /// reviewer named is gone. The count-only implementation reports
    /// `drifted: false`; the window rule reports drift.
    #[test]
    fn resolve_reports_drifted_when_the_anchored_occurrence_moved_and_a_fresh_copy_appeared() {
        const HEAD: &str = "alpha\nbeta\nTARGET\ngamma\n";
        let source = MemorySourceRepo::new()
            .with_file(
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "src/lib.rs",
                HEAD,
            )
            .with_file(
                "tip",
                "src/lib.rs",
                "alpha\nbeta\nEDITED\ngamma\ndelta\nTARGET\nepsilon\n",
            );
        let review = review_with(Some(Anchor::FileQuote {
            path: "src/lib.rs".to_string(),
            quote: "TARGET".to_string(),
            occurrence: 1,
            start_line: 3,
            end_line: 3,
        }));
        let Resolution::Original { drifted, range } =
            resolve_change_comments(&source, &review, "tip")[0]
                .resolution
                .clone()
        else {
            panic!("expected an Original resolution");
        };
        assert_eq!(
            occurrence_count(
                "alpha\nbeta\nEDITED\ngamma\ndelta\nTARGET\nepsilon\n",
                "TARGET"
            ),
            occurrence_count(HEAD, "TARGET"),
            "the fixture is only meaningful while the count is unchanged"
        );
        assert!(
            drifted,
            "an anchored occurrence edited away is drift even when a fresh copy appears elsewhere"
        );
        // The reported range still indexes the head-side content.
        assert_eq!(&HEAD[range], "TARGET");
    }

    /// AC2 edge case: a quote whose line-extended span is the whole file
    /// has no neighbor to compare against, so the window rule degenerates
    /// and the count clause decides — never permanent drift.
    #[test]
    fn a_single_line_file_falls_back_to_the_count_comparison() {
        let source = MemorySourceRepo::new()
            .with_file(
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "src/lib.rs",
                "only",
            )
            .with_file("tip", "src/lib.rs", "only");
        let review = review_with(Some(Anchor::FileQuote {
            path: "src/lib.rs".to_string(),
            quote: "only".to_string(),
            occurrence: 1,
            start_line: 1,
            end_line: 1,
        }));
        let Resolution::Original { drifted, .. } =
            resolve_change_comments(&source, &review, "tip")[0].resolution
        else {
            panic!("expected an Original resolution");
        };
        assert!(
            !drifted,
            "an unchanged single-line file must not read as drift"
        );
    }

    /// AC2 edge case: overlapping self-similar quotes. The window walker
    /// must enumerate the same non-overlapping population
    /// `occurrence_count` counts, or the two halves of the disjunction
    /// disagree.
    #[test]
    fn the_window_walker_uses_the_same_non_overlapping_population_as_the_count() {
        assert_eq!(occurrence_count("aaaa", "aa"), 2);
        let head = "x\naaaa\ny\n";
        let range = head.find("aa").unwrap()..head.find("aa").unwrap() + 2;
        assert!(occurrence_survives(head, head, "aa", &range));
        // The identical line moved between different neighbors is drift.
        assert!(!occurrence_survives(head, "p\naaaa\nq\n", "aa", &range));
    }

    /// AC2 edge case: content with no trailing newline, CRLF line endings,
    /// and a quote ending inside a multi-byte character must all slice on
    /// character boundaries rather than panicking.
    #[test]
    fn the_line_window_is_char_safe_and_line_ending_agnostic() {
        for content in ["a\nb\ncafé", "a\r\ncafé\r\nb\r\n", "café", "\n\n", ""] {
            for start in 0..=content.len() {
                for end in start..=content.len() {
                    let _ = line_window(content, &(start..end));
                }
            }
        }
        let content = "a\ncafé\nb\n";
        let start = content.find("café").unwrap();
        let (before, span, after) = line_window(content, &(start..start + "café".len()));
        assert_eq!(before, Some("a"));
        assert_eq!(span, "café");
        assert_eq!(after, Some("b"));
    }

    #[test]
    fn resolve_reports_unresolved_when_the_path_is_gone_at_the_tip() {
        let source = MemorySourceRepo::new().with_file(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "src/lib.rs",
            CONTENT,
        );
        let review = review_with(Some(anchor()));
        let resolved = resolve_change_comments(&source, &review, "tip");
        assert_eq!(resolved[0].resolution, Resolution::Unresolved);
    }

    #[test]
    fn resolve_reports_unresolved_for_a_whole_document_comment() {
        let source = MemorySourceRepo::new()
            .with_file(
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "src/lib.rs",
                CONTENT,
            )
            .with_file("tip", "src/lib.rs", CONTENT);
        let review = review_with(None);
        assert_eq!(
            resolve_change_comments(&source, &review, "tip")[0].resolution,
            Resolution::Unresolved
        );
    }

    #[test]
    fn resolve_reports_unresolved_when_the_file_is_gone_at_head() {
        let source = MemorySourceRepo::new().with_file("tip", "src/lib.rs", CONTENT);
        let review = review_with(Some(anchor()));
        assert_eq!(
            resolve_change_comments(&source, &review, "tip")[0].resolution,
            Resolution::Unresolved
        );
    }

    // --- permalink ---

    #[test]
    fn permalink_for_a_single_line_anchor_omits_the_end() {
        let link = permalink_for("abc123", &anchor()).unwrap();
        assert_eq!(link.to_string(), "rdm:src/src/lib.rs@abc123#L3");
    }

    #[test]
    fn permalink_for_a_multi_line_anchor_carries_both_ends() {
        let a = Anchor::FileQuote {
            path: "a/b.rs".to_string(),
            quote: "x".to_string(),
            occurrence: 1,
            start_line: 4,
            end_line: 9,
        };
        assert_eq!(
            permalink_for("deadbeef", &a).unwrap().to_string(),
            "rdm:src/a/b.rs@deadbeef#L4-L9"
        );
    }

    #[test]
    fn permalink_for_a_text_quote_anchor_is_none() {
        let a = Anchor::TextQuote {
            quote: "q".to_string(),
            prefix: String::new(),
            suffix: String::new(),
        };
        assert!(permalink_for("abc", &a).is_none());
    }

    #[test]
    fn permalinks_round_trip_through_link_parse() {
        let link = permalink_for("abc123", &anchor()).unwrap();
        assert_eq!(crate::link::parse(&link.to_string()).unwrap(), link);
    }

    // --- path normalization ---

    #[test]
    fn normalize_source_path_strips_a_dot_slash_prefix() {
        assert_eq!(normalize_source_path("./src/lib.rs").unwrap(), "src/lib.rs");
        assert_eq!(normalize_source_path(" src/lib.rs ").unwrap(), "src/lib.rs");
    }

    #[test]
    fn normalize_source_path_rejects_non_relative_paths() {
        for bad in [
            "/abs/path.rs",
            "../escape.rs",
            "a/../../b.rs",
            "a\\b.rs",
            "",
        ] {
            assert!(
                normalize_source_path(bad).is_err(),
                "expected {bad:?} to be rejected"
            );
        }
    }

    /// AC1: the `rdm:src/<path>@<rev>#L<n>` grammar reserves `@` and `#`,
    /// and nothing escapes them, so a path carrying either is refused at
    /// intake rather than being stored as an anchor whose permalink parses
    /// back to a different file.
    #[test]
    fn normalize_source_path_rejects_permalink_delimiters() {
        for (bad, want) in [
            ("src/a@b.rs", '@'),
            ("src/a#L3.rs", '#'),
            ("a@", '@'),
            ("#x", '#'),
            // Checked against the NORMALIZED value: the `./` prefix is
            // stripped first, so the reported path and character are the
            // ones a reader can act on.
            ("./web/app/@modal/page.tsx", '@'),
        ] {
            let err = normalize_source_path(bad).unwrap_err();
            let Error::ChangePathNotLinkable { delimiter, .. } = err else {
                panic!("expected ChangePathNotLinkable for {bad:?}, got {err:?}");
            };
            assert_eq!(delimiter, want, "wrong delimiter reported for {bad:?}");
            let msg = normalize_source_path(bad).unwrap_err().to_string();
            assert!(msg.contains(want), "message must name the character: {msg}");
            assert!(
                msg.contains("rdm:src/<path>@<rev>#L<n>"),
                "message must name the grammar: {msg}"
            );
        }
        let msg = normalize_source_path("./web/app/@modal/page.tsx")
            .unwrap_err()
            .to_string();
        assert!(
            msg.contains("'web/app/@modal/page.tsx'"),
            "the normalized path, not the raw input, is what the message names: {msg}"
        );
    }

    /// AC1: characters the grammar does NOT reserve stay accepted — the
    /// rule is minimal, not a general punctuation ban.
    #[test]
    fn normalize_source_path_accepts_unreserved_punctuation() {
        for good in [
            "src/a?b.rs",
            "src/a%20b.rs",
            "dir with space/x.rs",
            "src/café.rs",
        ] {
            assert!(
                normalize_source_path(good).is_ok(),
                "expected {good:?} to be accepted"
            );
        }
    }

    /// AC1: the invariant the refusal buys — every path
    /// `normalize_source_path` accepts survives `Link::Code` →
    /// `to_string()` → `link::parse` byte for byte.
    #[test]
    fn source_paths_round_trip_through_the_code_link_grammar() {
        let head = "abc123";
        for path in [
            "src/lib.rs",
            "a/b/c-d_e.rs",
            "dir with space/x.rs",
            "src/café.rs",
            "src/a?b.rs",
        ] {
            let normalized = normalize_source_path(path).unwrap();
            for (start_line, end_line) in [(7u32, 7u32), (4, 9)] {
                let anchor = Anchor::FileQuote {
                    path: normalized.clone(),
                    quote: "q".to_string(),
                    occurrence: 1,
                    start_line,
                    end_line,
                };
                let link = permalink_for(head, &anchor).unwrap();
                let parsed = crate::link::parse(&link.to_string()).unwrap();
                let Link::Code {
                    path: got_path,
                    rev: got_rev,
                    lines,
                } = parsed
                else {
                    panic!("expected a code link for {path:?}");
                };
                assert_eq!(got_path, normalized, "path did not round-trip: {path:?}");
                assert_eq!(got_rev.as_deref(), Some(head));
                assert_eq!(
                    lines,
                    Some((start_line, (end_line > start_line).then_some(end_line)))
                );
            }
        }
    }

    // ---- resolve_change_target -------------------------------------------

    /// Head sha, base sha, and a diff the two share, for target tests.
    fn target_repo() -> (String, String, MemorySourceRepo) {
        let head = "a".repeat(40);
        let base = "b".repeat(40);
        let source = MemorySourceRepo::new()
            .with_rev("HEAD", &head)
            .with_rev("aaaaaaa", &head)
            .with_rev("topic", &head)
            .with_rev("main", &base)
            .with_merge_base(&head, "main", &base)
            .with_branch("phase/auth/phase-1-design");
        (head, base, source)
    }

    #[test]
    fn resolve_change_target_pins_head_base_and_branch() {
        let (head, base, source) = target_repo();
        for rev in ["HEAD", "aaaaaaa", "topic"] {
            let (target, branch) = resolve_change_target(&source, rev, None, "main").unwrap();
            assert_eq!(
                target,
                ReviewTarget::Change {
                    head: head.clone(),
                    base: Some(base.clone()),
                },
                "{rev} must pin the same full-sha target"
            );
            assert_eq!(branch.as_deref(), Some("phase/auth/phase-1-design"));
        }
    }

    #[test]
    fn resolve_change_target_prefers_an_explicit_base_over_the_merge_base() {
        let (head, _, source) = target_repo();
        let other = "c".repeat(40);
        let source = source.with_rev("v1.0", &other);
        let (target, _) = resolve_change_target(&source, "HEAD", Some("v1.0"), "main").unwrap();
        assert_eq!(
            target,
            ReviewTarget::Change {
                head,
                base: Some(other),
            }
        );
    }

    #[test]
    fn resolve_change_target_rejects_an_unknown_head_and_an_unknown_base() {
        let (_, _, source) = target_repo();
        assert!(matches!(
            resolve_change_target(&source, "nope", None, "main"),
            Err(Error::ChangeRevisionNotFound(rev)) if rev == "nope"
        ));
        assert!(matches!(
            resolve_change_target(&source, "HEAD", Some("nope"), "main"),
            Err(Error::ChangeBaseNotFound(rev)) if rev == "nope"
        ));
    }

    #[test]
    fn resolve_change_target_names_base_when_there_is_no_merge_base() {
        // Unrelated histories: the head resolves, the merge base does not.
        let head = "a".repeat(40);
        let source = MemorySourceRepo::new().with_rev("HEAD", &head);
        let err = resolve_change_target(&source, "HEAD", None, "main").unwrap_err();
        assert!(matches!(err, Error::ChangeNoMergeBase { .. }));
        let text = err.to_string();
        assert!(
            text.contains("--base") && text.contains("main"),
            "a missing merge base must name --base and the branch: {text}"
        );
    }

    #[test]
    fn resolve_change_target_reports_no_branch_on_a_detached_head() {
        let head = "a".repeat(40);
        let base = "b".repeat(40);
        let source = MemorySourceRepo::new()
            .with_rev("HEAD", &head)
            .with_merge_base(&head, "main", &base);
        let (_, branch) = resolve_change_target(&source, "HEAD", None, "main").unwrap();
        assert_eq!(branch, None, "a detached HEAD must not stamp a branch");
    }

    #[test]
    fn resolve_change_target_honors_a_non_default_default_branch() {
        let head = "a".repeat(40);
        let base = "b".repeat(40);
        let source = MemorySourceRepo::new()
            .with_rev("HEAD", &head)
            .with_merge_base(&head, "develop", &base);
        assert!(resolve_change_target(&source, "HEAD", None, "main").is_err());
        let (target, _) = resolve_change_target(&source, "HEAD", None, "develop").unwrap();
        assert_eq!(
            target,
            ReviewTarget::Change {
                head,
                base: Some(base)
            }
        );
    }

    // ---- derive_change_anchor --------------------------------------------

    /// A two-line file at `head` whose second line the change touched.
    fn anchor_repo() -> MemorySourceRepo {
        MemorySourceRepo::new()
            .with_file(
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "src/lib.rs",
                "fn untouched() {}\nfn touched() {}\n",
            )
            .with_diff(
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "src/lib.rs",
                "@@ -2 +2 @@\n-old\n+fn touched() {}\n",
            )
    }

    #[test]
    fn derive_change_anchor_without_path_or_quote_is_a_whole_change_comment() {
        let source = anchor_repo();
        assert_eq!(
            derive_change_anchor(
                &source,
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
                None,
                None,
                None
            )
            .unwrap(),
            None
        );
    }

    #[test]
    fn derive_change_anchor_requires_both_halves_of_the_pair() {
        let source = anchor_repo();
        assert!(matches!(
            derive_change_anchor(
                &source,
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
                None,
                Some("src/lib.rs"),
                None
            ),
            Err(Error::ChangePathNeedsQuote)
        ));
        assert!(matches!(
            derive_change_anchor(
                &source,
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
                Some("fn touched"),
                None,
                None
            ),
            Err(Error::ChangeQuoteNeedsPath)
        ));
    }

    #[test]
    fn derive_change_anchor_anchors_inside_a_touched_hunk() {
        let source = anchor_repo();
        let anchor = derive_change_anchor(
            &source,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
            Some("fn touched"),
            Some("./src/lib.rs"),
            None,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            anchor,
            Anchor::FileQuote {
                // The leading `./` is normalized away so the emitted
                // permalink parses.
                path: "src/lib.rs".to_string(),
                quote: "fn touched".to_string(),
                occurrence: 1,
                start_line: 2,
                end_line: 2,
            }
        );
    }

    #[test]
    fn derive_change_anchor_refuses_a_quote_outside_every_touched_hunk() {
        let source = anchor_repo();
        let err = derive_change_anchor(
            &source,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
            Some("fn untouched"),
            Some("src/lib.rs"),
            None,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            Error::QuoteOutsideChangedHunks {
                nearest: Some((2, 2)),
                ..
            }
        ));
    }

    #[test]
    fn derive_change_anchor_reports_a_path_missing_at_head() {
        let source = anchor_repo();
        let err = derive_change_anchor(
            &source,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
            Some("anything"),
            Some("src/gone.rs"),
            None,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            Error::ChangePathNotInRevision { ref path, .. } if path == "src/gone.rs"
        ));
    }

    #[test]
    fn derive_change_anchor_reports_an_untouched_path_distinctly() {
        // The file exists at head but the change does not touch it: no
        // nearest hunk, so the message must not invent one.
        let source = MemorySourceRepo::new().with_file(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "src/other.rs",
            "fn other() {}\n",
        );
        let err = derive_change_anchor(
            &source,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
            Some("fn other"),
            Some("src/other.rs"),
            None,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            Error::QuoteOutsideChangedHunks { nearest: None, .. }
        ));
        assert!(
            err.to_string().contains("is not touched by"),
            "an untouched path needs its own message: {err}"
        );
    }

    #[test]
    fn derive_change_anchor_with_no_recorded_base_names_the_untouched_path() {
        // A hand-edited review frontmatter with no base yields no hunks
        // rather than anchoring blindly.
        let source = anchor_repo();
        let err = derive_change_anchor(
            &source,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            None,
            Some("fn touched"),
            Some("src/lib.rs"),
            None,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            Error::QuoteOutsideChangedHunks { nearest: None, .. }
        ));
    }

    #[test]
    fn derive_change_anchor_disambiguates_with_occurrence() {
        let source = MemorySourceRepo::new()
            .with_file(
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "src/lib.rs",
                "dup\ndup\n",
            )
            .with_diff(
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "src/lib.rs",
                "@@ -1,2 +1,2 @@\n+dup\n+dup\n",
            );
        // Ambiguous without --occurrence...
        assert!(matches!(
            derive_change_anchor(
                &source,
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
                Some("dup"),
                Some("src/lib.rs"),
                None
            ),
            Err(Error::QuoteAmbiguous { .. })
        ));
        // ...and the 2nd occurrence records the line it really sits on.
        let anchor = derive_change_anchor(
            &source,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
            Some("dup"),
            Some("src/lib.rs"),
            Some(2),
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            anchor,
            Anchor::FileQuote {
                path: "src/lib.rs".to_string(),
                quote: "dup".to_string(),
                occurrence: 2,
                start_line: 2,
                end_line: 2,
            }
        );
    }

    #[test]
    fn derive_change_anchor_rejects_a_non_relative_path() {
        let source = anchor_repo();
        assert!(
            derive_change_anchor(
                &source,
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
                Some("fn touched"),
                Some("../escape.rs"),
                None
            )
            .is_err()
        );
    }

    // --- object-kind eligibility (directory/submodule anchors) ---

    #[test]
    fn derive_change_anchor_rejects_a_tree_path() {
        let source = MemorySourceRepo::new().with_object_kind(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "sub",
            SourceObjectKind::Tree,
        );
        let err = derive_change_anchor(
            &source,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
            Some("a.txt"),
            Some("sub"),
            None,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            Error::ChangePathNotAFile {
                ref path,
                found: SourceObjectKind::Tree,
                ..
            } if path == "sub"
        ));
        assert!(
            err.to_string().contains("directory"),
            "message must name it as a directory: {err}"
        );
    }

    #[test]
    fn derive_change_anchor_rejects_a_gitlink_path() {
        let source = MemorySourceRepo::new().with_object_kind(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "vendor/lib",
            SourceObjectKind::Gitlink,
        );
        let err = derive_change_anchor(
            &source,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
            Some("anything"),
            Some("vendor/lib"),
            None,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            Error::ChangePathNotAFile {
                ref path,
                found: SourceObjectKind::Gitlink,
                ..
            } if path == "vendor/lib"
        ));
        assert!(
            err.to_string().contains("submodule"),
            "message must name it as a submodule: {err}"
        );
    }

    #[test]
    fn derive_change_anchor_succeeds_on_an_explicit_blob_control() {
        let source = MemorySourceRepo::new()
            .with_object_kind(
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "src/lib.rs",
                SourceObjectKind::Blob,
            )
            .with_file(
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "src/lib.rs",
                "fn touched() {}\n",
            )
            .with_diff(
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "src/lib.rs",
                "@@ -1 +1 @@\n-old\n+fn touched() {}\n",
            );
        let anchor = derive_change_anchor(
            &source,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
            Some("fn touched"),
            Some("src/lib.rs"),
            None,
        )
        .unwrap();
        assert!(anchor.is_some());
    }

    #[test]
    fn derive_change_anchor_treats_a_tree_listing_lookalike_blob_as_valid_content() {
        // Content that reads like `git show`'s own tree-listing text must
        // still anchor when the real object type says Blob — proof the fix
        // checks git's object type, never content prefixes.
        let content = "tree abc123:sub\n\na.txt\nb.txt\n";
        let source = MemorySourceRepo::new()
            .with_object_kind(
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "weird.txt",
                SourceObjectKind::Blob,
            )
            .with_file(
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "weird.txt",
                content,
            )
            .with_diff(
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "weird.txt",
                "@@ -0,0 +1,4 @@\n+tree abc123:sub\n+\n+a.txt\n+b.txt\n",
            );
        let anchor = derive_change_anchor(
            &source,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
            Some("a.txt"),
            Some("weird.txt"),
            None,
        )
        .unwrap();
        assert!(
            anchor.is_some(),
            "a Blob whose content resembles a tree listing must still anchor"
        );
    }

    #[test]
    fn resolve_change_comment_reports_unresolved_for_a_pre_existing_tree_anchor() {
        // Simulates a stored anchor written before this check existed (or by
        // hand): resolution must degrade it to Unresolved rather than
        // treating `git show`'s tree-listing text as quotable.
        let source = MemorySourceRepo::new().with_object_kind(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "sub",
            SourceObjectKind::Tree,
        );
        let review = review_with(Some(Anchor::FileQuote {
            path: "sub".to_string(),
            quote: "a.txt".to_string(),
            occurrence: 1,
            start_line: 1,
            end_line: 1,
        }));
        let resolved =
            resolve_change_comments(&source, &review, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        assert_eq!(resolved[0].resolution, Resolution::Unresolved);
        assert!(resolved[0].quote.is_none());
    }

    #[test]
    fn resolve_change_comment_reports_unresolved_when_the_tip_path_becomes_a_tree_or_gitlink() {
        // The head-side anchor is fully valid (a blob whose quote resolves
        // there); only the *tip*-side object kind changes, from Blob to
        // Tree/Gitlink. This exercises the second `object_kind_at` guard —
        // distinct from the pre-existing-bad-anchor test above, which only
        // ever seeds a non-Blob kind at `head` and returns via the first
        // guard.
        let head = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        for tip_kind in [SourceObjectKind::Tree, SourceObjectKind::Gitlink] {
            let source = MemorySourceRepo::new()
                .with_object_kind(head, "src/lib.rs", SourceObjectKind::Blob)
                .with_file(head, "src/lib.rs", CONTENT)
                .with_object_kind("tip", "src/lib.rs", tip_kind)
                // Content IS seeded at `tip` and still contains the quote:
                // if the tip-side kind guard were missing (or checked the
                // wrong revision), resolution would fall through to this
                // content and wrongly report Original instead of
                // Unresolved.
                .with_file("tip", "src/lib.rs", CONTENT);
            let review = review_with(Some(anchor()));
            let resolved = resolve_change_comments(&source, &review, "tip");
            assert_eq!(
                resolved[0].resolution,
                Resolution::Unresolved,
                "a path that became a {tip_kind:?} at the tip must not resolve"
            );
            assert!(resolved[0].quote.is_none());
        }
    }

    #[test]
    fn change_anchor_ineligibility_names_a_directory_and_a_submodule_and_clears_for_a_blob() {
        let head = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let source = MemorySourceRepo::new()
            .with_object_kind(head, "sub", SourceObjectKind::Tree)
            .with_object_kind(head, "vendor/lib", SourceObjectKind::Gitlink)
            .with_object_kind(head, "src/lib.rs", SourceObjectKind::Blob);

        let anchor_at = |path: &str| Anchor::FileQuote {
            path: path.to_string(),
            quote: "x".to_string(),
            occurrence: 1,
            start_line: 1,
            end_line: 1,
        };

        let note = change_anchor_ineligibility(&source, head, &anchor_at("sub")).unwrap();
        assert!(note.contains("directory"), "{note}");

        let note = change_anchor_ineligibility(&source, head, &anchor_at("vendor/lib")).unwrap();
        assert!(note.contains("submodule"), "{note}");

        assert!(change_anchor_ineligibility(&source, head, &anchor_at("src/lib.rs")).is_none());
    }

    /// The anchor a derivation produces must be the one resolution accepts:
    /// derive at head, resolve against a tip that still carries the quote.
    #[test]
    fn a_derived_anchor_round_trips_through_resolution() {
        let source = anchor_repo().with_file(
            "tip",
            "src/lib.rs",
            "fn untouched() {}\nfn touched() {} // extended\n",
        );
        let anchor = derive_change_anchor(
            &source,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
            Some("fn touched"),
            Some("src/lib.rs"),
            None,
        )
        .unwrap();
        let review = review_with(anchor);
        let resolved = resolve_change_comments(&source, &review, "tip");
        assert!(matches!(
            resolved[0].resolution,
            Resolution::Original { drifted: false, .. }
        ));
    }
    struct NoSourceAccess;
    impl SourceRepo for NoSourceAccess {
        fn rev_parse(&self, _: &str) -> Result<Option<String>> {
            panic!("unexpected rev_parse")
        }
        fn merge_base(&self, _: &str, _: &str) -> Result<Option<String>> {
            panic!("unexpected merge_base")
        }
        fn file_at(&self, _: &str, _: &str) -> Result<Option<String>> {
            panic!("unexpected file_at")
        }
        fn unified_diff(&self, _: &str, _: &str, _: &str) -> Result<Option<String>> {
            panic!("unexpected diff")
        }
        fn head(&self) -> Result<Option<String>> {
            panic!("unexpected head")
        }
        fn current_branch(&self) -> Result<Option<String>> {
            panic!("unexpected branch")
        }
        fn object_kind_at(
            &self,
            _: &str,
            _: &str,
        ) -> Result<Option<crate::source::SourceObjectKind>> {
            panic!("unexpected object_kind_at")
        }
    }

    #[test]
    fn malformed_direct_core_targets_never_access_source() {
        for field in ["head", "base"] {
            for bad in ["HEAD", "--output=fixture", "", "é"] {
                let head = if field == "head" {
                    bad.to_string()
                } else {
                    "a".repeat(40)
                };
                let base = if field == "base" {
                    bad.to_string()
                } else {
                    "b".repeat(40)
                };
                for (quote, path) in [(None, None), (Some("quote"), Some("a.txt"))] {
                    assert!(
                        matches!(derive_change_anchor(&NoSourceAccess, &head, Some(&base), quote, path, None),
                        Err(Error::InvalidStoredChangeRevision { field: f, .. }) if f == field)
                    );
                }
                for anchor in [
                    None,
                    Some(Anchor::FileQuote {
                        path: "a.txt".into(),
                        quote: "quote".into(),
                        occurrence: 1,
                        start_line: 1,
                        end_line: 1,
                    }),
                ] {
                    let mut review = review_with(anchor);
                    review.target = ReviewTarget::Change {
                        head: head.clone(),
                        base: Some(base.clone()),
                    };
                    assert_eq!(
                        resolve_change_comment(
                            &NoSourceAccess,
                            &review,
                            &review.comments[0],
                            "topic"
                        )
                        .resolution,
                        Resolution::Unresolved
                    );
                    assert_eq!(
                        resolve_change_comments(&NoSourceAccess, &review, "topic")[0].resolution,
                        Resolution::Unresolved
                    );
                }
            }
        }
    }

    #[test]
    fn resolving_a_target_rejects_malformed_adapter_identities() {
        let head = "a".repeat(40);
        let source = MemorySourceRepo::new().with_rev("HEAD", "short");
        assert!(matches!(
            resolve_change_target(&source, "HEAD", None, "main"),
            Err(Error::InvalidStoredChangeRevision { field: "head", .. })
        ));
        let source = MemorySourceRepo::new()
            .with_rev("HEAD", &head)
            .with_rev("main", "short")
            .with_merge_base(&head, "main", "short");
        for explicit in [None, Some("main")] {
            assert!(matches!(
                resolve_change_target(&source, "HEAD", explicit, "main"),
                Err(Error::InvalidStoredChangeRevision { field: "base", .. })
            ));
        }
    }
}

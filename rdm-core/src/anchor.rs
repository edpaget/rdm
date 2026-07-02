//! Anchor resolution: locating a review comment's quoted span within a
//! target body, including after the body has been edited.
//!
//! Two layers:
//!
//! - [`resolve`](crate::anchor::resolve) is pure text resolution: given a
//!   body and an [`Anchor`](crate::model::Anchor), find the byte range the
//!   anchor points at.
//! - [`resolve_against_history`](crate::anchor::resolve_against_history)
//!   is history-aware: it resolves against the body the reviewer actually
//!   saw (at the review's `created_commit`), checks whether the span has
//!   since drifted, and degrades gracefully — to a current-body match, and
//!   finally to [`Resolution::Unresolved`](crate::anchor::Resolution) —
//!   when history or the target document is unreachable. It never fails.
//!
//! # Resolution algorithm
//!
//! For an [`Anchor::TextQuote`](crate::model::Anchor) `{ quote, prefix,
//! suffix }`, [`resolve`](crate::anchor::resolve) walks a decision ladder:
//!
//! 1. **Exact, unique** — if `quote` occurs exactly once in the body, that
//!    occurrence wins.
//! 2. **Exact, context-disambiguated** — if `quote` occurs multiple times,
//!    each occurrence is scored by how many bytes of its surrounding text
//!    agree with the anchor's `prefix` (compared right-to-left against the
//!    text before the occurrence) and `suffix` (compared left-to-right
//!    against the text after it). The highest-scoring occurrence wins;
//!    ties go to the earliest occurrence.
//! 3. **Fuzzy context bracketing** — if `quote` does not occur at all (the
//!    body drifted), the span is inferred from the surviving context: the
//!    longest tail of `prefix` found in the body marks the start, and the
//!    longest head of `suffix` found after it marks the end; the text
//!    between them is the drifted span. Among all such prefix/suffix
//!    occurrence pairs, non-empty spans are preferred (an empty span —
//!    surviving context runs that merely abut — is chosen only when no
//!    non-empty pair exists), and within that the pair bracketing the
//!    **shortest** span is chosen (ties go to the earliest), which keeps
//!    compound drift from bracketing a span covering most of the document.
//!    The shortest-span rule is relative, not an absolute cap: when only a
//!    single surviving pair exists it can still bracket most of the
//!    document. Both context sides are
//!    required, and each surviving run must keep at least 3 chars (or the
//!    anchor's full context, if shorter) — a quote whose surrounding
//!    context was also deleted, or an anchor captured with empty context,
//!    cannot be recovered and yields `None`. The inferred span can still
//!    be wrong when the surviving context is short and repeats elsewhere
//!    in the document; callers should treat fuzzy matches as best-effort.
//! 4. **None** — nothing above matched.
//!
//! All offsets come from matching `&str` patterns and trimming on
//! [`char_indices`](str::char_indices), so returned ranges always land on
//! `char` boundaries — resolution never panics on multi-byte UTF-8
//! content.

use std::ops::Range;

use crate::error::Error;
use crate::io;
use crate::model::{Anchor, CommentDoc, CommentDocKind, Review, ReviewComment, ReviewTarget};
use crate::store::VersionedStore;

/// Resolves an anchor to a byte range within `body`.
///
/// See the [module documentation](self) for the full decision ladder
/// (exact → context-disambiguated → fuzzy context bracketing → `None`).
///
/// The returned range is valid for slicing `body` (`&body[range]`) and its
/// endpoints always fall on `char` boundaries. Returns `None` for
/// [`Anchor::Unknown`], for an empty quote, and when neither the quote nor
/// enough of its context can be found.
///
/// # Panics
///
/// Never panics, including on multi-byte UTF-8 content: all candidate
/// offsets come from `&str` pattern matches and `char`-boundary trimming.
///
/// # Examples
///
/// ```
/// use rdm_core::anchor::resolve;
/// use rdm_core::model::Anchor;
///
/// let body = "Alpha beta gamma.";
/// let anchor = Anchor::TextQuote {
///     quote: "beta".to_string(),
///     prefix: "Alpha ".to_string(),
///     suffix: " gamma".to_string(),
/// };
/// let range = resolve(body, &anchor).unwrap();
/// assert_eq!(&body[range], "beta");
/// ```
pub fn resolve(body: &str, anchor: &Anchor) -> Option<Range<usize>> {
    let Anchor::TextQuote {
        quote,
        prefix,
        suffix,
    } = anchor
    else {
        return None;
    };
    if quote.is_empty() {
        return None;
    }

    let candidates: Vec<usize> = body.match_indices(quote.as_str()).map(|(i, _)| i).collect();
    match candidates.as_slice() {
        [] => fuzzy_bracket(body, prefix, suffix),
        [start] => Some(*start..*start + quote.len()),
        starts => {
            // Score each occurrence by surrounding-context agreement; the
            // highest score wins, ties go to the earliest occurrence.
            let mut best_start = starts[0];
            let mut best_score = 0usize;
            for (i, &start) in starts.iter().enumerate() {
                let end = start + quote.len();
                let score = common_suffix_len(&body[..start], prefix)
                    + common_prefix_len(&body[end..], suffix);
                if i == 0 || score > best_score {
                    best_start = start;
                    best_score = score;
                }
            }
            Some(best_start..best_start + quote.len())
        }
    }
}

/// Counts the bytes of the longest common trailing run between `text` and
/// `context`, comparing `char` by `char` from the right.
fn common_suffix_len(text: &str, context: &str) -> usize {
    text.chars()
        .rev()
        .zip(context.chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(a, _)| a.len_utf8())
        .sum()
}

/// Counts the bytes of the longest common leading run between `text` and
/// `context`, comparing `char` by `char` from the left.
fn common_prefix_len(text: &str, context: &str) -> usize {
    text.chars()
        .zip(context.chars())
        .take_while(|(a, b)| a == b)
        .map(|(a, _)| a.len_utf8())
        .sum()
}

/// Minimum surviving context (in chars) the fuzzy fallback accepts.
///
/// Trimming context below this would match near-arbitrary text (a lone
/// space occurs almost everywhere), so shorter runs are treated as "context
/// did not survive" — unless the anchor's full context is itself shorter,
/// in which case an exact full-context match is still accepted.
const MIN_FUZZY_CONTEXT_CHARS: usize = 3;

/// Returns the longest tail of `prefix` (trimming leading chars) that
/// occurs somewhere in `body` and is at least
/// [`MIN_FUZZY_CONTEXT_CHARS`] long (or the full `prefix`, if shorter).
fn longest_matching_tail<'a>(body: &str, prefix: &'a str) -> Option<&'a str> {
    let total = prefix.chars().count();
    let min_chars = total.min(MIN_FUZZY_CONTEXT_CHARS);
    if min_chars == 0 {
        return None;
    }
    for (trimmed, (i, _)) in prefix.char_indices().enumerate() {
        if total - trimmed < min_chars {
            break;
        }
        let tail = &prefix[i..];
        if body.contains(tail) {
            return Some(tail);
        }
    }
    None
}

/// Returns the longest head of `suffix` (trimming trailing chars) that
/// occurs somewhere in `body` and is at least
/// [`MIN_FUZZY_CONTEXT_CHARS`] long (or the full `suffix`, if shorter).
fn longest_matching_head<'a>(body: &str, suffix: &'a str) -> Option<&'a str> {
    let total = suffix.chars().count();
    let min_chars = total.min(MIN_FUZZY_CONTEXT_CHARS);
    if min_chars == 0 {
        return None;
    }
    let mut end = suffix.len();
    let mut chars = total;
    while chars >= min_chars {
        let head = &suffix[..end];
        if body.contains(head) {
            return Some(head);
        }
        end = suffix[..end]
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0);
        chars -= 1;
    }
    None
}

/// Fuzzy fallback: infer the drifted span from the surviving context.
///
/// Finds the longest tail of `prefix` and the longest head of `suffix`
/// present in `body`, then, over every (prefix occurrence, first suffix
/// occurrence after it) pair, prefers non-empty spans — an empty span
/// (context runs that merely abut, a coincidental adjacency) is chosen
/// only when no non-empty pair exists — and within that picks the pair
/// bracketing the shortest span (ties: earliest). The shortest-span rule
/// is relative, not an absolute cap: a single surviving pair can still
/// bracket most of the document. Requires both context sides; returns
/// `None` otherwise.
fn fuzzy_bracket(body: &str, prefix: &str, suffix: &str) -> Option<Range<usize>> {
    let ptail = longest_matching_tail(body, prefix)?;
    let shead = longest_matching_head(body, suffix)?;

    let mut best: Option<Range<usize>> = None;
    for (i, _) in body.match_indices(ptail) {
        let p_end = i + ptail.len();
        if let Some(rel) = body[p_end..].find(shead) {
            let s_start = p_end + rel;
            let len = s_start - p_end;
            // Lexicographic (is_empty, len): non-empty beats empty, then
            // shorter beats longer; ties keep the earliest (iteration is
            // in ascending position order and the comparison is strict).
            let better = match &best {
                None => true,
                Some(b) => (len == 0, len) < (b.is_empty(), b.len()),
            };
            if better {
                best = Some(p_end..s_start);
            }
        }
    }
    best
}

/// Outcome of resolving a review comment's anchor against history and the
/// current plan repo state.
///
/// The two resolved variants index **different bodies**: callers must
/// reload the matching body before slicing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// The anchor resolved in the body as it existed at the review's
    /// `created_commit` — the version the reviewer actually saw.
    Original {
        /// Byte range into the *historical* body — the document's body as
        /// it existed at the review's `created_commit` (what
        /// [`crate::io::load_roadmap_at`] / [`crate::io::load_phase_at`] /
        /// [`crate::io::load_task_at`] return while the document still
        /// exists, or [`crate::store::VersionedStore::fetch_body_at`]
        /// regardless). Not valid against the current body.
        range: Range<usize>,
        /// `true` when the current body no longer resolves the same anchor
        /// to text identical to the span the reviewer saw — because the
        /// span was edited, deleted, or the target document is gone.
        drifted: bool,
    },
    /// History was unreachable (unknown revision, file absent at that
    /// revision, or a backend without history), so the anchor was resolved
    /// against the current body instead.
    Current {
        /// Byte range into the *current* body (the one returned by
        /// [`crate::io::load_roadmap`] / [`crate::io::load_phase`] /
        /// [`crate::io::load_task`]).
        range: Range<usize>,
    },
    /// The anchor could not be resolved in any available body: the comment
    /// has no anchor, the anchor type is unrecognized, the target document
    /// is dangling, or the quoted span (and its context) is gone.
    Unresolved,
}

/// The document a comment's anchor should be resolved against.
#[derive(Clone, Copy)]
enum DocSelector<'a> {
    Roadmap { roadmap: &'a str },
    Phase { roadmap: &'a str, stem: &'a str },
    Task { slug: &'a str },
}

/// Picks the document a comment points at: its `doc` selector when it
/// scopes a roadmap review to one of the roadmap's phases, otherwise the
/// review's own target. A `doc` on a non-roadmap review (which the ops
/// layer rejects at write time) is ignored defensively.
fn doc_selector<'a>(review: &'a Review, comment: &'a ReviewComment) -> DocSelector<'a> {
    if let (
        Some(CommentDoc {
            kind: CommentDocKind::Phase,
            stem,
        }),
        ReviewTarget::Roadmap { roadmap },
    ) = (&comment.doc, &review.target)
    {
        return DocSelector::Phase { roadmap, stem };
    }
    match &review.target {
        ReviewTarget::Roadmap { roadmap } => DocSelector::Roadmap { roadmap },
        ReviewTarget::Phase { roadmap, stem } => DocSelector::Phase { roadmap, stem },
        ReviewTarget::Task { slug } => DocSelector::Task { slug },
    }
}

/// Store path of the selected document.
fn doc_path(project: &str, selector: DocSelector<'_>) -> crate::store::RelPath {
    match selector {
        DocSelector::Roadmap { roadmap } => crate::paths::roadmap_path(project, roadmap),
        DocSelector::Phase { roadmap, stem } => crate::paths::phase_path(project, roadmap, stem),
        DocSelector::Task { slug } => crate::paths::task_path(project, slug),
    }
}

/// Loads the selected document's body as it existed at `sha`, reading
/// purely from history via [`VersionedStore::fetch_body_at`].
///
/// Deliberately does not require the document to *currently* exist (unlike
/// [`crate::io::load_roadmap_at`] and friends, which also load the current
/// frontmatter): a review's anchor must still resolve against the body the
/// reviewer saw even after the target document has been deleted.
fn load_body_at(
    store: &impl VersionedStore,
    project: &str,
    selector: DocSelector<'_>,
    sha: &str,
) -> crate::error::Result<String> {
    let content = store.fetch_body_at(&doc_path(project, selector), sha)?;
    let (_, body) = crate::markdown::split_frontmatter(&content)?;
    Ok(body.to_string())
}

/// Loads the selected document's current body.
fn load_body_current(
    store: &impl VersionedStore,
    project: &str,
    selector: DocSelector<'_>,
) -> crate::error::Result<String> {
    match selector {
        DocSelector::Roadmap { roadmap } => {
            io::load_roadmap(store, project, roadmap).map(|d| d.body)
        }
        DocSelector::Phase { roadmap, stem } => {
            io::load_phase(store, project, roadmap, stem).map(|d| d.body)
        }
        DocSelector::Task { slug } => io::load_task(store, project, slug).map(|d| d.body),
    }
}

/// Resolves a review comment's anchor against the body the reviewer saw,
/// falling back to the current body, and degrading to
/// [`Resolution::Unresolved`] instead of failing.
///
/// The ladder:
///
/// 1. If the review records a `created_commit`, load the comment's document
///    (its `doc` selector, or the review target) at that revision — purely
///    from history via [`VersionedStore::fetch_body_at`], so a document
///    that has since been deleted still resolves — and [`resolve`] there.
///    On success, returns [`Resolution::Original`] with `drifted`
///    reporting whether the *current* body still resolves the same anchor
///    to identical text (a deleted or renamed target counts as drifted).
/// 2. Otherwise — no recorded commit, unknown revision, document absent at
///    that revision (including a dangling `doc` selector), historyless
///    backend, or the anchor simply not found in the historical body —
///    resolve against the current body and return [`Resolution::Current`].
/// 3. Otherwise, [`Resolution::Unresolved`]. Whole-document comments
///    (no anchor) and [`Anchor::Unknown`] are always `Unresolved`.
///
/// Store errors are classified rather than propagated: benign history
/// errors ([`Error::RevisionUnknown`], [`Error::BodyAtRevisionMissing`],
/// [`Error::HistoryUnavailable`]) and unparseable historical content
/// degrade down the ladder, and any failure to load the current body
/// (e.g. a dangling target) degrades the current-body attempt. This
/// function is deliberately infallible, so unexpected failures (e.g.
/// [`Error::Io`]) also degrade; a fallible sibling that surfaces them can
/// be added later without changing this contract.
///
/// # Panics
///
/// Never panics.
pub fn resolve_against_history(
    store: &impl VersionedStore,
    project: &str,
    review: &Review,
    comment: &ReviewComment,
) -> Resolution {
    let Some(anchor) = &comment.anchor else {
        return Resolution::Unresolved;
    };
    if matches!(anchor, Anchor::Unknown { .. }) {
        return Resolution::Unresolved;
    }
    let selector = doc_selector(review, comment);

    if let Some(sha) = &review.created_commit {
        match load_body_at(store, project, selector, sha) {
            Ok(orig_body) => {
                if let Some(range) = resolve(&orig_body, anchor) {
                    let quote_text = &orig_body[range.clone()];
                    let drifted = match load_body_current(store, project, selector) {
                        Ok(cur_body) => match resolve(&cur_body, anchor) {
                            Some(cur_range) => &cur_body[cur_range] != quote_text,
                            None => true,
                        },
                        // Target deleted or renamed since the review: the
                        // span the reviewer saw no longer exists as-is.
                        Err(_) => true,
                    };
                    return Resolution::Original { range, drifted };
                }
                // Anchor not found in the historical body: fall through to
                // the current body.
            }
            // Benign history errors: the recorded revision is unknown, the
            // document was absent at it (including a dangling `doc`
            // selector), or the backend has no history.
            Err(
                Error::RevisionUnknown { .. }
                | Error::BodyAtRevisionMissing { .. }
                | Error::HistoryUnavailable,
            ) => {}
            // Corrupt historical content: degrade rather than fail.
            Err(Error::FrontmatterMissing | Error::FrontmatterParse(_)) => {}
            // Unexpected failures (e.g. `Error::Io`) would ideally surface,
            // but this resolver is contractually infallible; degrade here
            // and leave propagation to a future fallible sibling.
            Err(_) => {}
        }
    }

    match load_body_current(store, project, selector) {
        Ok(cur_body) => match resolve(&cur_body, anchor) {
            Some(range) => Resolution::Current { range },
            None => Resolution::Unresolved,
        },
        Err(_) => Resolution::Unresolved,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::io::{write_phase, write_roadmap, write_task};
    use crate::model::{
        Phase, PhaseStatus, Priority, ReviewCommentStatus, ReviewState, Roadmap, Task, TaskStatus,
    };
    use crate::store::{MemoryStore, Store};

    fn tq(quote: &str, prefix: &str, suffix: &str) -> Anchor {
        Anchor::TextQuote {
            quote: quote.to_string(),
            prefix: prefix.to_string(),
            suffix: suffix.to_string(),
        }
    }

    // -- pure resolve --

    #[test]
    fn resolve_unique_quote_exact() {
        let body = "The quick brown fox jumps over the lazy dog.";
        let range = resolve(body, &tq("brown fox", "quick ", " jumps")).unwrap();
        assert_eq!(&body[range], "brown fox");
    }

    #[test]
    fn resolve_duplicate_quote_disambiguated_by_prefix() {
        let body = "call foo() here, then call foo() there";
        let range = resolve(body, &tq("call foo()", "then ", "")).unwrap();
        assert_eq!(range.start, body.rfind("call foo()").unwrap());
    }

    #[test]
    fn resolve_duplicate_quote_disambiguated_by_suffix() {
        let body = "call foo() here, then call foo() there";
        let range = resolve(body, &tq("call foo()", "", " there")).unwrap();
        assert_eq!(range.start, body.rfind("call foo()").unwrap());
    }

    #[test]
    fn resolve_duplicate_quote_prefix_and_suffix_combined() {
        // Three occurrences of "Q" (offsets 4, 21, 38), scored by summed
        // context agreement against prefix "PPPP" and suffix "SSSS":
        //   #1 (4):  prefix 4 ("PPPP") + suffix 0            = 4
        //   #2 (21): prefix 0          + suffix 4 ("SSSS")   = 4
        //   #3 (38): prefix 3 ("PPP")  + suffix 3 ("SSS")    = 6
        // #3 wins uniquely on the combined score even though neither
        // dimension alone selects it (prefix alone favors #1, suffix alone
        // favors #2).
        let body = "PPPPQnnnn extra1 nnnnQSSSS extra2 zPPPQSSSz extra3";
        let range = resolve(body, &tq("Q", "PPPP", "SSSS")).unwrap();
        assert_eq!(range, 38..39);
        assert_eq!(&body[range], "Q");
    }

    #[test]
    fn resolve_duplicate_no_context_returns_first() {
        let body = "dup text and dup text";
        let range = resolve(body, &tq("dup text", "", "")).unwrap();
        assert_eq!(range, 0.."dup text".len());
    }

    #[test]
    fn resolve_drifted_body_via_fuzzy_bracket() {
        // Anchor captured against: "prefix OLD QUOTE suffix"
        let body = "Some intro. prefix NEW DRIFTED TEXT suffix. Trailing.";
        let range = resolve(body, &tq("OLD QUOTE", "prefix ", " suffix")).unwrap();
        assert_eq!(&body[range], "NEW DRIFTED TEXT");
    }

    #[test]
    fn resolve_fuzzy_prefers_shortest_bracketed_span() {
        // The context pair occurs twice; the second pair brackets a shorter
        // span. Compound drift must not select the document-spanning pair.
        let body = "pre AAAA long long long suf ... pre B suf";
        let range = resolve(body, &tq("GONE", "pre ", " suf")).unwrap();
        assert_eq!(&body[range], "B");
    }

    #[test]
    fn resolve_fuzzy_prefers_non_empty_span_over_coincidental_adjacency() {
        // "presuf" makes the surviving prefix run abut the surviving suffix
        // run — a zero-width span. The genuine drifted span later in the
        // body must win over that coincidental adjacency.
        let body = "intro presuf blah pre DRIFTED suf tail";
        let range = resolve(body, &tq("GONE", "pre", "suf")).unwrap();
        assert_eq!(range, 21..30);
        assert_eq!(&body[range], " DRIFTED ");
    }

    #[test]
    fn resolve_fuzzy_min_context_three_chars_matches() {
        // Only 3 chars of each context side survive — exactly the floor.
        let body = "abc NEW xyz";
        let range = resolve(body, &tq("OLD", "QQQabc", "xyzQQQ")).unwrap();
        assert_eq!(range, 3..8);
        assert_eq!(&body[range], " NEW ");
    }

    #[test]
    fn resolve_fuzzy_min_context_below_floor_returns_none() {
        // Only 2 chars of the prefix survive ("ab") — below the 3-char
        // floor, so the fuzzy fallback refuses even though the suffix is
        // fully present.
        let body = "ab NEW xyz";
        assert_eq!(resolve(body, &tq("OLD", "QQQab", "xyz")), None);
    }

    #[test]
    fn resolve_fuzzy_requires_suffix_context() {
        // The prefix context survives but the suffix context is entirely
        // absent — both sides are required, so no fuzzy match.
        let body = "abc NEW nothing-else";
        assert_eq!(resolve(body, &tq("OLD", "QQQabc", "zzz")), None);
    }

    #[test]
    fn resolve_fuzzy_requires_prefix_context() {
        // Mirror case: the suffix context survives but the prefix context
        // is entirely absent.
        let body = "nothing NEW xyz";
        assert_eq!(resolve(body, &tq("OLD", "qqq", "xyzZZZ")), None);
    }

    #[test]
    fn resolve_quote_deleted_entirely_returns_none() {
        let body = "Completely unrelated content now.";
        assert_eq!(
            resolve(body, &tq("old quote", "old prefix ", " old suffix")),
            None
        );
    }

    #[test]
    fn resolve_absent_quote_no_context_returns_none() {
        let body = "Some body text.";
        assert_eq!(resolve(body, &tq("missing", "", "")), None);
    }

    #[test]
    fn resolve_unknown_anchor_returns_none() {
        let anchor = Anchor::Unknown {
            anchor_type: "line-range".to_string(),
            raw: serde_yaml::Value::Null,
        };
        assert_eq!(resolve("any body", &anchor), None);
    }

    #[test]
    fn resolve_empty_quote_returns_none() {
        assert_eq!(resolve("body", &tq("", "pre", "suf")), None);
    }

    #[test]
    fn resolve_multibyte_quote_on_char_boundary() {
        let body = "Café notes: the résumé draft — naïve approach.";
        let range = resolve(body, &tq("résumé", "the ", " draft")).unwrap();
        assert!(body.is_char_boundary(range.start));
        assert!(body.is_char_boundary(range.end));
        assert_eq!(&body[range], "résumé");
    }

    #[test]
    fn resolve_multibyte_fuzzy_bracket_trims_on_char_boundary() {
        // Context is multi-byte and only partially survives, so trimming
        // must walk char boundaries without panicking.
        let body = "…héllo wörld NEW süffix tëxt…";
        // prefix ends with "héllo wörld " (survives partially), suffix
        // starts with " süffix" (survives partially).
        let range = resolve(body, &tq("ÖLD", "gøne héllo wörld ", " süffix gøne")).unwrap();
        assert!(body.is_char_boundary(range.start));
        assert!(body.is_char_boundary(range.end));
        assert_eq!(&body[range], "NEW");
    }

    // -- resolve_against_history --

    fn seed_roadmap(store: &mut MemoryStore, body: &str) {
        let doc = Document {
            frontmatter: Roadmap {
                project: "test".to_string(),
                roadmap: "alpha".to_string(),
                title: "Alpha".to_string(),
                phases: vec!["phase-1-one".to_string()],
                dependencies: None,
                priority: None,
                tags: None,
            },
            body: body.to_string(),
        };
        write_roadmap(store, "test", "alpha", &doc).unwrap();
    }

    fn seed_phase(store: &mut MemoryStore, body: &str) {
        let doc = Document {
            frontmatter: Phase {
                phase: 1,
                title: "One".to_string(),
                status: PhaseStatus::NotStarted,
                tags: None,
                completed: None,
                commit: None,
                review_sha: None,
                review_branch: None,
                difficulty: None,
                model: None,
                blocked_reason: None,
            },
            body: body.to_string(),
        };
        write_phase(store, "test", "alpha", "phase-1-one", &doc).unwrap();
    }

    fn seed_task(store: &mut MemoryStore, body: &str) {
        let doc = Document {
            frontmatter: Task {
                project: "test".to_string(),
                title: "Fix".to_string(),
                status: TaskStatus::Open,
                priority: Priority::Medium,
                created: chrono::NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(),
                tags: None,
                completed: None,
                commit: None,
                review_sha: None,
                review_branch: None,
            },
            body: body.to_string(),
        };
        write_task(store, "test", "fix", &doc).unwrap();
    }

    fn comment(anchor: Option<Anchor>, doc: Option<CommentDoc>) -> ReviewComment {
        ReviewComment {
            id: 1,
            doc,
            status: ReviewCommentStatus::Open,
            applied_commit: None,
            anchor,
            body: "Please reconsider this.".to_string(),
            reply: None,
        }
    }

    fn review(target: ReviewTarget, created_commit: Option<&str>) -> Review {
        Review {
            id: "2026-07-01-1200-abcd".to_string(),
            author: "reviewer".to_string(),
            target,
            state: ReviewState::Submitted,
            verdict: None,
            created: chrono::Utc::now(),
            submitted: None,
            created_commit: created_commit.map(str::to_string),
            comments: Vec::new(),
        }
    }

    fn task_target() -> ReviewTarget {
        ReviewTarget::Task {
            slug: "fix".to_string(),
        }
    }

    #[test]
    fn history_resolves_original_not_drifted() {
        let mut store = MemoryStore::new();
        seed_task(&mut store, "intro. the quoted span here. outro.");
        store.commit().unwrap();
        let sha = store.head_sha().unwrap();

        let rev = review(task_target(), Some(&sha));
        let c = comment(Some(tq("quoted span", "the ", " here")), None);
        let res = resolve_against_history(&store, "test", &rev, &c);
        match res {
            Resolution::Original { range, drifted } => {
                assert!(!drifted);
                // Range indexes the historical body, which equals the
                // current body here.
                let body = crate::io::load_task_at(&store, "test", "fix", &sha)
                    .unwrap()
                    .body;
                assert_eq!(&body[range], "quoted span");
            }
            other => panic!("expected Original, got {other:?}"),
        }
    }

    #[test]
    fn history_flags_drift_when_current_changed() {
        let mut store = MemoryStore::new();
        seed_task(&mut store, "intro. the quoted span here. outro.");
        store.commit().unwrap();
        let sha1 = store.head_sha().unwrap();
        seed_task(&mut store, "intro. the reworded span here. outro.");
        store.commit().unwrap();

        let rev = review(task_target(), Some(&sha1));
        let c = comment(Some(tq("quoted span", "the ", " here")), None);
        match resolve_against_history(&store, "test", &rev, &c) {
            Resolution::Original { range, drifted } => {
                assert!(drifted);
                let orig = crate::io::load_task_at(&store, "test", "fix", &sha1)
                    .unwrap()
                    .body;
                assert_eq!(&orig[range], "quoted span");
            }
            other => panic!("expected Original, got {other:?}"),
        }
    }

    #[test]
    fn history_falls_back_to_current_on_unknown_sha() {
        let mut store = MemoryStore::new();
        seed_task(&mut store, "current body with the quoted span here.");
        store.commit().unwrap();

        let rev = review(task_target(), Some("mem-nope"));
        let c = comment(Some(tq("quoted span", "the ", " here")), None);
        match resolve_against_history(&store, "test", &rev, &c) {
            Resolution::Current { range } => {
                let cur = crate::io::load_task(&store, "test", "fix").unwrap().body;
                assert_eq!(&cur[range], "quoted span");
            }
            other => panic!("expected Current, got {other:?}"),
        }
    }

    #[test]
    fn history_unresolved_on_unknown_sha_and_current_miss() {
        let mut store = MemoryStore::new();
        seed_task(&mut store, "a body that does not contain the anchor.");
        store.commit().unwrap();

        let rev = review(task_target(), Some("mem-nope"));
        let c = comment(
            Some(tq("vanished quote", "gone prefix ", " gone suffix")),
            None,
        );
        assert_eq!(
            resolve_against_history(&store, "test", &rev, &c),
            Resolution::Unresolved
        );
    }

    #[test]
    fn history_falls_back_to_current_on_missing_at_sha() {
        let mut store = MemoryStore::new();
        store.commit().unwrap();
        let pre = store.head_sha().unwrap(); // task not yet created here
        seed_task(&mut store, "now the quoted span here exists.");
        store.commit().unwrap();

        let rev = review(task_target(), Some(&pre));
        let c = comment(Some(tq("quoted span", "the ", " here")), None);
        match resolve_against_history(&store, "test", &rev, &c) {
            Resolution::Current { range } => {
                let cur = crate::io::load_task(&store, "test", "fix").unwrap().body;
                assert_eq!(&cur[range], "quoted span");
            }
            other => panic!("expected Current, got {other:?}"),
        }
    }

    #[test]
    fn history_falls_back_to_current_when_created_commit_none() {
        let mut store = MemoryStore::new();
        seed_task(&mut store, "the quoted span here.");
        store.commit().unwrap();

        let rev = review(task_target(), None);
        let c = comment(Some(tq("quoted span", "the ", " here")), None);
        match resolve_against_history(&store, "test", &rev, &c) {
            Resolution::Current { range } => {
                let cur = crate::io::load_task(&store, "test", "fix").unwrap().body;
                assert_eq!(&cur[range], "quoted span");
            }
            other => panic!("expected Current, got {other:?}"),
        }
    }

    #[test]
    fn history_drifted_true_when_target_deleted() {
        let mut store = MemoryStore::new();
        seed_task(&mut store, "intro. the quoted span here. outro.");
        store.commit().unwrap();
        let sha = store.head_sha().unwrap();
        store
            .delete(&crate::paths::task_path("test", "fix"))
            .unwrap();
        store.commit().unwrap();

        // The target is gone from the current state, but the reviewer's
        // span must still resolve from history — flagged as drifted.
        let rev = review(task_target(), Some(&sha));
        let c = comment(Some(tq("quoted span", "the ", " here")), None);
        match resolve_against_history(&store, "test", &rev, &c) {
            Resolution::Original { range, drifted } => {
                assert!(drifted);
                let content = store
                    .fetch_body_at(&crate::paths::task_path("test", "fix"), &sha)
                    .unwrap();
                let (_, body) = crate::markdown::split_frontmatter(&content).unwrap();
                assert_eq!(&body[range], "quoted span");
            }
            other => panic!("expected Original, got {other:?}"),
        }
    }

    #[test]
    fn history_unresolved_on_dangling_doc() {
        let mut store = MemoryStore::new();
        seed_roadmap(&mut store, "roadmap body.");
        store.commit().unwrap();
        let sha = store.head_sha().unwrap();

        let rev = review(
            ReviewTarget::Roadmap {
                roadmap: "alpha".to_string(),
            },
            Some(&sha),
        );
        let c = comment(
            Some(tq("anything", "", "")),
            Some(CommentDoc {
                kind: CommentDocKind::Phase,
                stem: "ghost".to_string(),
            }),
        );
        assert_eq!(
            resolve_against_history(&store, "test", &rev, &c),
            Resolution::Unresolved
        );
    }

    #[test]
    fn history_unresolved_on_unknown_anchor() {
        let mut store = MemoryStore::new();
        seed_task(&mut store, "body.");
        store.commit().unwrap();
        let sha = store.head_sha().unwrap();

        let rev = review(task_target(), Some(&sha));
        let c = comment(
            Some(Anchor::Unknown {
                anchor_type: "line-range".to_string(),
                raw: serde_yaml::Value::Null,
            }),
            None,
        );
        assert_eq!(
            resolve_against_history(&store, "test", &rev, &c),
            Resolution::Unresolved
        );
    }

    #[test]
    fn history_unresolved_on_whole_document_comment() {
        let mut store = MemoryStore::new();
        seed_task(&mut store, "body.");
        store.commit().unwrap();
        let sha = store.head_sha().unwrap();

        let rev = review(task_target(), Some(&sha));
        let c = comment(None, None);
        assert_eq!(
            resolve_against_history(&store, "test", &rev, &c),
            Resolution::Unresolved
        );
    }

    #[test]
    fn history_resolves_comment_doc_phase_scope() {
        let mut store = MemoryStore::new();
        seed_roadmap(&mut store, "roadmap body without the quote.");
        seed_phase(&mut store, "phase body: the quoted span here.");
        store.commit().unwrap();
        let sha = store.head_sha().unwrap();

        let rev = review(
            ReviewTarget::Roadmap {
                roadmap: "alpha".to_string(),
            },
            Some(&sha),
        );
        let c = comment(
            Some(tq("quoted span", "the ", " here")),
            Some(CommentDoc {
                kind: CommentDocKind::Phase,
                stem: "phase-1-one".to_string(),
            }),
        );
        match resolve_against_history(&store, "test", &rev, &c) {
            Resolution::Original { range, drifted } => {
                assert!(!drifted);
                let body = crate::io::load_phase_at(&store, "test", "alpha", "phase-1-one", &sha)
                    .unwrap()
                    .body;
                assert_eq!(&body[range], "quoted span");
            }
            other => panic!("expected Original, got {other:?}"),
        }
    }

    #[test]
    fn history_current_fallback_multibyte() {
        let mut store = MemoryStore::new();
        seed_task(&mut store, "Café notes: the résumé draft — naïve.");
        store.commit().unwrap();

        let rev = review(task_target(), Some("mem-nope"));
        let c = comment(Some(tq("résumé", "the ", " draft")), None);
        match resolve_against_history(&store, "test", &rev, &c) {
            Resolution::Current { range } => {
                let cur = crate::io::load_task(&store, "test", "fix").unwrap().body;
                assert!(cur.is_char_boundary(range.start));
                assert!(cur.is_char_boundary(range.end));
                assert_eq!(&cur[range], "résumé");
            }
            other => panic!("expected Current, got {other:?}"),
        }
    }
}

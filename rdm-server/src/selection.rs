//! Selection→anchor derivation for the web select-to-anchor flow.
//!
//! Client-side JavaScript maps a DOM selection over an annotated body
//! (see [`crate::markdown::render_markdown_annotated`]) to a **source**
//! byte range and ships it, together with the visible text the user
//! selected, to the server. This module owns the server side of that
//! contract: it treats the client's offsets purely as a *hint* and
//! re-derives the anchor from the authoritative body, refusing to store
//! anything it cannot prove correct.
//!
//! # The never-wrong-anchor guarantee
//!
//! [`anchor_from_selection`](crate::selection::anchor_from_selection)
//! runs three guards, in order; failing any one
//! yields [`SelectionOutcome::NoAnchor`](crate::selection::SelectionOutcome::NoAnchor)
//! (the caller degrades to a general
//! comment):
//!
//! 1. **Bounds** — the range is non-empty, in bounds, on `char`
//!    boundaries, and not whitespace-only.
//! 2. **Round-trip through core** — the source slice is turned into a
//!    text-quote anchor via [`rdm_core::anchor::derive_text_quote`]
//!    (occurrence-disambiguated when the quote repeats), and
//!    [`rdm_core::anchor::resolve`] must re-locate that anchor at
//!    *exactly* the claimed byte range.
//! 3. **Rendered cross-check** — the visible text of the source range
//!    (reconstructed from [`crate::markdown::source_runs`]) must equal the
//!    text the client says the user selected, compared
//!    whitespace-insensitively. This is the guard that proves the offsets
//!    actually correspond to what the user highlighted: a client that
//!    mapped the selection to the wrong source position produces a slice
//!    whose visible text does not match.

use rdm_core::anchor::{derive_text_quote, resolve};
use rdm_core::model::Anchor;

use crate::markdown::source_runs;

/// Outcome of validating a client-supplied selection descriptor.
#[derive(Debug, Clone, PartialEq)]
pub enum SelectionOutcome {
    /// The selection maps to a verified text-quote anchor; safe to store.
    Anchored(Anchor),
    /// The selection could not be verified — the caller must degrade to a
    /// general (un-anchored) comment. A wrong anchor is never returned.
    NoAnchor,
}

/// Derives a verified [`Anchor::TextQuote`] for the source byte range
/// `sel_start..sel_end` of `body`, cross-checked against `rendered_text`
/// (the visible text the user selected in the rendered page).
///
/// See the [module documentation](self) for the guard ladder. Returns
/// [`SelectionOutcome::NoAnchor`] instead of ever returning an anchor
/// that does not re-resolve to exactly the claimed range.
#[must_use]
pub fn anchor_from_selection(
    body: &str,
    sel_start: usize,
    sel_end: usize,
    rendered_text: &str,
) -> SelectionOutcome {
    // Guard 1: bounds and boundaries.
    if sel_start >= sel_end
        || sel_end > body.len()
        || !body.is_char_boundary(sel_start)
        || !body.is_char_boundary(sel_end)
    {
        return SelectionOutcome::NoAnchor;
    }
    let quote = &body[sel_start..sel_end];
    if quote.trim().is_empty() {
        return SelectionOutcome::NoAnchor;
    }

    // Guard 2: derive through core and require an exact-range round-trip.
    // `match_indices` enumerates occurrences exactly the way
    // `derive_text_quote` counts them (non-overlapping, from the start),
    // so the position of our match *is* the 0-based occurrence index. A
    // self-overlapping quote whose match at `sel_start` is not on that
    // enumeration fails here and degrades.
    let Some(occurrence) = body
        .match_indices(quote)
        .position(|(at, _)| at == sel_start)
    else {
        return SelectionOutcome::NoAnchor;
    };
    let Ok(anchor) = derive_text_quote(body, quote, Some(occurrence + 1), None) else {
        return SelectionOutcome::NoAnchor;
    };
    if resolve(body, &anchor) != Some(sel_start..sel_end) {
        return SelectionOutcome::NoAnchor;
    }

    // Guard 3: the visible text of the range must match what the user saw.
    let Some(expected) = visible_text_of_range(body, sel_start, sel_end) else {
        return SelectionOutcome::NoAnchor;
    };
    let expected_cmp = strip_whitespace(&expected);
    if expected_cmp.is_empty() || expected_cmp != strip_whitespace(rendered_text) {
        return SelectionOutcome::NoAnchor;
    }

    SelectionOutcome::Anchored(anchor)
}

/// Reconstructs the visible (rendered) text of the source byte range
/// `start..end`, by intersecting it with the body's text runs.
///
/// "Clean" runs (content byte-identical to their source slice) contribute
/// the sliced overlap; "opaque" runs (inline code with backticks, decoded
/// entities) contribute their full content, and a *partial* overlap with
/// an opaque run is unmappable (`None`) — there is no byte-accurate way
/// to clip inside it. A range touching no run at all (markers-only) is
/// also `None`.
fn visible_text_of_range(body: &str, start: usize, end: usize) -> Option<String> {
    let mut out = String::new();
    let mut any = false;
    for run in source_runs(body) {
        let s = run.range.start.max(start);
        let e = run.range.end.min(end);
        if s >= e {
            continue;
        }
        any = true;
        if run.content.as_bytes() == &body.as_bytes()[run.range.clone()] {
            // `s`/`e` are max/min of char boundaries, hence boundaries.
            out.push_str(&body[s..e]);
        } else {
            if s != run.range.start || e != run.range.end {
                return None;
            }
            out.push_str(&run.content);
        }
    }
    any.then_some(out)
}

/// Removes every Unicode whitespace character, making the guard-3
/// comparison insensitive to soft breaks, block gaps, and newline-vs-space
/// differences between DOM text and source text.
fn strip_whitespace(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Byte range of `needle` in `haystack` (first occurrence).
    fn range_of(haystack: &str, needle: &str) -> (usize, usize) {
        let start = haystack.find(needle).expect("needle present");
        (start, start + needle.len())
    }

    /// Asserts the selection anchors AND the derived anchor re-resolves to
    /// exactly the claimed byte range.
    fn assert_roundtrip(body: &str, start: usize, end: usize, rendered: &str) -> Anchor {
        match anchor_from_selection(body, start, end, rendered) {
            SelectionOutcome::Anchored(anchor) => {
                assert_eq!(
                    resolve(body, &anchor),
                    Some(start..end),
                    "anchor must re-resolve to the exact selected range"
                );
                assert!(
                    matches!(&anchor, Anchor::TextQuote { quote, .. } if quote == &body[start..end]),
                    "quote must be the source slice"
                );
                anchor
            }
            SelectionOutcome::NoAnchor => {
                panic!("expected Anchored for {:?} in {body:?}", &body[start..end])
            }
        }
    }

    // -- round-trip matrix --

    #[test]
    fn roundtrip_plain_text() {
        let body = "The quick brown fox jumps.\n";
        let (s, e) = range_of(body, "quick brown");
        assert_roundtrip(body, s, e, "quick brown");
    }

    #[test]
    fn roundtrip_across_bold_boundary() {
        let body = "start **bold** end\n";
        let (s, e) = range_of(body, "start **bold** end");
        assert_roundtrip(body, s, e, "start bold end");
    }

    #[test]
    fn roundtrip_partial_bold_crossing() {
        // Selection starts in plain text and ends inside the bold span:
        // the slice carries an unmatched `**`, but the run reconstruction
        // still yields the visible text.
        let body = "start **bold** end\n";
        let s = body.find("start").unwrap();
        let e = body.find("bo").unwrap() + 2; // "start **bo"
        assert_roundtrip(body, s, e, "start bo");
    }

    #[test]
    fn roundtrip_italic() {
        let body = "an *emphatic* word\n";
        let (s, e) = range_of(body, "an *emphatic* word");
        assert_roundtrip(body, s, e, "an emphatic word");
    }

    #[test]
    fn roundtrip_inline_code_opaque_snap() {
        // The client snaps a selection inside inline code to the whole
        // backtick-inclusive run; the rendered text is the code content.
        let body = "use `foo()` here\n";
        let (s, e) = range_of(body, "`foo()`");
        assert_roundtrip(body, s, e, "foo()");
    }

    #[test]
    fn roundtrip_across_inline_code() {
        let body = "use `foo()` here\n";
        let (s, e) = range_of(body, "use `foo()` here");
        assert_roundtrip(body, s, e, "use foo() here");
    }

    #[test]
    fn roundtrip_entity_bearing_run() {
        // `&amp;` decodes to `&`: an opaque run the client snaps whole.
        let body = "AT&amp;T works fine\n";
        let (s, e) = range_of(body, "AT&amp;T");
        assert_roundtrip(body, s, e, "AT&T");
    }

    #[test]
    fn roundtrip_across_soft_break() {
        let body = "line one\nline two\n";
        let (s, e) = range_of(body, "one\nline");
        // The client concatenates clipped run text: "one" + "line".
        assert_roundtrip(body, s, e, "oneline");
    }

    #[test]
    fn roundtrip_across_hard_break() {
        let body = "line one  \nline two\n";
        let s = body.find("one").unwrap();
        let e = body.find("two").unwrap() + 3;
        assert_roundtrip(body, s, e, "one line two");
    }

    #[test]
    fn roundtrip_list_item() {
        let body = "- first item\n- second item\n";
        let (s, e) = range_of(body, "second item");
        assert_roundtrip(body, s, e, "second item");
    }

    #[test]
    fn roundtrip_table_cell() {
        let body = "| a | b |\n|---|---|\n| cell one | cell two |\n";
        let (s, e) = range_of(body, "cell two");
        assert_roundtrip(body, s, e, "cell two");
    }

    #[test]
    fn roundtrip_heading() {
        let body = "# Big Heading\n\nBody text.\n";
        let (s, e) = range_of(body, "Big Heading");
        assert_roundtrip(body, s, e, "Big Heading");
    }

    #[test]
    fn roundtrip_multibyte_utf8() {
        let body = "Café **résumé** naïve — 日本語 text\n";
        let (s, e) = range_of(body, "Café **résumé** naïve");
        assert_roundtrip(body, s, e, "Café résumé naïve");
        let (s, e) = range_of(body, "日本語");
        assert_roundtrip(body, s, e, "日本語");
    }

    #[test]
    fn roundtrip_repeated_quote_uses_occurrence() {
        let body = "alpha beta gamma alpha beta delta\n";
        // The SECOND "beta".
        let s = body.rfind("beta").unwrap();
        let e = s + "beta".len();
        let anchor = assert_roundtrip(body, s, e, "beta");
        // The derived context must disambiguate towards the second one.
        match anchor {
            Anchor::TextQuote { prefix, .. } => {
                assert!(prefix.contains("gamma alpha"), "prefix: {prefix:?}")
            }
            other => panic!("unexpected anchor: {other:?}"),
        }
    }

    // -- negative guards --

    #[test]
    fn noanchor_on_out_of_bounds() {
        let body = "short body\n";
        assert_eq!(
            anchor_from_selection(body, 0, 999, "short body"),
            SelectionOutcome::NoAnchor
        );
    }

    #[test]
    fn noanchor_on_reversed_or_empty_range() {
        let body = "some body text\n";
        assert_eq!(
            anchor_from_selection(body, 5, 5, ""),
            SelectionOutcome::NoAnchor
        );
        assert_eq!(
            anchor_from_selection(body, 8, 4, "body"),
            SelectionOutcome::NoAnchor
        );
    }

    #[test]
    fn noanchor_on_non_char_boundary() {
        let body = "héllo world\n";
        // é is bytes 1..3; offset 2 lands mid-char.
        assert_eq!(
            anchor_from_selection(body, 2, 7, "llo w"),
            SelectionOutcome::NoAnchor
        );
    }

    #[test]
    fn noanchor_on_whitespace_only_selection() {
        let body = "a   b\n";
        assert_eq!(
            anchor_from_selection(body, 1, 4, "   "),
            SelectionOutcome::NoAnchor
        );
    }

    #[test]
    fn noanchor_when_rendered_text_mismatches_slice() {
        // Valid offsets, but the client claims the user selected different
        // text — the wrong-mapping case the cross-check exists to catch.
        let body = "first sentence here. second sentence there.\n";
        let (s, e) = range_of(body, "first sentence");
        assert_eq!(
            anchor_from_selection(body, s, e, "second sentence"),
            SelectionOutcome::NoAnchor
        );
    }

    #[test]
    fn noanchor_on_markers_only_selection() {
        // A range covering only formatting markers touches no text run.
        let body = "start **bold** end\n";
        let s = body.find("**").unwrap();
        assert_eq!(
            anchor_from_selection(body, s, s + 2, ""),
            SelectionOutcome::NoAnchor
        );
    }

    #[test]
    fn roundtrip_partial_inline_code_selection_snaps_whole_run() {
        // The user visually selects "use `fo" — partway into the code
        // span. The client snaps the opaque run's endpoint to the whole
        // run (offsets) AND contributes the run's full text content
        // (rendered_text), so the pair stays consistent and anchors.
        let body = "use `foo()` here\n";
        let end = body.find('`').unwrap() + "`foo()`".len();
        assert_roundtrip(body, 0, end, "use foo()");
    }

    #[test]
    fn selfoverlap_aligned_occurrences_anchor_with_correct_occurrence() {
        // "aa" in "aaaa": the non-overlapping enumeration has exactly two
        // occurrences, at 0 and 2 — both anchor, each resolving back to
        // its own span.
        let body = "aaaa";
        assert_roundtrip(body, 0, 2, "aa");
        assert_roundtrip(body, 2, 4, "aa");
    }

    #[test]
    fn noanchor_on_selfoverlap_misaligned_occurrence() {
        // The middle "aa" (1..3) is a real substring match but not on the
        // non-overlapping occurrence enumeration `derive_text_quote`
        // counts by — it cannot be expressed as an occurrence index, so
        // it degrades rather than anchoring the wrong span.
        let body = "aaaa";
        assert_eq!(
            anchor_from_selection(body, 1, 3, "aa"),
            SelectionOutcome::NoAnchor
        );
    }

    #[test]
    fn noanchor_on_partial_opaque_overlap() {
        // A range clipping into an entity run cannot be mapped.
        let body = "AT&amp;T works\n";
        let s = body.find("&amp;").unwrap();
        // Covers only "&am" — a strict subset of the opaque run.
        assert_eq!(
            anchor_from_selection(body, s, s + 3, "&"),
            SelectionOutcome::NoAnchor
        );
    }

    // -- CLI-vs-web derivation split (history-capable store) --

    /// Pins the arbitrated capture semantics ahead of a git-backed server
    /// store: web selections are made against the RENDERED CURRENT body,
    /// so the web path derives against
    /// `rdm_core::anchor::current_body_for_comment` and anchors text
    /// added after the review started — while the CLI-semantics path
    /// (quotes typed blind, pinned to `created_commit` via
    /// `rdm_core::anchor::body_for_comment`) rejects that same text.
    #[test]
    fn web_path_anchors_current_body_where_cli_pinned_path_rejects() {
        use rdm_core::document::Document;
        use rdm_core::model::{Priority, Review, ReviewState, ReviewTarget, Task, TaskStatus};
        use rdm_core::store::{MemoryStore, Store, VersionedStore};

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
            rdm_core::io::write_task(store, "test", "fix", &doc).unwrap();
        }

        let mut store = MemoryStore::new();
        seed_task(&mut store, "Original paragraph.");
        store.commit().unwrap();
        let created_sha = store.head_sha().unwrap();
        // The document gains a sentence AFTER the review started.
        seed_task(&mut store, "Original paragraph. Added since review.");
        store.commit().unwrap();

        let review = Review {
            id: "2026-07-01-1200-abcd".to_string(),
            author: "reviewer".to_string(),
            target: ReviewTarget::Task {
                slug: "fix".to_string(),
            },
            state: ReviewState::Draft,
            verdict: None,
            created: chrono::Utc::now(),
            submitted: None,
            created_commit: Some(created_sha.clone()),
            comments: Vec::new(),
        };

        // WEB path: the browser selected "Added since review" in the
        // rendered current body; the anchor derives and round-trips.
        let current =
            rdm_core::anchor::current_body_for_comment(&store, "test", &review, None).unwrap();
        let (s, e) = range_of(&current, "Added since review");
        match anchor_from_selection(&current, s, e, "Added since review") {
            SelectionOutcome::Anchored(anchor) => {
                assert_eq!(resolve(&current, &anchor), Some(s..e));
            }
            SelectionOutcome::NoAnchor => panic!("web path must anchor against the current body"),
        }

        // CLI-semantics path: the created_commit-pinned body predates the
        // text, so a blind quote for it is rejected outright.
        let (pinned, at) =
            rdm_core::anchor::body_for_comment(&store, "test", &review, None).unwrap();
        assert_eq!(at.as_deref(), Some(created_sha.as_str()));
        assert!(!pinned.contains("Added since review"));
        let err =
            rdm_core::anchor::derive_text_quote(&pinned, "Added since review", None, at.as_deref())
                .unwrap_err();
        assert!(matches!(err, rdm_core::error::Error::QuoteNotFound { .. }));
    }
}

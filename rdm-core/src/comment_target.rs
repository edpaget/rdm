//! A review comment's **coarsest true target**: the most specific thing it
//! can honestly link to.
//!
//! A comment is not always anchored. It may be whole-document on purpose, or
//! its anchor may have been lost when it was written (the stored
//! [`AnchorState`] says why), or it may predate the field entirely. Readers
//! that want to attach a comment to code — reverse lookup, per-comment links
//! in review JSON — need one answer to "what does this comment point at?",
//! and that answer lives here so they share it rather than re-deriving it
//! from [`crate::change`] internals.
//!
//! The rungs, for a `change/` review with head `H`:
//!
//! 1. an [`Anchor::FileQuote`] whose path is a file at `H` → the line
//!    **range** (`rdm:src/<path>@H#L<a>-L<b>`);
//! 2. an anchorless comment whose stored `intended_path` is a file at `H` →
//!    the whole **file** (`rdm:src/<path>@H`);
//! 3. otherwise → the **commit** (`rdm:src@H`).
//!
//! A plan-document review (roadmap, phase, task, plan) always targets the
//! **plan item** itself: it has no source commit to fall back to.
//!
//! A target is **never inferred from the change's diff** — nothing here
//! reads [`SourceRepo::unified_diff`]. When the source repository cannot say
//! whether a path exists (the checkout lacks the head, or git fails), the rung
//! is chosen **fail-open** and marked [`CommentTarget::unverified`]: the range
//! was verified when it was written, and the head is immutable.

use crate::change::{ineligibility_note, permalink_for};
use crate::link::Link;
use crate::model::{Anchor, AnchorState, CommentDocKind, Review, ReviewComment, ReviewTarget};
use crate::source::{SourceObjectKind, SourceObjectLookup, SourceRepo};

/// How specific a [`CommentTarget`] is, from finest to coarsest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetLevel {
    /// A line range of a source file at the reviewed head.
    Range,
    /// A whole source file at the reviewed head.
    File,
    /// The reviewed head commit itself.
    Commit,
    /// The plan item a plan-document review is about.
    PlanItem,
}

impl TargetLevel {
    /// The level's JSON/text spelling: `range`, `file`, `commit` or
    /// `plan-item`.
    ///
    /// ```
    /// use rdm_core::comment_target::TargetLevel;
    ///
    /// assert_eq!(TargetLevel::PlanItem.as_str(), "plan-item");
    /// ```
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            TargetLevel::Range => "range",
            TargetLevel::File => "file",
            TargetLevel::Commit => "commit",
            TargetLevel::PlanItem => "plan-item",
        }
    }
}

/// Why a [`CommentTarget`] is coarser than an intact anchor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetReason {
    /// The comment's stored [`AnchorState`]: whole-document on purpose, or a
    /// reason recorded when the comment was written.
    Stored(AnchorState),
    /// Read time: the anchor's path is not a file at the reviewed head (a
    /// directory, a submodule, or absent) — the note
    /// [`crate::change::change_anchor_ineligibility`] reports.
    Ineligible(String),
    /// An anchorless comment with no stored anchor state (a review written
    /// before the field existed).
    NotRecorded,
}

/// A comment's coarsest true target, as returned by [`comment_target`].
#[derive(Debug, Clone, PartialEq)]
pub struct CommentTarget {
    /// The `rdm:` link to the target.
    pub link: Link,
    /// How specific the target is.
    pub level: TargetLevel,
    /// Why the target is not an intact anchor. `None` only for an intact
    /// [`TargetLevel::Range`] or an anchored [`TargetLevel::PlanItem`]
    /// comment.
    pub reason: Option<TargetReason>,
    /// `true` only when the rung was chosen fail-open, because the source
    /// repository could not say whether the path exists at the head.
    pub unverified: bool,
}

/// Whether a path exists as a file at the reviewed head — the one question
/// the change rungs ask of a source repository.
///
/// A tri-state so a caller with no source repository at all can answer
/// [`Existence::Unknown`] without a second copy of the rungs.
enum Existence {
    /// A regular file.
    Exists,
    /// Not a file: a directory, a submodule, or absent. Carries the
    /// ineligibility note for that case.
    NotAFile(String),
    /// Not knowable here (the head is missing, or the repository failed).
    Unknown,
}

/// Derives `comment`'s coarsest true target within `review` (see the
/// [module docs](self) for the rungs).
///
/// `source` answers only existence questions — which object kind a path is
/// at the reviewed head — and is not consulted at all for a plan-document
/// review, or for a change review whose stored identity is malformed (which
/// is treated like an unanswerable repository: fail-open, unverified).
///
/// # Panics
///
/// Never panics.
///
/// # Examples
///
/// A comment whose anchor was refused, stored with its reason and the file
/// it meant, targets that whole file when it exists at the head:
///
/// ```
/// use rdm_core::comment_target::{comment_target, TargetLevel, TargetReason};
/// use rdm_core::model::{AnchorState, Review, ReviewComment, ReviewCommentStatus, ReviewState, ReviewTarget};
/// use rdm_core::source::MemorySourceRepo;
///
/// let head = "a".repeat(40);
/// let comment = ReviewComment {
///     id: 1,
///     doc: None,
///     status: ReviewCommentStatus::Open,
///     applied_commit: None,
///     anchor: None,
///     anchor_state: Some(AnchorState::OutsideHunk),
///     intended_path: Some("src/lib.rs".to_string()),
///     body: "a finding".to_string(),
///     reply: None,
/// };
/// let review = Review {
///     id: "r1".to_string(),
///     author: "tester".to_string(),
///     target: ReviewTarget::Change { head: head.clone(), base: None },
///     state: ReviewState::Draft,
///     verdict: None,
///     created: chrono::Utc::now(),
///     submitted: None,
///     created_commit: None,
///     implements: None,
///     change_branch: None,
///     comments: vec![comment.clone()],
/// };
/// let source = MemorySourceRepo::new().with_file(&head, "src/lib.rs", "fn main() {}\n");
///
/// let target = comment_target(&source, &review, &comment);
/// assert_eq!(target.level, TargetLevel::File);
/// assert_eq!(target.link.to_string(), format!("rdm:src/src/lib.rs@{head}"));
/// assert_eq!(target.reason, Some(TargetReason::Stored(AnchorState::OutsideHunk)));
/// assert!(!target.unverified);
/// ```
#[must_use]
pub fn comment_target(
    source: &impl SourceRepo,
    review: &Review,
    comment: &ReviewComment,
) -> CommentTarget {
    let ReviewTarget::Change { head, .. } = &review.target else {
        return plan_item_target(review, comment);
    };
    if review.target.validate_stored_identity().is_err() {
        return change_target(head, comment, |_| Existence::Unknown);
    }
    change_target(head, comment, |path| {
        let lookup = source.object_kind_at(head, path);
        match &lookup {
            Ok(SourceObjectLookup::Kind(SourceObjectKind::Blob)) => Existence::Exists,
            Ok(SourceObjectLookup::RevMissing) | Err(_) => Existence::Unknown,
            Ok(SourceObjectLookup::PathMissing | SourceObjectLookup::Kind(_)) => {
                Existence::NotAFile(ineligibility_note(path, head, &lookup).unwrap_or_default())
            }
        }
    })
}

/// The stored-reason half of a comment's [`TargetReason`].
fn stored_reason(comment: &ReviewComment) -> TargetReason {
    comment
        .anchor_state
        .clone()
        .map_or(TargetReason::NotRecorded, TargetReason::Stored)
}

/// The rung decision for a `change/` review at `head`, given only an
/// existence answer per path — the one place the change rungs live.
fn change_target(
    head: &str,
    comment: &ReviewComment,
    exists: impl Fn(&str) -> Existence,
) -> CommentTarget {
    let commit = |reason: TargetReason| CommentTarget {
        link: Link::Commit {
            rev: head.to_string(),
        },
        level: TargetLevel::Commit,
        reason: Some(reason),
        unverified: false,
    };
    if let Some(anchor) = &comment.anchor
        && let Anchor::FileQuote { path, .. } = anchor
        && let Some(permalink) = permalink_for(head, anchor)
    {
        let range = |unverified| CommentTarget {
            link: permalink.clone(),
            level: TargetLevel::Range,
            reason: None,
            unverified,
        };
        return match exists(path) {
            Existence::Exists => range(false),
            Existence::Unknown => range(true),
            Existence::NotAFile(note) => commit(TargetReason::Ineligible(note)),
        };
    }
    let reason = stored_reason(comment);
    let Some(path) = comment.intended_path.as_deref() else {
        return commit(reason);
    };
    let file = |unverified| CommentTarget {
        link: Link::Code {
            path: path.to_string(),
            rev: Some(head.to_string()),
            lines: None,
        },
        level: TargetLevel::File,
        reason: Some(reason.clone()),
        unverified,
    };
    match exists(path) {
        Existence::Exists => file(false),
        Existence::Unknown => file(true),
        Existence::NotAFile(_) => commit(reason),
    }
}

/// The plan-item rung: the document the comment is about — a roadmap
/// review's phase when the comment is scoped to one, otherwise the review's
/// own target.
fn plan_item_target(review: &Review, comment: &ReviewComment) -> CommentTarget {
    let item = match (&review.target, &comment.doc) {
        (ReviewTarget::Roadmap { roadmap }, Some(doc)) if doc.kind == CommentDocKind::Phase => {
            ReviewTarget::Phase {
                roadmap: roadmap.clone(),
                stem: doc.stem.clone(),
            }
        }
        (target, _) => target.clone(),
    };
    CommentTarget {
        link: Link::Item(item),
        level: TargetLevel::PlanItem,
        reason: if comment.anchor.is_some() {
            None
        } else {
            Some(stored_reason(comment))
        },
        unverified: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CommentDoc, ReviewCommentStatus, ReviewState};
    use crate::source::MemorySourceRepo;

    const HEAD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn review(target: ReviewTarget) -> Review {
        Review {
            id: "r1".to_string(),
            author: "tester".to_string(),
            target,
            state: ReviewState::Draft,
            verdict: None,
            created: chrono::Utc::now(),
            submitted: None,
            created_commit: None,
            implements: None,
            change_branch: None,
            comments: vec![],
        }
    }

    fn change_review() -> Review {
        review(ReviewTarget::Change {
            head: HEAD.to_string(),
            base: Some("b".repeat(40)),
        })
    }

    fn comment(
        anchor: Option<Anchor>,
        anchor_state: Option<AnchorState>,
        intended_path: Option<&str>,
    ) -> ReviewComment {
        ReviewComment {
            id: 1,
            doc: None,
            status: ReviewCommentStatus::Open,
            applied_commit: None,
            anchor,
            anchor_state,
            intended_path: intended_path.map(str::to_string),
            body: "b".to_string(),
            reply: None,
        }
    }

    fn file_quote(path: &str) -> Anchor {
        Anchor::FileQuote {
            path: path.to_string(),
            quote: "fn main() {}".to_string(),
            occurrence: 1,
            start_line: 3,
            end_line: 4,
        }
    }

    fn seeded() -> MemorySourceRepo {
        MemorySourceRepo::new()
            .with_rev(HEAD, HEAD)
            .with_file(HEAD, "src/lib.rs", "fn main() {}\n")
    }

    fn head_commit() -> Link {
        Link::Commit {
            rev: HEAD.to_string(),
        }
    }

    #[test]
    fn target_levels_spell_as_kebab_case() {
        let spelled: Vec<&str> = [
            TargetLevel::Range,
            TargetLevel::File,
            TargetLevel::Commit,
            TargetLevel::PlanItem,
        ]
        .into_iter()
        .map(TargetLevel::as_str)
        .collect();
        assert_eq!(spelled, ["range", "file", "commit", "plan-item"]);
    }

    #[test]
    fn a_file_quote_on_a_file_at_head_targets_its_range() {
        let c = comment(Some(file_quote("src/lib.rs")), None, None);
        let t = comment_target(&seeded(), &change_review(), &c);
        assert_eq!(t.level, TargetLevel::Range);
        assert_eq!(
            t.link.to_string(),
            format!("rdm:src/src/lib.rs@{HEAD}#L3-L4")
        );
        assert_eq!(t.reason, None);
        assert!(!t.unverified);
    }

    #[test]
    fn a_failed_anchor_whose_file_exists_targets_the_whole_file() {
        let c = comment(None, Some(AnchorState::OutsideHunk), Some("src/lib.rs"));
        let t = comment_target(&seeded(), &change_review(), &c);
        assert_eq!(t.level, TargetLevel::File);
        assert_eq!(
            t.link,
            Link::Code {
                path: "src/lib.rs".to_string(),
                rev: Some(HEAD.to_string()),
                lines: None,
            }
        );
        assert_eq!(
            t.reason,
            Some(TargetReason::Stored(AnchorState::OutsideHunk))
        );
        assert!(!t.unverified);
    }

    #[test]
    fn a_failed_anchor_whose_file_is_absent_at_head_targets_the_commit() {
        let c = comment(None, Some(AnchorState::OutsideHunk), Some("src/gone.rs"));
        let t = comment_target(&seeded(), &change_review(), &c);
        assert_eq!(t.level, TargetLevel::Commit);
        assert_eq!(t.link, head_commit());
        assert_eq!(
            t.reason,
            Some(TargetReason::Stored(AnchorState::OutsideHunk))
        );
        assert!(!t.unverified);
    }

    #[test]
    fn an_anchorless_comment_without_an_intended_path_targets_the_commit() {
        let c = comment(None, Some(AnchorState::QuoteNotFound), None);
        let t = comment_target(&seeded(), &change_review(), &c);
        assert_eq!(t.level, TargetLevel::Commit);
        assert_eq!(t.link, head_commit());
        assert_eq!(
            t.reason,
            Some(TargetReason::Stored(AnchorState::QuoteNotFound))
        );
    }

    #[test]
    fn a_whole_document_comment_targets_the_commit_with_its_stored_state() {
        let c = comment(None, Some(AnchorState::WholeDocument), None);
        let t = comment_target(&seeded(), &change_review(), &c);
        assert_eq!(t.level, TargetLevel::Commit);
        assert_eq!(
            t.reason,
            Some(TargetReason::Stored(AnchorState::WholeDocument))
        );
    }

    #[test]
    fn a_legacy_anchorless_comment_is_not_recorded() {
        let c = comment(None, None, None);
        let t = comment_target(&seeded(), &change_review(), &c);
        assert_eq!(t.level, TargetLevel::Commit);
        assert_eq!(t.reason, Some(TargetReason::NotRecorded));
    }

    #[test]
    fn a_file_quote_naming_a_directory_targets_the_commit_as_ineligible() {
        let source = seeded().with_object_kind(HEAD, "src", SourceObjectKind::Tree);
        let c = comment(Some(file_quote("src")), None, None);
        let t = comment_target(&source, &change_review(), &c);
        assert_eq!(t.level, TargetLevel::Commit);
        assert_eq!(t.link, head_commit());
        assert_eq!(
            t.reason,
            Some(TargetReason::Ineligible(
                "'src' is a directory, not a file".to_string()
            ))
        );
    }

    #[test]
    fn an_intended_path_with_the_head_missing_is_an_unverified_file() {
        let source = seeded().with_missing_rev(HEAD);
        let c = comment(None, Some(AnchorState::OutsideHunk), Some("src/lib.rs"));
        let t = comment_target(&source, &change_review(), &c);
        assert_eq!(t.level, TargetLevel::File);
        assert!(t.unverified);
    }

    #[test]
    fn a_file_quote_with_the_head_missing_is_an_unverified_range() {
        let source = seeded().with_missing_rev(HEAD);
        let c = comment(Some(file_quote("src/lib.rs")), None, None);
        let t = comment_target(&source, &change_review(), &c);
        assert_eq!(t.level, TargetLevel::Range);
        assert!(t.unverified);
    }

    #[test]
    fn a_malformed_stored_identity_skips_the_source_and_fails_open() {
        let bad = review(ReviewTarget::Change {
            head: "not-a-sha".to_string(),
            base: None,
        });
        let c = comment(None, Some(AnchorState::OutsideHunk), Some("src/lib.rs"));
        let t = comment_target(&seeded(), &bad, &c);
        assert_eq!(t.level, TargetLevel::File);
        assert!(t.unverified);
    }

    #[test]
    fn a_task_review_targets_the_task() {
        let task = review(ReviewTarget::Task {
            slug: "fix-login".to_string(),
        });
        let c = comment(None, Some(AnchorState::WholeDocument), None);
        let t = comment_target(&MemorySourceRepo::new(), &task, &c);
        assert_eq!(t.level, TargetLevel::PlanItem);
        assert_eq!(t.link.to_string(), "rdm:task/fix-login");
        assert_eq!(
            t.reason,
            Some(TargetReason::Stored(AnchorState::WholeDocument))
        );
        let anchored = comment(
            Some(Anchor::TextQuote {
                quote: "q".to_string(),
                prefix: String::new(),
                suffix: String::new(),
            }),
            None,
            None,
        );
        assert_eq!(
            comment_target(&MemorySourceRepo::new(), &task, &anchored).reason,
            None
        );
    }

    #[test]
    fn a_roadmap_review_comment_scoped_to_a_phase_targets_the_phase() {
        let roadmap = review(ReviewTarget::Roadmap {
            roadmap: "auth".to_string(),
        });
        let mut c = comment(None, None, None);
        c.doc = Some(CommentDoc {
            kind: CommentDocKind::Phase,
            stem: "phase-1-design".to_string(),
        });
        let t = comment_target(&MemorySourceRepo::new(), &roadmap, &c);
        assert_eq!(t.level, TargetLevel::PlanItem);
        assert_eq!(t.link.to_string(), "rdm:phase/auth/phase-1-design");
        assert_eq!(t.reason, Some(TargetReason::NotRecorded));
    }
}

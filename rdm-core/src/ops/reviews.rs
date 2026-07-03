//! Review-document operations: create, comment on, submit, transition,
//! list, and delete stored reviews, enforcing the review lifecycle
//! (`draft` → `submitted` → `addressed` | `dismissed`).
//!
//! Not to be confused with [`crate::ops::review`], which enumerates plan
//! items awaiting review (the needs-review queue). This module operates on
//! the [`Review`] documents stored under a project's `reviews/` directory.

use std::sync::atomic::{AtomicU32, Ordering};

use chrono::{DateTime, Utc};

use crate::document::Document;
use crate::error::{Error, Result};
use crate::model::{
    Anchor, CommentDoc, CommentDocKind, Review, ReviewComment, ReviewCommentStatus, ReviewState,
    ReviewTarget, ReviewTargetKind, Verdict,
};
use crate::store::{DirEntryKind, Store, VersionedStore};

/// Parses a review target reference into a [`ReviewTarget`].
///
/// Accepted forms:
///
/// - `roadmap/<slug>`
/// - `phase/<roadmap-slug>/<stem-or-number>` — the phase may be named by its
///   file stem or its number, resolved the same way phase identifiers are
///   everywhere else (via
///   [`resolve_phase_stem`](crate::ops::phase::resolve_phase_stem))
/// - `task/<slug>`
///
/// Existence of the target itself is *not* checked here — that stays with
/// [`create_review`] — but resolving a phase *number* requires listing the
/// roadmap's phases, so an unknown roadmap or number surfaces from that
/// lookup.
///
/// # Errors
///
/// Returns [`Error::InvalidReviewTargetRef`] when `reference` matches none
/// of the accepted forms, or whatever
/// [`resolve_phase_stem`](crate::ops::phase::resolve_phase_stem) returns for
/// an unresolvable phase number.
pub fn parse_review_target_ref(
    store: &impl Store,
    project: &str,
    reference: &str,
) -> Result<ReviewTarget> {
    let invalid = || Error::InvalidReviewTargetRef(reference.to_string());
    let (kind, rest) = reference.split_once('/').ok_or_else(invalid)?;
    match kind {
        "roadmap" if !rest.is_empty() && !rest.contains('/') => Ok(ReviewTarget::Roadmap {
            roadmap: rest.to_string(),
        }),
        "task" if !rest.is_empty() && !rest.contains('/') => Ok(ReviewTarget::Task {
            slug: rest.to_string(),
        }),
        "phase" => {
            let (roadmap, phase) = rest.split_once('/').ok_or_else(invalid)?;
            if roadmap.is_empty() || phase.is_empty() || phase.contains('/') {
                return Err(invalid());
            }
            let stem = crate::ops::phase::resolve_phase_stem(store, project, roadmap, phase)?;
            Ok(ReviewTarget::Phase {
                roadmap: roadmap.to_string(),
                stem,
            })
        }
        _ => Err(invalid()),
    }
}

/// Parses a comment document reference (`phase/<stem-or-number>`) into a
/// [`CommentDoc`] scoped to `roadmap`.
///
/// The phase may be named by stem or number, resolved via
/// [`resolve_phase_stem`](crate::ops::phase::resolve_phase_stem). Scope
/// validity (the phase belonging to the reviewed roadmap) is enforced by
/// [`add_comment`]/[`update_comment`], not here.
///
/// # Errors
///
/// Returns [`Error::InvalidCommentDocRef`] when `reference` is not of the
/// form `phase/<stem-or-number>`, or whatever
/// [`resolve_phase_stem`](crate::ops::phase::resolve_phase_stem) returns for
/// an unresolvable phase number.
pub fn parse_comment_doc_ref(
    store: &impl Store,
    project: &str,
    roadmap: &str,
    reference: &str,
) -> Result<CommentDoc> {
    let invalid = || Error::InvalidCommentDocRef(reference.to_string());
    let (kind, phase) = reference.split_once('/').ok_or_else(invalid)?;
    if kind != "phase" || phase.is_empty() || phase.contains('/') {
        return Err(invalid());
    }
    let stem = crate::ops::phase::resolve_phase_stem(store, project, roadmap, phase)?;
    Ok(CommentDoc {
        kind: CommentDocKind::Phase,
        stem,
    })
}

/// Maximum attempts to find a non-colliding review id before giving up.
const MAX_ID_ATTEMPTS: u32 = 20;

/// Process-local counter mixed into review-id suffixes so two ids generated
/// at the same instant within one process still differ.
static ID_COUNTER: AtomicU32 = AtomicU32::new(0);

/// Generates a timestamp-based review id: `YYYY-MM-DD-HHMM-xxxx` where
/// `xxxx` is a 4-hex-digit suffix derived from the timestamp, a
/// process-local counter, and the process id.
fn generate_review_id(now: DateTime<Utc>) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    now.timestamp_nanos_opt()
        .unwrap_or_default()
        .hash(&mut hasher);
    ID_COUNTER.fetch_add(1, Ordering::Relaxed).hash(&mut hasher);
    std::process::id().hash(&mut hasher);
    let suffix = (hasher.finish() & 0xffff) as u16;
    format!("{}-{suffix:04x}", now.format("%Y-%m-%d-%H%M"))
}

/// Returns the first candidate id that does not collide with an existing
/// review file, retrying up to [`MAX_ID_ATTEMPTS`] times.
fn next_available_review_id(
    store: &impl Store,
    project: &str,
    mut candidate: impl FnMut() -> String,
) -> Result<String> {
    for _ in 0..MAX_ID_ATTEMPTS {
        let id = candidate();
        if !store.exists(&crate::paths::review_path(project, &id)) {
            return Ok(id);
        }
    }
    Err(Error::ReviewIdExhausted)
}

/// Validates that a review target currently exists in the plan repo.
fn validate_target_exists(store: &impl Store, project: &str, target: &ReviewTarget) -> Result<()> {
    match target {
        ReviewTarget::Roadmap { roadmap } => {
            if !store.exists(&crate::paths::roadmap_path(project, roadmap)) {
                return Err(Error::ReviewTargetMissing(format!("roadmap '{roadmap}'")));
            }
        }
        ReviewTarget::Phase { roadmap, stem } => {
            if !store.exists(&crate::paths::phase_path(project, roadmap, stem)) {
                return Err(Error::ReviewTargetMissing(format!(
                    "phase '{stem}' in roadmap '{roadmap}'"
                )));
            }
        }
        ReviewTarget::Task { slug } => {
            if !store.exists(&crate::paths::task_path(project, slug)) {
                return Err(Error::ReviewTargetMissing(format!("task '{slug}'")));
            }
        }
    }
    Ok(())
}

/// Validates a comment's `doc` scope against the review's target.
///
/// Only roadmap reviews may scope a comment to a document, and that
/// document must be a phase of the reviewed roadmap.
fn validate_comment_doc(
    store: &impl Store,
    project: &str,
    target: &ReviewTarget,
    doc: &CommentDoc,
) -> Result<()> {
    match target {
        ReviewTarget::Roadmap { roadmap } => match doc.kind {
            CommentDocKind::Phase => {
                if !store.exists(&crate::paths::phase_path(project, roadmap, &doc.stem)) {
                    return Err(Error::CommentDocOutOfScope(format!(
                        "phase '{}' is not part of roadmap '{roadmap}'",
                        doc.stem
                    )));
                }
                Ok(())
            }
        },
        ReviewTarget::Phase { .. } | ReviewTarget::Task { .. } => {
            Err(Error::CommentDocNotApplicable)
        }
    }
}

/// Request describing a new review to create.
///
/// There is no [`Default`] impl: unlike [`CreateTask`](crate::ops::CreateTask),
/// the `target` has no sensible default, so every field is supplied
/// explicitly.
#[derive(Debug, Clone)]
pub struct CreateReview<'a> {
    /// Project the review belongs to.
    pub project: &'a str,
    /// Who is authoring the review (free-form: email, agent name, etc.).
    pub author: &'a str,
    /// The plan item under review. Must exist at creation time.
    pub target: ReviewTarget,
    /// Markdown summary body below the frontmatter. `None` yields an empty
    /// body (fine for a draft; a review with no comments must have a
    /// non-empty summary by submit time).
    pub body: Option<&'a str>,
}

/// Creates a new draft review of an existing plan item.
///
/// The review id is generated from the current UTC time plus a short
/// collision-resistant suffix (e.g. `2026-07-01-1430-a1b2`) and is unique
/// within the project. `created_commit` is stamped from
/// [`VersionedStore::head_sha`] **before** any write, so it always points at
/// the committed version of the target the reviewer saw — including under
/// staging mode, where the review file itself is not yet committed. A store
/// with no history yet (unborn HEAD) yields `created_commit: None`.
///
/// # Errors
///
/// Returns [`Error::ProjectNotFound`] if the project doesn't exist,
/// [`Error::ReviewTargetMissing`] if the target roadmap/phase/task doesn't
/// exist, [`Error::ReviewIdExhausted`] if repeated id-generation attempts all
/// collided, [`Error::Git`] if the store's git state cannot be read (a
/// `head_sha` failure other than an unborn HEAD), [`Error::Io`] if file
/// creation fails, or [`Error::FrontmatterParse`] if frontmatter
/// serialization fails.
pub fn create_review(
    store: &mut impl VersionedStore,
    req: CreateReview<'_>,
) -> Result<Document<Review>> {
    let CreateReview {
        project,
        author,
        target,
        body,
    } = req;
    if !store.exists(&crate::paths::project_md_path(project)) {
        return Err(Error::ProjectNotFound(project.to_string()));
    }
    validate_target_exists(store, project, &target)?;

    let created_commit = match store.head_sha() {
        Ok(sha) => Some(sha),
        Err(Error::HistoryUnavailable) => None,
        Err(e) => return Err(e),
    };
    let now = Utc::now();
    let id = next_available_review_id(store, project, || generate_review_id(now))?;

    let doc = Document {
        frontmatter: Review {
            id: id.clone(),
            author: author.to_string(),
            target,
            state: ReviewState::Draft,
            verdict: None,
            created: now,
            submitted: None,
            created_commit,
            comments: Vec::new(),
        },
        body: body.unwrap_or_default().to_string(),
    };
    crate::io::write_review(store, project, &id, &doc)?;
    Ok(doc)
}

/// Request describing a new comment to add to a draft review.
#[derive(Debug, Clone)]
pub struct AddComment<'a> {
    /// Project the review belongs to.
    pub project: &'a str,
    /// Id of the review to add the comment to.
    pub review_id: &'a str,
    /// The comment text (Markdown).
    pub body: &'a str,
    /// Optional document scope (roadmap reviews only): points the comment
    /// at one of the roadmap's phases.
    pub doc: Option<CommentDoc>,
    /// Optional anchor into the target's body. Anchors are stored verbatim
    /// and are **not** resolved here — an anchor that no longer matches the
    /// target's body (drift) is expected and surfaced later.
    pub anchor: Option<Anchor>,
}

/// Adds a comment to a draft review.
///
/// The comment id is `max(existing ids) + 1` — removing a comment never
/// renumbers the survivors, though the freed *highest* id may be reused by
/// a subsequent add.
///
/// # Errors
///
/// Returns [`Error::ReviewNotFound`] if the review doesn't exist,
/// [`Error::ReviewNotDraft`] if the review has been submitted (comment
/// structure is immutable after submit), [`Error::CommentDocNotApplicable`]
/// if `doc` is set on a phase or task review, [`Error::CommentDocOutOfScope`]
/// if `doc` names a phase outside the reviewed roadmap, [`Error::Io`] on
/// read/write failure, or [`Error::FrontmatterMissing`]/
/// [`Error::FrontmatterParse`] on a malformed review file.
pub fn add_comment(store: &mut impl Store, req: AddComment<'_>) -> Result<Document<Review>> {
    let AddComment {
        project,
        review_id,
        body,
        doc,
        anchor,
    } = req;
    let mut review_doc = crate::io::load_review(store, project, review_id)?;
    if review_doc.frontmatter.state != ReviewState::Draft {
        return Err(Error::ReviewNotDraft(review_id.to_string()));
    }
    if let Some(ref d) = doc {
        validate_comment_doc(store, project, &review_doc.frontmatter.target, d)?;
    }
    let next_id = review_doc
        .frontmatter
        .comments
        .iter()
        .map(|c| c.id)
        .max()
        .map_or(1, |m| m + 1);
    review_doc.frontmatter.comments.push(ReviewComment {
        id: next_id,
        doc,
        status: ReviewCommentStatus::Open,
        applied_commit: None,
        anchor,
        body: body.to_string(),
        reply: None,
    });
    crate::io::write_review(store, project, review_id, &review_doc)?;
    Ok(review_doc)
}

/// How an update should treat a comment's optional anchor.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum AnchorUpdate {
    /// Leave the existing anchor unchanged.
    #[default]
    Keep,
    /// Replace the anchor with this value.
    Set(Anchor),
    /// Clear the anchor (make the comment whole-document).
    Clear,
}

/// How an update should treat a comment's optional document scope.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum DocUpdate {
    /// Leave the existing doc scope unchanged.
    #[default]
    Keep,
    /// Replace the doc scope with this value (validated against the
    /// review's target).
    Set(CommentDoc),
    /// Clear the doc scope (point the comment back at the review's own
    /// target document).
    Clear,
}

/// Request describing an update to a single review comment.
///
/// Structural fields (`body`, `anchor`, `doc`) are mutable only while the
/// review is a draft; resolution fields (`status`, `applied_commit`,
/// `reply`) are mutable only after submission.
#[derive(Debug, Clone, Default)]
pub struct UpdateComment<'a> {
    /// Project the review belongs to.
    pub project: &'a str,
    /// Id of the review containing the comment.
    pub review_id: &'a str,
    /// Id of the comment to update.
    pub comment_id: u32,
    /// New comment text; `None` keeps the existing body. Draft-only.
    pub body: Option<&'a str>,
    /// Anchor update. Draft-only.
    pub anchor: AnchorUpdate,
    /// Document-scope update. Draft-only.
    pub doc: DocUpdate,
    /// New resolution status; `None` keeps the existing one. Submitted-only.
    pub status: Option<ReviewCommentStatus>,
    /// Commit SHA recorded when the comment was addressed; `None` keeps the
    /// existing value. Submitted-only.
    pub applied_commit: Option<&'a str>,
    /// Agent reply note; `None` keeps the existing value. Submitted-only.
    pub reply: Option<&'a str>,
}

/// Updates a single comment within a review, enforcing the lifecycle rules.
///
/// While the review is a **draft**, only the comment's structure (`body`,
/// `anchor`, `doc`) may change. Once **submitted**, only its resolution
/// (`status`, `applied_commit`, `reply`) may change. Mixing both kinds in
/// one call fails on whichever rule the review's current state violates.
///
/// # Errors
///
/// Returns [`Error::ReviewNotFound`] if the review doesn't exist,
/// [`Error::CommentNotFound`] if the comment id isn't in the review,
/// [`Error::ReviewNotDraft`] if a structural change is attempted after
/// submission, [`Error::ReviewNotSubmitted`] if a resolution change is
/// attempted before submission (or after the review reached a terminal
/// state), [`Error::CommentDocOutOfScope`]/[`Error::CommentDocNotApplicable`]
/// for an invalid `doc`, [`Error::Io`] on read/write failure, or
/// [`Error::FrontmatterMissing`]/[`Error::FrontmatterParse`] on a malformed
/// review file.
pub fn update_comment(store: &mut impl Store, req: UpdateComment<'_>) -> Result<Document<Review>> {
    let UpdateComment {
        project,
        review_id,
        comment_id,
        body,
        anchor,
        doc,
        status,
        applied_commit,
        reply,
    } = req;
    let mut review_doc = crate::io::load_review(store, project, review_id)?;
    if !review_doc
        .frontmatter
        .comments
        .iter()
        .any(|c| c.id == comment_id)
    {
        return Err(Error::CommentNotFound {
            review_id: review_id.to_string(),
            comment_id,
        });
    }

    let structure_change = body.is_some() || anchor != AnchorUpdate::Keep || doc != DocUpdate::Keep;
    let resolution_change = status.is_some() || applied_commit.is_some() || reply.is_some();
    if structure_change && review_doc.frontmatter.state != ReviewState::Draft {
        return Err(Error::ReviewNotDraft(review_id.to_string()));
    }
    if resolution_change && review_doc.frontmatter.state != ReviewState::Submitted {
        return Err(Error::ReviewNotSubmitted(review_id.to_string()));
    }
    if let DocUpdate::Set(ref d) = doc {
        validate_comment_doc(store, project, &review_doc.frontmatter.target, d)?;
    }

    let comment = review_doc
        .frontmatter
        .comments
        .iter_mut()
        .find(|c| c.id == comment_id)
        .expect("comment presence checked above");
    if let Some(b) = body {
        comment.body = b.to_string();
    }
    match anchor {
        AnchorUpdate::Keep => {}
        AnchorUpdate::Set(a) => comment.anchor = Some(a),
        AnchorUpdate::Clear => comment.anchor = None,
    }
    match doc {
        DocUpdate::Keep => {}
        DocUpdate::Set(d) => comment.doc = Some(d),
        DocUpdate::Clear => comment.doc = None,
    }
    if let Some(s) = status {
        comment.status = s;
    }
    if let Some(sha) = applied_commit {
        comment.applied_commit = Some(sha.to_string());
    }
    if let Some(r) = reply {
        comment.reply = Some(r.to_string());
    }

    crate::io::write_review(store, project, review_id, &review_doc)?;
    Ok(review_doc)
}

/// Removes a comment from a draft review.
///
/// Remaining comments keep their ids — nothing is renumbered.
///
/// # Errors
///
/// Returns [`Error::ReviewNotFound`] if the review doesn't exist,
/// [`Error::ReviewNotDraft`] if the review has been submitted,
/// [`Error::CommentNotFound`] if the comment id isn't in the review,
/// [`Error::Io`] on read/write failure, or [`Error::FrontmatterMissing`]/
/// [`Error::FrontmatterParse`] on a malformed review file.
pub fn remove_comment(
    store: &mut impl Store,
    project: &str,
    review_id: &str,
    comment_id: u32,
) -> Result<Document<Review>> {
    let mut review_doc = crate::io::load_review(store, project, review_id)?;
    if review_doc.frontmatter.state != ReviewState::Draft {
        return Err(Error::ReviewNotDraft(review_id.to_string()));
    }
    let before = review_doc.frontmatter.comments.len();
    review_doc
        .frontmatter
        .comments
        .retain(|c| c.id != comment_id);
    if review_doc.frontmatter.comments.len() == before {
        return Err(Error::CommentNotFound {
            review_id: review_id.to_string(),
            comment_id,
        });
    }
    crate::io::write_review(store, project, review_id, &review_doc)?;
    Ok(review_doc)
}

/// Submits a draft review, stamping its verdict and submission time.
///
/// `verdict` is `Option<Verdict>` (rather than a required parameter) so that
/// frontends without argument-level "required" enforcement still get the
/// matchable [`Error::ReviewMissingVerdict`] from core.
///
/// # Errors
///
/// Returns [`Error::ReviewNotFound`] if the review doesn't exist,
/// [`Error::ReviewNotDraft`] if the review was already submitted or is
/// terminal, [`Error::ReviewMissingVerdict`] if `verdict` is `None`,
/// [`Error::ReviewEmpty`] if the review has no comments and no summary,
/// [`Error::Io`] on read/write failure, or [`Error::FrontmatterMissing`]/
/// [`Error::FrontmatterParse`] on a malformed review file.
pub fn submit_review(
    store: &mut impl Store,
    project: &str,
    review_id: &str,
    verdict: Option<Verdict>,
) -> Result<Document<Review>> {
    let mut review_doc = crate::io::load_review(store, project, review_id)?;
    if review_doc.frontmatter.state != ReviewState::Draft {
        return Err(Error::ReviewNotDraft(review_id.to_string()));
    }
    let verdict = verdict.ok_or_else(|| Error::ReviewMissingVerdict(review_id.to_string()))?;
    if review_doc.frontmatter.comments.is_empty() && review_doc.body.trim().is_empty() {
        return Err(Error::ReviewEmpty(review_id.to_string()));
    }
    review_doc.frontmatter.verdict = Some(verdict);
    review_doc.frontmatter.state = ReviewState::Submitted;
    review_doc.frontmatter.submitted = Some(Utc::now());
    crate::io::write_review(store, project, review_id, &review_doc)?;
    Ok(review_doc)
}

/// Replaces a draft review's overall summary (the markdown body below the
/// frontmatter).
///
/// Draft-only, like every other structural mutation in this module. Routes
/// through [`BodyUpdate`](crate::ops::BodyUpdate), so replacing a non-empty
/// summary with an empty string requires the explicit
/// [`Clear`](crate::ops::BodyUpdate::Clear) opt-in — the same clobber
/// protection every other body-bearing entity gets.
///
/// # Errors
///
/// Returns [`Error::ReviewNotFound`] if the review doesn't exist,
/// [`Error::ReviewNotDraft`] if the review has been submitted,
/// [`Error::BodyClobberRefused`] if an empty `Set` would clobber a non-empty
/// summary, [`Error::Io`] on read/write failure, or
/// [`Error::FrontmatterMissing`]/[`Error::FrontmatterParse`] on a malformed
/// review file.
pub fn set_summary(
    store: &mut impl Store,
    project: &str,
    review_id: &str,
    body: crate::ops::BodyUpdate,
) -> Result<Document<Review>> {
    let mut review_doc = crate::io::load_review(store, project, review_id)?;
    if review_doc.frontmatter.state != ReviewState::Draft {
        return Err(Error::ReviewNotDraft(review_id.to_string()));
    }
    body.apply(&mut review_doc.body)?;
    crate::io::write_review(store, project, review_id, &review_doc)?;
    Ok(review_doc)
}

/// A terminal state transition for a review.
///
/// `Draft` and `Submitted` are deliberately unrepresentable here:
/// `Submitted` is only reachable through [`submit_review`] (verdict-gated),
/// and nothing ever transitions back to `Draft`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewTransition {
    /// Close the review as addressed (submitted-only; requires every
    /// comment to be terminal).
    Addressed,
    /// Dismiss the review without acting on it (the human escape hatch,
    /// valid from draft or submitted).
    Dismissed,
}

/// Transitions a review to a terminal state per the lifecycle state machine.
///
/// `Addressed` requires the review to be `submitted` and every comment to be
/// terminal (`addressed` or `wont-fix`). `Dismissed` is valid from `draft`
/// or `submitted`. Terminal states reject any further transition.
///
/// # Errors
///
/// Returns [`Error::ReviewNotFound`] if the review doesn't exist,
/// [`Error::ReviewInvalidTransition`] if the state machine forbids the move,
/// [`Error::ReviewOpenComments`] if `Addressed` is requested while comments
/// remain open, [`Error::Io`] on read/write failure, or
/// [`Error::FrontmatterMissing`]/[`Error::FrontmatterParse`] on a malformed
/// review file.
pub fn update_review(
    store: &mut impl Store,
    project: &str,
    review_id: &str,
    transition: ReviewTransition,
) -> Result<Document<Review>> {
    let mut review_doc = crate::io::load_review(store, project, review_id)?;
    let from = review_doc.frontmatter.state;
    match transition {
        ReviewTransition::Addressed => {
            if from != ReviewState::Submitted {
                return Err(Error::ReviewInvalidTransition {
                    review_id: review_id.to_string(),
                    from,
                    to: ReviewState::Addressed,
                });
            }
            let open_count = review_doc
                .frontmatter
                .comments
                .iter()
                .filter(|c| !c.status.is_terminal())
                .count();
            if open_count > 0 {
                return Err(Error::ReviewOpenComments {
                    review_id: review_id.to_string(),
                    open_count,
                });
            }
            review_doc.frontmatter.state = ReviewState::Addressed;
        }
        ReviewTransition::Dismissed => {
            if from.is_terminal() {
                return Err(Error::ReviewInvalidTransition {
                    review_id: review_id.to_string(),
                    from,
                    to: ReviewState::Dismissed,
                });
            }
            review_doc.frontmatter.state = ReviewState::Dismissed;
        }
    }
    crate::io::write_review(store, project, review_id, &review_doc)?;
    Ok(review_doc)
}

/// Criteria for filtering a list of reviews.
///
/// Each populated field narrows the result set independently; a review is
/// kept only if it satisfies all populated criteria. The default value (all
/// fields `None`) keeps every review. Mirrors
/// [`TaskFilter`](crate::ops::task::TaskFilter).
#[derive(Debug, Clone, Default)]
pub struct ReviewFilter {
    /// Target criterion: keep only reviews of exactly this target.
    pub target: Option<ReviewTarget>,
    /// Target-kind criterion: keep only reviews whose target has this kind
    /// discriminant (any roadmap / any phase / any task), regardless of
    /// which one.
    pub target_kind: Option<ReviewTargetKind>,
    /// State criterion: keep only reviews in exactly this state.
    pub state: Option<ReviewState>,
    /// Verdict criterion: keep only reviews stamped with exactly this
    /// verdict (a draft with no verdict never matches).
    pub verdict: Option<Verdict>,
    /// Author criterion: keep only reviews by exactly this author.
    pub author: Option<String>,
}

/// Returns whether `review` satisfies every populated criterion in `filter`.
#[must_use]
pub fn review_matches(review: &Review, filter: &ReviewFilter) -> bool {
    filter.target.as_ref().is_none_or(|t| &review.target == t)
        && filter.target_kind.is_none_or(|k| review.target.kind() == k)
        && filter.state.is_none_or(|s| review.state == s)
        && filter.verdict.is_none_or(|v| review.verdict == Some(v))
        && filter.author.as_deref().is_none_or(|a| review.author == a)
}

/// Filters `reviews` to those satisfying `filter`, preserving order.
///
/// An owned convenience wrapper over [`review_matches`] for callers holding
/// [`list_reviews`] output.
#[must_use]
pub fn filter_reviews(
    reviews: Vec<(String, Document<Review>)>,
    filter: &ReviewFilter,
) -> Vec<(String, Document<Review>)> {
    reviews
        .into_iter()
        .filter(|(_, doc)| review_matches(&doc.frontmatter, filter))
        .collect()
}

/// Lists a project's change-request queue: submitted reviews whose verdict
/// is `request-changes`, sorted by review id.
///
/// This is the single definition of "reviews an agent must act on", shared
/// by the CLI (`rdm review requests`) and the MCP server
/// (`rdm_review_requests`) so the two surfaces can never disagree.
///
/// # Errors
///
/// Returns [`Error::ProjectNotFound`] if the project does not exist,
/// [`Error::Io`] if the reviews directory cannot be read, or
/// [`Error::FrontmatterMissing`]/[`Error::FrontmatterParse`] if a review
/// file has invalid frontmatter.
pub fn change_requests(
    store: &impl Store,
    project: &str,
) -> Result<Vec<(String, Document<Review>)>> {
    Ok(filter_reviews(
        list_reviews(store, project)?,
        &ReviewFilter {
            state: Some(ReviewState::Submitted),
            verdict: Some(Verdict::RequestChanges),
            ..Default::default()
        },
    ))
}

/// Loads a single review by id.
///
/// # Errors
///
/// Returns [`Error::ReviewNotFound`] if the review doesn't exist,
/// [`Error::Io`] on read failure, or [`Error::FrontmatterMissing`]/
/// [`Error::FrontmatterParse`] on a malformed review file.
pub fn get_review(store: &impl Store, project: &str, review_id: &str) -> Result<Document<Review>> {
    crate::io::load_review(store, project, review_id)
}

/// Deletes a review file.
///
/// Only draft reviews may be deleted without `force` — submitted and
/// terminal reviews are part of the record and require an explicit
/// override.
///
/// # Errors
///
/// Returns [`Error::ReviewNotFound`] if the review doesn't exist,
/// [`Error::ReviewNotDraft`] if the review is not a draft and `force` is
/// `false`, [`Error::Io`] on read/delete failure, or
/// [`Error::FrontmatterMissing`]/[`Error::FrontmatterParse`] on a malformed
/// review file.
pub fn delete_review(
    store: &mut impl Store,
    project: &str,
    review_id: &str,
    force: bool,
) -> Result<()> {
    let review_doc = crate::io::load_review(store, project, review_id)?;
    if !force && review_doc.frontmatter.state != ReviewState::Draft {
        return Err(Error::ReviewNotDraft(review_id.to_string()));
    }
    store.delete(&crate::paths::review_path(project, review_id))?;
    Ok(())
}

/// Lists all reviews for a project, sorted by review id.
///
/// Returns `(id, Document<Review>)` tuples. Returns an empty vec if the
/// reviews directory doesn't exist. Compose with [`filter_reviews`] to
/// narrow by target, state, verdict, or author.
///
/// # Errors
///
/// Returns [`Error::ProjectNotFound`] if the project does not exist,
/// [`Error::Io`] if the directory cannot be read, or
/// [`Error::FrontmatterMissing`]/[`Error::FrontmatterParse`] if a
/// review file has invalid frontmatter.
pub fn list_reviews(store: &impl Store, project: &str) -> Result<Vec<(String, Document<Review>)>> {
    if !store.exists(&crate::paths::project_md_path(project)) {
        return Err(Error::ProjectNotFound(project.to_string()));
    }
    let dir = crate::paths::reviews_dir(project);
    let entries = store.list(&dir)?;

    let mut reviews: Vec<(String, Document<Review>)> = Vec::new();
    for entry in entries {
        if entry.kind != DirEntryKind::File {
            continue;
        }
        if !entry.name.ends_with(".md") {
            continue;
        }
        let id = entry.name.trim_end_matches(".md").to_string();
        let doc = crate::io::load_review(store, project, &id)?;
        reviews.push((id, doc));
    }
    reviews.sort_by(|(a, _), (b, _)| a.cmp(b));
    Ok(reviews)
}

/// Open-review / open-comment tallies for a single roadmap or task.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReviewCounts {
    /// Number of open ([`ReviewState::Submitted`]) reviews on the item.
    pub open_reviews: usize,
    /// Number of open (non-terminal) comments across those reviews.
    pub open_comments: usize,
}

/// Per-target open-review aggregates: roadmap slug → counts and task slug →
/// counts.
///
/// Phase-targeted reviews roll up into their **parent roadmap's** counts —
/// listing surfaces (`INDEX.md` and the web list pages) have one row per
/// roadmap, and both must report the same numbers, so this is the single
/// counting shape they all consume.
#[derive(Debug, Clone, Default)]
pub struct OpenReviewCounts {
    /// Roadmap slug → counts. Phase-targeted reviews are included under
    /// their parent roadmap's slug.
    pub roadmaps: std::collections::HashMap<String, ReviewCounts>,
    /// Task slug → counts.
    pub tasks: std::collections::HashMap<String, ReviewCounts>,
}

/// Aggregates open reviews and their open comments per target.
///
/// Only [`ReviewState::Submitted`] reviews count as open: drafts are not yet
/// feedback and terminal reviews (`addressed`/`dismissed`) are resolved.
/// Within a counted review, only comments whose status is not terminal add
/// to `open_comments`. Phase-targeted reviews roll up into their parent
/// roadmap's entry (see [`OpenReviewCounts`]). Dangling targets
/// (renamed/deleted items) are harmless: nothing looks them up.
///
/// This is the shared counting pass behind both `INDEX.md` generation and
/// the web list pages — they must never disagree.
///
/// # Errors
///
/// Returns [`Error::ProjectNotFound`] if the project does not exist,
/// [`Error::Io`] if the reviews directory cannot be read, or
/// [`Error::FrontmatterMissing`]/[`Error::FrontmatterParse`] if a review
/// file is malformed.
pub fn count_open_reviews(store: &impl Store, project: &str) -> Result<OpenReviewCounts> {
    Ok(count_open_reviews_in(&list_reviews(store, project)?))
}

/// [`count_open_reviews`] over an already-loaded review list, for callers
/// that need the full list *and* the counts without a second store pass.
#[must_use]
pub fn count_open_reviews_in(reviews: &[(String, Document<Review>)]) -> OpenReviewCounts {
    let mut counts = OpenReviewCounts::default();
    for (_, doc) in reviews {
        let review = &doc.frontmatter;
        if review.state != ReviewState::Submitted {
            continue;
        }
        let open_comments = review
            .comments
            .iter()
            .filter(|c| !c.status.is_terminal())
            .count();
        let (map, key) = match &review.target {
            ReviewTarget::Roadmap { roadmap } => (&mut counts.roadmaps, roadmap),
            ReviewTarget::Phase { roadmap, .. } => (&mut counts.roadmaps, roadmap),
            ReviewTarget::Task { slug } => (&mut counts.tasks, slug),
        };
        let entry = map.entry(key.clone()).or_default();
        entry.open_reviews += 1;
        entry.open_comments += open_comments;
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Project, ReviewState, ReviewTarget};
    use crate::store::MemoryStore;
    use chrono::TimeZone;

    fn setup_store() -> MemoryStore {
        let mut store = MemoryStore::new();
        let project_doc = Document {
            frontmatter: Project {
                name: "test".to_string(),
                title: "Test Project".to_string(),
            },
            body: String::new(),
        };
        store
            .write(
                &crate::paths::project_md_path("test"),
                project_doc.render().unwrap(),
            )
            .unwrap();
        store
    }

    fn sample_review(id: &str) -> Document<Review> {
        Document {
            frontmatter: Review {
                id: id.to_string(),
                author: "ed".to_string(),
                target: ReviewTarget::Task {
                    slug: "fix-login".to_string(),
                },
                state: ReviewState::Draft,
                verdict: None,
                created: chrono::Utc.with_ymd_and_hms(2026, 7, 1, 14, 30, 0).unwrap(),
                submitted: None,
                created_commit: None,
                comments: vec![],
            },
            body: "Review summary.".to_string(),
        }
    }

    #[test]
    fn list_reviews_sorted_by_id() {
        let mut store = setup_store();
        for id in [
            "2026-07-01-1430-zz99",
            "2026-06-30-0900-aa11",
            "2026-07-01-0800-mm55",
        ] {
            let doc = sample_review(id);
            crate::io::write_review(&mut store, "test", id, &doc).unwrap();
        }
        let reviews = list_reviews(&store, "test").unwrap();
        assert_eq!(reviews.len(), 3);
        assert_eq!(reviews[0].0, "2026-06-30-0900-aa11");
        assert_eq!(reviews[1].0, "2026-07-01-0800-mm55");
        assert_eq!(reviews[2].0, "2026-07-01-1430-zz99");
        assert_eq!(reviews[0].1.frontmatter.id, "2026-06-30-0900-aa11");
    }

    #[test]
    fn list_reviews_missing_dir_is_empty() {
        let store = setup_store();
        let reviews = list_reviews(&store, "test").unwrap();
        assert!(reviews.is_empty());
    }

    #[test]
    fn list_reviews_project_not_found() {
        let store = MemoryStore::new();
        let result = list_reviews(&store, "nonexistent");
        assert!(matches!(result, Err(Error::ProjectNotFound(_))));
    }

    // -- id generation --

    #[test]
    fn generate_review_id_format() {
        let now = chrono::Utc.with_ymd_and_hms(2026, 7, 1, 14, 30, 5).unwrap();
        let id = generate_review_id(now);
        assert!(id.starts_with("2026-07-01-1430-"), "got: {id}");
        let suffix = id.rsplit('-').next().unwrap();
        assert_eq!(suffix.len(), 4);
        assert!(
            suffix.chars().all(|c| c.is_ascii_hexdigit()),
            "suffix should be hex, got: {suffix}"
        );
    }

    #[test]
    fn generate_review_id_same_instant_still_unique() {
        let now = chrono::Utc.with_ymd_and_hms(2026, 7, 1, 14, 30, 5).unwrap();
        let a = generate_review_id(now);
        let b = generate_review_id(now);
        assert_ne!(a, b, "counter jitter must break same-instant ties");
    }

    #[test]
    fn next_available_review_id_returns_first_free_candidate() {
        let mut store = setup_store();
        crate::io::write_review(&mut store, "test", "taken", &sample_review("taken")).unwrap();
        let mut calls = 0;
        let id = next_available_review_id(&store, "test", || {
            calls += 1;
            if calls == 1 { "taken" } else { "free" }.to_string()
        })
        .unwrap();
        assert_eq!(id, "free");
        assert_eq!(calls, 2);
    }

    #[test]
    fn next_available_review_id_exhausted_after_repeated_collisions() {
        let mut store = setup_store();
        crate::io::write_review(&mut store, "test", "stuck", &sample_review("stuck")).unwrap();
        let mut calls = 0;
        let result = next_available_review_id(&store, "test", || {
            calls += 1;
            "stuck".to_string()
        });
        assert!(matches!(result, Err(Error::ReviewIdExhausted)));
        assert_eq!(calls, MAX_ID_ATTEMPTS);
    }

    // -- review filtering (pure) --

    fn review_for_filter(
        target: ReviewTarget,
        state: ReviewState,
        verdict: Option<Verdict>,
        author: &str,
    ) -> Review {
        Review {
            id: "2026-07-01-1430-a1b2".to_string(),
            author: author.to_string(),
            target,
            state,
            verdict,
            created: chrono::Utc.with_ymd_and_hms(2026, 7, 1, 14, 30, 0).unwrap(),
            submitted: None,
            created_commit: None,
            comments: vec![],
        }
    }

    fn task_target(slug: &str) -> ReviewTarget {
        ReviewTarget::Task {
            slug: slug.to_string(),
        }
    }

    #[test]
    fn review_filter_default_matches_everything() {
        let filter = ReviewFilter::default();
        let review = review_for_filter(task_target("a"), ReviewState::Draft, None, "ed");
        assert!(review_matches(&review, &filter));
        let review = review_for_filter(
            task_target("b"),
            ReviewState::Dismissed,
            Some(Verdict::Approve),
            "bot",
        );
        assert!(review_matches(&review, &filter));
    }

    #[test]
    fn review_filter_by_target_exact_match() {
        let filter = ReviewFilter {
            target: Some(task_target("a")),
            ..Default::default()
        };
        let matching = review_for_filter(task_target("a"), ReviewState::Draft, None, "ed");
        assert!(review_matches(&matching, &filter));
        let other_task = review_for_filter(task_target("b"), ReviewState::Draft, None, "ed");
        assert!(!review_matches(&other_task, &filter));
        let roadmap = review_for_filter(
            ReviewTarget::Roadmap {
                roadmap: "a".to_string(),
            },
            ReviewState::Draft,
            None,
            "ed",
        );
        assert!(!review_matches(&roadmap, &filter));
    }

    #[test]
    fn review_filter_by_target_kind() {
        let filter = ReviewFilter {
            target_kind: Some(ReviewTargetKind::Task),
            ..Default::default()
        };
        // Any task matches, regardless of slug.
        let task_a = review_for_filter(task_target("a"), ReviewState::Draft, None, "ed");
        assert!(review_matches(&task_a, &filter));
        let task_b = review_for_filter(task_target("b"), ReviewState::Draft, None, "ed");
        assert!(review_matches(&task_b, &filter));
        // Other kinds do not.
        let roadmap = review_for_filter(
            ReviewTarget::Roadmap {
                roadmap: "a".to_string(),
            },
            ReviewState::Draft,
            None,
            "ed",
        );
        assert!(!review_matches(&roadmap, &filter));
        let phase = review_for_filter(
            ReviewTarget::Phase {
                roadmap: "a".to_string(),
                stem: "phase-1-one".to_string(),
            },
            ReviewState::Draft,
            None,
            "ed",
        );
        assert!(!review_matches(&phase, &filter));
        // Combined with an exact target, both criteria must hold.
        let filter = ReviewFilter {
            target: Some(task_target("a")),
            target_kind: Some(ReviewTargetKind::Roadmap),
            ..Default::default()
        };
        assert!(!review_matches(&task_a, &filter));
    }

    #[test]
    fn review_filter_by_state() {
        let filter = ReviewFilter {
            state: Some(ReviewState::Submitted),
            ..Default::default()
        };
        let submitted = review_for_filter(
            task_target("a"),
            ReviewState::Submitted,
            Some(Verdict::Comment),
            "ed",
        );
        assert!(review_matches(&submitted, &filter));
        let draft = review_for_filter(task_target("a"), ReviewState::Draft, None, "ed");
        assert!(!review_matches(&draft, &filter));
    }

    #[test]
    fn review_filter_by_verdict() {
        let filter = ReviewFilter {
            verdict: Some(Verdict::RequestChanges),
            ..Default::default()
        };
        let matching = review_for_filter(
            task_target("a"),
            ReviewState::Submitted,
            Some(Verdict::RequestChanges),
            "ed",
        );
        assert!(review_matches(&matching, &filter));
        let approve = review_for_filter(
            task_target("a"),
            ReviewState::Submitted,
            Some(Verdict::Approve),
            "ed",
        );
        assert!(!review_matches(&approve, &filter));
        // A draft with no verdict never matches a verdict filter.
        let no_verdict = review_for_filter(task_target("a"), ReviewState::Draft, None, "ed");
        assert!(!review_matches(&no_verdict, &filter));
    }

    #[test]
    fn review_filter_by_author() {
        let filter = ReviewFilter {
            author: Some("ed".to_string()),
            ..Default::default()
        };
        let ed = review_for_filter(task_target("a"), ReviewState::Draft, None, "ed");
        assert!(review_matches(&ed, &filter));
        let bot = review_for_filter(task_target("a"), ReviewState::Draft, None, "bot");
        assert!(!review_matches(&bot, &filter));
    }

    #[test]
    fn review_filter_combines_criteria_as_and() {
        let filter = ReviewFilter {
            target: Some(task_target("a")),
            target_kind: Some(ReviewTargetKind::Task),
            state: Some(ReviewState::Submitted),
            verdict: Some(Verdict::Approve),
            author: Some("ed".to_string()),
        };
        let all_match = review_for_filter(
            task_target("a"),
            ReviewState::Submitted,
            Some(Verdict::Approve),
            "ed",
        );
        assert!(review_matches(&all_match, &filter));
        // One mismatched criterion fails the AND.
        let wrong_author = review_for_filter(
            task_target("a"),
            ReviewState::Submitted,
            Some(Verdict::Approve),
            "bot",
        );
        assert!(!review_matches(&wrong_author, &filter));
    }

    #[test]
    fn filter_reviews_keeps_matching_in_order() {
        let reviews = vec![
            (
                "r1".to_string(),
                Document {
                    frontmatter: review_for_filter(
                        task_target("a"),
                        ReviewState::Draft,
                        None,
                        "ed",
                    ),
                    body: String::new(),
                },
            ),
            (
                "r2".to_string(),
                Document {
                    frontmatter: review_for_filter(
                        task_target("a"),
                        ReviewState::Submitted,
                        Some(Verdict::Approve),
                        "ed",
                    ),
                    body: String::new(),
                },
            ),
            (
                "r3".to_string(),
                Document {
                    frontmatter: review_for_filter(
                        task_target("a"),
                        ReviewState::Draft,
                        None,
                        "bot",
                    ),
                    body: String::new(),
                },
            ),
        ];
        let filter = ReviewFilter {
            state: Some(ReviewState::Draft),
            ..Default::default()
        };
        let kept = filter_reviews(reviews, &filter);
        let ids: Vec<&str> = kept.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, vec!["r1", "r3"]);
    }

    // -- change_requests --

    fn write_review_with(
        store: &mut MemoryStore,
        id: &str,
        state: ReviewState,
        verdict: Option<Verdict>,
    ) {
        let mut doc = sample_review(id);
        doc.frontmatter.state = state;
        doc.frontmatter.verdict = verdict;
        crate::io::write_review(store, "test", id, &doc).unwrap();
    }

    #[test]
    fn change_requests_keeps_only_submitted_request_changes() {
        let mut store = setup_store();
        write_review_with(
            &mut store,
            "2026-07-01-0900-aaaa",
            ReviewState::Submitted,
            Some(Verdict::RequestChanges),
        );
        write_review_with(
            &mut store,
            "2026-07-01-1000-bbbb",
            ReviewState::Submitted,
            Some(Verdict::RequestChanges),
        );
        let queue = change_requests(&store, "test").unwrap();
        let ids: Vec<&str> = queue.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, vec!["2026-07-01-0900-aaaa", "2026-07-01-1000-bbbb"]);
    }

    #[test]
    fn change_requests_excludes_drafts_approvals_and_terminal_reviews() {
        let mut store = setup_store();
        // Draft (no verdict yet) — not actionable.
        write_review_with(&mut store, "r-draft", ReviewState::Draft, None);
        // Submitted but approved / neutral — nothing to change.
        write_review_with(
            &mut store,
            "r-approve",
            ReviewState::Submitted,
            Some(Verdict::Approve),
        );
        write_review_with(
            &mut store,
            "r-comment",
            ReviewState::Submitted,
            Some(Verdict::Comment),
        );
        // Terminal request-changes reviews — already handled.
        write_review_with(
            &mut store,
            "r-addressed",
            ReviewState::Addressed,
            Some(Verdict::RequestChanges),
        );
        write_review_with(
            &mut store,
            "r-dismissed",
            ReviewState::Dismissed,
            Some(Verdict::RequestChanges),
        );
        // The one live change request.
        write_review_with(
            &mut store,
            "r-live",
            ReviewState::Submitted,
            Some(Verdict::RequestChanges),
        );
        let queue = change_requests(&store, "test").unwrap();
        assert_eq!(queue.len(), 1);
        assert_eq!(queue[0].0, "r-live");
    }

    #[test]
    fn change_requests_project_not_found() {
        let store = MemoryStore::new();
        let result = change_requests(&store, "nonexistent");
        assert!(matches!(result, Err(Error::ProjectNotFound(_))));
    }

    // -- target / doc reference parsing --

    /// A store with a `test` project, roadmap `alpha` (one phase,
    /// `phase-1-one`), and task `fix-login`.
    fn setup_store_with_items() -> MemoryStore {
        let mut store = setup_store();
        crate::ops::roadmap::create_roadmap(
            &mut store,
            crate::ops::roadmap::CreateRoadmap {
                project: "test",
                slug: "alpha",
                title: "Alpha",
                body: None,
                ..Default::default()
            },
        )
        .unwrap();
        crate::ops::phase::create_phase(
            &mut store,
            crate::ops::phase::CreatePhase {
                project: "test",
                roadmap: "alpha",
                slug: "one",
                title: "One",
                number: Some(1),
                ..Default::default()
            },
        )
        .unwrap();
        crate::ops::task::create_task(
            &mut store,
            crate::ops::task::CreateTask {
                project: "test",
                slug: "fix-login",
                title: "Fix login",
                ..Default::default()
            },
        )
        .unwrap();
        store
    }

    #[test]
    fn parse_target_ref_roadmap() {
        let store = setup_store_with_items();
        let target = parse_review_target_ref(&store, "test", "roadmap/alpha").unwrap();
        assert_eq!(
            target,
            ReviewTarget::Roadmap {
                roadmap: "alpha".to_string()
            }
        );
    }

    #[test]
    fn parse_target_ref_task() {
        let store = setup_store_with_items();
        let target = parse_review_target_ref(&store, "test", "task/fix-login").unwrap();
        assert_eq!(
            target,
            ReviewTarget::Task {
                slug: "fix-login".to_string()
            }
        );
    }

    #[test]
    fn parse_target_ref_phase_by_stem() {
        let store = setup_store_with_items();
        let target = parse_review_target_ref(&store, "test", "phase/alpha/phase-1-one").unwrap();
        assert_eq!(
            target,
            ReviewTarget::Phase {
                roadmap: "alpha".to_string(),
                stem: "phase-1-one".to_string(),
            }
        );
    }

    #[test]
    fn parse_target_ref_phase_by_number() {
        let store = setup_store_with_items();
        let target = parse_review_target_ref(&store, "test", "phase/alpha/1").unwrap();
        assert_eq!(
            target,
            ReviewTarget::Phase {
                roadmap: "alpha".to_string(),
                stem: "phase-1-one".to_string(),
            }
        );
    }

    #[test]
    fn parse_target_ref_rejects_malformed() {
        let store = setup_store_with_items();
        for bad in [
            "",
            "alpha",
            "roadmap/",
            "roadmap/a/b",
            "task/",
            "task/a/b",
            "phase/alpha",
            "phase//1",
            "phase/alpha/",
            "phase/alpha/1/extra",
            "widget/alpha",
        ] {
            let err = parse_review_target_ref(&store, "test", bad).unwrap_err();
            assert!(
                matches!(err, Error::InvalidReviewTargetRef(_)),
                "{bad:?} should be invalid, got {err:?}"
            );
            assert!(
                err.to_string().contains("roadmap/<slug>"),
                "error must show accepted forms: {err}"
            );
        }
    }

    #[test]
    fn parse_target_ref_unknown_phase_number_errors() {
        let store = setup_store_with_items();
        let err = parse_review_target_ref(&store, "test", "phase/alpha/9").unwrap_err();
        assert!(matches!(err, Error::PhaseNotFound(_)), "got {err:?}");
    }

    #[test]
    fn parse_comment_doc_ref_by_stem_and_number() {
        let store = setup_store_with_items();
        let doc = parse_comment_doc_ref(&store, "test", "alpha", "phase/phase-1-one").unwrap();
        assert_eq!(doc.stem, "phase-1-one");
        let doc = parse_comment_doc_ref(&store, "test", "alpha", "phase/1").unwrap();
        assert_eq!(doc.stem, "phase-1-one");
        assert_eq!(doc.kind, CommentDocKind::Phase);
    }

    #[test]
    fn parse_comment_doc_ref_rejects_malformed() {
        let store = setup_store_with_items();
        for bad in ["", "phase-1-one", "phase/", "phase/a/b", "task/x"] {
            let err = parse_comment_doc_ref(&store, "test", "alpha", bad).unwrap_err();
            assert!(
                matches!(err, Error::InvalidCommentDocRef(_)),
                "{bad:?} should be invalid, got {err:?}"
            );
        }
    }

    // -- set_summary --

    #[test]
    fn set_summary_replaces_draft_body() {
        let mut store = setup_store_with_items();
        let doc = create_review(
            &mut store,
            CreateReview {
                project: "test",
                author: "ed",
                target: ReviewTarget::Task {
                    slug: "fix-login".to_string(),
                },
                body: Some("first draft"),
            },
        )
        .unwrap();
        let id = doc.frontmatter.id.clone();
        let updated = set_summary(
            &mut store,
            "test",
            &id,
            crate::ops::BodyUpdate::Set("final summary".to_string()),
        )
        .unwrap();
        assert_eq!(updated.body, "final summary");
    }

    #[test]
    fn set_summary_rejects_empty_clobber_without_clear() {
        let mut store = setup_store_with_items();
        let doc = create_review(
            &mut store,
            CreateReview {
                project: "test",
                author: "ed",
                target: ReviewTarget::Task {
                    slug: "fix-login".to_string(),
                },
                body: Some("existing summary"),
            },
        )
        .unwrap();
        let id = doc.frontmatter.id.clone();
        let err = set_summary(
            &mut store,
            "test",
            &id,
            crate::ops::BodyUpdate::Set(String::new()),
        )
        .unwrap_err();
        assert!(matches!(err, Error::BodyClobberRefused), "got {err:?}");
        // Clear is the explicit opt-in.
        let cleared = set_summary(&mut store, "test", &id, crate::ops::BodyUpdate::Clear).unwrap();
        assert_eq!(cleared.body, "");
    }

    #[test]
    fn set_summary_rejects_submitted_review() {
        let mut store = setup_store_with_items();
        let doc = create_review(
            &mut store,
            CreateReview {
                project: "test",
                author: "ed",
                target: ReviewTarget::Task {
                    slug: "fix-login".to_string(),
                },
                body: Some("summary"),
            },
        )
        .unwrap();
        let id = doc.frontmatter.id.clone();
        submit_review(&mut store, "test", &id, Some(Verdict::Approve)).unwrap();
        let err = set_summary(
            &mut store,
            "test",
            &id,
            crate::ops::BodyUpdate::Set("late edit".to_string()),
        )
        .unwrap_err();
        assert!(matches!(err, Error::ReviewNotDraft(_)), "got {err:?}");
    }

    // -- count_open_reviews --

    fn counted_review(
        id: &str,
        target: ReviewTarget,
        state: ReviewState,
        comment_statuses: &[ReviewCommentStatus],
    ) -> (String, Document<Review>) {
        let mut doc = sample_review(id);
        doc.frontmatter.target = target;
        doc.frontmatter.state = state;
        doc.frontmatter.comments = comment_statuses
            .iter()
            .enumerate()
            .map(|(i, status)| ReviewComment {
                id: (i + 1) as u32,
                doc: None,
                status: *status,
                applied_commit: None,
                anchor: None,
                body: "note".to_string(),
                reply: None,
            })
            .collect();
        (id.to_string(), doc)
    }

    #[test]
    fn count_open_reviews_counts_only_submitted() {
        use ReviewCommentStatus::Open;
        let reviews = vec![
            counted_review(
                "r1",
                ReviewTarget::Task {
                    slug: "fix".to_string(),
                },
                ReviewState::Submitted,
                &[Open, Open],
            ),
            counted_review(
                "r2",
                ReviewTarget::Task {
                    slug: "fix".to_string(),
                },
                ReviewState::Draft,
                &[Open],
            ),
            counted_review(
                "r3",
                ReviewTarget::Task {
                    slug: "fix".to_string(),
                },
                ReviewState::Addressed,
                &[Open],
            ),
            counted_review(
                "r4",
                ReviewTarget::Task {
                    slug: "fix".to_string(),
                },
                ReviewState::Dismissed,
                &[Open],
            ),
        ];
        let counts = count_open_reviews_in(&reviews);
        assert_eq!(
            counts.tasks.get("fix").copied(),
            Some(ReviewCounts {
                open_reviews: 1,
                open_comments: 2
            })
        );
    }

    #[test]
    fn count_open_reviews_excludes_terminal_comments() {
        let reviews = vec![counted_review(
            "r1",
            ReviewTarget::Task {
                slug: "fix".to_string(),
            },
            ReviewState::Submitted,
            &[
                ReviewCommentStatus::Open,
                ReviewCommentStatus::Addressed,
                ReviewCommentStatus::WontFix,
            ],
        )];
        let counts = count_open_reviews_in(&reviews);
        assert_eq!(
            counts.tasks.get("fix").copied(),
            Some(ReviewCounts {
                open_reviews: 1,
                open_comments: 1
            })
        );
    }

    #[test]
    fn count_open_reviews_rolls_phase_reviews_into_parent_roadmap() {
        use ReviewCommentStatus::Open;
        let reviews = vec![
            counted_review(
                "r1",
                ReviewTarget::Roadmap {
                    roadmap: "alpha".to_string(),
                },
                ReviewState::Submitted,
                &[Open],
            ),
            counted_review(
                "r2",
                ReviewTarget::Phase {
                    roadmap: "alpha".to_string(),
                    stem: "phase-1-one".to_string(),
                },
                ReviewState::Submitted,
                &[Open, Open],
            ),
        ];
        let counts = count_open_reviews_in(&reviews);
        assert_eq!(
            counts.roadmaps.get("alpha").copied(),
            Some(ReviewCounts {
                open_reviews: 2,
                open_comments: 3
            })
        );
        assert!(counts.tasks.is_empty());
    }

    #[test]
    fn count_open_reviews_task_and_roadmap_slugs_do_not_collide() {
        use ReviewCommentStatus::Open;
        let reviews = vec![
            counted_review(
                "r1",
                ReviewTarget::Roadmap {
                    roadmap: "same".to_string(),
                },
                ReviewState::Submitted,
                &[Open],
            ),
            counted_review(
                "r2",
                ReviewTarget::Task {
                    slug: "same".to_string(),
                },
                ReviewState::Submitted,
                &[Open, Open],
            ),
        ];
        let counts = count_open_reviews_in(&reviews);
        assert_eq!(counts.roadmaps.get("same").unwrap().open_comments, 1);
        assert_eq!(counts.tasks.get("same").unwrap().open_comments, 2);
    }

    #[test]
    fn count_open_reviews_empty_when_no_reviews() {
        let counts = count_open_reviews_in(&[]);
        assert!(counts.roadmaps.is_empty());
        assert!(counts.tasks.is_empty());
    }

    #[test]
    fn count_open_reviews_store_pass_matches_in_memory_pass() {
        // The store-reading entry point and the slice-based pass must agree:
        // INDEX generation and the web handlers may enter through either.
        let mut store = setup_store();
        let (id, doc) = counted_review(
            "2026-07-01-1200-aaaa",
            ReviewTarget::Task {
                slug: "fix".to_string(),
            },
            ReviewState::Submitted,
            &[ReviewCommentStatus::Open],
        );
        crate::io::write_review(&mut store, "test", &id, &doc).unwrap();
        let via_store = count_open_reviews(&store, "test").unwrap();
        let via_slice = count_open_reviews_in(&list_reviews(&store, "test").unwrap());
        assert_eq!(via_store.tasks.get("fix"), via_slice.tasks.get("fix"));
        assert_eq!(via_store.tasks.get("fix").unwrap().open_reviews, 1);
    }
}

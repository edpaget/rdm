//! Review-document operations: create, comment on, submit, transition,
//! list, and delete stored reviews, enforcing the review lifecycle
//! (`draft` → `submitted` → `addressed` | `dismissed`).
//!
//! Not to be confused with [`crate::ops::review`], which enumerates plan
//! items awaiting review (the needs-review queue). This module operates on
//! the [`Review`] documents stored under a project's `reviews/` directory.

use chrono::{DateTime, Utc};

use crate::document::Document;
use crate::error::{Error, Result};
use crate::model::{
    Anchor, AnchorState, CommentDoc, CommentDocKind, ParseError, PlanStatus, Review, ReviewComment,
    ReviewCommentStatus, ReviewState, ReviewTarget, ReviewTargetKind, Verdict,
};
use crate::source::SourceRepo;
use crate::store::{DirEntryKind, Store, VersionedStore};

/// The branch a `change/` review's merge base is computed against.
///
/// Precedence: the project's own `source.default_branch`, else the plan
/// repo's `rdm.toml` `default_branch`, else the literal `"main"` — the same
/// ladder [`crate::ops::links::resolve_code_link`] walks for `rdm:src/`
/// resolution, kept in core so every interface that starts a change review
/// picks the same branch.
///
/// A project that cannot be loaded degrades to `config_default`: naming the
/// branch is not the place to fail a `review start`, and the merge-base
/// lookup that consumes it reports its own actionable error.
#[must_use]
pub fn source_default_branch(
    store: &impl Store,
    project: &str,
    config_default: Option<&str>,
) -> String {
    crate::io::load_project(store, project)
        .ok()
        .and_then(|doc| doc.frontmatter.source)
        .and_then(|source| source.default_branch)
        .or_else(|| config_default.map(str::to_string))
        .unwrap_or_else(|| "main".to_string())
}

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
/// - `plan/<slug>`
/// - `change/<head-sha>`
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
    let target: ReviewTarget = reference
        .parse()
        .map_err(|_| Error::InvalidReviewTargetRef(reference.to_string()))?;
    match target {
        ReviewTarget::Phase { roadmap, stem } => {
            let stem = crate::ops::phase::resolve_phase_stem(store, project, &roadmap, &stem)?;
            Ok(ReviewTarget::Phase { roadmap, stem })
        }
        other => Ok(other),
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

/// Generates a timestamp-based review id (`YYYY-MM-DD-HHMM-xxxx`) through
/// the shared [`generate_timestamp_id`](crate::ops::id::generate_timestamp_id).
fn generate_review_id(now: DateTime<Utc>) -> String {
    crate::ops::id::generate_timestamp_id(now)
}

/// Returns the first candidate id that does not collide with an existing
/// review file, retrying up to
/// [`MAX_ID_ATTEMPTS`](crate::ops::id::MAX_ID_ATTEMPTS) times.
fn next_available_review_id(
    store: &impl Store,
    project: &str,
    candidate: impl FnMut() -> String,
) -> Result<String> {
    crate::ops::id::next_available_id(
        |id| store.exists(&crate::paths::review_path(project, id)),
        candidate,
        Error::ReviewIdExhausted,
    )
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
        ReviewTarget::Plan { slug } => {
            if !store.exists(&crate::paths::plan_path(project, slug)) {
                return Err(Error::ReviewTargetMissing(format!("plan '{slug}'")));
            }
        }
        // A change target names commits in the project's *source*
        // repository; the plan repo holds nothing to check. The head/base
        // pair is resolved and proven against the real repository by the
        // caller (the CLI's `resolve_change_target`) before `create_review`
        // ever runs, so re-validating it here would need a source-repo
        // dependency rdm-core deliberately does not have.
        ReviewTarget::Change { .. } => {}
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
        ReviewTarget::Phase { .. }
        | ReviewTarget::Task { .. }
        | ReviewTarget::Plan { .. }
        | ReviewTarget::Change { .. } => Err(Error::CommentDocNotApplicable),
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
    /// The implementation plan the reviewed change implements, as a
    /// [`ReviewTarget::Plan`] reference.
    ///
    /// Only accepted on a [`ReviewTarget::Change`] review, and only when
    /// the named plan exists. Restricting it to change targets keeps the
    /// field from accumulating meanings: on a document review the target
    /// *is* the thing under review, so "which plan does this implement"
    /// has no answer.
    pub implements: Option<ReviewTarget>,
    /// Source-repository branch the reviewed change was on, when the
    /// checkout had one. Stamped by the caller; used only to pick the tip
    /// drift is measured against.
    pub change_branch: Option<String>,
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
/// Returns [`Error::InvalidStoredChangeRevision`] for malformed resolved change
/// identities (head or present base must be 40 lowercase ASCII hex characters).
///
/// Returns [`Error::ProjectNotFound`] if the project doesn't exist,
/// [`Error::ReviewTargetMissing`] if the target roadmap/phase/task doesn't
/// exist, [`Error::ReviewImplementsNotApplicable`] if `implements` is set on
/// anything but a change review, [`Error::ReviewImplementsInvalidKind`] if
/// it names something other than `plan/<slug>`,
/// [`Error::ReviewImplementsMissing`] if that plan doesn't exist,
/// [`Error::ReviewIdExhausted`] if repeated id-generation attempts all
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
        implements,
        change_branch,
    } = req;
    target.validate_stored_identity()?;
    if !store.exists(&crate::paths::project_md_path(project)) {
        return Err(Error::ProjectNotFound(project.to_string()));
    }
    validate_target_exists(store, project, &target)?;
    if let Some(plan_ref) = &implements {
        if !matches!(target, ReviewTarget::Change { .. }) {
            return Err(Error::ReviewImplementsNotApplicable(target.label()));
        }
        let ReviewTarget::Plan { slug } = plan_ref else {
            return Err(Error::ReviewImplementsInvalidKind(plan_ref.label()));
        };
        if !store.exists(&crate::paths::plan_path(project, slug)) {
            return Err(Error::ReviewImplementsMissing(slug.clone()));
        }
    }

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
            implements,
            change_branch,
            comments: Vec::new(),
        },
        body: body.unwrap_or_default().to_string(),
    };
    crate::io::write_review(store, project, &id, &doc)?;
    Ok(doc)
}

/// Request describing a new comment to add to a draft review.
///
/// [`Default`] yields empty identifiers and every optional field unset, so a
/// caller names only what it supplies (`..Default::default()`).
#[derive(Debug, Clone, Default)]
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
    /// Why the comment has no anchor. Only legal when `anchor` is `None`;
    /// left `None` there, [`add_comment`] stamps
    /// [`AnchorState::WholeDocument`].
    pub anchor_state: Option<AnchorState>,
    /// The source path an anchorless `change/` review comment is about.
    /// Only legal when `anchor` is `None` and the review targets a change;
    /// normalized through [`crate::change::normalize_source_path`] before it
    /// is stored.
    pub intended_path: Option<&'a str>,
}

/// Adds a comment to a draft review.
///
/// The comment id is `max(existing ids) + 1` — removing a comment never
/// renumbers the survivors, though the freed *highest* id may be reused by
/// a subsequent add.
///
/// An anchorless comment always records an [`AnchorState`]: the caller's
/// `anchor_state` when it supplied one, otherwise
/// [`AnchorState::WholeDocument`] — a comment written without an anchor and
/// without a reason is whole-document on purpose. An `intended_path` alone
/// is therefore a deliberate whole-file comment. An anchored comment never
/// carries either field.
///
/// # Errors
///
/// Returns [`Error::InvalidStoredChangeRevision`] for malformed resolved change
/// identities (head or present base must be 40 lowercase ASCII hex characters).
///
/// Returns [`Error::ReviewNotFound`] if the review doesn't exist,
/// [`Error::ReviewNotDraft`] if the review has been submitted (comment
/// structure is immutable after submit), [`Error::CommentDocNotApplicable`]
/// if `doc` is set on a phase or task review, [`Error::CommentDocOutOfScope`]
/// if `doc` names a phase outside the reviewed roadmap,
/// [`Error::CommentAnchorStateWithAnchor`] if `anchor` is set together with
/// `anchor_state` or `intended_path`, [`Error::CommentPathNotApplicable`] if
/// `intended_path` is set on a review that does not target a change,
/// whatever [`crate::change::normalize_source_path`] returns for an
/// unusable `intended_path`, [`Error::Io`] on read/write failure, or
/// [`Error::FrontmatterMissing`]/[`Error::FrontmatterParse`] on a malformed
/// review file.
pub fn add_comment(store: &mut impl Store, req: AddComment<'_>) -> Result<Document<Review>> {
    let AddComment {
        project,
        review_id,
        body,
        doc,
        anchor,
        anchor_state,
        intended_path,
    } = req;
    if anchor.is_some() && (anchor_state.is_some() || intended_path.is_some()) {
        return Err(Error::CommentAnchorStateWithAnchor);
    }
    let mut review_doc = crate::io::load_review(store, project, review_id)?;
    if review_doc.frontmatter.state != ReviewState::Draft {
        return Err(Error::ReviewNotDraft(review_id.to_string()));
    }
    if let Some(ref d) = doc {
        validate_comment_doc(store, project, &review_doc.frontmatter.target, d)?;
    }
    let intended_path = match intended_path {
        None => None,
        Some(p) => {
            if !matches!(review_doc.frontmatter.target, ReviewTarget::Change { .. }) {
                return Err(Error::CommentPathNotApplicable(
                    review_doc.frontmatter.target.label(),
                ));
            }
            Some(crate::change::normalize_source_path(p)?)
        }
    };
    let anchor_state = match anchor {
        Some(_) => None,
        None => Some(anchor_state.unwrap_or(AnchorState::WholeDocument)),
    };
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
        anchor_state,
        intended_path,
        body: body.to_string(),
        reply: None,
    });
    crate::io::write_review(store, project, review_id, &review_doc)?;
    Ok(review_doc)
}

/// What [`add_comment_with_anchor`] does when core refuses the requested
/// anchor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OnAnchorFailure {
    /// Fail the add with core's refusal (the historical behavior).
    #[default]
    Refuse,
    /// Store the comment without an anchor, recording the refusal's
    /// [`AnchorState`] reason (and, on a change review, the requested path
    /// as the intended path).
    WholeDocument,
}

impl OnAnchorFailure {
    /// The flag spelling: `refuse` or `whole-document`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            OnAnchorFailure::Refuse => "refuse",
            OnAnchorFailure::WholeDocument => "whole-document",
        }
    }
}

impl std::fmt::Display for OnAnchorFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for OnAnchorFailure {
    type Err = ParseError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "refuse" => Ok(OnAnchorFailure::Refuse),
            "whole-document" => Ok(OnAnchorFailure::WholeDocument),
            other => Err(ParseError::new(
                "anchor-failure mode",
                other,
                "refuse or whole-document",
            )),
        }
    }
}

/// The anchor a caller asks [`add_comment_with_anchor`] to derive: the raw
/// `--quote`/`--path`/`--occurrence` inputs, before any derivation.
#[derive(Debug, Clone, Copy, Default)]
pub struct AnchorRequest<'a> {
    /// The exact text to anchor to.
    pub quote: Option<&'a str>,
    /// On a `change/` review: the source-repo-relative file the quote lives
    /// in. Refused on every other review kind.
    pub path: Option<&'a str>,
    /// Which occurrence of `quote` to anchor to (1-based).
    pub occurrence: Option<usize>,
}

/// The outcome of [`add_comment_with_anchor`].
#[derive(Debug, Clone)]
pub struct AddedComment {
    /// The review as written, including the new comment.
    pub doc: Document<Review>,
    /// Id of the comment just added.
    pub comment_id: u32,
    /// Core's refusal text when the requested anchor was refused and the
    /// comment was stored whole-document instead; `None` when it anchored
    /// (or no anchor was requested).
    pub refusal: Option<String>,
}

/// Maps an anchor refusal to the [`AnchorState`] reason it records.
///
/// Returns `None` for every error that is not an anchor refusal — usage
/// errors (a quote without a path and the like), store and IO failures, a
/// source repository that cannot be queried — so the fall-back mode of
/// [`add_comment_with_anchor`] never turns them into a stored comment.
///
/// # Examples
///
/// ```
/// use rdm_core::error::Error;
/// use rdm_core::model::AnchorState;
/// use rdm_core::ops::reviews::anchor_refusal_reason;
///
/// let refusal = Error::QuoteNotFound { quote: "x".to_string(), commit: None };
/// assert_eq!(anchor_refusal_reason(&refusal), Some(AnchorState::QuoteNotFound));
/// assert_eq!(anchor_refusal_reason(&Error::ChangeQuoteNeedsPath), None);
/// ```
#[must_use]
pub fn anchor_refusal_reason(err: &Error) -> Option<AnchorState> {
    match err {
        Error::QuoteNotFound { .. } => Some(AnchorState::QuoteNotFound),
        Error::QuoteAmbiguous { .. } => Some(AnchorState::Ambiguous),
        Error::QuoteOccurrenceOutOfRange { .. } => Some(AnchorState::OccurrenceOutOfRange),
        Error::QuoteOutsideChangedHunks { .. } => Some(AnchorState::OutsideHunk),
        Error::ChangePathNotInRevision { .. } => Some(AnchorState::PathMissing),
        Error::ChangePathNotAFile { .. } => Some(AnchorState::PathNotAFile),
        Error::ChangePathNotLinkable { .. } => Some(AnchorState::PathNotLinkable),
        Error::ChangeHeadNotInSource { .. } => Some(AnchorState::HeadNotInSource),
        _ => None,
    }
}

/// Adds a comment to a draft review, deriving its anchor from `anchor` —
/// and, under [`OnAnchorFailure::WholeDocument`], storing it whole-document
/// with the refusal's reason when core refuses that anchor.
///
/// Derivation follows the review's target:
///
/// - a `change/` review derives an [`Anchor::FileQuote`] through
///   [`crate::change::derive_change_anchor`] against `source`;
/// - any other review derives an [`Anchor::TextQuote`] from the comment's
///   document as of the review's `created_commit`
///   ([`crate::anchor::body_for_comment`] then
///   [`crate::anchor::derive_text_quote`]).
///
/// With neither `quote` nor `path` requested, this is exactly
/// [`add_comment`] with `req` unchanged (so `req.anchor_state` and
/// `req.intended_path` pass through). When an anchor *is* requested,
/// `req.anchor`, `req.anchor_state` and `req.intended_path` must all be
/// unset.
///
/// Under [`OnAnchorFailure::WholeDocument`], a refusal that
/// [`anchor_refusal_reason`] maps is stored as an anchorless comment with
/// that reason, plus — on a change review — the requested `path` as its
/// intended path, when that path normalizes (a path refused as unlinkable or
/// not repo-relative is not stored). The refusal text is returned in
/// [`AddedComment::refusal`]. Every other error propagates in either mode.
///
/// `source` is only consulted for a change review that requests an anchor;
/// every other caller may pass `None`.
///
/// # Errors
///
/// Returns [`Error::CommentAnchorStateWithAnchor`] when an anchor is
/// requested alongside `req.anchor`, `req.anchor_state` or
/// `req.intended_path`, [`Error::ReviewNotDraft`] for a submitted review,
/// [`Error::ChangeAnchorNeedsSource`] when a change review requests an
/// anchor and `source` is `None`, [`Error::CommentPathNotApplicable`] when
/// `path` is set on a non-change review, the anchor refusal itself under
/// [`OnAnchorFailure::Refuse`], any non-refusal error from derivation
/// (usage errors such as [`Error::ChangeQuoteNeedsPath`], a source
/// repository that cannot be queried, a document that cannot be loaded), or
/// anything [`add_comment`] returns.
pub fn add_comment_with_anchor<S: SourceRepo>(
    store: &mut impl VersionedStore,
    source: Option<&S>,
    req: AddComment<'_>,
    anchor: AnchorRequest<'_>,
    on_failure: OnAnchorFailure,
) -> Result<AddedComment> {
    let added = |doc: Document<Review>, refusal: Option<String>| {
        let comment_id = doc
            .frontmatter
            .comments
            .last()
            .map(|c| c.id)
            .unwrap_or_default();
        AddedComment {
            doc,
            comment_id,
            refusal,
        }
    };
    if anchor.quote.is_none() && anchor.path.is_none() {
        return Ok(added(add_comment(store, req)?, None));
    }
    if req.anchor.is_some() || req.anchor_state.is_some() || req.intended_path.is_some() {
        return Err(Error::CommentAnchorStateWithAnchor);
    }
    let review = crate::io::load_review(store, req.project, req.review_id)?;
    if review.frontmatter.state != ReviewState::Draft {
        return Err(Error::ReviewNotDraft(req.review_id.to_string()));
    }
    let is_change = matches!(review.frontmatter.target, ReviewTarget::Change { .. });
    let derived = match &review.frontmatter.target {
        ReviewTarget::Change { head, base } => {
            let Some(source) = source else {
                return Err(Error::ChangeAnchorNeedsSource);
            };
            crate::change::derive_change_anchor(
                source,
                head,
                base.as_deref(),
                anchor.quote,
                anchor.path,
                anchor.occurrence,
            )
        }
        other => {
            if anchor.path.is_some() {
                return Err(Error::CommentPathNotApplicable(other.label()));
            }
            match anchor.quote {
                None => Ok(None),
                Some(quote) => {
                    let (body, at) = crate::anchor::body_for_comment(
                        store,
                        req.project,
                        &review.frontmatter,
                        req.doc.as_ref(),
                    )?;
                    crate::anchor::derive_text_quote(&body, quote, anchor.occurrence, at.as_deref())
                        .map(Some)
                }
            }
        }
    };
    match derived {
        Ok(derived) => {
            let doc = add_comment(
                store,
                AddComment {
                    anchor: derived,
                    ..req
                },
            )?;
            Ok(added(doc, None))
        }
        Err(e) => {
            let reason = match on_failure {
                OnAnchorFailure::Refuse => None,
                OnAnchorFailure::WholeDocument => anchor_refusal_reason(&e),
            };
            let Some(reason) = reason else {
                return Err(e);
            };
            let intended_path = if is_change {
                anchor
                    .path
                    .and_then(|p| crate::change::normalize_source_path(p).ok())
            } else {
                None
            };
            let doc = add_comment(
                store,
                AddComment {
                    anchor: None,
                    anchor_state: Some(reason),
                    intended_path: intended_path.as_deref(),
                    ..req
                },
            )?;
            Ok(added(doc, Some(e.to_string())))
        }
    }
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
/// Returns [`Error::InvalidStoredChangeRevision`] for malformed resolved change
/// identities (head or present base must be 40 lowercase ASCII hex characters).
///
/// Returns [`Error::InvalidAppliedCommit`] if `applied_commit` is not exactly
/// 40 lowercase ASCII hex characters.
///
/// Returns [`Error::ReviewNotFound`] if the review doesn't exist,
/// [`Error::CommentNotFound`] if the comment id isn't in the review,
/// [`Error::ReviewNotDraft`] if a structural change is attempted after
/// submission, [`Error::ReviewNotSubmitted`] if any resolution field
/// (`status`, `applied_commit`, or `reply`) is changed while the review is
/// still a draft, [`Error::ReviewClosed`] if `status` is changed after the
/// review has reached a terminal state (`applied_commit` and `reply` remain
/// correctable there), [`Error::CommentDocOutOfScope`]/[`Error::CommentDocNotApplicable`]
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
    if let Some(sha) = applied_commit
        && !crate::model::is_full_commit_sha(sha)
    {
        return Err(Error::InvalidAppliedCommit(sha.to_string()));
    }
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
    let status_change = status.is_some();
    let provenance_change = applied_commit.is_some() || reply.is_some();
    if structure_change && review_doc.frontmatter.state != ReviewState::Draft {
        return Err(Error::ReviewNotDraft(review_id.to_string()));
    }
    if (status_change || provenance_change) && review_doc.frontmatter.state == ReviewState::Draft {
        return Err(Error::ReviewNotSubmitted(review_id.to_string()));
    }
    if status_change && review_doc.frontmatter.state != ReviewState::Submitted {
        return Err(Error::ReviewClosed {
            review_id: review_id.to_string(),
            state: review_doc.frontmatter.state,
        });
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
    // An anchored comment carries neither anchor-state field; a cleared
    // anchor is a deliberate whole-document comment.
    match anchor {
        AnchorUpdate::Keep => {}
        AnchorUpdate::Set(a) => {
            comment.anchor = Some(a);
            comment.anchor_state = None;
            comment.intended_path = None;
        }
        AnchorUpdate::Clear => {
            comment.anchor = None;
            comment.anchor_state = Some(AnchorState::WholeDocument);
            comment.intended_path = None;
        }
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
/// Returns [`Error::InvalidStoredChangeRevision`] for malformed resolved change
/// identities (head or present base must be 40 lowercase ASCII hex characters).
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
/// # Plan status derivation
///
/// When the review targets a [`ReviewTarget::Plan`], submitting it derives
/// the plan's status from the verdict rather than requiring a second write:
/// [`Verdict::Approve`] sets [`PlanStatus::Approved`],
/// [`Verdict::RequestChanges`] sets [`PlanStatus::ChangesRequested`], and
/// [`Verdict::Comment`] leaves the status alone. Two rules bound it, both
/// enforced by [`crate::ops::plan::set_plan_status`]:
///
/// - A [`PlanStatus::Superseded`] plan is **never downgraded** — reviewing a
///   stale plan and approving it leaves it superseded.
/// - The derived write is **non-fatal**: a plan that no longer exists is
///   skipped silently rather than failing the submit, the same tolerance the
///   `Done:` hook applies. The review is the durable record of the feedback;
///   a submit must never fail because a derived write could not land.
///
/// The derivation is idempotent — re-deriving the same status rewrites the
/// same value.
///
/// # Errors
///
/// Returns [`Error::InvalidStoredChangeRevision`] for malformed resolved change
/// identities (head or present base must be 40 lowercase ASCII hex characters).
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

    // Derive a plan's status from the verdict — see this function's
    // "Plan status derivation" section for the two rules that bound it.
    if let ReviewTarget::Plan { slug } = &review_doc.frontmatter.target {
        let derived = match verdict {
            Verdict::Approve => Some(PlanStatus::Approved),
            Verdict::RequestChanges => Some(PlanStatus::ChangesRequested),
            Verdict::Comment => None,
        };
        if let Some(status) = derived {
            // Non-fatal: a deleted plan target never fails the submit.
            let _ = crate::ops::plan::set_plan_status(store, project, slug, status);
        }
    }

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
/// Returns [`Error::InvalidStoredChangeRevision`] for malformed resolved change
/// identities (head or present base must be 40 lowercase ASCII hex characters).
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
/// Returns [`Error::InvalidStoredChangeRevision`] for malformed resolved change
/// identities (head or present base must be 40 lowercase ASCII hex characters).
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
    // `same_item`, not `==`: a `change/<sha>` reference parsed from the
    // command line carries no `base`, while the stored target normally does.
    filter
        .target
        .as_ref()
        .is_none_or(|t| review.target.same_item(t))
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
/// by the CLI (`rdm review requests`) and rdm-server so the two surfaces can
/// never disagree.
///
/// # Errors
///
/// Returns [`Error::InvalidStoredChangeRevision`] for malformed resolved change
/// identities (head or present base must be 40 lowercase ASCII hex characters).
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
/// Returns [`Error::InvalidStoredChangeRevision`] for malformed resolved change
/// identities (head or present base must be 40 lowercase ASCII hex characters).
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
/// Returns [`Error::InvalidStoredChangeRevision`] for malformed resolved change
/// identities (head or present base must be 40 lowercase ASCII hex characters).
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
/// Returns [`Error::InvalidStoredChangeRevision`] for malformed resolved change
/// identities (head or present base must be 40 lowercase ASCII hex characters).
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
/// Returns [`Error::InvalidStoredChangeRevision`] for malformed resolved change
/// identities (head or present base must be 40 lowercase ASCII hex characters).
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
            // Plans have no counts bucket: `OpenReviewCounts` backs the
            // server's roadmap/task list pages, and plan list pages are out
            // of scope until rdm-server gains real plan support.
            ReviewTarget::Plan { .. } => continue,
            // Likewise: a change review belongs to no roadmap or task page.
            ReviewTarget::Change { .. } => continue,
        };
        let entry = map.entry(key.clone()).or_default();
        entry.open_reviews += 1;
        entry.open_comments += open_comments;
    }
    counts
}

/// One source-repository commit a `change/<sha>` review references
/// structurally — its target's `head` or `base`, or a comment's
/// `applied_commit` — as found by [`review_commit_refs`].
///
/// The stored value is kept verbatim: the on-disk review format does not
/// change, and the `rdm:src@<sha>` commit link is derived from it on read
/// (see [`ReviewCommitRef::link`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewCommitRef {
    /// The ordinal id of the comment carrying the reference, or `None` for
    /// the target's `head`/`base`.
    pub comment: Option<u32>,
    /// The frontmatter field the value lives in: `"head"`, `"base"`, or
    /// `"applied_commit"`.
    pub field: &'static str,
    /// The stored value, verbatim.
    pub rev: String,
}

impl ReviewCommitRef {
    /// The `rdm:src@<rev>` commit link this reference derives —
    /// [`Link::Commit`](crate::link::Link::Commit) over [`Self::rev`].
    #[must_use]
    pub fn link(&self) -> crate::link::Link {
        crate::link::Link::Commit {
            rev: self.rev.clone(),
        }
    }
}

/// Every source-repository commit `review` references structurally, in a
/// fixed order: the target's `head`, then its `base` when present, then each
/// comment's `applied_commit` in comment order.
///
/// This is the single derivation behind `review show --format json`'s
/// `head_link`/`base_link`/`applied_commit_link` keys, `link check`'s
/// review coverage, and `rdm backlinks rdm:src@<sha>` — the stored format
/// keeps raw SHAs and every consumer derives the link from them here.
///
/// Rules:
///
/// - A review whose target is not [`ReviewTarget::Change`] yields nothing:
///   on a roadmap, phase, task, or plan review, `applied_commit` is a
///   **plan-repo** SHA, so it is never a source-repository commit link.
/// - A value becomes a reference only if `rdm:src@<value>` parses as a
///   commit link ([`crate::link::parse`]), so the grammar lives only in
///   [`crate::link`]. `head` and `base` always pass (they are validated as
///   full SHAs at load). A legacy `applied_commit` that is not commit-shaped
///   (e.g. `HEAD`, written before the value was validated) is skipped
///   silently — it was never written as a link. An abbreviated hex value is
///   kept.
#[must_use]
pub fn review_commit_refs(review: &Review) -> Vec<ReviewCommitRef> {
    let ReviewTarget::Change { head, base } = &review.target else {
        return Vec::new();
    };
    let candidates = std::iter::once((None, "head", head.as_str()))
        .chain(base.as_deref().map(|b| (None, "base", b)))
        .chain(review.comments.iter().filter_map(|c| {
            c.applied_commit
                .as_deref()
                .map(|sha| (Some(c.id), "applied_commit", sha))
        }));
    candidates
        .filter(|(_, _, rev)| {
            matches!(
                crate::link::parse(&format!("rdm:src@{rev}")),
                Ok(crate::link::Link::Commit { .. })
            )
        })
        .map(|(comment, field, rev)| ReviewCommitRef {
            comment,
            field,
            rev: rev.to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Project, ReviewState, ReviewTarget};
    use crate::ops::id::MAX_ID_ATTEMPTS;
    use crate::store::MemoryStore;
    use chrono::TimeZone;

    fn setup_store() -> MemoryStore {
        let mut store = MemoryStore::new();
        let project_doc = Document {
            frontmatter: Project {
                name: "test".to_string(),
                title: "Test Project".to_string(),
                source: None,
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
                implements: None,
                change_branch: None,
            },
            body: "Review summary.".to_string(),
        }
    }

    /// Rewrites the test project with a `source` block.
    fn with_source(store: &mut MemoryStore, repo: &str, default_branch: Option<&str>) {
        let doc = Document {
            frontmatter: Project {
                name: "test".to_string(),
                title: "Test Project".to_string(),
                source: Some(crate::model::Source {
                    repo: repo.to_string(),
                    default_branch: default_branch.map(str::to_string),
                }),
            },
            body: String::new(),
        };
        store
            .write(
                &crate::paths::project_md_path("test"),
                doc.render().unwrap(),
            )
            .unwrap();
    }

    #[test]
    fn source_default_branch_walks_project_then_config_then_main() {
        let mut store = setup_store();
        // No `source` block at all: the plan repo's config wins, else "main".
        assert_eq!(source_default_branch(&store, "test", None), "main");
        assert_eq!(
            source_default_branch(&store, "test", Some("trunk")),
            "trunk"
        );
        // A `source` block with no default_branch is still not an answer.
        with_source(&mut store, "/tmp/src", None);
        assert_eq!(
            source_default_branch(&store, "test", Some("trunk")),
            "trunk"
        );
        // The project's own default_branch outranks the config's.
        with_source(&mut store, "/tmp/src", Some("develop"));
        assert_eq!(
            source_default_branch(&store, "test", Some("trunk")),
            "develop"
        );
    }

    #[test]
    fn source_default_branch_degrades_for_an_unknown_project() {
        let store = setup_store();
        assert_eq!(
            source_default_branch(&store, "nope", Some("trunk")),
            "trunk"
        );
        assert_eq!(source_default_branch(&store, "nope", None), "main");
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
            implements: None,
            change_branch: None,
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
            let message = err.to_string();
            // The full accepted-forms enumeration, including `plan/<slug>`
            // and `change/<head-sha>` (added by commit 8b3c1a2) — asserting
            // only the pre-existing `roadmap/<slug>` substring would stay
            // green even if those two forms were dropped from the Display
            // text again, since that substring predates the fix.
            for form in [
                "roadmap/<slug>",
                "phase/<roadmap-slug>/<stem-or-number>",
                "task/<slug>",
                "plan/<slug>",
                "change/<head-sha>",
            ] {
                assert!(
                    message.contains(form),
                    "error must show accepted form {form:?}: {message}"
                );
            }
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
                implements: None,
                change_branch: None,
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
                implements: None,
                change_branch: None,
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
                implements: None,
                change_branch: None,
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

    // -- AC2: plan status derived from the review verdict --

    /// Seeds a plan (implementing the fixture task) and returns its slug.
    fn seed_plan(store: &mut MemoryStore, slug: &str) -> String {
        crate::ops::plan::create_plan(
            store,
            crate::ops::plan::CreatePlan {
                project: "test",
                slug,
                title: "A plan",
                implements: ReviewTarget::Task {
                    slug: "fix-login".to_string(),
                },
                supersedes: None,
                body: Some("The approach.\n"),
            },
        )
        .unwrap();
        slug.to_string()
    }

    /// Starts and submits a review on `plan/<slug>` with `verdict`, returning
    /// the plan's resulting status.
    fn submit_on_plan(
        store: &mut MemoryStore,
        plan_slug: &str,
        verdict: Verdict,
    ) -> crate::model::PlanStatus {
        let doc = create_review(
            store,
            CreateReview {
                project: "test",
                author: "ed",
                target: ReviewTarget::Plan {
                    slug: plan_slug.to_string(),
                },
                body: Some("Looks reasonable."),
                implements: None,
                change_branch: None,
            },
        )
        .unwrap();
        let id = doc.frontmatter.id.clone();
        submit_review(store, "test", &id, Some(verdict)).unwrap();
        crate::io::load_plan(store, "test", plan_slug)
            .unwrap()
            .frontmatter
            .status
    }

    #[test]
    fn submit_approve_flips_plan_to_approved() {
        let mut store = setup_store_with_items();
        seed_plan(&mut store, "impl-login");
        assert_eq!(
            submit_on_plan(&mut store, "impl-login", Verdict::Approve),
            PlanStatus::Approved
        );
    }

    /// The orchestrator's plan-approval wait is a single origin-blind
    /// `rdm plan show` read (see `.claude/skills/rdm-dispatch-phase/SKILL.md`):
    /// a human's approve review and a review-workflow-persisted one must be the
    /// same write, so nothing downstream can branch on who authored it.
    #[test]
    fn submit_approve_flips_plan_to_approved_whoever_authored_it() {
        for author in ["ed", "rdm-wf-plan-review", "someone-else"] {
            let mut store = setup_store_with_items();
            seed_plan(&mut store, "impl-login");
            let doc = create_review(
                &mut store,
                CreateReview {
                    project: "test",
                    author,
                    target: ReviewTarget::Plan {
                        slug: "impl-login".to_string(),
                    },
                    body: Some("Looks reasonable."),
                    implements: None,
                    change_branch: None,
                },
            )
            .unwrap();
            submit_review(
                &mut store,
                "test",
                &doc.frontmatter.id,
                Some(Verdict::Approve),
            )
            .unwrap();
            assert_eq!(
                crate::io::load_plan(&store, "test", "impl-login")
                    .unwrap()
                    .frontmatter
                    .status,
                PlanStatus::Approved,
                "author {author} should flip the plan the same way"
            );
        }
    }

    #[test]
    fn submit_request_changes_flips_plan_to_changes_requested() {
        let mut store = setup_store_with_items();
        seed_plan(&mut store, "impl-login");
        assert_eq!(
            submit_on_plan(&mut store, "impl-login", Verdict::RequestChanges),
            PlanStatus::ChangesRequested
        );
    }

    #[test]
    fn submit_comment_leaves_plan_status() {
        let mut store = setup_store_with_items();
        seed_plan(&mut store, "impl-login");
        assert_eq!(
            submit_on_plan(&mut store, "impl-login", Verdict::Comment),
            PlanStatus::Draft
        );
    }

    #[test]
    fn submit_on_superseded_plan_does_not_downgrade() {
        let mut store = setup_store_with_items();
        seed_plan(&mut store, "impl-v1");
        crate::ops::plan::create_plan(
            &mut store,
            crate::ops::plan::CreatePlan {
                project: "test",
                slug: "impl-v2",
                title: "Second attempt",
                implements: ReviewTarget::Task {
                    slug: "fix-login".to_string(),
                },
                supersedes: Some(ReviewTarget::Plan {
                    slug: "impl-v1".to_string(),
                }),
                body: None,
            },
        )
        .unwrap();
        // Reviewing the stale v1 and approving it must not resurrect it.
        assert_eq!(
            submit_on_plan(&mut store, "impl-v1", Verdict::Approve),
            PlanStatus::Superseded
        );
    }

    #[test]
    fn submit_on_deleted_plan_still_succeeds() {
        let mut store = setup_store_with_items();
        seed_plan(&mut store, "impl-login");
        let doc = create_review(
            &mut store,
            CreateReview {
                project: "test",
                author: "ed",
                target: ReviewTarget::Plan {
                    slug: "impl-login".to_string(),
                },
                body: Some("Looks reasonable."),
                implements: None,
                change_branch: None,
            },
        )
        .unwrap();
        let id = doc.frontmatter.id.clone();
        crate::ops::plan::delete_plan(&mut store, "test", "impl-login").unwrap();
        // The review is the durable record: a derived write that cannot land
        // never fails the submit.
        let submitted = submit_review(&mut store, "test", &id, Some(Verdict::Approve)).unwrap();
        assert_eq!(submitted.frontmatter.state, ReviewState::Submitted);
        assert_eq!(submitted.frontmatter.verdict, Some(Verdict::Approve));
    }

    #[test]
    fn review_start_on_a_missing_plan_reports_a_missing_target() {
        let mut store = setup_store_with_items();
        let err = create_review(
            &mut store,
            CreateReview {
                project: "test",
                author: "ed",
                target: ReviewTarget::Plan {
                    slug: "no-such-plan".to_string(),
                },
                body: Some("summary"),
                implements: None,
                change_branch: None,
            },
        )
        .unwrap_err();
        match err {
            Error::ReviewTargetMissing(msg) => assert!(msg.contains("plan"), "{msg}"),
            other => panic!("expected ReviewTargetMissing, got {other:?}"),
        }
    }

    #[test]
    fn comment_doc_is_not_applicable_on_a_plan_review() {
        let mut store = setup_store_with_items();
        seed_plan(&mut store, "impl-login");
        let doc = create_review(
            &mut store,
            CreateReview {
                project: "test",
                author: "ed",
                target: ReviewTarget::Plan {
                    slug: "impl-login".to_string(),
                },
                body: Some("summary"),
                implements: None,
                change_branch: None,
            },
        )
        .unwrap();
        let err = add_comment(
            &mut store,
            AddComment {
                project: "test",
                review_id: &doc.frontmatter.id,
                body: "Scoped feedback.",
                doc: Some(CommentDoc {
                    kind: CommentDocKind::Phase,
                    stem: "phase-1-one".to_string(),
                }),
                anchor: None,
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(matches!(err, Error::CommentDocNotApplicable), "{err:?}");
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
                anchor_state: None,
                intended_path: None,
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
        // the web handlers may enter through either.
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
    #[test]
    fn malformed_change_identity_is_matchable_across_all_review_consumers() {
        for field in ["head", "base"] {
            for value in [
                "",
                "abc123",
                "HEAD",
                "--output=elsewhere",
                "é",
                "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
                "gggggggggggggggggggggggggggggggggggggggg",
                " aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "HEAD~1",
            ] {
                let mut store = setup_store();
                crate::io::write_review(&mut store, "test", "valid", &sample_review("valid"))
                    .unwrap();
                let mut doc = sample_review("bad");
                doc.frontmatter.target = ReviewTarget::Change {
                    head: if field == "head" {
                        value.into()
                    } else {
                        "a".repeat(40)
                    },
                    base: Some(if field == "base" {
                        value.into()
                    } else {
                        "b".repeat(40)
                    }),
                };
                let is_expected = |err: Error| {
                    assert!(
                        matches!(err, Error::InvalidStoredChangeRevision { field: f, value: v } if f == field && v == value)
                    );
                };
                is_expected(crate::io::write_review(&mut store, "test", "bad", &doc).unwrap_err());
                assert!(!store.exists(&crate::paths::review_path("test", "bad")));
                // Structural serialization/deserialization deliberately accepts it;
                // the trusted loading boundary must reject it consistently.
                store
                    .write(
                        &crate::paths::review_path("test", "bad"),
                        doc.render().unwrap(),
                    )
                    .unwrap();
                is_expected(crate::io::load_review(&store, "test", "bad").unwrap_err());
                is_expected(get_review(&store, "test", "bad").unwrap_err());
                is_expected(list_reviews(&store, "test").unwrap_err());
                is_expected(change_requests(&store, "test").unwrap_err());
                is_expected(count_open_reviews(&store, "test").unwrap_err());
                is_expected(
                    crate::search::search(
                        &store,
                        "Review",
                        &crate::search::SearchFilter::default(),
                    )
                    .unwrap_err(),
                );
                is_expected(
                    add_comment(
                        &mut store,
                        AddComment {
                            project: "test",
                            review_id: "bad",
                            body: "x",
                            doc: None,
                            anchor: None,
                            ..Default::default()
                        },
                    )
                    .unwrap_err(),
                );
                is_expected(
                    update_comment(
                        &mut store,
                        UpdateComment {
                            project: "test",
                            review_id: "bad",
                            comment_id: 1,
                            body: None,
                            anchor: AnchorUpdate::Keep,
                            doc: DocUpdate::Keep,
                            status: None,
                            applied_commit: None,
                            reply: None,
                        },
                    )
                    .unwrap_err(),
                );
                is_expected(remove_comment(&mut store, "test", "bad", 1).unwrap_err());
                is_expected(
                    submit_review(&mut store, "test", "bad", Some(Verdict::Approve)).unwrap_err(),
                );
                is_expected(
                    set_summary(
                        &mut store,
                        "test",
                        "bad",
                        crate::ops::BodyUpdate::Set("x".into()),
                    )
                    .unwrap_err(),
                );
                is_expected(
                    update_review(&mut store, "test", "bad", ReviewTransition::Dismissed)
                        .unwrap_err(),
                );
                is_expected(delete_review(&mut store, "test", "bad", true).unwrap_err());
                is_expected(
                    create_review(
                        &mut store,
                        CreateReview {
                            project: "test",
                            author: "test",
                            target: doc.frontmatter.target,
                            body: None,
                            implements: None,
                            change_branch: None,
                        },
                    )
                    .unwrap_err(),
                );
            }
        }
    }

    #[test]
    fn syntactic_change_identities_round_trip_offline_including_absent_base() {
        let mut store = setup_store();
        for base in [None, Some("b".repeat(40))] {
            let mut doc = sample_review("valid");
            doc.body.push('\n');
            doc.frontmatter.target = ReviewTarget::Change {
                head: "a".repeat(40),
                base,
            };
            crate::io::write_review(&mut store, "test", "valid", &doc).unwrap();
            assert_eq!(get_review(&store, "test", "valid").unwrap(), doc);
        }
        // Deleted non-change targets still load.
        crate::io::write_review(&mut store, "test", "dangling", &sample_review("dangling"))
            .unwrap();
        assert!(get_review(&store, "test", "dangling").is_ok());
        for rev in ["HEAD", "abc123", "topic"] {
            let target: ReviewTarget = format!("change/{rev}").parse().unwrap();
            assert!(matches!(target, ReviewTarget::Change { head, base: None } if head == rev));
        }
    }

    struct NoPlanAccess;
    impl Store for NoPlanAccess {
        fn read(&self, _: &crate::store::RelPath) -> Result<String> {
            panic!("unexpected read")
        }
        fn exists(&self, _: &crate::store::RelPath) -> bool {
            panic!("unexpected exists")
        }
        fn list(&self, _: &crate::store::RelPath) -> Result<Vec<crate::store::DirEntry>> {
            panic!("unexpected list")
        }
        fn write(&mut self, _: &crate::store::RelPath, _: String) -> Result<()> {
            panic!("unexpected write")
        }
        fn delete(&mut self, _: &crate::store::RelPath) -> Result<()> {
            panic!("unexpected delete")
        }
        fn commit(&mut self) -> Result<()> {
            panic!("unexpected commit")
        }
        fn discard(&mut self) {
            panic!("unexpected discard")
        }
    }
    impl VersionedStore for NoPlanAccess {
        fn head_sha(&self) -> Result<String> {
            panic!("unexpected plan Git inspection")
        }
        fn fetch_body_at(&self, _: &crate::store::RelPath, _: &str) -> Result<String> {
            panic!("unexpected history")
        }
    }

    #[test]
    fn invalid_creation_precedes_even_plan_store_inspection() {
        for (head, base, field) in [
            ("HEAD".into(), Some("b".repeat(40)), "head"),
            ("a".repeat(40), Some("--all".into()), "base"),
        ] {
            assert!(matches!(create_review(&mut NoPlanAccess, CreateReview {
                project: "test", author: "test", target: ReviewTarget::Change { head, base },
                body: None, implements: None, change_branch: None,
            }), Err(Error::InvalidStoredChangeRevision { field: f, .. }) if f == field));
        }
    }

    // --- review_commit_refs: derive-on-read source commit references ---

    fn comment_with(id: u32, applied_commit: Option<&str>) -> ReviewComment {
        ReviewComment {
            anchor_state: None,
            intended_path: None,
            id,
            doc: None,
            status: if applied_commit.is_some() {
                ReviewCommentStatus::Addressed
            } else {
                ReviewCommentStatus::Open
            },
            applied_commit: applied_commit.map(str::to_string),
            anchor: None,
            body: "Comment.".to_string(),
            reply: None,
        }
    }

    fn change_review(base: Option<&str>, comments: Vec<ReviewComment>) -> Review {
        let mut review = sample_review("2026-07-01-1430-a1b2").frontmatter;
        review.target = ReviewTarget::Change {
            head: "a".repeat(40),
            base: base.map(str::to_string),
        };
        review.comments = comments;
        review
    }

    #[test]
    fn review_commit_refs_yields_head_base_then_applied_commits_in_order() {
        let base = "b".repeat(40);
        let applied = "c".repeat(40);
        let review = change_review(
            Some(&base),
            vec![comment_with(1, None), comment_with(2, Some(&applied))],
        );
        let refs = review_commit_refs(&review);
        assert_eq!(
            refs,
            vec![
                ReviewCommitRef {
                    comment: None,
                    field: "head",
                    rev: "a".repeat(40),
                },
                ReviewCommitRef {
                    comment: None,
                    field: "base",
                    rev: base.clone(),
                },
                ReviewCommitRef {
                    comment: Some(2),
                    field: "applied_commit",
                    rev: applied.clone(),
                },
            ]
        );
        assert_eq!(
            refs[2].link(),
            crate::link::Link::Commit {
                rev: applied.clone()
            }
        );
        assert_eq!(refs[2].link().to_string(), format!("rdm:src@{applied}"));
    }

    #[test]
    fn review_commit_refs_omits_an_absent_base() {
        let review = change_review(None, vec![]);
        let refs = review_commit_refs(&review);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].field, "head");
    }

    #[test]
    fn review_commit_refs_is_empty_for_a_plan_document_review() {
        let mut review = sample_review("2026-07-01-1430-a1b2").frontmatter;
        review.comments = vec![comment_with(1, Some(&"d".repeat(40)))];
        assert!(review_commit_refs(&review).is_empty());
    }

    #[test]
    fn review_commit_refs_skips_legacy_non_hex_and_keeps_abbreviated() {
        let review = change_review(
            None,
            vec![
                comment_with(1, Some("abc1234def56")),
                comment_with(2, Some("HEAD")),
            ],
        );
        let refs = review_commit_refs(&review);
        assert_eq!(refs.len(), 2, "{refs:?}");
        assert_eq!(refs[1].comment, Some(1));
        assert_eq!(refs[1].rev, "abc1234def56");
        assert!(refs.iter().all(|r| r.rev != "HEAD"));
    }

    // --- anchor state, intended path, and the anchor fall-back ---

    const HEAD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const BASE: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    /// A draft `change/` review stored under `id`, over [`anchor_source`].
    fn store_with_change_review(id: &str) -> MemoryStore {
        let mut store = setup_store();
        let mut doc = sample_review(id);
        doc.frontmatter.target = ReviewTarget::Change {
            head: HEAD.to_string(),
            base: Some(BASE.to_string()),
        };
        crate::io::write_review(&mut store, "test", id, &doc).unwrap();
        store
    }

    /// A two-line file at `HEAD` whose second line the change touched.
    fn anchor_source() -> crate::source::MemorySourceRepo {
        crate::source::MemorySourceRepo::new()
            .with_rev(HEAD, HEAD)
            .with_file(HEAD, "src/lib.rs", "fn untouched() {}\nfn touched() {}\n")
            .with_diff(
                BASE,
                HEAD,
                "src/lib.rs",
                "@@ -2 +2 @@\n-old\n+fn touched() {}\n",
            )
    }

    fn only_comment(doc: &Document<Review>) -> &ReviewComment {
        assert_eq!(doc.frontmatter.comments.len(), 1);
        &doc.frontmatter.comments[0]
    }

    #[test]
    fn add_comment_stamps_whole_document_on_an_anchorless_comment() {
        let mut store = setup_store_with_items();
        let doc = sample_review("r1");
        crate::io::write_review(&mut store, "test", "r1", &doc).unwrap();
        let doc = add_comment(
            &mut store,
            AddComment {
                project: "test",
                review_id: "r1",
                body: "Overall.",
                ..Default::default()
            },
        )
        .unwrap();
        let c = only_comment(&doc);
        assert_eq!(c.anchor_state, Some(AnchorState::WholeDocument));
        assert_eq!(c.intended_path, None);
    }

    #[test]
    fn add_comment_rejects_an_anchor_with_a_state_or_path() {
        let mut store = store_with_change_review("r1");
        let anchor = Some(Anchor::TextQuote {
            quote: "q".to_string(),
            prefix: String::new(),
            suffix: String::new(),
        });
        for (state, path) in [
            (Some(AnchorState::OutsideHunk), None),
            (None, Some("src/lib.rs")),
        ] {
            let err = add_comment(
                &mut store,
                AddComment {
                    project: "test",
                    review_id: "r1",
                    body: "b",
                    anchor: anchor.clone(),
                    anchor_state: state,
                    intended_path: path,
                    ..Default::default()
                },
            )
            .unwrap_err();
            assert!(matches!(err, Error::CommentAnchorStateWithAnchor), "{err}");
        }
    }

    #[test]
    fn add_comment_rejects_an_intended_path_on_a_task_review() {
        let mut store = setup_store_with_items();
        let doc = sample_review("r1");
        crate::io::write_review(&mut store, "test", "r1", &doc).unwrap();
        let err = add_comment(
            &mut store,
            AddComment {
                project: "test",
                review_id: "r1",
                body: "b",
                intended_path: Some("src/lib.rs"),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(
            matches!(&err, Error::CommentPathNotApplicable(label) if label == "task/fix-login"),
            "{err}"
        );
    }

    #[test]
    fn add_comment_stores_a_normalized_intended_path_with_its_reason() {
        let mut store = store_with_change_review("r1");
        let doc = add_comment(
            &mut store,
            AddComment {
                project: "test",
                review_id: "r1",
                body: "b",
                anchor_state: Some(AnchorState::PathMissing),
                intended_path: Some("./src/x.rs"),
                ..Default::default()
            },
        )
        .unwrap();
        let c = only_comment(&doc);
        assert_eq!(c.anchor_state, Some(AnchorState::PathMissing));
        assert_eq!(c.intended_path.as_deref(), Some("src/x.rs"));
    }

    #[test]
    fn update_comment_set_clears_and_clear_restamps_the_anchor_state() {
        let mut store = store_with_change_review("r1");
        add_comment(
            &mut store,
            AddComment {
                project: "test",
                review_id: "r1",
                body: "b",
                anchor_state: Some(AnchorState::OutsideHunk),
                intended_path: Some("src/lib.rs"),
                ..Default::default()
            },
        )
        .unwrap();
        let anchor = Anchor::FileQuote {
            path: "src/lib.rs".to_string(),
            quote: "fn touched() {}".to_string(),
            occurrence: 1,
            start_line: 2,
            end_line: 2,
        };
        let doc = update_comment(
            &mut store,
            UpdateComment {
                project: "test",
                review_id: "r1",
                comment_id: 1,
                anchor: AnchorUpdate::Set(anchor),
                ..Default::default()
            },
        )
        .unwrap();
        let c = only_comment(&doc);
        assert!(c.anchor.is_some());
        assert_eq!(
            (c.anchor_state.clone(), c.intended_path.clone()),
            (None, None)
        );
        let doc = update_comment(
            &mut store,
            UpdateComment {
                project: "test",
                review_id: "r1",
                comment_id: 1,
                anchor: AnchorUpdate::Clear,
                ..Default::default()
            },
        )
        .unwrap();
        let c = only_comment(&doc);
        assert_eq!(c.anchor, None);
        assert_eq!(c.anchor_state, Some(AnchorState::WholeDocument));
        assert_eq!(c.intended_path, None);
    }

    #[test]
    fn anchor_refusal_reason_maps_exactly_the_anchor_refusals() {
        let cases = [
            (
                Error::QuoteNotFound {
                    quote: "q".to_string(),
                    commit: None,
                },
                AnchorState::QuoteNotFound,
            ),
            (
                Error::QuoteAmbiguous {
                    quote: "q".to_string(),
                    commit: None,
                    occurrences: vec![],
                },
                AnchorState::Ambiguous,
            ),
            (
                Error::QuoteOccurrenceOutOfRange {
                    quote: "q".to_string(),
                    occurrence: 3,
                    available: 1,
                },
                AnchorState::OccurrenceOutOfRange,
            ),
            (
                Error::QuoteOutsideChangedHunks {
                    path: "p".to_string(),
                    start_line: 1,
                    end_line: 1,
                    nearest: None,
                    range: "r".to_string(),
                },
                AnchorState::OutsideHunk,
            ),
            (
                Error::ChangePathNotInRevision {
                    path: "p".to_string(),
                    rev: HEAD.to_string(),
                },
                AnchorState::PathMissing,
            ),
            (
                Error::ChangePathNotAFile {
                    path: "p".to_string(),
                    rev: HEAD.to_string(),
                    found: crate::source::SourceObjectKind::Tree,
                },
                AnchorState::PathNotAFile,
            ),
            (
                Error::ChangePathNotLinkable {
                    path: "a@b".to_string(),
                    delimiter: '@',
                },
                AnchorState::PathNotLinkable,
            ),
            (
                Error::ChangeHeadNotInSource {
                    head: HEAD.to_string(),
                    location: None,
                },
                AnchorState::HeadNotInSource,
            ),
        ];
        for (err, want) in cases {
            assert_eq!(anchor_refusal_reason(&err), Some(want), "{err}");
        }
        for not_a_refusal in [
            Error::ChangeQuoteNeedsPath,
            Error::ChangePathNeedsQuote,
            Error::ChangeAnchorNeedsSource,
            Error::CommentPathNotApplicable("task/t".to_string()),
            Error::ReviewNotDraft("r".to_string()),
            Error::Git("boom".to_string()),
        ] {
            assert_eq!(
                anchor_refusal_reason(&not_a_refusal),
                None,
                "{not_a_refusal}"
            );
        }
    }

    fn fall_back(
        store: &mut MemoryStore,
        source: &crate::source::MemorySourceRepo,
        quote: Option<&str>,
        path: Option<&str>,
        mode: OnAnchorFailure,
    ) -> Result<AddedComment> {
        add_comment_with_anchor(
            store,
            Some(source),
            AddComment {
                project: "test",
                review_id: "r1",
                body: "finding",
                ..Default::default()
            },
            AnchorRequest {
                quote,
                path,
                occurrence: None,
            },
            mode,
        )
    }

    #[test]
    fn add_comment_with_anchor_anchors_when_core_accepts() {
        let mut store = store_with_change_review("r1");
        let added = fall_back(
            &mut store,
            &anchor_source(),
            Some("fn touched() {}"),
            Some("src/lib.rs"),
            OnAnchorFailure::WholeDocument,
        )
        .unwrap();
        assert_eq!(added.comment_id, 1);
        assert_eq!(added.refusal, None);
        let c = only_comment(&added.doc);
        assert!(matches!(c.anchor, Some(Anchor::FileQuote { .. })));
        assert_eq!(
            (c.anchor_state.clone(), c.intended_path.clone()),
            (None, None)
        );
    }

    #[test]
    fn add_comment_with_anchor_stores_an_outside_hunk_refusal_with_its_path() {
        let mut store = store_with_change_review("r1");
        let added = fall_back(
            &mut store,
            &anchor_source(),
            Some("fn untouched() {}"),
            Some("./src/lib.rs"),
            OnAnchorFailure::WholeDocument,
        )
        .unwrap();
        assert!(added.refusal.is_some());
        let c = only_comment(&added.doc);
        assert_eq!(c.anchor, None);
        assert_eq!(c.anchor_state, Some(AnchorState::OutsideHunk));
        assert_eq!(c.intended_path.as_deref(), Some("src/lib.rs"));
        // The persisted review agrees with the returned document.
        let stored = crate::io::load_review(&store, "test", "r1").unwrap();
        assert_eq!(only_comment(&stored), c);
    }

    #[test]
    fn add_comment_with_anchor_stores_a_quote_not_found_refusal() {
        let mut store = store_with_change_review("r1");
        let added = fall_back(
            &mut store,
            &anchor_source(),
            Some("absent text"),
            Some("src/lib.rs"),
            OnAnchorFailure::WholeDocument,
        )
        .unwrap();
        let c = only_comment(&added.doc);
        assert_eq!(c.anchor_state, Some(AnchorState::QuoteNotFound));
        assert_eq!(c.intended_path.as_deref(), Some("src/lib.rs"));
        assert!(added.refusal.unwrap().contains("absent text"));
    }

    #[test]
    fn add_comment_with_anchor_does_not_store_an_unlinkable_path() {
        let mut store = store_with_change_review("r1");
        let added = fall_back(
            &mut store,
            &anchor_source(),
            Some("fn touched() {}"),
            Some("src/a@b.rs"),
            OnAnchorFailure::WholeDocument,
        )
        .unwrap();
        let c = only_comment(&added.doc);
        assert_eq!(c.anchor_state, Some(AnchorState::PathNotLinkable));
        assert_eq!(c.intended_path, None);
    }

    #[test]
    fn add_comment_with_anchor_propagates_a_usage_error_even_in_fall_back_mode() {
        let mut store = store_with_change_review("r1");
        let err = fall_back(
            &mut store,
            &anchor_source(),
            Some("fn touched() {}"),
            None,
            OnAnchorFailure::WholeDocument,
        )
        .unwrap_err();
        assert!(matches!(err, Error::ChangeQuoteNeedsPath), "{err}");
        let stored = crate::io::load_review(&store, "test", "r1").unwrap();
        assert!(stored.frontmatter.comments.is_empty(), "nothing is written");
    }

    #[test]
    fn add_comment_with_anchor_refuse_mode_propagates_the_refusal() {
        let mut store = store_with_change_review("r1");
        let err = fall_back(
            &mut store,
            &anchor_source(),
            Some("fn untouched() {}"),
            Some("src/lib.rs"),
            OnAnchorFailure::Refuse,
        )
        .unwrap_err();
        assert!(
            matches!(err, Error::QuoteOutsideChangedHunks { .. }),
            "{err}"
        );
        let stored = crate::io::load_review(&store, "test", "r1").unwrap();
        assert!(stored.frontmatter.comments.is_empty(), "nothing is written");
    }

    #[test]
    fn add_comment_with_anchor_needs_a_source_for_a_change_anchor() {
        let mut store = store_with_change_review("r1");
        let err = add_comment_with_anchor::<crate::source::MemorySourceRepo>(
            &mut store,
            None,
            AddComment {
                project: "test",
                review_id: "r1",
                body: "b",
                ..Default::default()
            },
            AnchorRequest {
                quote: Some("q"),
                path: Some("src/lib.rs"),
                occurrence: None,
            },
            OnAnchorFailure::WholeDocument,
        )
        .unwrap_err();
        assert!(matches!(err, Error::ChangeAnchorNeedsSource), "{err}");
    }

    #[test]
    fn add_comment_with_anchor_falls_back_on_a_plan_document_quote() {
        let mut store = setup_store_with_items();
        let doc = sample_review("r1");
        crate::io::write_review(&mut store, "test", "r1", &doc).unwrap();
        let added = add_comment_with_anchor::<crate::source::MemorySourceRepo>(
            &mut store,
            None,
            AddComment {
                project: "test",
                review_id: "r1",
                body: "b",
                ..Default::default()
            },
            AnchorRequest {
                quote: Some("absent"),
                path: None,
                occurrence: None,
            },
            OnAnchorFailure::WholeDocument,
        )
        .unwrap();
        let c = only_comment(&added.doc);
        assert_eq!(c.anchor_state, Some(AnchorState::QuoteNotFound));
        assert_eq!(c.intended_path, None, "a plan document has no source path");
    }

    #[test]
    fn on_anchor_failure_parses_both_modes_and_rejects_others() {
        assert_eq!(
            "refuse".parse::<OnAnchorFailure>().unwrap(),
            OnAnchorFailure::Refuse
        );
        assert_eq!(
            "whole-document".parse::<OnAnchorFailure>().unwrap(),
            OnAnchorFailure::WholeDocument
        );
        let err = "ignore".parse::<OnAnchorFailure>().unwrap_err().to_string();
        assert!(
            err.contains("refuse") && err.contains("whole-document"),
            "{err}"
        );
    }
}

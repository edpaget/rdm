//! Shared assembly of the detail pages' Reviews sections.
//!
//! The roadmap, phase, and task detail handlers all render the same thing:
//! every non-draft review of the page's document, each comment resolved via
//! `rdm_core::anchor::resolve_comments`, plus the inline highlight spans to
//! thread through `render_markdown_with_highlights`. This module is the one
//! implementation of that sequence; the handlers only differ in the
//! [`PageDoc`](crate::review_views::PageDoc) they pass (which also drives the cross-document links between
//! roadmap reviews and the phases their `doc`-scoped comments point at).

use crate::markdown::{HighlightSpan, render_markdown};
use crate::templates::{
    DocLink, DocOption, DraftCommentView, DraftPanelView, DraftReviewView, ReviewCommentView,
    ReviewView, comment_status_class, comment_status_label, relative_time, review_state_class,
    review_state_label, verdict_class, verdict_label,
};
use rdm_core::anchor::Resolution;
use rdm_core::document::Document;
use rdm_core::model::{
    Anchor, CommentDoc, CommentDocKind, Phase, Review, ReviewComment, ReviewState, ReviewTarget,
};

/// Detail-page path for a review target (the page its reviews render on).
///
/// The one implementation of "target → detail href", shared by the JSON
/// API's `target` link relation and the form handlers' redirect
/// destinations.
#[must_use]
pub fn target_detail_href(project: &str, target: &ReviewTarget) -> String {
    match target {
        ReviewTarget::Roadmap { roadmap } => format!("/projects/{project}/roadmaps/{roadmap}"),
        ReviewTarget::Phase { roadmap, stem } => {
            format!("/projects/{project}/roadmaps/{roadmap}/phases/{stem}")
        }
        ReviewTarget::Task { slug } => format!("/projects/{project}/tasks/{slug}"),
    }
}

/// The document a detail page renders, deciding which reviews it shows,
/// which comments highlight inline in its body, and which link elsewhere.
pub enum PageDoc<'a> {
    /// A roadmap detail page. `phases` (the roadmap's phase list, as
    /// already loaded by the handler) labels the cross-links for
    /// `doc`-scoped comments.
    Roadmap {
        /// Roadmap slug.
        roadmap: &'a str,
        /// The roadmap's phases, for cross-link labels.
        phases: &'a [(String, Document<Phase>)],
    },
    /// A phase detail page.
    Phase {
        /// Parent roadmap slug.
        roadmap: &'a str,
        /// Phase file stem.
        stem: &'a str,
    },
    /// A task detail page.
    Task {
        /// Task slug.
        slug: &'a str,
    },
}

/// The assembled Reviews section for one detail page.
pub struct PageReviews {
    /// Non-draft reviews to render, oldest first.
    pub reviews: Vec<ReviewView>,
    /// Inline highlight spans (byte ranges into the page document's
    /// **current** markdown body) for
    /// [`render_markdown_with_highlights`](crate::markdown::render_markdown_with_highlights).
    /// Empty when highlighting was disabled.
    pub highlights: Vec<HighlightSpan>,
}

impl PageReviews {
    /// Renders the page document's markdown `body` to HTML.
    ///
    /// The two instrumentation modes are exclusive: with `annotate` set
    /// (the viewer has an open draft, so the select-to-anchor gesture is
    /// live) the body renders through
    /// [`render_markdown_annotated`](crate::markdown::render_markdown_annotated)
    /// and the collected highlight spans are skipped; otherwise resolved
    /// review anchors highlight inline as before (plain
    /// [`render_markdown`] when there are none).
    #[must_use]
    pub fn render_body(&self, body: &str, annotate: bool) -> String {
        if annotate {
            crate::markdown::render_markdown_annotated(body)
        } else if self.highlights.is_empty() {
            render_markdown(body)
        } else {
            crate::markdown::render_markdown_with_highlights(body, &self.highlights)
        }
    }
}

/// Loads, filters, resolves, and shapes every review shown on a detail page.
///
/// Drafts are always excluded. Reviews are ordered oldest-first by their
/// `created` timestamp. `allow_highlight` disables inline-highlight
/// collection (used when the page renders a historical `?at=` body, whose
/// bytes the current-body resolution ranges do not index) — quote previews
/// still render.
///
/// # Errors
///
/// Propagates `rdm_core::ops::reviews::list_reviews` failures (project not
/// found, unreadable reviews directory, malformed review file).
pub fn page_reviews(
    store: &impl rdm_core::store::VersionedStore,
    project: &str,
    page: &PageDoc<'_>,
    allow_highlight: bool,
) -> Result<PageReviews, rdm_core::error::Error> {
    let all = rdm_core::ops::reviews::list_reviews(store, project)?;
    let mut selected: Vec<(String, Document<Review>)> = all
        .into_iter()
        .filter(|(_, doc)| {
            doc.frontmatter.state != ReviewState::Draft && review_applies(page, &doc.frontmatter)
        })
        .collect();
    selected.sort_by_key(|(_, doc)| doc.frontmatter.created);

    let mut reviews = Vec::with_capacity(selected.len());
    let mut highlights = Vec::new();
    for (id, doc) in &selected {
        let review = &doc.frontmatter;
        let resolutions = rdm_core::anchor::resolve_comments(store, project, review);
        let mut comments = Vec::new();
        for (comment, resolved) in review.comments.iter().zip(&resolutions) {
            if !comment_shown(page, review, comment) {
                continue;
            }
            let anchor_ref = format!("{id}-c{}", comment.id);
            let on_page_body = comment_targets_page_body(page, review, comment);

            let (quote_text, quote_highlightable, outdated) = match &comment.anchor {
                None => (None, false, false),
                Some(anchor) => match &resolved.resolution {
                    Resolution::Current { range } => {
                        let highlightable = on_page_body && allow_highlight;
                        if highlightable {
                            highlights.push(HighlightSpan {
                                range: range.clone(),
                                anchor_ref: anchor_ref.clone(),
                            });
                        }
                        (resolved.quote.clone(), highlightable, false)
                    }
                    // Resolved against the historical body: the range does
                    // not index the current body, so no inline highlight;
                    // drift decides the badge.
                    Resolution::Original { drifted, .. } => {
                        (resolved.quote.clone(), false, *drifted)
                    }
                    // Nothing resolvable: fall back to the quote stored in
                    // the anchor itself (what the reviewer originally
                    // selected).
                    Resolution::Unresolved => (stored_quote(anchor), false, true),
                },
            };

            comments.push(ReviewCommentView {
                anchor_ref,
                status: comment_status_label(&comment.status).to_string(),
                status_class: comment_status_class(&comment.status).to_string(),
                applied_commit: comment.applied_commit.clone(),
                body_html: render_markdown(&comment.body),
                reply_html: comment.reply.as_deref().map(render_markdown),
                cross_link: cross_link(project, page, id, review, comment),
                anchor_label: comment
                    .anchor
                    .is_none()
                    .then(|| "Whole document".to_string()),
                quote_text,
                quote_highlightable,
                outdated,
            });
        }
        reviews.push(ReviewView {
            id: id.clone(),
            author: review.author.clone(),
            created_relative: relative_time(review.created),
            state: review_state_label(&review.state).to_string(),
            state_class: review_state_class(&review.state).to_string(),
            verdict: review
                .verdict
                .as_ref()
                .map(|v| verdict_label(v).to_string()),
            verdict_class: review
                .verdict
                .as_ref()
                .map(|v| verdict_class(v).to_string()),
            summary_html: render_markdown(&doc.body),
            comments,
            dismiss_href: (review.state == ReviewState::Submitted)
                .then(|| format!("/projects/{project}/reviews/{id}/form/dismiss")),
        });
    }
    Ok(PageReviews {
        reviews,
        highlights,
    })
}

/// Assembles the draft-review panel for a detail page: the visitor's open
/// draft on `target` (matched by the `rdm_author` cookie identity), or the
/// data for a "Start review" form when they have none.
///
/// Draft privacy: only a draft whose author equals `author` is surfaced —
/// other visitors' drafts stay invisible, matching the non-draft filter in
/// [`page_reviews`]. With no author identity (`None`), no draft is ever
/// resumed. When several drafts by the same author exist on the document
/// (possible via the API), the oldest (lowest id) wins, deterministically.
///
/// `phases` supplies the `doc`-scope dropdown options and is non-empty only
/// for roadmap detail pages.
///
/// # Errors
///
/// Propagates `rdm_core::ops::reviews::list_reviews` failures (project not
/// found, unreadable reviews directory, malformed review file).
pub fn draft_panel(
    store: &impl rdm_core::store::Store,
    project: &str,
    target: &ReviewTarget,
    phases: &[(String, Document<Phase>)],
    author: Option<&str>,
) -> Result<DraftPanelView, rdm_core::error::Error> {
    let draft = match author {
        None => None,
        Some(author) => {
            let all = rdm_core::ops::reviews::list_reviews(store, project)?;
            let matches = rdm_core::ops::reviews::filter_reviews(
                all,
                &rdm_core::ops::reviews::ReviewFilter {
                    target: Some(target.clone()),
                    state: Some(ReviewState::Draft),
                    verdict: None,
                    author: Some(author.to_string()),
                    ..Default::default()
                },
            );
            // `list_reviews` is id-sorted, so the first match is stable.
            matches.into_iter().next().map(|(id, doc)| {
                let comments = doc
                    .frontmatter
                    .comments
                    .iter()
                    .map(|c| DraftCommentView {
                        id: c.id,
                        body_md: c.body.clone(),
                        doc_label: draft_doc_label(phases, c.doc.as_ref()),
                        doc_options: doc_options(phases, c.doc.as_ref()),
                        anchor_preview: c.anchor.as_ref().and_then(stored_quote),
                    })
                    .collect();
                DraftReviewView {
                    id,
                    summary_md: doc.body.clone(),
                    comments,
                }
            })
        }
    };
    Ok(DraftPanelView {
        target_ref: target.label(),
        author_value: author.unwrap_or_default().to_string(),
        draft,
        doc_options: doc_options(phases, None),
    })
}

/// Builds the `doc`-scope `<select>` options from a roadmap's phases,
/// flagging the option matching `current` as selected. Empty when the page
/// has no phases (phase and task pages — no dropdown renders).
///
/// When `current` names a stem missing from the live phase list (the phase
/// was deleted after the comment was scoped), a synthetic selected option
/// carrying that stale stem is appended, so an untouched edit-form submit
/// round-trips the scope unchanged instead of silently clearing it (a
/// `<select>` always posts a value; without the synthetic entry the browser
/// would fall back to "Whole roadmap").
fn doc_options(
    phases: &[(String, Document<Phase>)],
    current: Option<&CommentDoc>,
) -> Vec<DocOption> {
    let current_stem = current.map(|d| d.stem.as_str());
    let mut options: Vec<DocOption> = phases
        .iter()
        .map(|(stem, doc)| DocOption {
            stem: stem.clone(),
            label: format!("Phase {}: {}", doc.frontmatter.phase, doc.frontmatter.title),
            selected: Some(stem.as_str()) == current_stem,
        })
        .collect();
    if let Some(stem) = current_stem
        && !options.iter().any(|o| o.stem == stem)
    {
        options.push(DocOption {
            stem: stem.to_string(),
            label: format!("Phase {stem} (deleted)"),
            selected: true,
        });
    }
    options
}

/// Display label for a draft comment's scope: "Whole document", or the
/// targeted phase (falling back to the raw stem, marked deleted, when it
/// isn't in `phases`).
fn draft_doc_label(phases: &[(String, Document<Phase>)], doc: Option<&CommentDoc>) -> String {
    match doc {
        None => "Whole document".to_string(),
        Some(CommentDoc {
            kind: CommentDocKind::Phase,
            stem,
        }) => phases
            .iter()
            .find(|(s, _)| s == stem)
            .map(|(_, d)| format!("Phase {}: {}", d.frontmatter.phase, d.frontmatter.title))
            .unwrap_or_else(|| format!("Phase {stem} (deleted)")),
    }
}

/// Whether a review belongs on the given page at all.
fn review_applies(page: &PageDoc<'_>, review: &Review) -> bool {
    match (page, &review.target) {
        (PageDoc::Roadmap { roadmap, .. }, ReviewTarget::Roadmap { roadmap: r }) => r == roadmap,
        (
            PageDoc::Phase { roadmap, stem },
            ReviewTarget::Phase {
                roadmap: r,
                stem: s,
            },
        ) => r == roadmap && s == stem,
        // A roadmap review reaches a phase page when at least one of its
        // comments is doc-scoped into that phase.
        (PageDoc::Phase { roadmap, stem }, ReviewTarget::Roadmap { roadmap: r }) => {
            r == roadmap && review.comments.iter().any(|c| doc_scoped_to_stem(c, stem))
        }
        (PageDoc::Task { slug }, ReviewTarget::Task { slug: s }) => s == slug,
        _ => false,
    }
}

/// Whether one comment of an applicable review is rendered on the page.
///
/// Only one case filters: a roadmap review shown on a *phase* page shows
/// just the comments doc-scoped into that phase.
fn comment_shown(page: &PageDoc<'_>, review: &Review, comment: &ReviewComment) -> bool {
    match (page, &review.target) {
        (PageDoc::Phase { stem, .. }, ReviewTarget::Roadmap { .. }) => {
            doc_scoped_to_stem(comment, stem)
        }
        _ => true,
    }
}

/// Whether the comment's anchor resolves against the body the page renders
/// (a prerequisite for inline highlighting).
fn comment_targets_page_body(page: &PageDoc<'_>, review: &Review, comment: &ReviewComment) -> bool {
    match page {
        // On a roadmap page, doc-scoped comments point into phase bodies.
        PageDoc::Roadmap { .. } => comment.doc.is_none(),
        // On a phase page every shown comment targets the phase body:
        // phase-review comments directly, roadmap-review comments via the
        // doc filter in `comment_shown`.
        PageDoc::Phase { .. } => match &review.target {
            ReviewTarget::Roadmap { .. } => comment.doc.is_some(),
            _ => true,
        },
        PageDoc::Task { .. } => true,
    }
}

/// True when `comment` is doc-scoped to the phase with the given stem.
fn doc_scoped_to_stem(comment: &ReviewComment, stem: &str) -> bool {
    matches!(
        &comment.doc,
        Some(CommentDoc {
            kind: CommentDocKind::Phase,
            stem: s,
        }) if s == stem
    )
}

/// Cross-document link for a comment rendered away from its own document:
/// roadmap page → the phase a doc-scoped comment targets, and phase page →
/// the roadmap review a doc-scoped comment came from.
fn cross_link(
    project: &str,
    page: &PageDoc<'_>,
    review_id: &str,
    review: &Review,
    comment: &ReviewComment,
) -> Option<DocLink> {
    match page {
        PageDoc::Roadmap { roadmap, phases } => {
            let CommentDoc {
                kind: CommentDocKind::Phase,
                stem,
            } = comment.doc.as_ref()?;
            let label = phases
                .iter()
                .find(|(s, _)| s == stem)
                .map(|(_, doc)| {
                    format!(
                        "View in Phase {}: {}",
                        doc.frontmatter.phase, doc.frontmatter.title
                    )
                })
                .unwrap_or_else(|| format!("View in phase {stem}"));
            Some(DocLink {
                label,
                href: format!(
                    "/projects/{project}/roadmaps/{roadmap}/phases/{stem}#comment-{review_id}-c{}",
                    comment.id
                ),
            })
        }
        PageDoc::Phase { roadmap, .. } => match &review.target {
            ReviewTarget::Roadmap { .. } => Some(DocLink {
                label: format!("From roadmap review by {}", review.author),
                href: format!("/projects/{project}/roadmaps/{roadmap}#review-{review_id}"),
            }),
            _ => None,
        },
        PageDoc::Task { .. } => None,
    }
}

/// Returns the quote text stored in the anchor itself, for anchors that no
/// longer resolve anywhere.
fn stored_quote(anchor: &Anchor) -> Option<String> {
    match anchor {
        Anchor::TextQuote { quote, .. } => Some(quote.clone()),
        Anchor::Unknown { .. } => None,
    }
}

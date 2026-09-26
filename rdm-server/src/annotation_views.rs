//! Shapes phase 4's code-location annotations ([`CodeAnnotation`]) into what
//! the annotations page renders, and owns the one presentation of a review
//! comment's degradation ([`degradation`]) that the page and every detail
//! page's Reviews section share.
//!
//! Pure shaping: nothing here reads the source repository. The annotations
//! themselves come from `rdm_core::ops::links::code_annotations` (via
//! [`crate::source_repo::code_annotations_for_project`]); this module only
//! loads the annotating documents from the store to label them, and renders
//! each annotation's pinned location through the same `rdm:` link pipeline
//! a document body uses ([`crate::link_render::resolve_body_links`]).

use rdm_core::comment_target::TargetReason;
use rdm_core::error::Result;
use rdm_core::link::{BacklinkRef, CodeAnnotation, DocRef, Link};
use rdm_core::store::Store;

use crate::templates::ReferencedByEntry;

/// How a comment's degraded (or deliberately absent) anchor is shown: a
/// label and the CSS class that styles it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DegradationView {
    /// Human-readable label, e.g. "Whole document" or "Anchor lost:
    /// outside-hunk — meant src/lib.rs".
    pub label: String,
    /// CSS class: `anchor-whole-document` (neutral), `anchor-degraded`
    /// (warning) or `anchor-not-recorded` (muted).
    pub class: &'static str,
}

/// The one presentation of a review comment's [`TargetReason`], shared by
/// the annotations page and the Reviews section.
///
/// | reason | label | class |
/// |---|---|---|
/// | `Stored(whole-document)` | "Whole document", or "Whole file {path}" with an intended path | `anchor-whole-document` |
/// | `Stored(other)` | "Anchor lost: {state}", plus " — meant {path}" with an intended path | `anchor-degraded` |
/// | `Ineligible(note)` | "Anchor ineligible: {note}" | `anchor-degraded` |
/// | `NotRecorded` | "Anchor not recorded" | `anchor-not-recorded` |
/// | `None` (intact) | no badge | — |
///
/// A deliberate whole-document comment and a lost anchor therefore never
/// look alike.
///
/// # Examples
///
/// ```
/// use rdm_core::comment_target::TargetReason;
/// use rdm_core::model::AnchorState;
/// use rdm_server::annotation_views::degradation;
///
/// let lost = degradation(Some(&TargetReason::Stored(AnchorState::OutsideHunk)), None).unwrap();
/// assert_eq!(lost.class, "anchor-degraded");
/// let whole = degradation(Some(&TargetReason::Stored(AnchorState::WholeDocument)), None).unwrap();
/// assert_eq!(whole.label, "Whole document");
/// assert!(degradation(None, None).is_none());
/// ```
#[must_use]
pub fn degradation(
    reason: Option<&TargetReason>,
    intended_path: Option<&str>,
) -> Option<DegradationView> {
    let view = match reason? {
        TargetReason::Stored(state) if !state.is_degradation() => DegradationView {
            label: match intended_path {
                Some(path) => format!("Whole file {path}"),
                None => "Whole document".to_string(),
            },
            class: "anchor-whole-document",
        },
        TargetReason::Stored(state) => DegradationView {
            label: match intended_path {
                Some(path) => format!("Anchor lost: {} — meant {path}", state.as_str()),
                None => format!("Anchor lost: {}", state.as_str()),
            },
            class: "anchor-degraded",
        },
        TargetReason::Ineligible(note) => DegradationView {
            label: format!("Anchor ineligible: {note}"),
            class: "anchor-degraded",
        },
        TargetReason::NotRecorded => DegradationView {
            label: "Anchor not recorded".to_string(),
            class: "anchor-not-recorded",
        },
    };
    Some(view)
}

/// One annotation prepared for the annotations page.
#[derive(Debug, Clone)]
pub struct AnnotationView {
    /// The annotated location, rendered through the body-link pipeline: a
    /// live `rdm-link-code` permalink to the forge, or the non-navigable
    /// span, wrapped in an element whose `title` is the full `rdm:` link.
    pub permalink_html: String,
    /// The annotating document, or `None` when it no longer loads.
    pub annotator: Option<ReferencedByEntry>,
    /// Where in that document: "linked in body", "[head]", "comment 2", …
    pub provenance: String,
    /// `range`, `file` or `commit`.
    pub level: &'static str,
    /// The comment's degradation badge, when its target is not an intact
    /// anchor.
    pub degradation: Option<DegradationView>,
    /// `true` when core chose the comment's rung fail-open.
    pub unverified: bool,
}

/// Shapes `annotations` into one [`AnnotationView`] each, in core's order.
///
/// # Errors
///
/// Propagates [`crate::link_render::resolve_body_links`] failures (a
/// project that fails to load).
pub fn annotation_page(
    store: &impl Store,
    project: &str,
    annotations: &[CodeAnnotation],
) -> Result<Vec<AnnotationView>> {
    annotations
        .iter()
        .map(|annotation| {
            Ok(AnnotationView {
                permalink_html: permalink_html(store, project, &annotation.link)?,
                annotator: crate::link_render::doc_ref_view(store, project, &annotation.document),
                provenance: provenance(&annotation.document, &annotation.reference),
                level: annotation.level.as_str(),
                degradation: degradation(
                    annotation.reason.as_ref(),
                    intended_path(store, project, &annotation.document).as_deref(),
                ),
                unverified: annotation.unverified,
            })
        })
        .collect()
}

/// Renders `link` exactly as a body link to it renders — permalink or span,
/// plus its annotations anchor — inside a wrapper titled with the link.
///
/// # Errors
///
/// Propagates [`crate::link_render::resolve_body_links`] failures.
pub fn permalink_html(store: &impl Store, project: &str, link: &Link) -> Result<String> {
    let inline = inline_link_html(store, project, &escape_markdown(&short_display(link)), link)?;
    let mut title = String::new();
    for c in link.to_string().chars() {
        match c {
            '&' => title.push_str("&amp;"),
            '"' => title.push_str("&quot;"),
            '<' => title.push_str("&lt;"),
            '>' => title.push_str("&gt;"),
            c => title.push(c),
        }
    }
    Ok(format!(
        "<span class=\"annotation-permalink\" title=\"{title}\">{inline}</span>"
    ))
}

/// Renders a one-link markdown snippet `[{text_markdown}](<{link}>)` through
/// the body-link pipeline and returns its inline HTML (no paragraph
/// wrapper): exactly what a body link to `link` renders as.
///
/// # Errors
///
/// Propagates [`crate::link_render::resolve_body_links`] failures.
pub fn inline_link_html(
    store: &impl Store,
    project: &str,
    text_markdown: &str,
    link: &Link,
) -> Result<String> {
    let markdown = format!("[{text_markdown}](<{link}>)");
    let links = crate::link_render::resolve_body_links(store, project, None, &markdown)?;
    let html = crate::markdown::render_markdown_with_links(&markdown, &links);
    Ok(html
        .trim_end()
        .strip_prefix("<p>")
        .and_then(|h| h.strip_suffix("</p>"))
        .unwrap_or(&html)
        .to_string())
}

/// The short display text of an annotated location:
/// `{path}@{rev7}#L{a}[-L{b}]`, `{path}@{rev7}`, `{path}`, or
/// `commit {rev7}`.
fn short_display(link: &Link) -> String {
    let short = |rev: &str| rev.chars().take(7).collect::<String>();
    match link {
        Link::Code { path, rev, lines } => {
            let mut out = path.clone();
            if let Some(rev) = rev {
                out.push('@');
                out.push_str(&short(rev));
            }
            if let Some((start, end)) = lines {
                out.push_str(&format!("#L{start}"));
                if let Some(end) = end {
                    out.push_str(&format!("-L{end}"));
                }
            }
            out
        }
        Link::Commit { rev } => format!("commit {}", short(rev)),
        Link::Item(item) => item.label(),
    }
}

/// Backslash-escapes every ASCII punctuation character, so link text is
/// taken literally by the markdown parser.
fn escape_markdown(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_ascii_punctuation() {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Where in its document an annotation was found.
fn provenance(document: &DocRef, reference: &BacklinkRef) -> String {
    match reference {
        BacklinkRef::Body { .. } => "linked in body".to_string(),
        BacklinkRef::Field { field, .. } => format!("[{field}]"),
        BacklinkRef::Anchor => match document {
            DocRef::Review {
                comment: Some(n), ..
            } => format!("comment {n}"),
            _ => "anchor".to_string(),
        },
    }
}

/// The stored `intended_path` of the review comment `document` names, when
/// it is one and it has one.
fn intended_path(store: &impl Store, project: &str, document: &DocRef) -> Option<String> {
    let DocRef::Review {
        id,
        comment: Some(n),
    } = document
    else {
        return None;
    };
    let review = rdm_core::io::load_review(store, project, id).ok()?;
    review
        .frontmatter
        .comments
        .into_iter()
        .find(|c| c.id == *n)?
        .intended_path
}

#[cfg(test)]
mod tests {
    use super::*;
    use rdm_core::model::{
        Anchor, AnchorState, Plan, PlanStatus, Review, ReviewComment, ReviewCommentStatus,
        ReviewState, ReviewTarget, Verdict,
    };
    use rdm_core::source::MemorySourceRepo;
    use rdm_store_fs::FsStore;

    const HEAD: &str = "1111111111111111111111111111111111111111";

    #[test]
    fn degradation_distinguishes_every_reason() {
        use AnchorState as S;
        type Case<'a> = (
            Option<TargetReason>,
            Option<&'a str>,
            Option<(&'a str, &'a str)>,
        );
        let cases: Vec<Case> = vec![
            (None, None, None),
            (
                Some(TargetReason::Stored(S::WholeDocument)),
                None,
                Some(("Whole document", "anchor-whole-document")),
            ),
            (
                Some(TargetReason::Stored(S::WholeDocument)),
                Some("src/lib.rs"),
                Some(("Whole file src/lib.rs", "anchor-whole-document")),
            ),
            (
                Some(TargetReason::Stored(S::OutsideHunk)),
                None,
                Some(("Anchor lost: outside-hunk", "anchor-degraded")),
            ),
            (
                Some(TargetReason::Stored(S::PathMissing)),
                Some("src/gone.rs"),
                Some((
                    "Anchor lost: path-missing — meant src/gone.rs",
                    "anchor-degraded",
                )),
            ),
            (
                Some(TargetReason::Stored(S::Unknown("future-state".to_string()))),
                None,
                Some(("Anchor lost: future-state", "anchor-degraded")),
            ),
            (
                Some(TargetReason::Ineligible("src is a directory".to_string())),
                None,
                Some(("Anchor ineligible: src is a directory", "anchor-degraded")),
            ),
            (
                Some(TargetReason::NotRecorded),
                None,
                Some(("Anchor not recorded", "anchor-not-recorded")),
            ),
        ];
        for (reason, path, expected) in cases {
            let got = degradation(reason.as_ref(), path);
            assert_eq!(
                got.as_ref().map(|v| (v.label.as_str(), v.class)),
                expected,
                "{reason:?} / {path:?}"
            );
        }
    }

    fn comment(id: u32, anchor: Option<Anchor>, state: Option<AnchorState>) -> ReviewComment {
        ReviewComment {
            id,
            doc: None,
            status: ReviewCommentStatus::Open,
            applied_commit: None,
            anchor,
            anchor_state: state,
            intended_path: None,
            body: "finding".to_string(),
            reply: None,
        }
    }

    /// The permalink AC at the view layer: with a forge `source.repo`, every
    /// entry the contract returns renders a live href to that forge at its
    /// pinned revision — with a `#L` fragment for a range, none for a file.
    #[test]
    fn every_annotation_links_out_to_the_forge_at_its_pinned_revision() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = FsStore::new(dir.path());
        rdm_core::ops::init::init(&mut store).unwrap();
        rdm_core::ops::project::create_project(&mut store, "demo", "Demo").unwrap();
        let project = rdm_core::document::Document {
            frontmatter: rdm_core::model::Project {
                name: "demo".to_string(),
                title: "Demo".to_string(),
                source: Some(rdm_core::model::Source {
                    repo: "https://github.com/acme/demo".to_string(),
                    default_branch: None,
                }),
            },
            body: String::new(),
        };
        rdm_core::store::Store::write(
            &mut store,
            &rdm_core::store::RelPath::new("projects/demo/project.md").unwrap(),
            project.render().unwrap(),
        )
        .unwrap();
        rdm_core::ops::task::create_task(
            &mut store,
            rdm_core::ops::task::CreateTask {
                project: "demo",
                slug: "linker",
                title: "Linker",
                priority: rdm_core::model::Priority::Medium,
                body: Some(&format!("See [l](rdm:src/src/lib.rs@{HEAD}#L2).")),
                ..Default::default()
            },
        )
        .unwrap();
        let today = chrono::Utc::now().date_naive();
        rdm_core::io::write_plan(
            &mut store,
            "demo",
            "p",
            &rdm_core::document::Document {
                frontmatter: Plan {
                    project: "demo".to_string(),
                    plan: "p".to_string(),
                    title: "P".to_string(),
                    implements: ReviewTarget::Task {
                        slug: "linker".to_string(),
                    },
                    supersedes: None,
                    status: PlanStatus::Approved,
                    created: today,
                    updated: today,
                },
                body: String::new(),
            },
        )
        .unwrap();
        let mut file_comment = comment(2, None, Some(AnchorState::OutsideHunk));
        file_comment.intended_path = Some("src/lib.rs".to_string());
        rdm_core::io::write_review(
            &mut store,
            "demo",
            "r1",
            &rdm_core::document::Document {
                frontmatter: Review {
                    id: "r1".to_string(),
                    author: "reviewer".to_string(),
                    target: ReviewTarget::Change {
                        head: HEAD.to_string(),
                        base: None,
                    },
                    state: ReviewState::Submitted,
                    verdict: Some(Verdict::Comment),
                    created: chrono::Utc::now(),
                    submitted: Some(chrono::Utc::now()),
                    created_commit: None,
                    implements: Some(ReviewTarget::Plan {
                        slug: "p".to_string(),
                    }),
                    change_branch: None,
                    comments: vec![
                        comment(
                            1,
                            Some(Anchor::FileQuote {
                                path: "src/lib.rs".to_string(),
                                quote: "fn two() {}".to_string(),
                                occurrence: 1,
                                start_line: 2,
                                end_line: 2,
                            }),
                            None,
                        ),
                        file_comment,
                    ],
                },
                body: String::new(),
            },
        )
        .unwrap();
        rdm_core::store::Store::commit(&mut store).unwrap();

        let source = MemorySourceRepo::new()
            .with_head(HEAD)
            .with_rev(HEAD, HEAD)
            .with_file(HEAD, "src/lib.rs", "fn one() {}\nfn two() {}\n");
        let target =
            rdm_core::ops::links::parse_backlink_target(&store, "demo", "rdm:src/src/lib.rs")
                .unwrap();
        let annotations =
            rdm_core::ops::links::code_annotations(&store, &source, "demo", &target).unwrap();
        assert_eq!(annotations.len(), 3, "{annotations:?}");

        let views = annotation_page(&store, "demo", &annotations).unwrap();
        let blob = format!("https://github.com/acme/demo/blob/{HEAD}/src/lib.rs");
        for (annotation, view) in annotations.iter().zip(&views) {
            let expected_href = match &annotation.link {
                Link::Code {
                    lines: Some((a, None)),
                    ..
                } => format!("{blob}#L{a}"),
                Link::Code {
                    lines: Some((a, Some(b))),
                    ..
                } => format!("{blob}#L{a}-L{b}"),
                _ => blob.clone(),
            };
            assert!(
                view.permalink_html.contains(&format!(
                    r#"class="rdm-link-code" target="_blank" rel="noopener" href="{expected_href}""#
                )),
                "{}: {}",
                annotation.link,
                view.permalink_html
            );
            assert!(
                view.permalink_html
                    .contains(&format!(r#"title="{}""#, annotation.link)),
                "{}",
                view.permalink_html
            );
        }
        // The file-level entry is comment 2, degraded with its reason.
        let file = views.iter().find(|v| v.level == "file").unwrap();
        assert_eq!(file.provenance, "comment 2");
        assert_eq!(
            file.degradation.as_ref().map(|d| d.class),
            Some("anchor-degraded")
        );
        // The review's annotator links to the implemented task's page.
        assert_eq!(
            file.annotator.as_ref().unwrap().href,
            "/projects/demo/tasks/linker#comment-r1-c2"
        );
        let body = views
            .iter()
            .find(|v| v.provenance == "linked in body")
            .unwrap();
        assert_eq!(body.annotator.as_ref().unwrap().kind_label, "Task");
        assert!(body.degradation.is_none());
    }
}

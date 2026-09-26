//! `GET /projects/:project/annotations?target=<reference>` — what annotates a
//! code location.
//!
//! The one path-keyed resource: given a commit (`rdm:src@<sha>`) or a source
//! path (`rdm:src/<path>[@<rev>][#L<a>[-L<b>]]`), it lists the documents and
//! review comments that annotate it, exactly as `rdm backlinks <reference>`
//! does, from one call to `rdm_core::ops::links::code_annotations`. It
//! returns annotations only — never file contents: nothing here reads a file
//! through the source repository.

use askama::Template;
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};

use rdm_core::link::{BacklinkTarget, Link};

use crate::content_type::ResponseFormat;
use crate::error::{error_response, problem_response};
use crate::extract::hal_response;
use crate::hal::{HalLink, HalResource};
use crate::problem::ProblemDetail;
use crate::source_repo::AnnotationsOutcome;
use crate::state::AppState;
use crate::templates::{AnnotationsPage, WidenLink};

/// Query parameters of the annotations resource.
#[derive(Debug, Deserialize, Default)]
pub struct AnnotationsQuery {
    /// The code location: `rdm:src/<path>[@<rev>][#L<a>[-L<b>]]` or
    /// `rdm:src@<sha>`, the `rdm:` prefix optional — the argument
    /// `rdm backlinks` takes for a source-side target.
    pub target: Option<String>,
}

/// The HAL data fields of the annotations resource.
#[derive(Serialize)]
struct AnnotationsData {
    /// The queried location, as `BacklinkTarget::label` spells it.
    target: String,
    /// Why no source repository was read; absent when one was.
    #[serde(skip_serializing_if = "Option::is_none")]
    source_verification_skipped: Option<String>,
}

/// `GET /projects/:project/annotations?target=<reference>` — the documents
/// and review comments annotating a code location.
///
/// One content-negotiated handler over one call to core: HAL+JSON embeds
/// the `rdm backlinks --format json` entries verbatim under
/// `_embedded.annotations`; HTML lists each with its pinned permalink,
/// level and degradation, plus a Referenced-by section.
///
/// With no readable source repository the response is still `200`, with
/// `source_verification_skipped` (HAL) or a notice (HTML) and no list.
pub async fn get_annotations(
    format: ResponseFormat,
    State(state): State<AppState>,
    Path(project): Path<String>,
    Query(query): Query<AnnotationsQuery>,
) -> Result<Response, Response> {
    let Some(raw) = query.target.filter(|t| !t.trim().is_empty()) else {
        return Err(problem_response(
            ProblemDetail {
                problem_type: "about:blank".to_string(),
                title: "Bad Request".to_string(),
                status: 400,
                detail: Some(
                    "target query parameter is required: rdm:src/<path>[@<rev>][#L<a>-L<b>] or rdm:src@<sha>"
                        .to_string(),
                ),
                instance: None,
            },
            format,
        ));
    };
    let store = state.store();
    let target = rdm_core::ops::links::parse_backlink_target(&store, &project, &raw)
        .map_err(|e| error_response(e, format))?;
    let outcome = crate::source_repo::code_annotations_for_project(&store, &project, &target)
        .map_err(|e| error_response(e, format))?;
    let (annotations, source_note) = match outcome {
        AnnotationsOutcome::Found(annotations) => (Some(annotations), None),
        AnnotationsOutcome::SourceUnavailable(note) => (None, Some(note)),
    };
    let referenced_by = annotations
        .as_deref()
        .map(|a| crate::link_render::annotators_referenced_by(&store, &project, a))
        .unwrap_or_default();

    match format {
        ResponseFormat::HalJson => {
            let backlinks = referenced_by
                .into_iter()
                .map(|r| HalLink {
                    href: r.href,
                    title: Some(r.title),
                    templated: None,
                })
                .collect();
            let mut code_links = Vec::new();
            for annotation in annotations.iter().flatten() {
                if let Some(href) =
                    crate::link_render::code_permalink(&store, &project, &annotation.link)
                        .map_err(|e| error_response(e, format))?
                {
                    code_links.push(HalLink {
                        href,
                        title: Some(annotation.link.to_string()),
                        templated: None,
                    });
                }
            }
            let mut resource = HalResource::new(
                AnnotationsData {
                    target: target.label(),
                    source_verification_skipped: source_note,
                },
                crate::link_render::annotations_href_for(&project, &raw),
            )
            .with_link("project", HalLink::new(format!("/projects/{project}")))
            .with_links("rdm:backlink", backlinks)
            .with_links("rdm:code", code_links);
            if let Some(annotations) = &annotations {
                resource = resource.with_embedded(
                    "annotations",
                    annotations
                        .iter()
                        .map(|a| {
                            serde_json::to_value(rdm_core::json::code_annotation_to_json(a))
                                .expect("CodeAnnotationJson always serializes")
                        })
                        .collect(),
                );
            }
            Ok(hal_response(resource))
        }
        ResponseFormat::Html => {
            let views = match &annotations {
                Some(a) => crate::annotation_views::annotation_page(&store, &project, a)
                    .map_err(|e| error_response(e, format))?,
                None => Vec::new(),
            };
            let page = AnnotationsPage {
                target_label: target.label(),
                widen: widen_links(&project, &target),
                source_note,
                listed: annotations.is_some(),
                annotations: views,
                referenced_by,
                project,
            };
            Ok((
                [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
                page.render().expect("template rendering cannot fail"),
            )
                .into_response())
        }
    }
}

/// Links that widen a path query: "any revision" drops a `@rev`, "whole
/// file" drops a line range. A commit query has nothing to widen.
fn widen_links(project: &str, target: &BacklinkTarget) -> Vec<WidenLink> {
    let BacklinkTarget::Code { path, rev, lines } = target else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if rev.is_some() {
        out.push(WidenLink {
            label: "any revision",
            href: crate::link_render::annotations_href(
                project,
                &Link::Code {
                    path: path.clone(),
                    rev: None,
                    lines: *lines,
                },
            ),
        });
    }
    if lines.is_some() {
        out.push(WidenLink {
            label: "whole file",
            href: crate::link_render::annotations_href(
                project,
                &Link::Code {
                    path: path.clone(),
                    rev: rev.clone(),
                    lines: None,
                },
            ),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decoded(href: &str) -> String {
        let (_, query) = href.split_once("?target=").unwrap();
        percent_encoding::percent_decode_str(query)
            .decode_utf8()
            .unwrap()
            .into_owned()
    }

    fn widened(target: &BacklinkTarget) -> Vec<(&'static str, String)> {
        widen_links("demo", target)
            .into_iter()
            .map(|w| (w.label, decoded(&w.href)))
            .collect()
    }

    fn code(rev: Option<&str>, lines: Option<(u32, Option<u32>)>) -> BacklinkTarget {
        BacklinkTarget::Code {
            path: "src/lib.rs".to_string(),
            rev: rev.map(str::to_string),
            lines,
        }
    }

    #[test]
    fn widen_links_commit_target_has_nothing_to_widen() {
        let target = BacklinkTarget::Commit {
            rev: "abc1234".to_string(),
        };
        assert!(widen_links("demo", &target).is_empty());
    }

    #[test]
    fn widen_links_whole_file_at_head_has_nothing_to_widen() {
        assert!(widen_links("demo", &code(None, None)).is_empty());
    }

    #[test]
    fn widen_links_rev_only_offers_any_revision() {
        assert_eq!(
            widened(&code(Some("abc1234"), None)),
            vec![("any revision", "rdm:src/src/lib.rs".to_string())]
        );
    }

    #[test]
    fn widen_links_lines_only_offers_whole_file() {
        assert_eq!(
            widened(&code(None, Some((2, Some(4))))),
            vec![("whole file", "rdm:src/src/lib.rs".to_string())]
        );
    }

    #[test]
    fn widen_links_rev_and_lines_drop_one_each() {
        assert_eq!(
            widened(&code(Some("abc1234"), Some((2, None)))),
            vec![
                ("any revision", "rdm:src/src/lib.rs#L2".to_string()),
                ("whole file", "rdm:src/src/lib.rs@abc1234".to_string()),
            ]
        );
    }
}

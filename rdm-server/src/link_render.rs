//! Turns a document's `rdm:` links into what the browser needs: a rendering
//! treatment per link ([`RenderAction`], consumed by
//! [`crate::markdown`]'s `_with_links` render entry points) and the raw
//! resolved data the JSON API exposes (via `rdm_core::json`'s already
//! established `link resolve`/`link list` vocabulary).
//!
//! [`resolve_body_links`] is the one entry point: it walks a body's `rdm:`
//! links (via `rdm_core::link::extract_links`), resolves each against the
//! store (via `rdm_core::ops::links::resolve_link`), and classifies the
//! result into a [`BodyLink`] carrying both views. Resolution of a repeated
//! link to the same item target is memoized per call, since a body may
//! reference the same target more than once.
//!
//! [`referenced_by`] is the inverse: given a target, it loads
//! `rdm_core::ops::links::backlinks` and shapes each distinct referencing
//! document into a [`crate::templates::ReferencedByEntry`] for the
//! "Referenced by" section.

use std::ops::Range;

use rdm_core::error::Result;
use rdm_core::link::{DocRef, ItemRef, Link, Resolved};
use rdm_core::store::Store;

use crate::templates::ReferencedByEntry;

/// The rendering treatment for one `rdm:` link occurrence, decoupled from
/// `rdm_core`'s exact [`Resolved`] shape so [`crate::markdown`] never has to
/// match on it directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderAction {
    /// A link to a roadmap/phase/task that exists: an anchor to its detail
    /// page, carrying a CSS class (a bare `rdm-link-item` for a roadmap
    /// target, `rdm-link-item rdm-status-<status>` for a phase/task target).
    ItemLink {
        /// Href of the target's detail page.
        href: String,
        /// CSS class(es) for the anchor.
        class: String,
    },
    /// A `rdm:src/...` code reference.
    CodeLink {
        /// GitHub-style permalink, when the project has a `source`
        /// configured. `None` renders as a non-navigable styled span.
        web_url: Option<String>,
        /// Display text for the non-navigable span (`path@rev`, or bare
        /// `path` when no revision resolved) — unused when `web_url` is
        /// `Some`, since the original link text is kept in that case.
        no_link_display: String,
    },
    /// A link that must never render as a live `<a>`: a dangling item
    /// target, or a destination that failed to parse.
    Broken {
        /// Why the link is broken, surfaced as the span's `title` attribute.
        reason: String,
    },
}

/// One `rdm:` link found in a body, resolved and classified.
#[derive(Debug, Clone, PartialEq)]
pub struct BodyLink {
    /// Byte range of the link within the (pristine) body it was found in.
    pub range: Range<usize>,
    /// The raw `rdm:` URI text (reconstructed via [`Link::to_string`] for a
    /// successfully parsed link, or the original text for a malformed one).
    pub uri: String,
    /// The plan-repo-relative path to the target document, for an
    /// [`ItemRef`] link that resolved (mirrors `rdm-cli`'s `link
    /// resolve`/`link list` JSON shape, which needs this alongside
    /// [`Resolved`] — see [`rdm_core::json::resolved_to_json`]'s doc
    /// comment for why `Resolved` itself doesn't carry it).
    pub item_path: Option<String>,
    /// The raw resolution outcome, for JSON serialization via
    /// [`rdm_core::json::resolved_to_json`].
    pub resolved: Resolved,
    /// The rendering treatment for this occurrence.
    pub action: RenderAction,
}

/// Resolves and classifies every `rdm:` link in `body`, in document order.
///
/// `containing_commit` is the containing document's stamped `commit` (a
/// `Phase`/`Task`'s `commit` field) — `None` for a roadmap or review body,
/// neither of which carries one — used only for a code link's revision
/// default, per [`rdm_core::ops::links::resolve_code_link`].
///
/// Resolution of a repeated link to the same item target is memoized by the
/// target's label, so a body linking the same task twice does not load it
/// twice.
///
/// # Errors
///
/// Propagates any error from `rdm_core::ops::links::resolve_link` — see its
/// documentation (a project-load failure for a code link; a store failure
/// while normalizing a numeric phase stem for an item link).
pub fn resolve_body_links(
    store: &impl Store,
    project: &str,
    containing_commit: Option<&str>,
    body: &str,
) -> Result<Vec<BodyLink>> {
    let (links, diagnostics) = rdm_core::link::extract_links(body);

    let mut item_cache: std::collections::HashMap<
        String,
        (Option<String>, Resolved, RenderAction),
    > = std::collections::HashMap::new();

    let mut out = Vec::with_capacity(links.len() + diagnostics.len());
    for (range, link) in links {
        let uri = link.to_string();
        let (item_path, resolved, action) = match &link {
            Link::Item(target) => {
                let key = target.label();
                if let Some(cached) = item_cache.get(&key) {
                    cached.clone()
                } else {
                    let resolved = rdm_core::ops::links::resolve_link(
                        store,
                        project,
                        containing_commit,
                        &link,
                    )?;
                    let item_path = rdm_core::ops::links::item_ref_path(store, project, target)
                        .ok()
                        .map(|p| p.to_string());
                    let action = classify_item(store, project, &resolved);
                    let entry = (item_path, resolved, action);
                    item_cache.insert(key, entry.clone());
                    entry
                }
            }
            Link::Code { .. } => {
                let resolved =
                    rdm_core::ops::links::resolve_link(store, project, containing_commit, &link)?;
                let action = classify_code(&resolved);
                (None, resolved, action)
            }
        };
        out.push(BodyLink {
            range,
            uri,
            item_path,
            resolved,
            action,
        });
    }

    for diag in diagnostics {
        let reason = diag.error.to_string();
        out.push(BodyLink {
            range: diag.range,
            uri: diag.uri,
            item_path: None,
            resolved: Resolved::Broken {
                reason: reason.clone(),
            },
            action: RenderAction::Broken { reason },
        });
    }

    out.sort_by_key(|l| l.range.start);
    Ok(out)
}

/// Classifies a resolved item reference into its [`RenderAction`].
///
/// A dangling target (`exists: false`) is always [`RenderAction::Broken`].
/// An existing target gets an [`RenderAction::ItemLink`] whose class
/// carries the target's status (`rdm-status-<status>`) for a phase or task;
/// a roadmap target has no status field, so its class is the bare
/// `rdm-link-item`.
fn classify_item(store: &impl Store, project: &str, resolved: &Resolved) -> RenderAction {
    let Resolved::Item { target, exists } = resolved else {
        // `resolve_link` only ever returns `Resolved::Item` for a
        // `Link::Item` — see its doc comment.
        return RenderAction::Broken {
            reason: "internal error: expected an item resolution".to_string(),
        };
    };
    if !exists {
        return RenderAction::Broken {
            reason: format!("target not found: {}", target.label()),
        };
    }
    let href = crate::review_views::target_detail_href(project, target);
    let class = match target {
        ItemRef::Roadmap { .. } => "rdm-link-item".to_string(),
        ItemRef::Phase { roadmap, stem } => item_status_class(
            rdm_core::ops::phase::resolve_phase_stem(store, project, roadmap, stem)
                .ok()
                .and_then(|resolved_stem| {
                    rdm_core::io::load_phase(store, project, roadmap, &resolved_stem).ok()
                })
                .map(|doc| doc.frontmatter.status.to_string()),
        ),
        ItemRef::Task { slug } => item_status_class(
            rdm_core::io::load_task(store, project, slug)
                .ok()
                .map(|doc| doc.frontmatter.status.to_string()),
        ),
        // A plan has a status, but no rdm-server detail route yet, so it
        // renders with the bare item class until plan support lands.
        ItemRef::Plan { .. } => "rdm-link-item".to_string(),
    };
    RenderAction::ItemLink { href, class }
}

/// Builds an item link's CSS class from an optional status string:
/// `rdm-link-item rdm-status-<status>` when known, else the bare
/// `rdm-link-item` (a load failure between the existence check and here, or
/// a target kind with no status field).
fn item_status_class(status: Option<String>) -> String {
    match status {
        Some(status) => format!("rdm-link-item rdm-status-{status}"),
        None => "rdm-link-item".to_string(),
    }
}

/// Classifies a resolved code reference into its [`RenderAction`].
///
/// No filesystem/git verification happens here — a syntactically valid
/// code link always gets a permalink (when the project has `source`
/// configured) or the non-navigable styled span (otherwise), never a
/// broken span; path existence at that revision is `rdm link check`'s job.
fn classify_code(resolved: &Resolved) -> RenderAction {
    let Resolved::Code {
        path, rev, web_url, ..
    } = resolved
    else {
        return RenderAction::Broken {
            reason: "internal error: expected a code resolution".to_string(),
        };
    };
    let no_link_display = match rev {
        Some(rev) => format!("{path}@{rev}"),
        None => path.clone(),
    };
    RenderAction::CodeLink {
        web_url: web_url.clone(),
        no_link_display,
    }
}

/// Builds the JSON-API views of a body's resolved outgoing links: a HAL
/// link per occurrence that has a navigable href (an item link, or a code
/// link with a `web_url`; a broken link or a no-source code link is omitted
/// from this array, since there is nothing sensible to navigate to), and
/// the raw per-occurrence JSON in `rdm-cli`'s `link resolve`/`link list`
/// vocabulary (via `rdm_core::json::resolved_to_json`), for every
/// occurrence including broken ones.
#[must_use]
pub fn outgoing_link_views(
    body_links: &[BodyLink],
) -> (Vec<crate::hal::HalLink>, Vec<serde_json::Value>) {
    let mut hal = Vec::new();
    let mut json = Vec::new();
    for link in body_links {
        let resolved_json =
            rdm_core::json::resolved_to_json(&link.resolved, link.item_path.as_deref());
        json.push(
            serde_json::to_value(rdm_core::json::outgoing_link_to_json(
                &link.uri,
                resolved_json,
            ))
            .expect("OutgoingLinkJson always serializes"),
        );
        let href = match &link.action {
            RenderAction::ItemLink { href, .. } => Some(href.clone()),
            RenderAction::CodeLink {
                web_url: Some(url), ..
            } => Some(url.clone()),
            _ => None,
        };
        if let Some(href) = href {
            hal.push(crate::hal::HalLink {
                href,
                title: Some(link.uri.clone()),
                templated: None,
            });
        }
    }
    (hal, json)
}

/// Builds the JSON-API views of `target`'s backlinks: a HAL link per
/// distinct referencing document (reusing [`referenced_by`]'s href/title),
/// and the raw per-occurrence JSON in `rdm-cli`'s `backlinks` vocabulary
/// (via `rdm_core::json::backlink_entry_to_json`) — one entry per
/// occurrence, undeduplicated, matching the CLI.
///
/// # Errors
///
/// Propagates `rdm_core::ops::links::backlinks` failures — see its
/// documentation.
pub fn backlink_views(
    store: &impl Store,
    project: &str,
    target: &ItemRef,
) -> Result<(Vec<crate::hal::HalLink>, Vec<serde_json::Value>)> {
    let entries = rdm_core::ops::links::backlinks(store, project, target)?;
    let json = entries
        .iter()
        .map(|e| {
            serde_json::to_value(rdm_core::json::backlink_entry_to_json(e))
                .expect("BacklinkEntryJson always serializes")
        })
        .collect();
    let hal = referenced_by(store, project, target)?
        .into_iter()
        .map(|v| crate::hal::HalLink {
            href: v.href,
            title: Some(v.title),
            templated: None,
        })
        .collect();
    Ok((hal, json))
}

/// Loads, resolves, and shapes every distinct document referencing `target`
/// into a "Referenced by" entry, in [`DocRef`]'s canonical order.
///
/// Occurrences are deduplicated by referencing document, so a document
/// linking `target` more than once appears once. A referencing document
/// that fails to load (deleted between the backlink scan and now) is
/// silently skipped.
///
/// # Errors
///
/// Propagates `rdm_core::ops::links::backlinks` failures — see its
/// documentation.
pub fn referenced_by(
    store: &impl Store,
    project: &str,
    target: &ItemRef,
) -> Result<Vec<ReferencedByEntry>> {
    let entries = rdm_core::ops::links::backlinks(store, project, target)?;
    let mut seen: Vec<DocRef> = Vec::new();
    let mut out = Vec::new();
    for entry in entries {
        if seen.contains(&entry.document) {
            continue;
        }
        seen.push(entry.document.clone());
        if let Some(view) = doc_ref_view(store, project, &entry.document) {
            out.push(view);
        }
    }
    Ok(out)
}

/// Loads `doc` and builds its "Referenced by" entry, or `None` if it no
/// longer exists/loads.
fn doc_ref_view(store: &impl Store, project: &str, doc: &DocRef) -> Option<ReferencedByEntry> {
    match doc {
        DocRef::Roadmap { roadmap } => {
            let d = rdm_core::io::load_roadmap(store, project, roadmap).ok()?;
            Some(ReferencedByEntry {
                href: crate::review_views::target_detail_href(
                    project,
                    &ItemRef::Roadmap {
                        roadmap: roadmap.clone(),
                    },
                ),
                title: d.frontmatter.title,
                kind_label: "Roadmap".to_string(),
            })
        }
        DocRef::Phase { roadmap, stem } => {
            let d = rdm_core::io::load_phase(store, project, roadmap, stem).ok()?;
            Some(ReferencedByEntry {
                href: crate::review_views::target_detail_href(
                    project,
                    &ItemRef::Phase {
                        roadmap: roadmap.clone(),
                        stem: stem.clone(),
                    },
                ),
                title: format!("Phase {}: {}", d.frontmatter.phase, d.frontmatter.title),
                kind_label: "Phase".to_string(),
            })
        }
        DocRef::Task { slug } => {
            let d = rdm_core::io::load_task(store, project, slug).ok()?;
            Some(ReferencedByEntry {
                href: crate::review_views::target_detail_href(
                    project,
                    &ItemRef::Task { slug: slug.clone() },
                ),
                title: d.frontmatter.title,
                kind_label: "Task".to_string(),
            })
        }
        DocRef::Plan { slug } => {
            let d = rdm_core::io::load_plan(store, project, slug).ok()?;
            Some(ReferencedByEntry {
                href: crate::review_views::target_detail_href(
                    project,
                    &ItemRef::Plan { slug: slug.clone() },
                ),
                title: d.frontmatter.title,
                kind_label: "Plan".to_string(),
            })
        }
        DocRef::Review { id, comment } => {
            let d = rdm_core::io::load_review(store, project, id).ok()?;
            // Reviews render inline on their *target*'s detail page (see
            // `review_views::cross_link`), never at a standalone HTML
            // route -- `/projects/{project}/reviews/{id}` is JSON-only
            // (`handlers::reviews::get_review`). Point at the page the
            // review actually appears on, anchored to the comment or the
            // review's own summary.
            let mut href = crate::review_views::target_detail_href(project, &d.frontmatter.target);
            match comment {
                Some(c) => href.push_str(&format!("#comment-{id}-c{c}")),
                None => href.push_str(&format!("#review-{id}")),
            }
            Some(ReferencedByEntry {
                href,
                title: format!("Review by {}", d.frontmatter.author),
                kind_label: "Review".to_string(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rdm_core::model::Priority;
    use rdm_store_fs::FsStore;

    fn tmp_store() -> (tempfile::TempDir, FsStore) {
        let dir = tempfile::tempdir().unwrap();
        let mut store = FsStore::new(dir.path());
        rdm_core::ops::init::init(&mut store).unwrap();
        rdm_core::ops::project::create_project(&mut store, "demo", "Demo").unwrap();
        rdm_core::store::Store::commit(&mut store).unwrap();
        (dir, store)
    }

    fn seed_task(store: &mut FsStore, slug: &str, title: &str) {
        rdm_core::ops::task::create_task(
            store,
            rdm_core::ops::task::CreateTask {
                project: "demo",
                slug,
                title,
                priority: Priority::Medium,
                ..Default::default()
            },
        )
        .unwrap();
        rdm_core::store::Store::commit(store).unwrap();
    }

    fn seed_roadmap_with_phase(store: &mut FsStore, roadmap: &str, stem_title: &str, body: &str) {
        rdm_core::ops::roadmap::create_roadmap(
            store,
            rdm_core::ops::roadmap::CreateRoadmap {
                project: "demo",
                slug: roadmap,
                title: "Roadmap",
                ..Default::default()
            },
        )
        .unwrap();
        rdm_core::ops::phase::create_phase(
            store,
            rdm_core::ops::phase::CreatePhase {
                project: "demo",
                roadmap,
                slug: "design",
                title: stem_title,
                number: Some(1),
                body: Some(body),
                ..Default::default()
            },
        )
        .unwrap();
        rdm_core::store::Store::commit(store).unwrap();
    }

    /// Seeds a plan implementing `task/<implements_task>` with `body`. Plans
    /// have no rdm-server detail route yet, so this exists to exercise the
    /// `Plan` arms of `classify_item` and `doc_ref_view`, which the shared
    /// rendering module reaches today through any body that links to a plan
    /// and through `referenced_by` on anything a plan body links to.
    fn seed_plan(store: &mut FsStore, slug: &str, title: &str, implements_task: &str, body: &str) {
        rdm_core::ops::plan::create_plan(
            store,
            rdm_core::ops::plan::CreatePlan {
                project: "demo",
                slug,
                title,
                implements: ItemRef::Task {
                    slug: implements_task.to_string(),
                },
                supersedes: None,
                body: Some(body),
            },
        )
        .unwrap();
        rdm_core::store::Store::commit(store).unwrap();
    }

    #[test]
    fn item_link_to_existing_task_gets_status_class() {
        let (_dir, mut store) = tmp_store();
        seed_task(&mut store, "fix-bug", "Fix bug");
        let body = "See [the task](rdm:task/fix-bug) for details.";
        let links = resolve_body_links(&store, "demo", None, body).unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!(
            links[0].action,
            RenderAction::ItemLink {
                href: "/projects/demo/tasks/fix-bug".to_string(),
                class: "rdm-link-item rdm-status-open".to_string(),
            }
        );
    }

    #[test]
    fn item_link_to_dangling_task_is_broken() {
        let (_dir, store) = tmp_store();
        let body = "See [gone](rdm:task/does-not-exist).";
        let links = resolve_body_links(&store, "demo", None, body).unwrap();
        assert_eq!(links.len(), 1);
        assert!(
            matches!(&links[0].action, RenderAction::Broken { reason } if reason.contains("does-not-exist"))
        );
    }

    #[test]
    fn malformed_link_is_broken() {
        let (_dir, store) = tmp_store();
        let body = "See [bad](rdm:foo/bar).";
        let links = resolve_body_links(&store, "demo", None, body).unwrap();
        assert_eq!(links.len(), 1);
        assert!(matches!(&links[0].action, RenderAction::Broken { .. }));
    }

    #[test]
    fn code_link_without_source_has_no_web_url() {
        let (_dir, store) = tmp_store();
        let body = "See [src](rdm:src/a/b.rs#L5).";
        let links = resolve_body_links(&store, "demo", None, body).unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!(
            links[0].action,
            RenderAction::CodeLink {
                web_url: None,
                no_link_display: "a/b.rs".to_string(),
            }
        );
    }

    /// Configures `project`'s `source` field directly on disk (there is no
    /// public ops-level setter for it yet -- `Project::source` is written
    /// only by `rdm project create`'s hardcoded `None`), so this end-to-end
    /// test can drive a real `rdm:src/...@rev#L5-L12` link through the full
    /// store-config -> resolve -> render pipeline the AC requires, rather
    /// than a fabricated `RenderAction::CodeLink`.
    fn seed_project_source(store: &mut FsStore, project: &str, repo: &str) {
        let doc = rdm_core::document::Document {
            frontmatter: rdm_core::model::Project {
                name: project.to_string(),
                title: "Demo".to_string(),
                source: Some(rdm_core::model::Source {
                    repo: repo.to_string(),
                    default_branch: None,
                }),
            },
            body: String::new(),
        };
        let content = doc.render().unwrap();
        let path =
            rdm_core::store::RelPath::new(&format!("projects/{project}/project.md")).unwrap();
        rdm_core::store::Store::write(store, &path, content).unwrap();
        rdm_core::store::Store::commit(store).unwrap();
    }

    #[test]
    fn code_link_with_source_resolves_to_live_github_permalink_html() {
        let (_dir, mut store) = tmp_store();
        seed_project_source(&mut store, "demo", "https://github.com/acme/demo");

        let body = "See [src](rdm:src/a/b.rs@abc123#L5-L12) for details.";
        let links = resolve_body_links(&store, "demo", None, body).unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!(
            links[0].action,
            RenderAction::CodeLink {
                web_url: Some("https://github.com/acme/demo/blob/abc123/a/b.rs#L5-L12".to_string()),
                no_link_display: "a/b.rs@abc123".to_string(),
            }
        );

        // Drive the resolved action through the actual render pipeline, so
        // the whole chain -- store config, resolution, and the rendered
        // `<a>` markup a browser would follow -- is proven live, not
        // asserted piecemeal.
        let html = crate::markdown::render_markdown_with_links(body, &links);
        assert!(
            html.contains(
                r#"<a class="rdm-link-code" target="_blank" rel="noopener" href="https://github.com/acme/demo/blob/abc123/a/b.rs#L5-L12">src</a>"#
            ),
            "got: {html}"
        );
    }

    #[test]
    fn repeated_item_link_is_memoized() {
        let (_dir, mut store) = tmp_store();
        seed_task(&mut store, "fix-bug", "Fix bug");
        let body = "[a](rdm:task/fix-bug) and [b](rdm:task/fix-bug)";
        let links = resolve_body_links(&store, "demo", None, body).unwrap();
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].action, links[1].action);
    }

    #[test]
    fn referenced_by_finds_phase_linking_task() {
        let (_dir, mut store) = tmp_store();
        seed_task(&mut store, "fix-bug", "Fix bug");
        seed_roadmap_with_phase(
            &mut store,
            "auth",
            "Design",
            "See [the task](rdm:task/fix-bug).",
        );
        let entries = referenced_by(
            &store,
            "demo",
            &ItemRef::Task {
                slug: "fix-bug".to_string(),
            },
        )
        .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].href,
            "/projects/demo/roadmaps/auth/phases/phase-1-design"
        );
        assert!(entries[0].title.contains("Design"));
    }

    #[test]
    fn referenced_by_finds_review_comment_link_with_working_html_href() {
        let (_dir, mut store) = tmp_store();
        seed_task(&mut store, "fix-bug", "Fix bug");
        seed_task(&mut store, "reviewed-task", "Reviewed task");

        let review = rdm_core::ops::reviews::create_review(
            &mut store,
            rdm_core::ops::reviews::CreateReview {
                project: "demo",
                author: "ed",
                target: ItemRef::Task {
                    slug: "reviewed-task".to_string(),
                },
                body: None,
            },
        )
        .unwrap();
        let review_id = review.frontmatter.id.clone();
        rdm_core::ops::reviews::add_comment(
            &mut store,
            rdm_core::ops::reviews::AddComment {
                project: "demo",
                review_id: &review_id,
                body: "See [the other task](rdm:task/fix-bug).",
                doc: None,
                anchor: None,
            },
        )
        .unwrap();
        rdm_core::store::Store::commit(&mut store).unwrap();

        // The review targets `reviewed-task`, so its comment must show up
        // as a "Referenced by" entry on `fix-bug`'s page, pointing at a
        // real HTML anchor on the REVIEW'S TARGET's detail page -- never
        // the JSON-only `/reviews/{id}` route.
        let entries = referenced_by(
            &store,
            "demo",
            &ItemRef::Task {
                slug: "fix-bug".to_string(),
            },
        )
        .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].href,
            format!("/projects/demo/tasks/reviewed-task#comment-{review_id}-c1")
        );
        assert!(entries[0].title.contains("Review by"));
    }

    #[test]
    fn referenced_by_empty_when_no_backlinks() {
        let (_dir, mut store) = tmp_store();
        seed_task(&mut store, "lonely", "Lonely");
        let entries = referenced_by(
            &store,
            "demo",
            &ItemRef::Task {
                slug: "lonely".to_string(),
            },
        )
        .unwrap();
        assert!(entries.is_empty());
    }

    /// A roadmap target has no status field, so unlike the phase/task arms
    /// of `classify_item`, it must get the bare `rdm-link-item` class with
    /// no `rdm-status-*` suffix -- proven end to end through resolve AND
    /// render, since a roadmap-target `rdm:` link is never actually
    /// rendered on any page in the rest of the test suite (every occurrence
    /// elsewhere is used only as a backlink *source*, never fetched via
    /// `get_html`/a HAL request).
    #[test]
    fn item_link_to_roadmap_gets_bare_item_class() {
        let (_dir, mut store) = tmp_store();
        seed_roadmap_with_phase(&mut store, "auth", "Design", "Phase body.");
        let body = "See [the roadmap](rdm:roadmap/auth) for details.";
        let links = resolve_body_links(&store, "demo", None, body).unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!(
            links[0].action,
            RenderAction::ItemLink {
                href: "/projects/demo/roadmaps/auth".to_string(),
                class: "rdm-link-item".to_string(),
            }
        );

        let html = crate::markdown::render_markdown_with_links(body, &links);
        assert!(
            html.contains(
                r#"<a class="rdm-link-item" href="/projects/demo/roadmaps/auth">the roadmap</a>"#
            ),
            "got: {html}"
        );
    }

    /// [`outgoing_link_views`]'s doc comment promises a broken link and a
    /// no-`source` code link are omitted from the HAL array while still
    /// appearing in the raw per-occurrence JSON -- pin both halves of that
    /// contract, since neither was covered anywhere else (every handler
    /// test that inspects `_links["rdm:link"]`/`_embedded["links"]` uses
    /// exactly one, always-navigable item link).
    #[test]
    fn outgoing_link_views_omits_broken_and_no_url_links_from_hal_but_keeps_them_in_json() {
        let (_dir, mut store) = tmp_store();
        seed_task(&mut store, "fix-bug", "Fix bug");
        let body = "[ok](rdm:task/fix-bug) [gone](rdm:task/does-not-exist) [src](rdm:src/a/b.rs)";
        let links = resolve_body_links(&store, "demo", None, body).unwrap();
        assert_eq!(links.len(), 3);
        assert!(matches!(links[0].action, RenderAction::ItemLink { .. }));
        assert!(matches!(links[1].action, RenderAction::Broken { .. }));
        assert_eq!(
            links[2].action,
            RenderAction::CodeLink {
                web_url: None,
                no_link_display: "a/b.rs".to_string(),
            }
        );

        let (hal, json) = outgoing_link_views(&links);
        // Every occurrence, navigable or not, gets a raw JSON entry.
        assert_eq!(json.len(), 3);
        // Only the navigable item link makes it into the HAL array.
        assert_eq!(hal.len(), 1);
        assert_eq!(hal[0].href, "/projects/demo/tasks/fix-bug");
    }

    #[test]
    fn item_link_to_plan_gets_bare_item_class() {
        let (_dir, mut store) = tmp_store();
        seed_task(&mut store, "fix-bug", "Fix bug");
        seed_plan(
            &mut store,
            "impl-fix-bug",
            "Implement fix",
            "fix-bug",
            "Plan body.",
        );
        let body = "See [the plan](rdm:plan/impl-fix-bug) for details.";
        let links = resolve_body_links(&store, "demo", None, body).unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!(
            links[0].action,
            RenderAction::ItemLink {
                // The project-page stub href until plan detail routes land.
                href: "/projects/demo".to_string(),
                class: "rdm-link-item".to_string(),
            }
        );

        let html = crate::markdown::render_markdown_with_links(body, &links);
        assert!(
            html.contains(r#"<a class="rdm-link-item" href="/projects/demo">the plan</a>"#),
            "got: {html}"
        );
    }

    #[test]
    fn item_link_to_dangling_plan_is_broken() {
        let (_dir, store) = tmp_store();
        let body = "See [gone](rdm:plan/does-not-exist).";
        let links = resolve_body_links(&store, "demo", None, body).unwrap();
        assert_eq!(links.len(), 1);
        assert!(
            matches!(&links[0].action, RenderAction::Broken { reason } if reason.contains("does-not-exist"))
        );
    }

    #[test]
    fn referenced_by_finds_plan_linking_task() {
        let (_dir, mut store) = tmp_store();
        seed_task(&mut store, "fix-bug", "Fix bug");
        seed_task(&mut store, "other-task", "Other task");
        seed_plan(
            &mut store,
            "impl-fix-bug",
            "Implement fix",
            "other-task",
            "Depends on [the task](rdm:task/fix-bug).",
        );
        let entries = referenced_by(
            &store,
            "demo",
            &ItemRef::Task {
                slug: "fix-bug".to_string(),
            },
        )
        .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind_label, "Plan");
        assert_eq!(entries[0].title, "Implement fix");
        assert_eq!(entries[0].href, "/projects/demo");
    }
}

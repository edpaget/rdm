//! Resolution of parsed [`Link`]s and backlink discovery.
//!
//! [`crate::link::extract_links`] finds `rdm:` links inside a markdown body;
//! this module turns each parsed [`Link`] into something a consumer can act
//! on ([`Resolved`]), and inverts the relationship — given a target, finds
//! every document that references it ([`backlinks`]).

use crate::error::Result;
use crate::link::{BacklinkEntry, DocRef, ItemRef, Link, Resolved};
use crate::model::Project;
use crate::store::Store;

/// Resolves an item reference (`rdm:roadmap/...`, `rdm:phase/...`,
/// `rdm:task/...`) against the store.
///
/// Existence is checked the same way [`crate::io::load_roadmap`],
/// [`crate::io::load_phase`], and [`crate::io::load_task`] check it for
/// their own lookups — a direct [`Store::exists`] path check — except for a
/// [`ItemRef::Phase`] whose `stem` is a bare number, which is first resolved
/// against the roadmap's real phase stems via
/// [`crate::ops::phase::resolve_phase_stem`] (the same list-and-match those
/// callers use), matching `rdm phase show`/`update`'s own lookup convention.
///
/// A target that does not exist — including a *renamed* target: this store
/// has no redirect/alias mechanism, so a rename is indistinguishable from a
/// deletion — resolves to `Resolved::Item { exists: false, .. }`, never an
/// error. This is a deliberate, permanent contract: a dangling link is
/// normal (the author renamed or deleted the thing it pointed at), not
/// exceptional, so callers that want to warn about it inspect `exists`
/// themselves rather than handling an `Err`.
///
/// # Errors
///
/// Returns an error only for a genuine store failure while resolving a
/// numeric phase identifier — e.g. [`crate::error::Error::Io`] if the
/// roadmap's phase directory cannot be listed, or
/// [`crate::error::Error::FrontmatterParse`]/
/// [`crate::error::Error::FrontmatterMissing`] if a phase file along the way
/// has invalid frontmatter. An unresolvable roadmap or phase number itself
/// is *not* one of these errors — it folds into `exists: false` per the
/// contract above.
pub fn resolve_item_link(store: &impl Store, project: &str, target: &ItemRef) -> Result<Resolved> {
    let exists = match target {
        ItemRef::Roadmap { roadmap } => store.exists(&crate::paths::roadmap_path(project, roadmap)),
        ItemRef::Task { slug } => store.exists(&crate::paths::task_path(project, slug)),
        ItemRef::Phase { roadmap, stem } => {
            match crate::ops::phase::resolve_phase_stem(store, project, roadmap, stem) {
                Ok(resolved_stem) => {
                    store.exists(&crate::paths::phase_path(project, roadmap, &resolved_stem))
                }
                // Unknown roadmap, or a phase number that doesn't match any
                // phase in it — both are "doesn't exist", not a store
                // failure.
                Err(
                    crate::error::Error::RoadmapNotFound(_) | crate::error::Error::PhaseNotFound(_),
                ) => false,
                Err(e) => return Err(e),
            }
        }
    };
    Ok(Resolved::Item {
        target: target.clone(),
        exists,
    })
}

/// Builds a GitHub-style web URL for a source location.
///
/// `repo` may or may not end with a trailing `/` — exactly one is trimmed
/// before appending `/blob/{rev}/{path}`, so
/// `https://github.com/org/repo` and `https://github.com/org/repo/` produce
/// byte-identical output. `lines` appends a `#L<start>[-L<end>]` fragment in
/// GitHub's own format (1-based, inclusive): omitted when `lines` is
/// `None`, `#L<start>` (not `#L<start>-L<start>`) when the range's start and
/// end are equal, `#L<start>-L<end>` otherwise.
#[must_use]
pub fn build_web_url(
    repo: &str,
    rev: &str,
    path: &str,
    lines: Option<(u32, Option<u32>)>,
) -> String {
    let repo = repo.strip_suffix('/').unwrap_or(repo);
    let mut url = format!("{repo}/blob/{rev}/{path}");
    if let Some((start, end)) = lines {
        match end {
            Some(end) if end != start => url.push_str(&format!("#L{start}-L{end}")),
            _ => url.push_str(&format!("#L{start}")),
        }
    }
    url
}

/// Resolves a code reference (`rdm:src/...`) against a project's `source`
/// configuration.
///
/// Revision precedence, most to least specific: an explicit `@<rev>` on the
/// link wins; otherwise the containing phase/task's stamped `commit`
/// (`containing_commit`, which the caller — the ops-layer function that
/// already has that `Phase`/`Task` loaded — passes in); otherwise `None`.
///
/// Deliberately **not** in this chain: `review_sha`. It is also present on
/// `Phase`/`Task`, but it stamps the source-repo HEAD at the moment the item
/// entered `needs-review` — a different, possibly-superseded lifecycle
/// event from `commit` (the SHA that actually landed). Falling back to it
/// would point a resolved code link at a snapshot that may no longer be
/// reachable from the item's real landed history.
///
/// `web_url` is built from `project.source` (see
/// [`crate::model::Source`]) — deliberately distinct from, and never
/// reading, [`crate::config::Config`]'s unrelated plan-repo-wide
/// `default_branch` field (that one scopes post-commit `Done:`-hook branch
/// filtering, one value per plan repo; this one is per-project, for
/// `rdm:src/` resolution). When `project.source` is `None`, `web_url` is
/// `None` unconditionally. When it is `Some`, a URL is always built: using
/// the resolved `rev` above when `Some`, else `project.source`'s own
/// `default_branch`, else the hardcoded literal `"main"` — a pragmatic
/// default for a project that configured a `source.repo` but never set a
/// `default_branch`, documented here as a planning decision rather than
/// derived from anywhere else.
///
/// # Errors
///
/// Does not fail today — path/rev validity is intentionally out of scope
/// for core (see the phase body: core has no source-repo checkout to
/// validate against, and layering that on top is `link check`'s job in a
/// later phase). Returns [`Result`] for API symmetry with
/// [`resolve_item_link`]/[`resolve_link`] and so a future validating
/// variant is a non-breaking change.
#[allow(clippy::unnecessary_wraps)]
pub fn resolve_code_link(
    project: &Project,
    containing_commit: Option<&str>,
    path: &str,
    explicit_rev: Option<&str>,
    lines: Option<(u32, Option<u32>)>,
) -> Result<Resolved> {
    let rev = explicit_rev
        .map(String::from)
        .or_else(|| containing_commit.map(String::from));
    let web_url = project.source.as_ref().map(|source| {
        let rev_or_default = rev
            .clone()
            .or_else(|| source.default_branch.clone())
            .unwrap_or_else(|| "main".to_string());
        build_web_url(&source.repo, &rev_or_default, path, lines)
    });
    Ok(Resolved::Code {
        path: path.to_string(),
        rev,
        lines,
        web_url,
    })
}

/// Resolves any [`Link`] — item or code — against the store. The single
/// entry point later CLI/server/MCP phases dispatch through regardless of
/// link kind.
///
/// `containing_commit` is the stamped `commit` of the document the link was
/// found in (a `Phase` or `Task`'s `commit` field), used only for
/// [`Link::Code`]'s rev-default precedence — see [`resolve_code_link`]. Pass
/// `None` for links found in a roadmap body or a review, neither of which
/// carries a stamped commit.
///
/// # Errors
///
/// For [`Link::Item`], delegates to [`resolve_item_link`] — see its `#
/// Errors`. For [`Link::Code`], the project's `source` config must be loaded
/// first (to build a `web_url`) via [`crate::io::load_project`], which can
/// return [`crate::error::Error::ProjectNotFound`] if `project` doesn't
/// exist, [`crate::error::Error::Io`] if the project file cannot be read, or
/// [`crate::error::Error::FrontmatterMissing`]/
/// [`crate::error::Error::FrontmatterParse`] if its frontmatter is invalid —
/// all propagated unmodified via `?`; otherwise see [`resolve_code_link`],
/// which does not fail today.
pub fn resolve_link(
    store: &impl Store,
    project: &str,
    containing_commit: Option<&str>,
    link: &Link,
) -> Result<Resolved> {
    match link {
        Link::Item(target) => resolve_item_link(store, project, target),
        Link::Code { path, rev, lines } => {
            let project_doc = crate::io::load_project(store, project)?;
            resolve_code_link(
                &project_doc.frontmatter,
                containing_commit,
                path,
                rev.as_deref(),
                *lines,
            )
        }
    }
}

/// Scans every roadmap, phase, task, and review body (and review comment)
/// in `project` for `rdm:` item links pointing at `target`, returning one
/// [`BacklinkEntry`] per occurrence.
///
/// This is a full body scan over the whole project (`O(n)` in the number of
/// documents and their size) — no index is built or maintained. Fine at the
/// scale the phase body scopes this feature to; not optimized here, since
/// nothing in the phase requires it.
///
/// The same target referenced twice in one document (e.g. two links to the
/// same task in one phase body) produces two distinct entries with
/// different byte ranges — occurrences are never deduplicated. A review
/// whose own target has been renamed or deleted is scanned like any other
/// review; per [`resolve_item_link`]'s contract, a dangling reference is
/// never an error, so an orphaned review's comments are still scanned
/// without failing the whole call.
///
/// Results are sorted by [`DocRef`]'s derived order (document kind, then
/// the document's own identity), then by byte offset within the same
/// document, so output is deterministic across repeated calls even though
/// the underlying listing order is not otherwise guaranteed.
///
/// A [`ItemRef::Phase`] whose `stem` is a bare number is normalized to the
/// roadmap's canonical stem before matching (via [`normalize_item_ref`]) —
/// both `target` itself and every item link found in a scanned body — so a
/// link written as `rdm:phase/auth/1` and a caller-supplied
/// `rdm:phase/auth/phase-1-design` target are treated as the same reference,
/// exactly as [`resolve_item_link`] already treats the two forms as
/// equivalent for existence checks.
///
/// # Errors
///
/// Returns [`crate::error::Error::ProjectNotFound`] if the project doesn't
/// exist, [`crate::error::Error::RoadmapNotFound`] if a roadmap disappears
/// between listing and reading its phases, [`crate::error::Error::Io`] if a
/// directory cannot be listed or a file cannot be read, or
/// [`crate::error::Error::FrontmatterMissing`]/
/// [`crate::error::Error::FrontmatterParse`] if any scanned document has
/// invalid frontmatter — including while normalizing a numeric phase stem
/// found in a link (see [`normalize_item_ref`]).
pub fn backlinks(
    store: &impl Store,
    project: &str,
    target: &ItemRef,
) -> Result<Vec<BacklinkEntry>> {
    // Normalize the caller's target the same way `resolve_item_link` would,
    // so a caller passing a bare-number phase stem matches links written
    // with the canonical stem, and vice versa — see `normalize_item_ref`.
    let target = normalize_item_ref(store, project, target)?;
    let mut entries = Vec::new();

    for roadmap_doc in crate::ops::roadmap::list_roadmaps(store, project, None, None)? {
        let roadmap = roadmap_doc.frontmatter.roadmap.clone();
        collect_item_links(
            store,
            project,
            &roadmap_doc.body,
            &target,
            DocRef::Roadmap {
                roadmap: roadmap.clone(),
            },
            &mut entries,
        )?;

        for (stem, phase_doc) in crate::ops::phase::list_phases(store, project, &roadmap)? {
            collect_item_links(
                store,
                project,
                &phase_doc.body,
                &target,
                DocRef::Phase {
                    roadmap: roadmap.clone(),
                    stem,
                },
                &mut entries,
            )?;
        }
    }

    for (slug, task_doc) in crate::ops::task::list_tasks(store, project)? {
        collect_item_links(
            store,
            project,
            &task_doc.body,
            &target,
            DocRef::Task { slug },
            &mut entries,
        )?;
    }

    for (id, review_doc) in crate::ops::reviews::list_reviews(store, project)? {
        collect_item_links(
            store,
            project,
            &review_doc.body,
            &target,
            DocRef::Review {
                id: id.clone(),
                comment: None,
            },
            &mut entries,
        )?;
        for comment in &review_doc.frontmatter.comments {
            collect_item_links(
                store,
                project,
                &comment.body,
                &target,
                DocRef::Review {
                    id: id.clone(),
                    comment: Some(comment.id),
                },
                &mut entries,
            )?;
        }
    }

    entries.sort_by(|a, b| {
        a.document
            .cmp(&b.document)
            .then_with(|| a.byte_range.start.cmp(&b.byte_range.start))
    });

    Ok(entries)
}

/// Normalizes an [`ItemRef`] the same way [`resolve_item_link`] resolves one
/// before checking existence: a [`ItemRef::Phase`] whose `stem` is a bare
/// number is resolved to the roadmap's real phase stem via
/// [`crate::ops::phase::resolve_phase_stem`]. Every other variant, and a
/// `Phase` whose `stem` is already non-numeric, passes through unchanged.
///
/// This is what lets [`backlinks`] treat `rdm:phase/auth/1` and
/// `rdm:phase/auth/phase-1-design` as the same target — the two forms
/// [`resolve_item_link`] already treats as equivalent — regardless of which
/// form a link in a document body used or which form the caller's target
/// used.
///
/// An unresolvable roadmap or phase number folds to the input unchanged
/// (mirroring [`resolve_item_link`]'s `exists: false` contract): the
/// unresolved numeric stem simply will not equal any canonical stem, so it
/// drops out of the match rather than erroring.
///
/// # Errors
///
/// Returns an error only for a genuine store failure while resolving a
/// numeric phase identifier, per
/// [`crate::ops::phase::resolve_phase_stem`] — e.g.
/// [`crate::error::Error::Io`] or
/// [`crate::error::Error::FrontmatterParse`]/
/// [`crate::error::Error::FrontmatterMissing`].
fn normalize_item_ref(store: &impl Store, project: &str, item_ref: &ItemRef) -> Result<ItemRef> {
    match item_ref {
        ItemRef::Phase { roadmap, stem } => {
            match crate::ops::phase::resolve_phase_stem(store, project, roadmap, stem) {
                Ok(resolved_stem) => Ok(ItemRef::Phase {
                    roadmap: roadmap.clone(),
                    stem: resolved_stem,
                }),
                // Unknown roadmap, or a phase number that doesn't match any
                // phase in it — leave the reference as-is; it simply won't
                // equal a canonical stem, per this function's contract.
                Err(
                    crate::error::Error::RoadmapNotFound(_) | crate::error::Error::PhaseNotFound(_),
                ) => Ok(item_ref.clone()),
                Err(e) => Err(e),
            }
        }
        other => Ok(other.clone()),
    }
}

/// Finds every `rdm:` item link in `body` that normalizes (via
/// [`normalize_item_ref`]) to exactly `target`, pushing one
/// [`BacklinkEntry`] per occurrence tagged with `doc_ref`.
///
/// # Errors
///
/// Returns an error only for a genuine store failure while normalizing a
/// numeric phase stem found in a link — see [`normalize_item_ref`].
fn collect_item_links(
    store: &impl Store,
    project: &str,
    body: &str,
    target: &ItemRef,
    doc_ref: DocRef,
    out: &mut Vec<BacklinkEntry>,
) -> Result<()> {
    let (links, _diagnostics) = crate::link::extract_links(body);
    for (range, link) in links {
        if let Link::Item(item_ref) = link {
            let normalized = normalize_item_ref(store, project, &item_ref)?;
            if normalized == *target {
                out.push(BacklinkEntry {
                    document: doc_ref.clone(),
                    byte_range: range,
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::model::{
        Anchor, CommentDoc, CommentDocKind, Priority, Review, ReviewComment, ReviewCommentStatus,
        ReviewState, ReviewTarget, Source,
    };
    use crate::ops::CreateTask;
    use crate::store::MemoryStore;
    use chrono::{TimeZone, Utc};

    fn setup() -> MemoryStore {
        let mut store = MemoryStore::new();
        crate::ops::init::init(&mut store).unwrap();
        crate::ops::project::create_project(&mut store, "demo", "Demo").unwrap();
        store
    }

    // --- AC1: resolve_item_link never errors on a dangling target ---

    #[test]
    fn resolve_item_link_existing_task() {
        let mut store = setup();
        crate::ops::task::create_task(
            &mut store,
            CreateTask {
                project: "demo",
                slug: "fix-login",
                title: "Fix login",
                priority: Priority::Medium,
                tags: None,
                body: Some("Body."),
            },
        )
        .unwrap();
        let target = ItemRef::Task {
            slug: "fix-login".to_string(),
        };
        let resolved = resolve_item_link(&store, "demo", &target).unwrap();
        assert_eq!(
            resolved,
            Resolved::Item {
                target,
                exists: true
            }
        );
    }

    #[test]
    fn resolve_item_link_dangling_task_never_errors() {
        let store = setup();
        let target = ItemRef::Task {
            slug: "never-existed".to_string(),
        };
        // .unwrap() here, not .is_ok() — proves the Ok path is a real
        // Resolved::Item, not merely that no panic occurred.
        let resolved = resolve_item_link(&store, "demo", &target).unwrap();
        assert_eq!(
            resolved,
            Resolved::Item {
                target,
                exists: false
            }
        );
    }

    #[test]
    fn resolve_item_link_dangling_roadmap_never_errors() {
        let store = setup();
        let target = ItemRef::Roadmap {
            roadmap: "ghost".to_string(),
        };
        let resolved = resolve_item_link(&store, "demo", &target).unwrap();
        assert_eq!(
            resolved,
            Resolved::Item {
                target,
                exists: false
            }
        );
    }

    #[test]
    fn resolve_item_link_dangling_phase_in_unknown_roadmap_never_errors() {
        let store = setup();
        let target = ItemRef::Phase {
            roadmap: "ghost".to_string(),
            stem: "phase-1-design".to_string(),
        };
        let resolved = resolve_item_link(&store, "demo", &target).unwrap();
        assert_eq!(
            resolved,
            Resolved::Item {
                target,
                exists: false
            }
        );
    }

    #[test]
    fn resolve_item_link_dangling_phase_numeric_stem_unknown_roadmap_never_errors() {
        // Mirrors the test above but with a *numeric* stem, which is the
        // one shape that actually drives execution into
        // `resolve_phase_stem`'s `Err(RoadmapNotFound)` arm — a non-numeric
        // stem short-circuits before ever calling `list_phases`.
        let store = setup();
        let target = ItemRef::Phase {
            roadmap: "ghost".to_string(),
            stem: "1".to_string(),
        };
        let resolved = resolve_item_link(&store, "demo", &target).unwrap();
        assert_eq!(
            resolved,
            Resolved::Item {
                target,
                exists: false
            }
        );
    }

    #[test]
    fn resolve_item_link_dangling_phase_numeric_stem_not_found_in_known_roadmap_never_errors() {
        // The roadmap exists and has a phase 1, but phase 99 does not —
        // this drives `resolve_phase_stem`'s `Err(PhaseNotFound)` arm,
        // which `resolve_item_link` must also fold into `exists: false`
        // rather than propagating.
        let mut store = setup();
        crate::ops::roadmap::create_roadmap(
            &mut store,
            crate::ops::CreateRoadmap {
                project: "demo",
                slug: "auth",
                title: "Auth",
                ..Default::default()
            },
        )
        .unwrap();
        crate::ops::phase::create_phase(
            &mut store,
            crate::ops::CreatePhase {
                project: "demo",
                roadmap: "auth",
                slug: "design",
                title: "Design",
                number: Some(1),
                body: None,
                tags: None,
                difficulty: crate::ops::DifficultyUpdate::Keep,
                model: crate::ops::ModelTierUpdate::Keep,
            },
        )
        .unwrap();
        let target = ItemRef::Phase {
            roadmap: "auth".to_string(),
            stem: "99".to_string(),
        };
        let resolved = resolve_item_link(&store, "demo", &target).unwrap();
        assert_eq!(
            resolved,
            Resolved::Item {
                target,
                exists: false
            }
        );
    }

    #[test]
    fn resolve_item_link_existing_phase_by_stem() {
        let mut store = setup();
        crate::ops::roadmap::create_roadmap(
            &mut store,
            crate::ops::CreateRoadmap {
                project: "demo",
                slug: "auth",
                title: "Auth",
                ..Default::default()
            },
        )
        .unwrap();
        crate::ops::phase::create_phase(
            &mut store,
            crate::ops::CreatePhase {
                project: "demo",
                roadmap: "auth",
                slug: "design",
                title: "Design",
                number: Some(1),
                body: None,
                tags: None,
                difficulty: crate::ops::DifficultyUpdate::Keep,
                model: crate::ops::ModelTierUpdate::Keep,
            },
        )
        .unwrap();
        let target = ItemRef::Phase {
            roadmap: "auth".to_string(),
            stem: "phase-1-design".to_string(),
        };
        let resolved = resolve_item_link(&store, "demo", &target).unwrap();
        assert_eq!(
            resolved,
            Resolved::Item {
                target,
                exists: true
            }
        );
    }

    #[test]
    fn resolve_item_link_existing_phase_by_number() {
        let mut store = setup();
        crate::ops::roadmap::create_roadmap(
            &mut store,
            crate::ops::CreateRoadmap {
                project: "demo",
                slug: "auth",
                title: "Auth",
                ..Default::default()
            },
        )
        .unwrap();
        crate::ops::phase::create_phase(
            &mut store,
            crate::ops::CreatePhase {
                project: "demo",
                roadmap: "auth",
                slug: "design",
                title: "Design",
                number: Some(1),
                body: None,
                tags: None,
                difficulty: crate::ops::DifficultyUpdate::Keep,
                model: crate::ops::ModelTierUpdate::Keep,
            },
        )
        .unwrap();
        // The link's raw `stem` field is the bare number, unresolved — the
        // way `Link::Item`'s `ItemRef::Phase.stem` looks when the author
        // wrote `rdm:phase/auth/1` rather than the real stem.
        let target = ItemRef::Phase {
            roadmap: "auth".to_string(),
            stem: "1".to_string(),
        };
        let resolved = resolve_item_link(&store, "demo", &target).unwrap();
        assert_eq!(
            resolved,
            Resolved::Item {
                target,
                exists: true
            }
        );
    }

    // --- resolve_link: the single dispatch entry point ---

    #[test]
    fn resolve_link_dispatches_item_links_to_resolve_item_link() {
        let mut store = setup();
        crate::ops::task::create_task(
            &mut store,
            CreateTask {
                project: "demo",
                slug: "fix-login",
                title: "Fix login",
                priority: Priority::Medium,
                tags: None,
                body: Some("Body."),
            },
        )
        .unwrap();
        let target = ItemRef::Task {
            slug: "fix-login".to_string(),
        };
        let via_resolve_link =
            resolve_link(&store, "demo", None, &Link::Item(target.clone())).unwrap();
        let via_resolve_item_link = resolve_item_link(&store, "demo", &target).unwrap();
        assert_eq!(via_resolve_link, via_resolve_item_link);
        assert_eq!(
            via_resolve_link,
            Resolved::Item {
                target,
                exists: true
            }
        );
    }

    #[test]
    fn resolve_link_dispatches_code_links_and_threads_containing_commit() {
        let store = setup();
        let link = Link::Code {
            path: "src/a.rs".to_string(),
            rev: None,
            lines: Some((5, Some(12))),
        };
        let resolved = resolve_link(&store, "demo", Some("abc123"), &link).unwrap();
        assert_eq!(
            resolved,
            Resolved::Code {
                path: "src/a.rs".to_string(),
                rev: Some("abc123".to_string()),
                lines: Some((5, Some(12))),
                web_url: None,
            }
        );
    }

    #[test]
    fn resolve_link_dispatches_code_links_and_lets_explicit_rev_win_over_containing_commit() {
        let store = setup();
        let link = Link::Code {
            path: "src/a.rs".to_string(),
            rev: Some("def456".to_string()),
            lines: None,
        };
        let resolved = resolve_link(&store, "demo", Some("abc123"), &link).unwrap();
        assert_eq!(
            resolved,
            Resolved::Code {
                path: "src/a.rs".to_string(),
                rev: Some("def456".to_string()),
                lines: None,
                web_url: None,
            }
        );
    }

    #[test]
    fn resolve_link_code_link_errors_project_not_found() {
        let store = setup();
        let link = Link::Code {
            path: "src/a.rs".to_string(),
            rev: None,
            lines: None,
        };
        let err = resolve_link(&store, "no-such-project", None, &link).unwrap_err();
        assert!(
            matches!(err, crate::error::Error::ProjectNotFound(name) if name == "no-such-project")
        );
    }

    // --- AC2: code link rev-default precedence ---

    #[test]
    fn resolve_code_link_falls_back_to_containing_commit() {
        let project = Project {
            name: "demo".to_string(),
            title: "Demo".to_string(),
            source: None,
        };
        let resolved = resolve_code_link(&project, Some("abc123"), "a/b.rs", None, None).unwrap();
        assert_eq!(
            resolved,
            Resolved::Code {
                path: "a/b.rs".to_string(),
                rev: Some("abc123".to_string()),
                lines: None,
                web_url: None,
            }
        );
    }

    #[test]
    fn resolve_code_link_explicit_rev_overrides_stamp() {
        let project = Project {
            name: "demo".to_string(),
            title: "Demo".to_string(),
            source: None,
        };
        let resolved =
            resolve_code_link(&project, Some("abc123"), "a/b.rs", Some("def456"), None).unwrap();
        assert_eq!(
            resolved,
            Resolved::Code {
                path: "a/b.rs".to_string(),
                rev: Some("def456".to_string()),
                lines: None,
                web_url: None,
            }
        );
    }

    #[test]
    fn resolve_code_link_no_stamp_no_explicit_rev_is_none() {
        let project = Project {
            name: "demo".to_string(),
            title: "Demo".to_string(),
            source: None,
        };
        let resolved = resolve_code_link(&project, None, "a/b.rs", None, None).unwrap();
        assert_eq!(
            resolved,
            Resolved::Code {
                path: "a/b.rs".to_string(),
                rev: None,
                lines: None,
                web_url: None,
            }
        );
    }

    // --- AC3: web_url construction ---

    #[test]
    fn build_web_url_trailing_slash_is_normalized() {
        let with_slash = build_web_url("https://github.com/org/repo/", "main", "a/b.rs", None);
        let without_slash = build_web_url("https://github.com/org/repo", "main", "a/b.rs", None);
        assert_eq!(with_slash, without_slash);
        assert_eq!(with_slash, "https://github.com/org/repo/blob/main/a/b.rs");
    }

    #[test]
    fn build_web_url_line_fragment_formats() {
        assert_eq!(
            build_web_url(
                "https://github.com/org/repo",
                "main",
                "a.rs",
                Some((5, None))
            ),
            "https://github.com/org/repo/blob/main/a.rs#L5"
        );
        assert_eq!(
            build_web_url(
                "https://github.com/org/repo",
                "main",
                "a.rs",
                Some((5, Some(5)))
            ),
            "https://github.com/org/repo/blob/main/a.rs#L5"
        );
        assert_eq!(
            build_web_url(
                "https://github.com/org/repo",
                "main",
                "a.rs",
                Some((5, Some(12)))
            ),
            "https://github.com/org/repo/blob/main/a.rs#L5-L12"
        );
    }

    #[test]
    fn resolve_code_link_no_source_yields_no_web_url() {
        let project = Project {
            name: "demo".to_string(),
            title: "Demo".to_string(),
            source: None,
        };
        let resolved = resolve_code_link(&project, Some("abc123"), "a/b.rs", None, None).unwrap();
        assert!(matches!(resolved, Resolved::Code { web_url: None, .. }));
    }

    #[test]
    fn resolve_code_link_with_source_and_rev_builds_url() {
        let project = Project {
            name: "demo".to_string(),
            title: "Demo".to_string(),
            source: Some(Source {
                repo: "https://github.com/org/repo".to_string(),
                default_branch: None,
            }),
        };
        let resolved = resolve_code_link(
            &project,
            Some("abc123"),
            "a/b.rs",
            None,
            Some((5, Some(12))),
        )
        .unwrap();
        assert_eq!(
            resolved,
            Resolved::Code {
                path: "a/b.rs".to_string(),
                rev: Some("abc123".to_string()),
                lines: Some((5, Some(12))),
                web_url: Some("https://github.com/org/repo/blob/abc123/a/b.rs#L5-L12".to_string()),
            }
        );
    }

    #[test]
    fn resolve_code_link_with_source_no_rev_falls_back_to_default_branch() {
        let project = Project {
            name: "demo".to_string(),
            title: "Demo".to_string(),
            source: Some(Source {
                repo: "https://github.com/org/repo".to_string(),
                default_branch: Some("develop".to_string()),
            }),
        };
        let resolved = resolve_code_link(&project, None, "a/b.rs", None, None).unwrap();
        assert_eq!(
            resolved,
            Resolved::Code {
                path: "a/b.rs".to_string(),
                rev: None,
                lines: None,
                web_url: Some("https://github.com/org/repo/blob/develop/a/b.rs".to_string()),
            }
        );
    }

    #[test]
    fn resolve_code_link_with_source_no_rev_no_default_branch_falls_back_to_main() {
        let project = Project {
            name: "demo".to_string(),
            title: "Demo".to_string(),
            source: Some(Source {
                repo: "https://github.com/org/repo".to_string(),
                default_branch: None,
            }),
        };
        let resolved = resolve_code_link(&project, None, "a/b.rs", None, None).unwrap();
        assert_eq!(
            resolved,
            Resolved::Code {
                path: "a/b.rs".to_string(),
                rev: None,
                lines: None,
                web_url: Some("https://github.com/org/repo/blob/main/a/b.rs".to_string()),
            }
        );
    }

    // --- AC4: backlinks across roadmap/phase/task/review bodies ---

    fn write_review_fixture(store: &mut MemoryStore, project: &str, id: &str, target_body: &str) {
        let doc = Document {
            frontmatter: Review {
                id: id.to_string(),
                author: "ed".to_string(),
                target: ReviewTarget::Task {
                    slug: "fix-login".to_string(),
                },
                state: ReviewState::Draft,
                verdict: None,
                created: Utc.with_ymd_and_hms(2026, 7, 1, 14, 30, 0).unwrap(),
                submitted: None,
                created_commit: None,
                comments: vec![ReviewComment {
                    id: 1,
                    doc: None,
                    status: ReviewCommentStatus::Open,
                    applied_commit: None,
                    anchor: None,
                    body: target_body.to_string(),
                    reply: None,
                }],
            },
            body: "Whole-document summary, no link here.".to_string(),
        };
        crate::io::write_review(store, project, id, &doc).unwrap();
    }

    #[test]
    fn backlinks_finds_references_across_all_document_kinds_with_stable_order() {
        let mut store = setup();

        crate::ops::roadmap::create_roadmap(
            &mut store,
            crate::ops::CreateRoadmap {
                project: "demo",
                slug: "auth",
                title: "Auth",
                body: Some("See [the fix](rdm:task/fix-login) for background."),
                ..Default::default()
            },
        )
        .unwrap();
        crate::ops::phase::create_phase(
            &mut store,
            crate::ops::CreatePhase {
                project: "demo",
                roadmap: "auth",
                slug: "design",
                title: "Design",
                number: Some(1),
                body: Some("Depends on [fix](rdm:task/fix-login)."),
                tags: None,
                difficulty: crate::ops::DifficultyUpdate::Keep,
                model: crate::ops::ModelTierUpdate::Keep,
            },
        )
        .unwrap();
        crate::ops::task::create_task(
            &mut store,
            CreateTask {
                project: "demo",
                slug: "fix-login",
                title: "Fix login",
                priority: Priority::Medium,
                tags: None,
                body: Some("Body."),
            },
        )
        .unwrap();
        crate::ops::task::create_task(
            &mut store,
            CreateTask {
                project: "demo",
                slug: "other-task",
                title: "Other",
                priority: Priority::Medium,
                tags: None,
                body: Some("Also mentions [fix](rdm:task/fix-login) in passing."),
            },
        )
        .unwrap();
        write_review_fixture(
            &mut store,
            "demo",
            "2026-07-01-1430-a1b2",
            "Please see [the fix](rdm:task/fix-login) before merging.",
        );

        let target = ItemRef::Task {
            slug: "fix-login".to_string(),
        };
        let first = backlinks(&store, "demo", &target).unwrap();
        assert_eq!(first.len(), 4);

        let docs: Vec<&DocRef> = first.iter().map(|e| &e.document).collect();
        assert_eq!(
            docs,
            vec![
                &DocRef::Roadmap {
                    roadmap: "auth".to_string()
                },
                &DocRef::Phase {
                    roadmap: "auth".to_string(),
                    stem: "phase-1-design".to_string(),
                },
                &DocRef::Task {
                    slug: "other-task".to_string()
                },
                &DocRef::Review {
                    id: "2026-07-01-1430-a1b2".to_string(),
                    comment: Some(1),
                },
            ]
        );

        // Stability: calling again yields the identical order.
        let second = backlinks(&store, "demo", &target).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn backlinks_does_not_deduplicate_repeated_references_in_one_document() {
        let mut store = setup();
        crate::ops::task::create_task(
            &mut store,
            CreateTask {
                project: "demo",
                slug: "fix-login",
                title: "Fix login",
                priority: Priority::Medium,
                tags: None,
                body: Some("Body."),
            },
        )
        .unwrap();
        crate::ops::task::create_task(
            &mut store,
            CreateTask {
                project: "demo",
                slug: "twice",
                title: "Twice",
                priority: Priority::Medium,
                tags: None,
                body: Some("See [fix](rdm:task/fix-login) and again [fix](rdm:task/fix-login)."),
            },
        )
        .unwrap();
        let target = ItemRef::Task {
            slug: "fix-login".to_string(),
        };
        let entries = backlinks(&store, "demo", &target).unwrap();
        assert_eq!(entries.len(), 2);
        assert_ne!(entries[0].byte_range, entries[1].byte_range);
    }

    #[test]
    fn backlinks_scopes_a_comment_doc_pointer_without_error() {
        // A comment's `doc` field (roadmap-review phase scoping) is
        // orthogonal to backlink scanning, which reads only comment bodies —
        // confirm a comment carrying `doc` doesn't trip anything.
        let mut store = setup();
        crate::ops::task::create_task(
            &mut store,
            CreateTask {
                project: "demo",
                slug: "fix-login",
                title: "Fix login",
                priority: Priority::Medium,
                tags: None,
                body: Some("Body."),
            },
        )
        .unwrap();
        let doc = Document {
            frontmatter: Review {
                id: "2026-07-01-0800-cccc".to_string(),
                author: "ed".to_string(),
                target: ReviewTarget::Task {
                    slug: "fix-login".to_string(),
                },
                state: ReviewState::Draft,
                verdict: None,
                created: Utc.with_ymd_and_hms(2026, 7, 1, 8, 0, 0).unwrap(),
                submitted: None,
                created_commit: None,
                comments: vec![ReviewComment {
                    id: 3,
                    doc: Some(CommentDoc {
                        kind: CommentDocKind::Phase,
                        stem: "phase-1-design".to_string(),
                    }),
                    status: ReviewCommentStatus::Open,
                    applied_commit: None,
                    anchor: Some(Anchor::TextQuote {
                        quote: "quoted".to_string(),
                        prefix: String::new(),
                        suffix: String::new(),
                    }),
                    body: "See [fix](rdm:task/fix-login).".to_string(),
                    reply: None,
                }],
            },
            body: String::new(),
        };
        crate::io::write_review(&mut store, "demo", "2026-07-01-0800-cccc", &doc).unwrap();

        let target = ItemRef::Task {
            slug: "fix-login".to_string(),
        };
        let entries = backlinks(&store, "demo", &target).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].document,
            DocRef::Review {
                id: "2026-07-01-0800-cccc".to_string(),
                comment: Some(3),
            }
        );
    }

    #[test]
    fn backlinks_matches_bare_number_phase_link_against_canonical_stem_target() {
        // Mirrors `resolve_item_link_existing_phase_by_number`: a link
        // written with rdm's bare-number phase shorthand
        // (`rdm:phase/auth/1`) must be found when querying with the
        // phase's canonical stem, which is the form any real caller
        // holding an actual Phase document will have.
        let mut store = setup();
        crate::ops::roadmap::create_roadmap(
            &mut store,
            crate::ops::CreateRoadmap {
                project: "demo",
                slug: "auth",
                title: "Auth",
                ..Default::default()
            },
        )
        .unwrap();
        crate::ops::phase::create_phase(
            &mut store,
            crate::ops::CreatePhase {
                project: "demo",
                roadmap: "auth",
                slug: "design",
                title: "Design",
                number: Some(1),
                body: None,
                tags: None,
                difficulty: crate::ops::DifficultyUpdate::Keep,
                model: crate::ops::ModelTierUpdate::Keep,
            },
        )
        .unwrap();
        crate::ops::task::create_task(
            &mut store,
            CreateTask {
                project: "demo",
                slug: "references-by-number",
                title: "References by number",
                priority: Priority::Medium,
                tags: None,
                body: Some("See [the phase](rdm:phase/auth/1) for background."),
            },
        )
        .unwrap();

        let target = ItemRef::Phase {
            roadmap: "auth".to_string(),
            stem: "phase-1-design".to_string(),
        };
        let entries = backlinks(&store, "demo", &target).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].document,
            DocRef::Task {
                slug: "references-by-number".to_string(),
            }
        );

        // And the reverse: querying with the bare-number form still finds
        // a link written with the canonical stem.
        crate::ops::task::create_task(
            &mut store,
            CreateTask {
                project: "demo",
                slug: "references-by-stem",
                title: "References by stem",
                priority: Priority::Medium,
                tags: None,
                body: Some("See [the phase](rdm:phase/auth/phase-1-design) for background."),
            },
        )
        .unwrap();
        let numeric_target = ItemRef::Phase {
            roadmap: "auth".to_string(),
            stem: "1".to_string(),
        };
        let entries = backlinks(&store, "demo", &numeric_target).unwrap();
        assert_eq!(entries.len(), 2);
        let docs: Vec<&DocRef> = entries.iter().map(|e| &e.document).collect();
        assert!(docs.contains(&&DocRef::Task {
            slug: "references-by-number".to_string(),
        }));
        assert!(docs.contains(&&DocRef::Task {
            slug: "references-by-stem".to_string(),
        }));
    }

    #[test]
    fn backlinks_numeric_target_in_unknown_roadmap_degrades_gracefully() {
        // Mirrors `resolve_item_link_dangling_phase_numeric_stem_unknown_roadmap_never_errors`
        // for `normalize_item_ref`'s identical fold-to-unchanged branch: a
        // numeric stem against a roadmap that doesn't exist must not error
        // out of `backlinks`, and (having nothing to normalize to) simply
        // finds no matches.
        let store = setup();
        let target = ItemRef::Phase {
            roadmap: "ghost".to_string(),
            stem: "1".to_string(),
        };
        let entries = backlinks(&store, "demo", &target).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn backlinks_numeric_target_not_found_in_known_roadmap_degrades_gracefully() {
        // Mirrors `resolve_item_link_dangling_phase_numeric_stem_not_found_in_known_roadmap_never_errors`:
        // the roadmap exists but has no phase 99, so `normalize_item_ref`
        // folds to the input unchanged instead of erroring, and `backlinks`
        // still completes successfully with no matches.
        let mut store = setup();
        crate::ops::roadmap::create_roadmap(
            &mut store,
            crate::ops::CreateRoadmap {
                project: "demo",
                slug: "auth",
                title: "Auth",
                ..Default::default()
            },
        )
        .unwrap();
        crate::ops::phase::create_phase(
            &mut store,
            crate::ops::CreatePhase {
                project: "demo",
                roadmap: "auth",
                slug: "design",
                title: "Design",
                number: Some(1),
                body: None,
                tags: None,
                difficulty: crate::ops::DifficultyUpdate::Keep,
                model: crate::ops::ModelTierUpdate::Keep,
            },
        )
        .unwrap();
        let target = ItemRef::Phase {
            roadmap: "auth".to_string(),
            stem: "99".to_string(),
        };
        let entries = backlinks(&store, "demo", &target).unwrap();
        assert!(entries.is_empty());
    }
}

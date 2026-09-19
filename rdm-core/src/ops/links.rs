//! Resolution of parsed [`Link`]s and backlink discovery.
//!
//! [`crate::link::extract_links`] finds `rdm:` links inside a markdown body;
//! this module turns each parsed [`Link`] into something a consumer can act
//! on ([`Resolved`]), and inverts the relationship — given a target, finds
//! every document that references it ([`backlinks`]).

use std::ops::Range;

use crate::error::Result;
use crate::link::{BacklinkEntry, BacklinkRef, DocRef, ItemRef, Link, Resolved};
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
        ItemRef::Plan { slug } => store.exists(&crate::paths::plan_path(project, slug)),
        ItemRef::Phase { roadmap, stem } => {
            let (resolved_stem, found) = resolve_phase_stem_lenient(store, project, roadmap, stem)?;
            found && store.exists(&crate::paths::phase_path(project, roadmap, &resolved_stem))
        }
        // A change names source-repo commits, not a plan-repo document, so
        // there is nothing here to exist. `crate::link::parse` rejects the
        // `change` kind outright, so this is only reachable from a caller
        // holding a `ReviewTarget` directly.
        ItemRef::Change { .. } => {
            return Ok(Resolved::Broken {
                reason: crate::error::Error::ChangeTargetHasNoDocument(target.label()).to_string(),
            });
        }
    };
    Ok(Resolved::Item {
        target: target.clone(),
        exists,
    })
}

/// Resolves a possibly-numeric phase stem to the roadmap's canonical stem,
/// via [`crate::ops::phase::resolve_phase_stem`], folding an unknown roadmap
/// or an unmatched phase number into "not found" (`found: false`, `stem`
/// returned as given) rather than propagating them as errors — the same
/// leniency [`resolve_item_link`] has always applied. Any other error (a
/// genuine store failure, e.g. `Io` or a `FrontmatterParse`) is propagated.
///
/// Shared by [`resolve_item_link`] (existence check) and [`item_ref_path`]
/// (path lookup) so numeric-phase-stem normalization and its error-folding
/// policy live in exactly one place.
///
/// # Errors
///
/// Propagates any [`crate::ops::phase::resolve_phase_stem`] error other than
/// [`crate::error::Error::RoadmapNotFound`]/[`crate::error::Error::PhaseNotFound`].
fn resolve_phase_stem_lenient(
    store: &impl Store,
    project: &str,
    roadmap: &str,
    stem: &str,
) -> Result<(String, bool)> {
    match crate::ops::phase::resolve_phase_stem(store, project, roadmap, stem) {
        Ok(resolved_stem) => Ok((resolved_stem, true)),
        // Unknown roadmap, or a phase number that doesn't match any phase in
        // it — both are "doesn't exist", not a store failure.
        Err(crate::error::Error::RoadmapNotFound(_) | crate::error::Error::PhaseNotFound(_)) => {
            Ok((stem.to_string(), false))
        }
        Err(e) => Err(e),
    }
}

/// Computes the plan-repo-relative path an [`ItemRef`] names — the single
/// authoritative implementation of "what path does this reference resolve
/// to", shared by [`resolve_item_link`]'s own existence check (via
/// [`resolve_phase_stem_lenient`]) and any caller that additionally wants
/// the path itself (`rdm-cli`'s `link resolve`/`link list`, which pass it
/// through to [`crate::json::resolved_to_json`] since
/// [`Resolved::Item`](crate::link::Resolved::Item) doesn't carry one — see
/// that type's doc comment).
///
/// This does not check whether the resulting path exists — see
/// [`resolve_item_link`] for that. A dangling roadmap or unmatched numeric
/// phase stem is never an error: resolution falls back to the stem exactly
/// as written, mirroring `resolve_item_link`'s never-errors-on-dangling
/// contract, so the returned path may not correspond to any real file.
///
/// # Errors
///
/// Propagates any [`crate::ops::phase::resolve_phase_stem`] error other than
/// `RoadmapNotFound`/`PhaseNotFound` (e.g. `Io`, `FrontmatterParse`) when
/// `target` is a [`ItemRef::Phase`] with a numeric stem.
///
/// Returns [`crate::error::Error::ChangeTargetHasNoDocument`] when `target`
/// is an [`ItemRef::Change`]: a change names source-repository commits, not
/// a plan-repo document, so no path exists to return. This is the one case
/// where the never-errors-on-dangling contract above does not apply — the
/// reference is well-formed, there is simply nothing in the plan repo it
/// could name.
pub fn item_ref_path(
    store: &impl Store,
    project: &str,
    target: &ItemRef,
) -> Result<crate::store::RelPath> {
    Ok(match target {
        ItemRef::Roadmap { roadmap } => crate::paths::roadmap_path(project, roadmap),
        ItemRef::Task { slug } => crate::paths::task_path(project, slug),
        ItemRef::Plan { slug } => crate::paths::plan_path(project, slug),
        ItemRef::Phase { roadmap, stem } => {
            let (resolved_stem, _found) =
                resolve_phase_stem_lenient(store, project, roadmap, stem)?;
            crate::paths::phase_path(project, roadmap, &resolved_stem)
        }
        // No plan-repo path exists for a change target — see
        // [`resolve_item_link`].
        ItemRef::Change { .. } => {
            return Err(crate::error::Error::ChangeTargetHasNoDocument(
                target.label(),
            ));
        }
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
/// entry point later CLI/server phases dispatch through regardless of
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
///
/// Returns [`crate::error::Error::ChangeTargetHasNoDocument`] when `target`
/// is an [`ItemRef::Change`]: a change names source-repository commits, not
/// a plan-repo document, so it can never be the destination of an `rdm:`
/// item link or an `implements` field — the scan would always report zero
/// backlinks, which silently misrepresents "nothing references this" as
/// indistinguishable from "this kind of target can't be referenced at
/// all". Matches the actionable rejection [`load_document_body`] already
/// gives `link check`/`link list` for the same target kind.
pub fn backlinks(
    store: &impl Store,
    project: &str,
    target: &ItemRef,
) -> Result<Vec<BacklinkEntry>> {
    if let ItemRef::Change { .. } = target {
        return Err(crate::error::Error::ChangeTargetHasNoDocument(
            target.label(),
        ));
    }
    // Normalize the caller's target the same way `resolve_item_link` would,
    // so a caller passing a bare-number phase stem matches links written
    // with the canonical stem, and vice versa — see `normalize_item_ref`.
    let target = normalize_item_ref(store, project, target)?;
    let mut entries = Vec::new();

    walk_project_bodies(store, project, |doc_ref, body, _containing_commit| {
        collect_item_links(store, project, body, &target, doc_ref, &mut entries)
    })?;

    collect_implements_backlinks(store, project, &target, &mut entries)?;

    entries.sort_by(|a, b| {
        a.document
            .cmp(&b.document)
            .then_with(|| a.sort_key().cmp(&b.sort_key()))
    });

    Ok(entries)
}

/// Emits the **structural** backlinks to `target`: references carried in a
/// document's frontmatter rather than written as `rdm:` links in its body.
///
/// Two direct sources, plus exactly one transitive hop:
///
/// 1. Every plan whose `implements` names `target`.
/// 2. Every review whose `implements` names `target` (only a
///    `change/<sha>` review carries one, and it always names a plan).
/// 3. For a phase or task target: the change reviews implementing any plan
///    from (1) — reached through that plan and tagged with `via`, so the
///    chain is legible rather than looking direct.
///
/// **Depth is fixed at one hop and will stay that way.** Following a second
/// level (a plan superseding a plan, say) would make backlink output depend
/// on chain length and could revisit a document; one hop is enough to answer
/// the question this exists for — "what code reviews cover this phase?" —
/// and is trivially loop-free.
///
/// Two approved plans implementing the same phase, or a plan that supersedes
/// another, can both surface the same review; it is emitted once per
/// `(document, field, via)` triple and duplicates are dropped, so ordering
/// stays deterministic.
fn collect_implements_backlinks(
    store: &impl Store,
    project: &str,
    target: &ItemRef,
    out: &mut Vec<BacklinkEntry>,
) -> Result<()> {
    let push = |document: DocRef, via: Option<ItemRef>, out: &mut Vec<BacklinkEntry>| {
        let entry = BacklinkEntry {
            document,
            reference: BacklinkRef::Field {
                field: "implements",
                via,
            },
        };
        if !out.contains(&entry) {
            out.push(entry);
        }
    };

    // (1) plans implementing the target, and (3) their change reviews.
    let mut plan_refs: Vec<ItemRef> = Vec::new();
    for (slug, doc) in crate::ops::plan::list_plans(store, project)? {
        if normalize_item_ref(store, project, &doc.frontmatter.implements)? != *target {
            continue;
        }
        push(DocRef::Plan { slug: slug.clone() }, None, out);
        plan_refs.push(ItemRef::Plan { slug });
    }

    // (2) reviews whose `implements` names the target directly.
    let reviews = crate::ops::reviews::list_reviews(store, project)?;
    for (id, doc) in &reviews {
        let Some(implements) = &doc.frontmatter.implements else {
            continue;
        };
        if normalize_item_ref(store, project, implements)? == *target {
            push(
                DocRef::Review {
                    id: id.clone(),
                    comment: None,
                },
                None,
                out,
            );
        }
    }

    // (3) one hop: target → plan → change review.
    if !plan_refs.is_empty() {
        for (id, doc) in &reviews {
            let Some(implements) = &doc.frontmatter.implements else {
                continue;
            };
            if let Some(plan_ref) = plan_refs.iter().find(|p| *p == implements) {
                push(
                    DocRef::Review {
                        id: id.clone(),
                        comment: None,
                    },
                    Some(plan_ref.clone()),
                    out,
                );
            }
        }
    }
    Ok(())
}

/// Walks every roadmap, phase, task, plan, and review body (and review comment) in
/// `project`, invoking `f` once per document with its [`DocRef`] identity,
/// its markdown body, and (for a phase or task) its stamped `commit` — the
/// `containing_commit` [`resolve_link`] needs to resolve a code link's
/// revision precedence.
///
/// This is the shared document-walking loop behind both [`backlinks`] (which
/// filters for links matching one target) and [`check_project`] (which
/// resolves every link it finds) — factored out so the two scans can never
/// silently drift in which documents they visit.
///
/// # Errors
///
/// Returns [`crate::error::Error::ProjectNotFound`] if the project doesn't
/// exist, [`crate::error::Error::RoadmapNotFound`] if a roadmap disappears
/// between listing and reading its phases, [`crate::error::Error::Io`] if a
/// directory cannot be listed or a file cannot be read,
/// [`crate::error::Error::FrontmatterMissing`]/
/// [`crate::error::Error::FrontmatterParse`] if any scanned document has
/// invalid frontmatter, or whatever error `f` itself returns.
fn walk_project_bodies<F>(store: &impl Store, project: &str, mut f: F) -> Result<()>
where
    F: FnMut(DocRef, &str, Option<&str>) -> Result<()>,
{
    for roadmap_doc in crate::ops::roadmap::list_roadmaps(store, project, None, None)? {
        let roadmap = roadmap_doc.frontmatter.roadmap.clone();
        f(
            DocRef::Roadmap {
                roadmap: roadmap.clone(),
            },
            &roadmap_doc.body,
            None,
        )?;

        for (stem, phase_doc) in crate::ops::phase::list_phases(store, project, &roadmap)? {
            let commit = phase_doc.frontmatter.commit.clone();
            f(
                DocRef::Phase {
                    roadmap: roadmap.clone(),
                    stem,
                },
                &phase_doc.body,
                commit.as_deref(),
            )?;
        }
    }

    for (slug, task_doc) in crate::ops::task::list_tasks(store, project)? {
        let commit = task_doc.frontmatter.commit.clone();
        f(DocRef::Task { slug }, &task_doc.body, commit.as_deref())?;
    }

    // Plans are visited after tasks and before reviews so `DocRef`'s derived
    // `Ord` (roadmap < phase < task < plan < review) matches visit order.
    for (slug, plan_doc) in crate::ops::plan::list_plans(store, project)? {
        f(DocRef::Plan { slug }, &plan_doc.body, None)?;
    }

    for (id, review_doc) in crate::ops::reviews::list_reviews(store, project)? {
        f(
            DocRef::Review {
                id: id.clone(),
                comment: None,
            },
            &review_doc.body,
            None,
        )?;
        for comment in &review_doc.frontmatter.comments {
            f(
                DocRef::Review {
                    id: id.clone(),
                    comment: Some(comment.id),
                },
                &comment.body,
                None,
            )?;
        }
    }

    Ok(())
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
                    reference: BacklinkRef::Body { byte_range: range },
                });
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// `link check` — project- or document-wide validation
// ---------------------------------------------------------------------------

/// One `rdm:` item link found to be dangling — its target doesn't exist.
///
/// Never constructed for a code link: per [`resolve_item_link`]'s contract a
/// dangling *item* reference is a normal, expected outcome (the author
/// renamed or deleted the thing it pointed at), which is exactly what
/// `link check` exists to surface; a code link's path/rev validity is a
/// separate concern (see [`MissingAtRevFinding`]), since core has no
/// source-repo checkout to validate against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DanglingLink {
    /// The document the link was found in.
    pub document: DocRef,
    /// Byte range of the link within that document's body.
    pub byte_range: Range<usize>,
    /// The raw `rdm:` URI text, as written in the document.
    pub uri: String,
    /// The missing target.
    pub target: ItemRef,
}

/// One `rdm:src/` code link found while checking a project or document,
/// carried forward so a caller with a source-repo checkout (`rdm-cli`'s
/// `link check`, behind the `git` feature) can additionally verify the path
/// exists at the pinned revision — a check core itself cannot perform, since
/// it has no checkout to look in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeLinkFinding {
    /// The document the link was found in.
    pub document: DocRef,
    /// Byte range of the link within that document's body.
    pub byte_range: Range<usize>,
    /// Path to the file, relative to the source repository root.
    pub path: String,
    /// The resolved revision (see [`resolve_code_link`]'s precedence), or
    /// `None` when neither an explicit `@rev` nor a stamped commit was
    /// available.
    pub rev: Option<String>,
}

/// A malformed `rdm:` link destination found while checking a project or
/// document, tagged with the document it was found in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckDiagnostic {
    /// The document the malformed link was found in.
    pub document: DocRef,
    /// The parse diagnostic itself.
    pub diagnostic: crate::link::LinkDiagnostic,
}

/// A code link whose path does not exist at its pinned revision, found by a
/// caller's checkout-aware path-verification pass (never constructed by
/// [`check_project`]/[`check_document`] themselves — see [`LinkCheckReport`]'s
/// doc comment).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingAtRevFinding {
    /// The document the link was found in.
    pub document: DocRef,
    /// Byte range of the link within that document's body.
    pub byte_range: Range<usize>,
    /// Path to the file, relative to the source repository root.
    pub path: String,
    /// The revision the path was checked at.
    pub rev: String,
}

/// The result of checking a project (or one document) for broken `rdm:`
/// links — the aggregation [`check_project`]/[`check_document`] build by
/// resolving every link [`crate::link::extract_links`] finds.
///
/// `missing_at_rev` and `path_verification_skipped` are never populated by
/// [`check_project`]/[`check_document`] themselves: core has no source-repo
/// checkout to verify a code link's path against (see
/// [`crate::ops::links::resolve_code_link`]'s doc comment on why that
/// validation is deliberately out of scope here). A caller that does have a
/// checkout — `rdm-cli`'s `link check`, behind the `git` feature — fills
/// these two fields in after the fact, using [`code_links`](Self::code_links)
/// as its worklist, before handing the report to a formatter. This keeps one
/// report shape and one pair of formatters
/// ([`crate::display::format_link_check_report`] /
/// [`crate::json::link_check_report_to_json`]) serving both the
/// checkout-aware and checkout-less cases.
#[derive(Debug, Clone, Default)]
pub struct LinkCheckReport {
    /// Count of successfully-parsed `rdm:` links resolved (item or code) —
    /// distinct from `diagnostics`, which counts links that failed to parse
    /// in the first place.
    pub links_checked: usize,
    /// Item links whose target does not exist.
    pub dangling: Vec<DanglingLink>,
    /// Every code link found, for a caller-side path-verification pass.
    pub code_links: Vec<CodeLinkFinding>,
    /// Malformed `rdm:` destinations found while parsing.
    pub diagnostics: Vec<CheckDiagnostic>,
    /// Code links found missing at their pinned revision by a caller-side
    /// checkout-aware verification pass. Always empty from
    /// [`check_project`]/[`check_document`] themselves.
    pub missing_at_rev: Vec<MissingAtRevFinding>,
    /// Set by a caller when checkout-aware path verification did not run
    /// (not inside a checkout, or built without git support) so the report
    /// can say so explicitly rather than silently omitting `missing_at_rev`.
    /// Always `None` from [`check_project`]/[`check_document`] themselves.
    pub path_verification_skipped: Option<String>,
}

/// Resolves every `rdm:` link in `body` and folds the outcome into `report`.
///
/// # Errors
///
/// Propagates whatever [`resolve_link`] returns for a genuine store/project
/// failure (a dangling item target is never one of these — see
/// [`resolve_item_link`]'s contract).
fn check_body(
    store: &impl Store,
    project: &str,
    doc_ref: &DocRef,
    body: &str,
    containing_commit: Option<&str>,
    report: &mut LinkCheckReport,
) -> Result<()> {
    let (links, diagnostics) = crate::link::extract_links(body);

    for diagnostic in diagnostics {
        report.diagnostics.push(CheckDiagnostic {
            document: doc_ref.clone(),
            diagnostic,
        });
    }

    for (range, link) in links {
        let resolved = resolve_link(store, project, containing_commit, &link)?;
        report.links_checked += 1;
        match (&link, resolved) {
            (Link::Item(target), Resolved::Item { exists: false, .. }) => {
                report.dangling.push(DanglingLink {
                    document: doc_ref.clone(),
                    byte_range: range,
                    uri: link.to_string(),
                    target: target.clone(),
                });
            }
            (Link::Code { .. }, Resolved::Code { path, rev, .. }) => {
                report.code_links.push(CodeLinkFinding {
                    document: doc_ref.clone(),
                    byte_range: range,
                    path,
                    rev,
                });
            }
            // A resolved item link that exists needs no further action; a
            // `Resolved::Broken` isn't constructed by anything today (see
            // `Resolved::Broken`'s doc comment).
            _ => {}
        }
    }

    Ok(())
}

/// Loads the body (and, for a phase or task, the stamped `commit` used to
/// resolve a code link's revision) of the document `target` refers to.
///
/// Shared by [`check_document`] and `rdm-cli`'s `link list` command, which
/// both need "the one document an [`ItemRef`] names" rather than a whole
/// project scan.
///
/// # Errors
///
/// Returns [`crate::error::Error::RoadmapNotFound`],
/// [`crate::error::Error::PhaseNotFound`],
/// [`crate::error::Error::TaskNotFound`], or
/// [`crate::error::Error::PlanNotFound`] if the target doesn't exist,
/// [`crate::error::Error::Io`] on a read failure, or
/// [`crate::error::Error::FrontmatterMissing`]/
/// [`crate::error::Error::FrontmatterParse`] if its frontmatter is invalid —
/// including while resolving a numeric phase stem via
/// [`crate::ops::phase::resolve_phase_stem`].
///
/// Returns [`crate::error::Error::ChangeTargetHasNoDocument`] when `target`
/// is an [`ItemRef::Change`]: a change names source-repository commits
/// rather than a plan-repo document, so there is no body to load.
pub fn load_document_body(
    store: &impl Store,
    project: &str,
    target: &ItemRef,
) -> Result<(DocRef, String, Option<String>)> {
    match target {
        ItemRef::Roadmap { roadmap } => {
            let doc = crate::io::load_roadmap(store, project, roadmap)?;
            Ok((
                DocRef::Roadmap {
                    roadmap: roadmap.clone(),
                },
                doc.body,
                None,
            ))
        }
        ItemRef::Phase { roadmap, stem } => {
            let resolved_stem =
                crate::ops::phase::resolve_phase_stem(store, project, roadmap, stem)?;
            let doc = crate::io::load_phase(store, project, roadmap, &resolved_stem)?;
            let commit = doc.frontmatter.commit.clone();
            Ok((
                DocRef::Phase {
                    roadmap: roadmap.clone(),
                    stem: resolved_stem,
                },
                doc.body,
                commit,
            ))
        }
        ItemRef::Task { slug } => {
            let doc = crate::io::load_task(store, project, slug)?;
            let commit = doc.frontmatter.commit.clone();
            Ok((DocRef::Task { slug: slug.clone() }, doc.body, commit))
        }
        ItemRef::Plan { slug } => {
            let doc = crate::io::load_plan(store, project, slug)?;
            // A plan stamps no completing commit, so code links inside a plan
            // body have no containing-commit fallback revision.
            Ok((DocRef::Plan { slug: slug.clone() }, doc.body, None))
        }
        // A change has no plan-repo body to scan for outgoing links.
        ItemRef::Change { .. } => Err(crate::error::Error::ChangeTargetHasNoDocument(
            target.label(),
        )),
    }
}

/// Checks every roadmap, phase, task, and review body in `project` for
/// broken `rdm:` links: dangling item references, and (as a worklist for a
/// checkout-aware caller) every code link found — see [`LinkCheckReport`].
///
/// # Errors
///
/// Same as [`walk_project_bodies`]/[`resolve_link`]: propagates
/// [`crate::error::Error::ProjectNotFound`], `RoadmapNotFound`, `Io`,
/// `FrontmatterMissing`/`FrontmatterParse`, or (via a `Code` link)
/// `ProjectNotFound` while loading the project's `source` config.
pub fn check_project(store: &impl Store, project: &str) -> Result<LinkCheckReport> {
    let mut report = LinkCheckReport::default();
    walk_project_bodies(store, project, |doc_ref, body, containing_commit| {
        check_body(
            store,
            project,
            &doc_ref,
            body,
            containing_commit,
            &mut report,
        )
    })?;
    Ok(report)
}

/// Checks one document (a roadmap, phase, or task — see [`load_document_body`])
/// for broken `rdm:` links, scoped to just that document's own body.
///
/// A roadmap target checks only the roadmap's own body, not its phases —
/// scope it to a specific phase with `phase/<roadmap>/<stem-or-number>` to
/// check that instead.
///
/// # Errors
///
/// Same as [`load_document_body`]/[`resolve_link`]: propagates
/// `RoadmapNotFound`/`PhaseNotFound`/`TaskNotFound` if the target doesn't
/// exist, `Io`, `FrontmatterMissing`/`FrontmatterParse`, or `ProjectNotFound`
/// while loading the project's `source` config for a code link.
pub fn check_document(
    store: &impl Store,
    project: &str,
    target: &ItemRef,
) -> Result<LinkCheckReport> {
    let mut report = LinkCheckReport::default();
    let (doc_ref, body, containing_commit) = load_document_body(store, project, target)?;
    check_body(
        store,
        project,
        &doc_ref,
        &body,
        containing_commit.as_deref(),
        &mut report,
    )?;
    Ok(report)
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
    fn resolve_item_link_existing_roadmap() {
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
        let target = ItemRef::Roadmap {
            roadmap: "auth".to_string(),
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
    fn resolve_item_link_numeric_phase_stem_propagates_frontmatter_parse_error() {
        // A corrupted phase file discovered while resolving a numeric
        // stem is a genuine store failure, not a "doesn't exist" case —
        // it must propagate rather than fold into `exists: false` the way
        // RoadmapNotFound/PhaseNotFound do (see the two tests above).
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
        // Overwrite the phase file with a `phase` field of the wrong
        // type — valid YAML syntax, invalid for `Phase::phase: u32` — so
        // `list_phases` (which `resolve_phase_stem` calls to resolve a
        // numeric stem) fails with a real `FrontmatterParse`, not a
        // not-found variant.
        store
            .write(
                &crate::paths::phase_path("demo", "auth", "phase-1-design"),
                "---\nphase: not-a-number\ntitle: Design\nstatus: not-started\n---\n\nBody.\n"
                    .to_string(),
            )
            .unwrap();
        let target = ItemRef::Phase {
            roadmap: "auth".to_string(),
            stem: "1".to_string(),
        };
        let err = resolve_item_link(&store, "demo", &target).unwrap_err();
        assert!(matches!(err, crate::error::Error::FrontmatterParse(_)));
    }

    #[test]
    fn normalize_item_ref_numeric_phase_stem_propagates_frontmatter_parse_error() {
        // Mirrors the `resolve_item_link` test above for
        // `normalize_item_ref`'s identical `Err(e) => Err(e)` fallthrough.
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
        store
            .write(
                &crate::paths::phase_path("demo", "auth", "phase-1-design"),
                "---\nphase: not-a-number\ntitle: Design\nstatus: not-started\n---\n\nBody.\n"
                    .to_string(),
            )
            .unwrap();
        let item_ref = ItemRef::Phase {
            roadmap: "auth".to_string(),
            stem: "1".to_string(),
        };
        let err = normalize_item_ref(&store, "demo", &item_ref).unwrap_err();
        assert!(matches!(err, crate::error::Error::FrontmatterParse(_)));
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

    // --- item_ref_path: the shared path-lookup `resolve_item_link` and
    // `rdm-cli`'s `link resolve`/`link list` both build on ---

    #[test]
    fn item_ref_path_roadmap_and_task() {
        let store = setup();
        assert_eq!(
            item_ref_path(
                &store,
                "demo",
                &ItemRef::Roadmap {
                    roadmap: "auth".to_string()
                }
            )
            .unwrap()
            .as_str(),
            "projects/demo/roadmaps/auth/roadmap.md"
        );
        assert_eq!(
            item_ref_path(
                &store,
                "demo",
                &ItemRef::Task {
                    slug: "fix-login".to_string()
                }
            )
            .unwrap()
            .as_str(),
            "projects/demo/tasks/fix-login.md"
        );
    }

    #[test]
    fn item_ref_path_normalizes_a_numeric_phase_stem_to_the_canonical_one() {
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
        // Same path whether the caller names the phase by its canonical
        // stem or by its bare number — mirrors `resolve_item_link`'s own
        // existence check treating the two forms as equivalent.
        let by_number = item_ref_path(
            &store,
            "demo",
            &ItemRef::Phase {
                roadmap: "auth".to_string(),
                stem: "1".to_string(),
            },
        )
        .unwrap();
        let by_stem = item_ref_path(
            &store,
            "demo",
            &ItemRef::Phase {
                roadmap: "auth".to_string(),
                stem: "phase-1-design".to_string(),
            },
        )
        .unwrap();
        assert_eq!(by_number, by_stem);
        assert_eq!(
            by_stem.as_str(),
            "projects/demo/roadmaps/auth/phase-1-design.md"
        );
    }

    #[test]
    fn item_ref_path_falls_back_to_the_stem_as_written_for_a_dangling_phase_reference() {
        let store = setup();
        // Unknown roadmap: never an error, per `resolve_item_link`'s
        // never-errors-on-dangling contract, which this shares.
        let path = item_ref_path(
            &store,
            "demo",
            &ItemRef::Phase {
                roadmap: "ghost".to_string(),
                stem: "3".to_string(),
            },
        )
        .unwrap();
        assert_eq!(path.as_str(), "projects/demo/roadmaps/ghost/3.md");
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
                implements: None,
                change_branch: None,
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
        assert_ne!(entries[0].byte_range(), entries[1].byte_range());
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
                implements: None,
                change_branch: None,
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
    fn backlinks_finds_reference_in_review_summary_body() {
        // The doc comment on `backlinks` promises a scan of "every ...
        // review body (and review comment)" — this covers the review's own
        // top-level body (`DocRef::Review { comment: None, .. }`), which
        // every other review fixture in this file leaves link-free in
        // favor of putting the link in a comment.
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
                id: "2026-07-01-0900-dddd".to_string(),
                author: "ed".to_string(),
                target: ReviewTarget::Task {
                    slug: "fix-login".to_string(),
                },
                state: ReviewState::Draft,
                verdict: None,
                created: Utc.with_ymd_and_hms(2026, 7, 1, 9, 0, 0).unwrap(),
                submitted: None,
                created_commit: None,
                comments: vec![ReviewComment {
                    id: 1,
                    doc: None,
                    status: ReviewCommentStatus::Open,
                    applied_commit: None,
                    anchor: None,
                    body: "No link in this comment.".to_string(),
                    reply: None,
                }],
                implements: None,
                change_branch: None,
            },
            body: "Summary references [the fix](rdm:task/fix-login) directly.".to_string(),
        };
        crate::io::write_review(&mut store, "demo", "2026-07-01-0900-dddd", &doc).unwrap();

        let target = ItemRef::Task {
            slug: "fix-login".to_string(),
        };
        let entries = backlinks(&store, "demo", &target).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].document,
            DocRef::Review {
                id: "2026-07-01-0900-dddd".to_string(),
                comment: None,
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

    #[test]
    fn backlinks_errors_project_not_found() {
        // Mirrors `resolve_link_code_link_errors_project_not_found`: a
        // genuine store failure (here, from `list_roadmaps`'s own
        // project-existence check) must propagate, not fold into an empty
        // result the way a dangling target does.
        let store = setup();
        let target = ItemRef::Task {
            slug: "fix-login".to_string(),
        };
        let err = backlinks(&store, "no-such-project", &target).unwrap_err();
        assert!(
            matches!(err, crate::error::Error::ProjectNotFound(name) if name == "no-such-project")
        );
    }

    #[test]
    fn backlinks_rejects_a_change_target() {
        // A change/<sha> target names source-repository commits, not a
        // plan-repo document — `crate::link::parse` already refuses to
        // build a linkable `Link::Item` for one (`NotLinkable`), and the
        // sibling `link check`/`link list` commands reject it the same
        // actionable way via `load_document_body`. Before this test,
        // `backlinks` silently accepted a change target and always
        // returned an empty result (no body link, `implements` field, or
        // review can ever name a change), which is a stale-enumeration
        // mismatch of the same class this phase exists to close — see
        // `ChangeTargetHasNoDocument`.
        let store = setup();
        let target = ItemRef::Change {
            head: "a".repeat(40),
            base: None,
        };
        let err = backlinks(&store, "demo", &target).unwrap_err();
        assert!(
            matches!(err, crate::error::Error::ChangeTargetHasNoDocument(_)),
            "expected ChangeTargetHasNoDocument, got {err:?}"
        );
        assert!(
            err.to_string()
                .contains("names commits in the source repository")
        );
    }

    // --- structural `implements` backlinks and the one-hop rule ---

    /// Seeds `demo` with a task, a plan implementing it, a change review
    /// implementing that plan, and a second plan that supersedes the first —
    /// the shape the one-hop and dedup rules are about.
    fn seed_implements_chain(store: &mut MemoryStore) {
        crate::ops::task::create_task(
            store,
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
        crate::ops::plan::create_plan(
            store,
            crate::ops::plan::CreatePlan {
                project: "demo",
                slug: "plan-one",
                title: "Plan one",
                implements: ItemRef::Task {
                    slug: "fix-login".to_string(),
                },
                supersedes: None,
                body: Some("Plan body."),
            },
        )
        .unwrap();
    }

    fn write_change_review(store: &mut MemoryStore, id: &str, implements: Option<ItemRef>) {
        let doc = Document {
            frontmatter: Review {
                id: id.to_string(),
                author: "tester".to_string(),
                target: ReviewTarget::Change {
                    head: "a".repeat(40),
                    base: Some("b".repeat(40)),
                },
                state: ReviewState::Draft,
                verdict: None,
                created: chrono::Utc::now(),
                submitted: None,
                created_commit: None,
                implements,
                change_branch: None,
                comments: Vec::new(),
            },
            body: "Summary.".to_string(),
        };
        crate::io::write_review(store, "demo", id, &doc).unwrap();
    }

    #[test]
    fn backlinks_emit_a_plans_implements_field_as_a_structural_reference() {
        let mut store = setup();
        seed_implements_chain(&mut store);
        let entries = backlinks(
            &store,
            "demo",
            &ItemRef::Task {
                slug: "fix-login".to_string(),
            },
        )
        .unwrap();
        assert_eq!(
            entries,
            vec![BacklinkEntry {
                document: DocRef::Plan {
                    slug: "plan-one".to_string()
                },
                reference: BacklinkRef::Field {
                    field: "implements",
                    via: None
                },
            }]
        );
        // A frontmatter reference has no byte range, by construction.
        assert_eq!(entries[0].byte_range(), None);
    }

    #[test]
    fn backlinks_follow_exactly_one_hop_from_an_item_to_its_change_reviews() {
        let mut store = setup();
        seed_implements_chain(&mut store);
        write_change_review(
            &mut store,
            "2026-07-01-1200-aaaa",
            Some(ItemRef::Plan {
                slug: "plan-one".to_string(),
            }),
        );
        // A second-level chain: a review implementing a plan that
        // implements... a plan. Never followed.
        crate::ops::plan::create_plan(
            &mut store,
            crate::ops::plan::CreatePlan {
                project: "demo",
                slug: "plan-two",
                title: "Plan two",
                implements: ItemRef::Task {
                    slug: "fix-login".to_string(),
                },
                supersedes: Some(ItemRef::Plan {
                    slug: "plan-one".to_string(),
                }),
                body: Some("Plan body."),
            },
        )
        .unwrap();

        let entries = backlinks(
            &store,
            "demo",
            &ItemRef::Task {
                slug: "fix-login".to_string(),
            },
        )
        .unwrap();
        let review_entries: Vec<_> = entries
            .iter()
            .filter(|e| matches!(e.document, DocRef::Review { .. }))
            .collect();
        assert_eq!(
            review_entries.len(),
            1,
            "expected exactly one review entry, got {review_entries:?}"
        );
        assert_eq!(
            review_entries[0].reference,
            BacklinkRef::Field {
                field: "implements",
                via: Some(ItemRef::Plan {
                    slug: "plan-one".to_string()
                }),
            }
        );
    }

    #[test]
    fn backlinks_to_a_plan_name_its_change_reviews_directly() {
        let mut store = setup();
        seed_implements_chain(&mut store);
        write_change_review(
            &mut store,
            "2026-07-01-1200-aaaa",
            Some(ItemRef::Plan {
                slug: "plan-one".to_string(),
            }),
        );
        let entries = backlinks(
            &store,
            "demo",
            &ItemRef::Plan {
                slug: "plan-one".to_string(),
            },
        )
        .unwrap();
        // Direct: `via` is None, because the review names the plan itself.
        assert_eq!(
            entries,
            vec![BacklinkEntry {
                document: DocRef::Review {
                    id: "2026-07-01-1200-aaaa".to_string(),
                    comment: None,
                },
                reference: BacklinkRef::Field {
                    field: "implements",
                    via: None
                },
            }]
        );
    }

    #[test]
    fn backlinks_never_emit_the_same_review_twice_through_two_plans() {
        let mut store = setup();
        seed_implements_chain(&mut store);
        // A second plan implementing the SAME task; the review implements
        // only the first, so it must still appear exactly once.
        crate::ops::plan::create_plan(
            &mut store,
            crate::ops::plan::CreatePlan {
                project: "demo",
                slug: "plan-two",
                title: "Plan two",
                implements: ItemRef::Task {
                    slug: "fix-login".to_string(),
                },
                supersedes: None,
                body: Some("Plan body."),
            },
        )
        .unwrap();
        write_change_review(
            &mut store,
            "2026-07-01-1200-aaaa",
            Some(ItemRef::Plan {
                slug: "plan-one".to_string(),
            }),
        );
        let entries = backlinks(
            &store,
            "demo",
            &ItemRef::Task {
                slug: "fix-login".to_string(),
            },
        )
        .unwrap();
        let reviews = entries
            .iter()
            .filter(|e| matches!(e.document, DocRef::Review { .. }))
            .count();
        assert_eq!(
            reviews, 1,
            "expected no duplicate review entries: {entries:?}"
        );
        // Deterministic across repeated calls.
        let again = backlinks(
            &store,
            "demo",
            &ItemRef::Task {
                slug: "fix-login".to_string(),
            },
        )
        .unwrap();
        assert_eq!(entries, again);
    }

    #[test]
    fn a_change_review_without_implements_contributes_no_backlink() {
        let mut store = setup();
        seed_implements_chain(&mut store);
        write_change_review(&mut store, "2026-07-01-1200-aaaa", None);
        let entries = backlinks(
            &store,
            "demo",
            &ItemRef::Plan {
                slug: "plan-one".to_string(),
            },
        )
        .unwrap();
        assert!(entries.is_empty(), "{entries:?}");
    }

    #[test]
    fn a_change_target_has_no_plan_repo_document() {
        let store = setup();
        let target = ItemRef::Change {
            head: "a".repeat(40),
            base: None,
        };
        assert!(matches!(
            resolve_item_link(&store, "demo", &target).unwrap(),
            Resolved::Broken { .. }
        ));
        assert!(matches!(
            item_ref_path(&store, "demo", &target).unwrap_err(),
            crate::error::Error::ChangeTargetHasNoDocument(_)
        ));
        assert!(matches!(
            load_document_body(&store, "demo", &target).unwrap_err(),
            crate::error::Error::ChangeTargetHasNoDocument(_)
        ));
    }

    // --- `link check` / `check_project` / `check_document` ---

    #[test]
    fn check_document_clean_task_reports_zero_broken() {
        let mut store = setup();
        crate::ops::task::create_task(
            &mut store,
            CreateTask {
                project: "demo",
                slug: "clean",
                title: "Clean",
                priority: Priority::Medium,
                tags: None,
                body: Some("No links here."),
            },
        )
        .unwrap();
        let report = check_document(
            &store,
            "demo",
            &ItemRef::Task {
                slug: "clean".to_string(),
            },
        )
        .unwrap();
        assert!(report.dangling.is_empty());
        assert!(report.diagnostics.is_empty());
        assert!(report.code_links.is_empty());
        assert_eq!(report.links_checked, 0);
    }

    #[test]
    fn check_document_finds_dangling_link_with_uri_and_target() {
        let mut store = setup();
        crate::ops::task::create_task(
            &mut store,
            CreateTask {
                project: "demo",
                slug: "has-dangling",
                title: "Has dangling",
                priority: Priority::Medium,
                tags: None,
                body: Some("See [ghost](rdm:task/does-not-exist) for context."),
            },
        )
        .unwrap();
        let target = ItemRef::Task {
            slug: "has-dangling".to_string(),
        };
        let report = check_document(&store, "demo", &target).unwrap();
        assert_eq!(report.dangling.len(), 1);
        let dangling = &report.dangling[0];
        assert_eq!(
            dangling.document,
            DocRef::Task {
                slug: "has-dangling".to_string()
            }
        );
        assert_eq!(dangling.uri, "rdm:task/does-not-exist");
        assert_eq!(
            dangling.target,
            ItemRef::Task {
                slug: "does-not-exist".to_string()
            }
        );
        assert_eq!(report.links_checked, 1);
    }

    #[test]
    fn check_document_existing_target_is_not_dangling() {
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
                slug: "referrer",
                title: "Referrer",
                priority: Priority::Medium,
                tags: None,
                body: Some("See [fix](rdm:task/fix-login)."),
            },
        )
        .unwrap();
        let report = check_document(
            &store,
            "demo",
            &ItemRef::Task {
                slug: "referrer".to_string(),
            },
        )
        .unwrap();
        assert!(report.dangling.is_empty());
        assert_eq!(report.links_checked, 1);
    }

    #[test]
    fn check_document_roadmap_scope_does_not_fan_out_into_phases() {
        // A dangling link in a phase body must not surface when the check
        // is scoped to the roadmap itself — `--on roadmap/<slug>` checks
        // only the roadmap's own body.
        let mut store = setup();
        crate::ops::roadmap::create_roadmap(
            &mut store,
            crate::ops::CreateRoadmap {
                project: "demo",
                slug: "auth",
                title: "Auth",
                body: Some("No links in the roadmap body."),
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
                body: Some("See [ghost](rdm:task/does-not-exist)."),
                tags: None,
                difficulty: crate::ops::DifficultyUpdate::Keep,
                model: crate::ops::ModelTierUpdate::Keep,
            },
        )
        .unwrap();
        let report = check_document(
            &store,
            "demo",
            &ItemRef::Roadmap {
                roadmap: "auth".to_string(),
            },
        )
        .unwrap();
        assert!(report.dangling.is_empty());
        assert_eq!(report.links_checked, 0);
    }

    #[test]
    fn check_document_numeric_phase_stem_resolves_and_reports_canonical_doc_ref() {
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
                body: Some("See [ghost](rdm:task/does-not-exist)."),
                tags: None,
                difficulty: crate::ops::DifficultyUpdate::Keep,
                model: crate::ops::ModelTierUpdate::Keep,
            },
        )
        .unwrap();
        let report = check_document(
            &store,
            "demo",
            &ItemRef::Phase {
                roadmap: "auth".to_string(),
                stem: "1".to_string(),
            },
        )
        .unwrap();
        assert_eq!(report.dangling.len(), 1);
        assert_eq!(
            report.dangling[0].document,
            DocRef::Phase {
                roadmap: "auth".to_string(),
                stem: "phase-1-design".to_string(),
            }
        );
    }

    #[test]
    fn check_document_task_not_found_errors() {
        let store = setup();
        let err = check_document(
            &store,
            "demo",
            &ItemRef::Task {
                slug: "never-existed".to_string(),
            },
        )
        .unwrap_err();
        assert!(matches!(err, crate::error::Error::TaskNotFound(slug) if slug == "never-existed"));
    }

    #[test]
    fn check_project_aggregates_dangling_links_and_code_links_across_documents() {
        let mut store = setup();
        crate::ops::task::create_task(
            &mut store,
            CreateTask {
                project: "demo",
                slug: "a",
                title: "A",
                priority: Priority::Medium,
                tags: None,
                body: Some("See [ghost](rdm:task/missing) and [src](rdm:src/a/b.rs@abc#L5)."),
            },
        )
        .unwrap();
        crate::ops::task::create_task(
            &mut store,
            CreateTask {
                project: "demo",
                slug: "b",
                title: "B",
                priority: Priority::Medium,
                tags: None,
                body: Some("Also [ghost2](rdm:task/missing2)."),
            },
        )
        .unwrap();

        let report = check_project(&store, "demo").unwrap();
        assert_eq!(report.dangling.len(), 2);
        assert_eq!(report.code_links.len(), 1);
        assert_eq!(report.code_links[0].path, "a/b.rs");
        assert_eq!(report.code_links[0].rev, Some("abc".to_string()));
        assert!(report.diagnostics.is_empty());
        assert_eq!(report.links_checked, 3);
    }

    #[test]
    fn check_project_collects_parse_diagnostics() {
        let mut store = setup();
        crate::ops::task::create_task(
            &mut store,
            CreateTask {
                project: "demo",
                slug: "bad-link",
                title: "Bad link",
                priority: Priority::Medium,
                tags: None,
                body: Some("A [bad](rdm:foo/bar) link."),
            },
        )
        .unwrap();
        let report = check_project(&store, "demo").unwrap();
        assert_eq!(report.diagnostics.len(), 1);
        assert_eq!(report.diagnostics[0].diagnostic.uri, "rdm:foo/bar");
        assert!(report.dangling.is_empty());
    }

    #[test]
    fn check_project_errors_project_not_found() {
        let store = setup();
        let err = check_project(&store, "no-such-project").unwrap_err();
        assert!(
            matches!(err, crate::error::Error::ProjectNotFound(name) if name == "no-such-project")
        );
    }

    #[test]
    fn load_document_body_resolves_numeric_phase_stem() {
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
                body: Some("Phase body."),
                tags: None,
                difficulty: crate::ops::DifficultyUpdate::Keep,
                model: crate::ops::ModelTierUpdate::Keep,
            },
        )
        .unwrap();
        let (doc_ref, body, _commit) = load_document_body(
            &store,
            "demo",
            &ItemRef::Phase {
                roadmap: "auth".to_string(),
                stem: "1".to_string(),
            },
        )
        .unwrap();
        assert_eq!(
            doc_ref,
            DocRef::Phase {
                roadmap: "auth".to_string(),
                stem: "phase-1-design".to_string(),
            }
        );
        assert_eq!(body.trim(), "Phase body.");
    }
}

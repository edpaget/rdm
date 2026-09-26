//! Source-repository discovery for `change/<sha>` review anchor resolution.
//!
//! A change review's comment anchors live in the project's *source*
//! repository, so resolving them needs a readable checkout of it. The CLI
//! finds one by asking the invoking process where it is standing
//! (`rdm-cli/src/source_repo.rs`). A server has no meaningful cwd — the
//! process was started by a service manager, in a directory nobody chose for
//! this reason — so this module deliberately offers only ONE discovery route:
//! the project's configured `source.repo`, when it names a local directory.
//!
//! That is expressed by handing [`rdm_core::source_select::select_source`] a
//! [`SourceEnvironment`](rdm_core::source_select::SourceEnvironment) with
//! `in_checkout: false` and `checkout_is_plan_repo: false` *unconditionally*.
//! Those two constants are the point, not an omission: they make the server
//! structurally incapable of reading its own working directory — or the plan
//! repo — as a source repository, no matter what directory it happens to be
//! running in. Only the `ReadConfiguredLocal` cell can be reached.
//!
//! Every other outcome degrades to an explicit note, never to silence. The
//! read path's contract (`docs/change-reviews.md` § "The degrade rule") is that
//! a surface which cannot verify anchors says so — which is why the build
//! *without* the `git` feature still returns a note rather than nothing.

use rdm_core::model::Review;
use rdm_core::ops::reviews::ReviewResolution;

/// The note a surface reports when this server cannot resolve change-review
/// anchors at all.
///
/// Every cause is suffixed onto it, so the reason a reader sees always begins
/// by naming the surface and then narrows to the specific cause.
const NO_RESOLUTION_NOTE: &str = "this server does not resolve change-review anchors";

/// The whole per-review read for `review` — resolutions, per-comment notes,
/// the source note, and every comment's target — from
/// [`rdm_core::ops::reviews::resolve_review`], against the project's
/// configured local source repository, or degraded with a note saying why
/// there is none.
///
/// Core owns the read: the dispatch on the review's target kind, the
/// resolution ladder, and the fail-open targets with no source. This function
/// contributes only the discovery half and the no-source note, so the REST and
/// HTML surfaces can never disagree with `rdm review show --format json`.
///
/// # Panics
///
/// Never panics.
#[must_use]
pub fn resolve_review_for_project(
    store: &impl rdm_core::store::VersionedStore,
    project: &str,
    review: &Review,
) -> ReviewResolution {
    use rdm_core::ops::reviews::resolve_review;
    use rdm_core::source::MemorySourceRepo;

    // `MemorySourceRepo` only names the type of `None`; it is never built.
    let no_source = None::<&MemorySourceRepo>;
    if !matches!(review.target, rdm_core::model::ReviewTarget::Change { .. }) {
        return resolve_review(store, project, review, no_source);
    }
    #[cfg(feature = "git")]
    {
        match source_for(store, project) {
            Ok(source) => resolve_review(store, project, review, Some(&source)),
            Err(note) => ReviewResolution {
                source_note: Some(note),
                ..resolve_review(store, project, review, no_source)
            },
        }
    }
    // No adapter exists in this build, so there is never a source — but the
    // surface still degrades WITH a note, exactly like the `git` build.
    #[cfg(not(feature = "git"))]
    {
        ReviewResolution {
            source_note: Some(format!(
                "{NO_RESOLUTION_NOTE} — this build has no git support"
            )),
            ..resolve_review(store, project, review, no_source)
        }
    }
}

/// What [`code_annotations_for_project`] found for a code location.
#[derive(Debug)]
pub enum AnnotationsOutcome {
    /// The project's source repository was read: every annotation
    /// [`rdm_core::ops::links::code_annotations`] returned, in its order.
    Found(Vec<rdm_core::link::CodeAnnotation>),
    /// No readable source repository: the note saying why, prefixed like
    /// every other note in this module.
    SourceUnavailable(String),
}

/// Lists what annotates `target` (a commit or a source path) through
/// [`rdm_core::ops::links::code_annotations`], reading the project's
/// configured local source repository — the only place this server obtains
/// a source for it.
///
/// No source repository is not an error: it is
/// [`AnnotationsOutcome::SourceUnavailable`] with the note a surface reports
/// verbatim, and the server never falls back to its own working directory
/// or the plan repo (see the module documentation).
///
/// # Errors
///
/// Loading the project fails (an unknown project, or a `project.md` that
/// does not parse), or any error `code_annotations` returns once a source was
/// found (an item target, an unknown revision, a missing path or one that is
/// not a file, or a failing repository).
pub fn code_annotations_for_project(
    store: &impl rdm_core::store::VersionedStore,
    project: &str,
    target: &rdm_core::link::BacklinkTarget,
) -> rdm_core::error::Result<AnnotationsOutcome> {
    // Only an EXISTING project with no readable source fails open: an unknown
    // project, or one whose project.md does not parse, is that error — never
    // misreported as "configures no local source repo".
    rdm_core::io::load_project(store, project)?;
    // A plan item is not a code location: core refuses it before it asks the
    // source anything, so its refusal reaches the caller whether or not a
    // source repository is available — never masked as "could not look". The
    // empty repository handed over here is therefore never consulted.
    if matches!(target, rdm_core::link::BacklinkTarget::Item(_)) {
        return rdm_core::ops::links::code_annotations(
            store,
            &rdm_core::source::MemorySourceRepo::new(),
            project,
            target,
        )
        .map(AnnotationsOutcome::Found);
    }
    #[cfg(feature = "git")]
    {
        match source_for(store, project) {
            Ok(source) => rdm_core::ops::links::code_annotations(store, &source, project, target)
                .map(AnnotationsOutcome::Found),
            Err(note) => Ok(AnnotationsOutcome::SourceUnavailable(note)),
        }
    }
    #[cfg(not(feature = "git"))]
    {
        let _ = (store, project, target);
        Ok(AnnotationsOutcome::SourceUnavailable(format!(
            "{NO_RESOLUTION_NOTE} — this build has no git support"
        )))
    }
}

/// A readable source repository for `project`, or the note saying why there
/// is none.
///
/// # Errors
///
/// The `Err` arm is not a failure to propagate: it is the
/// `source_verification_skipped` note the caller reports verbatim. Nothing
/// here can fail a request.
#[cfg(feature = "git")]
pub fn source_for(
    store: &impl rdm_core::store::VersionedStore,
    project: &str,
) -> Result<rdm_git::GitSourceRepo, String> {
    use rdm_core::source_select::{
        ConfiguredSource, SourceEnvironment, SourceFallback, SourceSelection, select_source,
    };

    let source = rdm_core::io::load_project(store, project)
        .ok()
        .and_then(|doc| doc.frontmatter.source);
    let env = SourceEnvironment {
        // A server has no meaningful cwd — see the module documentation. Both
        // constants are load-bearing: they leave `ReadConfiguredLocal` as the
        // only reachable read.
        in_checkout: false,
        checkout_matches_configured: false,
        checkout_is_plan_repo: false,
        configured: source.as_ref().map(|src| ConfiguredSource {
            locator: src.repo.as_str(),
            is_local_dir: std::path::Path::new(&src.repo).is_dir(),
        }),
    };
    match select_source(&env, SourceFallback::ConfiguredLocal) {
        SourceSelection::ReadConfiguredLocal => {
            let src = source.as_ref().expect("a configured local source");
            Ok(rdm_git::GitSourceRepo::new(&src.repo))
        }
        // Unreachable by construction (`in_checkout: false`). Stated as a note
        // rather than a panic, so a future classifier change degrades instead
        // of taking the process down.
        SourceSelection::ReadCwdCheckout => Err(format!(
            "{NO_RESOLUTION_NOTE} — a server never reads its own working directory as a source repository"
        )),
        SourceSelection::Unavailable(cause) => Err(format!(
            "{NO_RESOLUTION_NOTE} — project '{project}' configures no local source repo ({})",
            describe(&cause)
        )),
    }
}

/// A short phrase naming why no configured local source was available.
///
/// Only the causes reachable with `in_checkout: false` can occur; the
/// checkout-shaped ones are mapped for exhaustiveness rather than reached.
#[cfg(feature = "git")]
fn describe(cause: &rdm_core::source_select::SourceUnavailable) -> &'static str {
    use rdm_core::source_select::SourceUnavailable as U;
    match cause {
        U::ConfiguredSourceNotLocal { .. } => "its `source.repo` is not a local directory",
        U::NoCheckoutAndNoSource | U::NoSourceConfigured => "its project.md sets no `source.repo`",
        U::NotInCheckout | U::CheckoutIsNotConfiguredSource { .. } | U::CheckoutIsPlanRepo => {
            "no local source repo resolved"
        }
    }
}

/// Resolves and validates an `applied_commit` value for `PATCH
/// /projects/:project/reviews/:review_id/comments/:comment_id`, before it is
/// threaded into [`rdm_core::ops::reviews::UpdateComment`].
///
/// Mirrors `rdm-cli`'s `resolve_applied_commit`
/// (`rdm-cli/src/commands/mod.rs`): `sha` may be an abbreviated SHA, `HEAD`,
/// a branch, or a tag, and the **resolved full SHA** is returned, not the
/// caller's literal input. The repository checked depends on the review's
/// target kind — [`rdm_core::model::ReviewTarget::Change`] resolves against
/// the project's configured local source repo (via [`source_for`]); every
/// other target kind (`roadmap`, `phase`, `task`, `plan`) resolves against
/// the plan repo rooted at `plan_root`.
///
/// This is a **hard refusal**, unlike this module's read path
/// ([`resolve_review_for_project`]), which degrades with a note
/// instead of failing. `applied_commit` mints a public `rdm:src/…@<sha>`
/// provenance record, so a SHA that cannot be verified against the correct
/// repository must block the write rather than be accepted unverified or
/// silently degraded — the same fail-closed contract `rdm-cli`'s
/// `--applied-commit` enforces.
///
/// Two distinct refusal causes are told apart, rather than both collapsing
/// into "does not resolve": [`rdm_git::discover_git_dir`] checks the checked
/// path is actually a git checkout *before* `rev_parse` ever runs, since
/// [`SourceRepo::rev_parse`](rdm_core::source::SourceRepo::rev_parse) maps
/// every non-zero git exit — including "not a git repository" and an
/// ambiguous abbreviated SHA — to the same `Ok(None)`, and reporting that as
/// "no such commit" would send a caller with a correct SHA to go recheck it
/// instead of fixing the repository. A SHA that genuinely fails to resolve
/// in a real git checkout is still reported as "does not resolve", now with
/// a hint that an abbreviated SHA may be ambiguous.
///
/// # Errors
///
/// Returns the detail string for a `400 Bad Request`: the checked path is
/// not a git checkout; `sha` does not resolve to a commit in the checked
/// repository (with a hint that an abbreviated SHA may be ambiguous); or,
/// for a `change/<sha>` review, every cause [`source_for`] itself reports
/// when no source repo could be discovered at all.
#[cfg(feature = "git")]
pub fn resolve_applied_commit(
    store: &impl rdm_core::store::VersionedStore,
    project: &str,
    plan_root: &std::path::Path,
    target: &rdm_core::model::ReviewTarget,
    sha: &str,
) -> Result<String, String> {
    use rdm_core::source::SourceRepo;

    let (repo_path, what) = match target {
        rdm_core::model::ReviewTarget::Change { .. } => {
            let repo = source_for(store, project)?;
            (
                repo.root().to_path_buf(),
                "the project's configured source repo",
            )
        }
        _ => (plan_root.to_path_buf(), "the plan repo"),
    };
    if rdm_git::discover_git_dir(&repo_path).is_err() {
        return Err(format!(
            "applied_commit cannot be checked: {what} at '{}' is not a git checkout",
            repo_path.display()
        ));
    }
    let repo = rdm_git::GitSourceRepo::new(&repo_path);
    let resolved = repo.rev_parse(sha).map_err(|e| e.to_string())?;
    resolved.ok_or_else(|| {
        format!(
            "applied_commit '{sha}' does not resolve to a commit in {what} at '{}' — if this is \
             an abbreviated SHA, it may be ambiguous; try a longer prefix or the full SHA",
            repo_path.display()
        )
    })
}

/// The [`resolve_applied_commit`] stub for builds without git support:
/// `applied_commit` cannot be verified without a repository to check it
/// against, so it is refused outright rather than accepted unverified.
///
/// # Errors
///
/// Always returns an error: this build has no git support.
#[cfg(not(feature = "git"))]
pub fn resolve_applied_commit(
    _store: &impl rdm_core::store::VersionedStore,
    _project: &str,
    _plan_root: &std::path::Path,
    _target: &rdm_core::model::ReviewTarget,
    _sha: &str,
) -> Result<String, String> {
    Err("applied_commit requires git support".to_string())
}

#[cfg(test)]
mod tests {
    /// Every note this module can produce begins by naming the surface, so a
    /// reader always learns *who* skipped before *why* — and never sees a
    /// null note where an explanation belongs.
    #[test]
    fn every_note_names_the_surface() {
        assert!(super::NO_RESOLUTION_NOTE.starts_with("this server does not resolve"));
    }
}

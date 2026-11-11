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
/// caller's literal input, together with the repository it resolved in.
///
/// Two repositories are candidates, tried in order
/// ([`rdm_git::resolve_commit_in_candidates`]): the one the review's target
/// kind implies first — the project's configured local source repo (via
/// [`source_for`]) for [`rdm_core::model::ReviewTarget::Change`], the plan
/// repo rooted at `plan_root` for every other kind (`roadmap`, `phase`,
/// `task`, `plan`) — then the other one. A candidate that is unavailable or
/// is not a git checkout is not fatal on its own. A SHA present in both
/// resolves in the first.
///
/// Only a commit-shaped `sha` (a hex SHA or SHA prefix) is looked for in the
/// second repository. A name such as `HEAD` or a branch means a different
/// commit in each repository, so it resolves in the first repository or is
/// refused — including when that repository is unavailable.
///
/// This is a **hard refusal**, unlike this module's read path
/// ([`resolve_review_for_project`]), which degrades with a note
/// instead of failing. `applied_commit` is a provenance record (and, for a
/// source commit, a public `rdm:src@<sha>` link), so a SHA that cannot be
/// verified in either repository must block the write rather than be
/// accepted unverified or silently degraded — the same fail-closed contract
/// `rdm-cli`'s `--applied-commit` enforces.
///
/// # Errors
///
/// Returns the detail string for a `400 Bad Request` when `sha` resolves in
/// no repository looked in. It names each one by role and path and says what
/// happened there: the SHA did not resolve (with a hint that an abbreviated
/// SHA may be ambiguous), the path is not a git checkout, or every cause
/// [`source_for`] itself reports when no source repo could be found at all.
#[cfg(feature = "git")]
pub fn resolve_applied_commit(
    store: &impl rdm_core::store::VersionedStore,
    project: &str,
    plan_root: &std::path::Path,
    target: &rdm_core::model::ReviewTarget,
    sha: &str,
) -> Result<(String, rdm_core::model::CommitRepo), String> {
    use rdm_core::model::CommitRepo;
    use rdm_git::{CommitCandidate, CommitLookup};

    let plan = CommitCandidate {
        repo: CommitRepo::Plan,
        location: Ok(plan_root.to_path_buf()),
    };
    let source = CommitCandidate {
        repo: CommitRepo::Source,
        location: source_for(store, project).map(|repo| repo.root().to_path_buf()),
    };
    let candidates = match target.implied_commit_repo() {
        CommitRepo::Source => [source, plan],
        CommitRepo::Plan => [plan, source],
    };
    match rdm_git::resolve_commit_in_candidates(&candidates, sha).map_err(|e| e.to_string())? {
        CommitLookup::Found { sha, repo } => Ok((sha, repo)),
        CommitLookup::NotFound {
            misses,
            primary_only,
        } => Err(applied_commit_refusal(sha, &misses, primary_only)),
    }
}

/// The refusal detail for an `applied_commit` that resolved in none of the
/// repositories looked in (`misses`, in the order they were tried).
#[cfg(feature = "git")]
fn applied_commit_refusal(
    sha: &str,
    misses: &[(rdm_core::model::CommitRepo, rdm_git::CandidateMiss)],
    primary_only: bool,
) -> String {
    use rdm_core::model::CommitRepo;
    use rdm_git::CandidateMiss;

    let role = |repo: &CommitRepo| match repo {
        CommitRepo::Plan => "the plan repo",
        CommitRepo::Source => "the project's configured source repo",
    };
    let places: Vec<String> = misses
        .iter()
        .map(|(repo, miss)| match miss {
            CandidateMiss::DidNotResolve(path) => format!("{} at '{}'", role(repo), path.display()),
            CandidateMiss::NotAGitCheckout(path) => format!(
                "{} at '{}' (which is not a git checkout)",
                role(repo),
                path.display()
            ),
            CandidateMiss::Unavailable(reason) => {
                format!("{} (not available: {reason})", role(repo))
            }
        })
        .collect();
    let mut detail = format!(
        "applied_commit '{sha}' does not resolve to a commit in {}",
        places.join(" or in ")
    );
    if primary_only {
        detail.push_str(
            " — a name such as HEAD or a branch is resolved only in the repository the review targets; pass the commit's SHA to have the other repository checked too",
        );
    } else if misses
        .iter()
        .any(|(_, miss)| matches!(miss, CandidateMiss::DidNotResolve(_)))
    {
        detail.push_str(
            " — if this is an abbreviated SHA, it may be ambiguous; try a longer prefix or the full SHA",
        );
    }
    detail
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
) -> Result<(String, rdm_core::model::CommitRepo), String> {
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

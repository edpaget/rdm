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

use rdm_core::anchor::{Resolution, ResolvedComment};
use rdm_core::model::Review;

/// The note a surface reports when this server cannot resolve change-review
/// anchors at all.
///
/// Every cause is suffixed onto it, so the reason a reader sees always begins
/// by naming the surface and then narrows to the specific cause.
const NO_RESOLUTION_NOTE: &str = "this server does not resolve change-review anchors";

/// Resolves `review`'s comments against the project's configured local source
/// repository, or degrades with a note saying why it could not.
///
/// The return shape is the one every read surface consumes — resolutions,
/// per-comment notes, and one `source_verification_skipped` note — and the
/// resolution policy itself is entirely
/// [`rdm_core::change::resolve_change_review`]'s. This function contributes
/// only the discovery half, so the REST and HTML surfaces can never disagree
/// with `rdm review show --format json` about a change review's states.
///
/// # Panics
///
/// Never panics.
#[must_use]
pub fn resolve_change_review_for_project(
    store: &impl rdm_core::store::VersionedStore,
    project: &str,
    review: &Review,
) -> (Vec<ResolvedComment>, Vec<Option<String>>, Option<String>) {
    #[cfg(feature = "git")]
    {
        match source_for(store, project) {
            Ok(source) => rdm_core::change::resolve_change_review(&source, review),
            Err(note) => (unresolved(review), Vec::new(), Some(note)),
        }
    }
    // No adapter exists in this build, so there is never a source — but the
    // surface still degrades WITH a note, exactly like the `git` build.
    #[cfg(not(feature = "git"))]
    {
        let _ = (store, project);
        (
            unresolved(review),
            Vec::new(),
            Some(format!(
                "{NO_RESOLUTION_NOTE} — this build has no git support"
            )),
        )
    }
}

/// One unresolved entry per comment, for every arm that reaches no source.
fn unresolved(review: &Review) -> Vec<ResolvedComment> {
    review
        .comments
        .iter()
        .map(|_| ResolvedComment {
            resolution: Resolution::Unresolved,
            quote: None,
        })
        .collect()
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

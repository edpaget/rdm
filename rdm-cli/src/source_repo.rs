//! Source-repository discovery: finding the checkout a `change/<sha>`
//! review's anchors resolve against, and confirming it is the project's
//! configured `source.repo` rather than merely *some* git repository that
//! happens to contain the invoking cwd.
//!
//! [`repo_matches_source`] moved here from `commands/link.rs`, which now
//! consumes it: `rdm link check`'s path verification and `rdm review --on
//! change/…` ask the same question ("is this checkout the project's
//! source?") and must never disagree about repository **identity**.
//!
//! Their **dispositions** deliberately differ, and an audit of the two
//! ladders confirmed it: link check is a lint that fails open, so it never
//! verifies against a repository the operator is not standing in, while
//! change review pins content and must either name its source or refuse.
//! [`rdm_core::source_select`] therefore owns the shared classification and
//! takes a [`SourceFallback`](rdm_core::source_select::SourceFallback)
//! parameter carrying exactly that difference; see its decision table.
//!
//! The two roles a path can play here are kept as separate values and must
//! never be swapped: the **identity root** (what
//! `rdm_git::worktree::discover_project_repo` answers, used only as the
//! left-hand side of [`repo_matches_source`]) and the **read root** (the
//! cwd, what a `GitSourceRepo` is actually constructed on). For a linked
//! worktree the identity root is the repository's *main* working tree, so
//! reading through it would make `change/HEAD` pin main's HEAD.

#[cfg(feature = "git")]
use anyhow::Context;
use anyhow::{Result, anyhow};

use crate::AppStore;

/// Whether the git repository at `repo` is actually the project's
/// configured `source.repo`.
///
/// `repo` is the checkout's **identity root**, never a path this function's
/// callers then read through — see the module documentation.
///
/// `source.repo` may name a filesystem path or a clone URL (see
/// [`rdm_core::model::Source`]'s doc comment): a filesystem path is compared
/// via canonicalized-path equality **first**, so a directory literally named
/// `…/repo.git` still matches itself; otherwise `repo`'s configured `origin`
/// remote (if any) is compared against `source.repo` via
/// [`rdm_core::source_select::locators_match`], which owns the pure
/// normalization half.
#[cfg(feature = "git")]
pub(crate) fn repo_matches_source(repo: &std::path::Path, source_repo: &str) -> bool {
    let source_path = std::path::Path::new(source_repo);
    if source_path.is_dir()
        && let (Ok(a), Ok(b)) = (repo.canonicalize(), source_path.canonicalize())
    {
        return a == b;
    }
    match rdm_git::remote_url(repo, "origin") {
        Ok(Some(origin)) => rdm_core::source_select::locators_match(&origin, source_repo),
        _ => false,
    }
}

/// Everything [`gather_source_environment`] observes about the invoking
/// process, for [`rdm_core::source_select::select_source`] to classify.
///
/// A named struct rather than a tuple: it already carries four values and
/// gained a fifth, and two of them are booleans that must never be swapped.
#[cfg(feature = "git")]
pub(crate) struct GatheredSourceEnvironment {
    /// The **read root** — the cwd, what a `GitSourceRepo` is constructed on.
    /// Never the identity root; see the module documentation.
    pub(crate) read_root: std::path::PathBuf,
    /// The project's configured `source.repo`, when it configures one.
    pub(crate) source: Option<rdm_core::model::Source>,
    /// Whether the cwd is inside a git checkout at all.
    pub(crate) in_checkout: bool,
    /// Whether that checkout is provably the configured source.
    pub(crate) checkout_matches_configured: bool,
    /// Whether that checkout is the **plan repo itself**.
    pub(crate) checkout_is_plan_repo: bool,
}

/// Gathers the [`SourceEnvironment`](rdm_core::source_select::SourceEnvironment)
/// both consumers classify, from the invoking process.
///
/// Returns the **read root** (the cwd) alongside the environment. The
/// checkout's identity root is consumed here and deliberately not returned:
/// nothing downstream may read through it.
///
/// `checkout_is_plan_repo` compares the cwd's discovered repository root with
/// the **discovered repository root of the plan store**, both canonicalized —
/// not `store.root()` raw. Going through discovery on both sides is what makes
/// a plan root nested inside its git repository, a symlinked `RDM_ROOT`, and a
/// plan repo opened through a linked worktree (where discovery answers with the
/// MAIN working tree) all compare correctly.
///
/// # Errors
///
/// Returns an error only when the current directory cannot be read.
#[cfg(feature = "git")]
pub(crate) fn gather_source_environment(
    store: &AppStore,
    project: &str,
) -> Result<GatheredSourceEnvironment> {
    let cwd = std::env::current_dir().context("failed to read the current directory")?;
    let source = rdm_core::io::load_project(store, project)
        .ok()
        .and_then(|doc| doc.frontmatter.source);
    // Identity only. For a linked worktree this is the repository's MAIN
    // working tree, which must never become the read root.
    let identity_root = rdm_git::worktree::discover_project_repo(&cwd).ok();
    let in_checkout = identity_root.is_some();
    let matches = match (&identity_root, &source) {
        (Some(identity_root), Some(src)) => repo_matches_source(identity_root, &src.repo),
        _ => false,
    };
    let checkout_is_plan_repo = match &identity_root {
        Some(identity_root) => same_repository(identity_root, store.root()),
        None => false,
    };
    Ok(GatheredSourceEnvironment {
        read_root: cwd,
        source,
        in_checkout,
        checkout_matches_configured: matches,
        checkout_is_plan_repo,
    })
}

/// Whether `repo` (already a discovered repository root) and the repository
/// containing `other` are the same repository.
///
/// Both sides go through `discover_project_repo` and `canonicalize`, so a
/// nested plan root, a symlinked path, and a linked worktree all reduce to the
/// same identity. A path that is not in a repository at all never matches.
#[cfg(feature = "git")]
fn same_repository(repo: &std::path::Path, other: &std::path::Path) -> bool {
    let Ok(other_root) = rdm_git::worktree::discover_project_repo(other) else {
        return false;
    };
    match (repo.canonicalize(), other_root.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => repo == other_root,
    }
}

/// Discovers the source repository a `change/<sha>` review reads from.
///
/// The decision itself is [`rdm_core::source_select::select_source`] under
/// [`SourceFallback::ConfiguredLocal`](rdm_core::source_select::SourceFallback::ConfiguredLocal);
/// this function is the adapter that gathers the environment and turns an
/// unavailable answer into an actionable message. See that function's
/// seven-environment decision table for the policy, and
/// `docs/change-reviews.md` § "Source discovery" for the prose.
///
/// The returned [`rdm_git::GitSourceRepo`] is rooted at the **cwd** rather
/// than the repository's main working tree, so `HEAD` and the current
/// branch reflect the linked worktree the operator is actually standing in
/// — exactly what `--on change/HEAD` has to pin. The classifier cannot
/// return anything else: it is handed booleans, never the identity root.
///
/// # Errors
///
/// Every failure is actionable and names what to do: not inside a checkout
/// and no local `source.repo`, or inside a checkout that is demonstrably a
/// different repository from the project's configured source. A build
/// without the `git` feature always fails, since a change review cannot
/// work without one.
#[cfg(feature = "git")]
pub(crate) fn discover_source_repo(
    store: &AppStore,
    project: &str,
) -> Result<rdm_git::GitSourceRepo> {
    use rdm_core::source_select::{
        ConfiguredSource, SourceEnvironment, SourceFallback, SourceSelection, SourceUnavailable,
        select_source,
    };

    let gathered = gather_source_environment(store, project)?;
    let source = gathered.source;
    let env = SourceEnvironment {
        in_checkout: gathered.in_checkout,
        checkout_matches_configured: gathered.checkout_matches_configured,
        checkout_is_plan_repo: gathered.checkout_is_plan_repo,
        configured: source.as_ref().map(|src| ConfiguredSource {
            locator: src.repo.as_str(),
            is_local_dir: std::path::Path::new(&src.repo).is_dir(),
        }),
    };

    match select_source(&env, SourceFallback::ConfiguredLocal) {
        SourceSelection::ReadCwdCheckout => Ok(rdm_git::GitSourceRepo::new(gathered.read_root)),
        SourceSelection::ReadConfiguredLocal => {
            let src = source.as_ref().expect("a configured local source");
            Ok(rdm_git::GitSourceRepo::new(&src.repo))
        }
        SourceSelection::Unavailable(SourceUnavailable::CheckoutIsNotConfiguredSource {
            locator,
        }) => Err(anyhow!(
            "the current directory is inside a git checkout, but not project '{project}''s configured source repo ({locator}) — run this from a checkout of that repository"
        )),
        SourceSelection::Unavailable(SourceUnavailable::ConfiguredSourceNotLocal { locator }) => {
            Err(anyhow!(
                "not inside a git checkout, and project '{project}''s configured source repo ({locator}) is not a local directory — run this from a checkout of that repository"
            ))
        }
        // E6b — standing in the plan repo with nothing configured. Reading it
        // would pin the plan repo's own HEAD and record a review of plan data
        // as though it were the project's code, so refuse and name both ways
        // out.
        SourceSelection::Unavailable(SourceUnavailable::CheckoutIsPlanRepo) => Err(anyhow!(
            "the current directory is the plan repo, not a source checkout — run this from the project's source checkout, or set `source.repo` in its project.md"
        )),
        // `NotInCheckout` and `NoSourceConfigured` are `CwdOnly`-only
        // outcomes (see the decision table); under `ConfiguredLocal` only
        // `NoCheckoutAndNoSource` reaches here.
        SourceSelection::Unavailable(_) => Err(anyhow!(
            "not inside a git checkout, and project '{project}' configures no source repo — run this from the project's source checkout, or set `source.repo` in its project.md"
        )),
    }
}

#[cfg(all(test, feature = "git"))]
mod tests {
    use super::*;
    use std::path::Path;
    use std::process::Command;
    use tempfile::TempDir;

    /// A `git` command in `dir` with the ambient git environment cleared —
    /// these tests must work unchanged from inside a git hook, which exports
    /// `GIT_DIR`/`GIT_WORK_TREE`/`GIT_INDEX_FILE`.
    fn git(dir: &Path, args: &[&str]) {
        let out = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?} failed");
    }

    #[test]
    fn repo_matches_source_compares_url_form_via_origin_remote() {
        let dir = TempDir::new().unwrap();
        git(dir.path(), &["init"]);
        git(
            dir.path(),
            &[
                "remote",
                "add",
                "origin",
                "https://example.com/org/repo.git",
            ],
        );
        assert!(repo_matches_source(
            dir.path(),
            "https://example.com/org/repo"
        ));
        assert!(repo_matches_source(
            dir.path(),
            "https://example.com/org/repo.git/"
        ));
        assert!(!repo_matches_source(
            dir.path(),
            "https://example.com/org/other"
        ));
    }

    #[test]
    fn repo_matches_source_false_when_origin_remote_unset() {
        let dir = TempDir::new().unwrap();
        git(dir.path(), &["init"]);
        assert!(!repo_matches_source(
            dir.path(),
            "https://example.com/org/repo"
        ));
    }

    #[test]
    fn repo_matches_source_compares_local_paths_by_canonical_form() {
        let dir = TempDir::new().unwrap();
        git(dir.path(), &["init"]);
        let as_str = dir.path().to_str().unwrap();
        assert!(repo_matches_source(dir.path(), as_str));
    }
}

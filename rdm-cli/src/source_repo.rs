//! Source-repository discovery: finding the checkout a `change/<sha>`
//! review's anchors resolve against, and confirming it is the project's
//! configured `source.repo` rather than merely *some* git repository that
//! happens to contain the invoking cwd.
//!
//! [`normalize_repo_locator`] and [`repo_matches_source`] moved here from
//! `commands/link.rs`, which now consumes them: `rdm link check`'s path
//! verification and `rdm review --on change/…` ask the same question ("is
//! this checkout the project's source?") and must never answer it two
//! different ways.

#[cfg(feature = "git")]
use anyhow::Context;
use anyhow::{Result, anyhow};

use crate::AppStore;

/// Trims a trailing `/` and a trailing `.git` (in that order) so a source
/// URL/path can be compared for equality regardless of those two common
/// stylistic variations — e.g. `https://example.com/org/repo` and
/// `https://example.com/org/repo.git/` normalize to the same value. Not a
/// full URL parse: it deliberately does not reconcile scheme differences
/// (`git@host:org/repo.git` vs `https://host/org/repo`), so those still
/// compare unequal.
///
/// Feature-gated with its callers: without the `git` feature there is no
/// checkout to compare against, so this would otherwise be dead code under
/// `-D warnings`.
#[cfg(feature = "git")]
pub(crate) fn normalize_repo_locator(locator: &str) -> String {
    locator
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .to_string()
}

/// Whether the git repository at `repo` is actually the project's
/// configured `source.repo`.
///
/// `source.repo` may name a filesystem path or a clone URL (see
/// [`rdm_core::model::Source`]'s doc comment): a filesystem path is compared
/// via canonicalized-path equality; otherwise `repo`'s configured `origin`
/// remote (if any) is compared against `source.repo`, both normalized via
/// [`normalize_repo_locator`].
#[cfg(feature = "git")]
pub(crate) fn repo_matches_source(repo: &std::path::Path, source_repo: &str) -> bool {
    let source_path = std::path::Path::new(source_repo);
    if source_path.is_dir()
        && let (Ok(a), Ok(b)) = (repo.canonicalize(), source_path.canonicalize())
    {
        return a == b;
    }
    match rdm_git::remote_url(repo, "origin") {
        Ok(Some(origin)) => normalize_repo_locator(&origin) == normalize_repo_locator(source_repo),
        _ => false,
    }
}

/// Discovers the source repository a `change/<sha>` review reads from.
///
/// Resolution order:
///
/// 1. The checkout containing the invoking cwd, when it matches the
///    project's configured `source.repo` (or when the project configures no
///    source at all — there is then nothing to contradict).
/// 2. Otherwise `source.repo` itself, when it names a local directory.
///
/// The returned [`rdm_git::GitSourceRepo`] is rooted at the *current*
/// checkout rather than the repository's main working tree, so `HEAD` and
/// the current branch reflect the linked worktree the operator is actually
/// standing in — exactly what `--on change/HEAD` has to pin.
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
    let source = rdm_core::io::load_project(store, project)
        .ok()
        .and_then(|doc| doc.frontmatter.source);

    let cwd = std::env::current_dir().context("failed to read the current directory")?;
    let discovered = rdm_git::worktree::discover_project_repo(&cwd).ok();

    match (&discovered, &source) {
        // In a checkout, and either it matches the configured source or the
        // project configures none: use the cwd so a linked worktree's own
        // HEAD/branch are what `change/HEAD` pins.
        (Some(repo), Some(src)) if repo_matches_source(repo, &src.repo) => {
            Ok(rdm_git::GitSourceRepo::new(cwd))
        }
        (Some(_), None) => Ok(rdm_git::GitSourceRepo::new(cwd)),
        // In a checkout that is NOT the project's source: prefer a local
        // `source.repo` if there is one, else say exactly what is wrong.
        (Some(_), Some(src)) => {
            if std::path::Path::new(&src.repo).is_dir() {
                return Ok(rdm_git::GitSourceRepo::new(&src.repo));
            }
            Err(anyhow!(
                "the current directory is inside a git checkout, but not project '{project}''s configured source repo ({}) — run this from a checkout of that repository",
                src.repo
            ))
        }
        (None, Some(src)) if std::path::Path::new(&src.repo).is_dir() => {
            Ok(rdm_git::GitSourceRepo::new(&src.repo))
        }
        (None, Some(src)) => Err(anyhow!(
            "not inside a git checkout, and project '{project}''s configured source repo ({}) is not a local directory — run this from a checkout of that repository",
            src.repo
        )),
        (None, None) => Err(anyhow!(
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

    #[test]
    fn normalize_repo_locator_trims_trailing_slash_then_dot_git() {
        assert_eq!(
            normalize_repo_locator("https://example.com/org/repo.git/"),
            "https://example.com/org/repo"
        );
        assert_eq!(
            normalize_repo_locator("https://example.com/org/repo.git"),
            "https://example.com/org/repo"
        );
        assert_eq!(
            normalize_repo_locator("https://example.com/org/repo/"),
            "https://example.com/org/repo"
        );
        assert_eq!(
            normalize_repo_locator("https://example.com/org/repo"),
            "https://example.com/org/repo"
        );
    }

    #[test]
    fn normalize_repo_locator_does_not_reconcile_scheme_differences() {
        assert_ne!(
            normalize_repo_locator("git@example.com:org/repo.git"),
            normalize_repo_locator("https://example.com/org/repo")
        );
    }

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

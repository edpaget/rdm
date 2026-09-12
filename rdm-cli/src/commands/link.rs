//! `rdm link check` / `rdm link list` / `rdm link resolve`.
//!
//! Thin dispatch over `rdm_core::ops::links`: parse the `--on`/positional
//! reference the same way `rdm review --on` does (via
//! `ops::reviews::parse_review_target_ref`), call the core op, and format
//! via `rdm_core::display`/`rdm_core::json`. The one piece that lives here
//! rather than in core is `check`'s checkout-aware code-link path
//! verification (behind the `git` feature), mirroring `phase.rs`'s
//! `needs_review_warning` pattern of a feature-gated block calling
//! `rdm_git`/`rdm_git::worktree::discover_project_repo` from the invoking
//! CWD.

use std::process;

use anyhow::{Context, Result};
use rdm_core::config::Config;
use rdm_core::link::Link;
use rdm_core::ops::links::LinkCheckReport;
use rdm_core::{display, json};

use crate::paths;
use crate::{AppStore, LinkCommand, OutputFormat};

/// Trims a trailing `/` and a trailing `.git` (in that order) so a source
/// URL/path can be compared for equality regardless of those two common
/// stylistic variations — e.g. `https://example.com/org/repo` and
/// `https://example.com/org/repo.git/` normalize to the same value. Not a
/// full URL parse: it deliberately does not reconcile scheme differences
/// (`git@host:org/repo.git` vs `https://host/org/repo`), so those still
/// compare unequal.
///
/// Feature-gated with its only caller, [`repo_matches_source`]: without the
/// `git` feature there is no checkout to compare against, so this would
/// otherwise be dead code under `-D warnings`.
#[cfg(feature = "git")]
fn normalize_repo_locator(locator: &str) -> String {
    locator
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .to_string()
}

/// Whether the git repository at `repo` is actually the project's
/// configured `source.repo`, not merely *some* git repository that happens
/// to contain the invoking `cwd`.
///
/// `source.repo` may name a filesystem path or a clone URL (see
/// [`rdm_core::model::Source`]'s doc comment): a filesystem path is compared
/// via canonicalized-path equality; otherwise `repo`'s configured `origin`
/// remote (if any) is compared against `source.repo`, both normalized via
/// [`normalize_repo_locator`].
#[cfg(feature = "git")]
fn repo_matches_source(repo: &std::path::Path, source_repo: &str) -> bool {
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

/// Runs `rdm link` subcommands.
///
/// # Errors
///
/// Returns an error if the project or `--on` target cannot be resolved, the
/// underlying check/list/resolve operation fails, or serialization fails.
pub fn run(
    command: LinkCommand,
    store: &mut AppStore,
    repo_config: &Config,
    format: OutputFormat,
) -> Result<()> {
    match command {
        LinkCommand::Check { on, project } => check(store, repo_config, format, on, project),
        LinkCommand::List { on, project } => list(store, repo_config, format, on, project),
        LinkCommand::Resolve { uri, project } => resolve(store, repo_config, format, uri, project),
    }
}

fn check(
    store: &mut AppStore,
    repo_config: &Config,
    format: OutputFormat,
    on: Option<String>,
    project: Option<String>,
) -> Result<()> {
    let project = paths::resolve_project(project, repo_config)?;
    let target = match &on {
        Some(reference) => Some(
            rdm_core::ops::reviews::parse_review_target_ref(store, &project, reference)
                .context("failed to resolve --on target")?,
        ),
        None => None,
    };

    let mut report = match &target {
        Some(t) => rdm_core::ops::links::check_document(store, &project, t)
            .context("failed to check document links")?,
        None => rdm_core::ops::links::check_project(store, &project)
            .context("failed to check project links")?,
    };

    verify_paths_at_pinned_rev(store, &project, &mut report);

    let on_label = target.as_ref().map(rdm_core::link::ItemRef::label);
    let report_json = json::link_check_report_to_json(&report, on_label.as_deref());
    let broken = !report.dangling.is_empty()
        || !report.diagnostics.is_empty()
        || !report.missing_at_rev.is_empty();

    match format {
        OutputFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(&report_json)
                .context("failed to serialize link check report")?
        ),
        OutputFormat::Human | OutputFormat::Table | OutputFormat::Markdown => {
            print!("{}", display::format_link_check_report(&report_json));
        }
    }

    if broken {
        process::exit(1);
    }
    Ok(())
}

/// Fills in `report.missing_at_rev`/`path_verification_skipped` — the one
/// enrichment [`rdm_core::ops::links::check_project`]/`check_document`
/// deliberately leave to the caller, since core has no source-repo checkout
/// to verify a code link's path against (see
/// [`rdm_core::ops::links::LinkCheckReport`]'s doc comment).
///
/// Distinct outcomes, never conflated:
/// - Git not installed, `cwd` not inside any checkout, the project has no
///   `source` configured, or the discovered checkout doesn't match the
///   project's configured `source.repo` (see [`repo_matches_source`]) — all
///   fall through to a "skipped" note explaining which of those applies,
///   rather than silently verifying against an unrelated repository.
/// - Inside the *matching* checkout, each code link's path is checked at its
///   pinned revision (falling back to the project's configured default
///   branch, then `"main"`, when the link itself carries no resolved rev —
///   mirroring `resolve_code_link`'s own `web_url` fallback).
/// - A single failed lookup — a spawn error, or a pinned revision this
///   checkout cannot resolve (e.g. a shallow clone missing the commit) — is
///   treated as inconclusive and fail-open, never a false "missing", and
///   never aborts the rest of the report.
fn verify_paths_at_pinned_rev(store: &AppStore, project: &str, report: &mut LinkCheckReport) {
    #[cfg(feature = "git")]
    {
        let cwd = match std::env::current_dir() {
            Ok(cwd) => cwd,
            Err(_) => {
                report.path_verification_skipped =
                    Some("could not determine the current directory".to_string());
                return;
            }
        };
        let repo = match rdm_git::worktree::discover_project_repo(&cwd) {
            Ok(repo) => repo,
            Err(_) => {
                report.path_verification_skipped =
                    Some("not inside a source-repo checkout".to_string());
                return;
            }
        };
        let source = rdm_core::io::load_project(store, project)
            .ok()
            .and_then(|doc| doc.frontmatter.source);
        let source = match source {
            Some(source) => source,
            None => {
                report.path_verification_skipped = Some(
                    "project has no configured source repo — skipping path verification"
                        .to_string(),
                );
                return;
            }
        };
        if !repo_matches_source(&repo, &source.repo) {
            report.path_verification_skipped = Some(format!(
                "cwd is inside a git checkout, but not the project's configured source ({}) — skipping path verification",
                source.repo
            ));
            return;
        }
        let default_rev = source.default_branch.unwrap_or_else(|| "main".to_string());
        for link in report.code_links.clone() {
            let rev = link.rev.clone().unwrap_or_else(|| default_rev.clone());
            // `Ok(None)` (the pinned rev doesn't resolve in this checkout —
            // e.g. a shallow clone) and `Err` (a spawn failure) are both
            // inconclusive: fail open rather than report a false "missing".
            let exists = rdm_git::path_exists_at_rev(&repo, &rev, &link.path)
                .unwrap_or(Some(true))
                .unwrap_or(true);
            if !exists {
                report
                    .missing_at_rev
                    .push(rdm_core::ops::links::MissingAtRevFinding {
                        document: link.document,
                        byte_range: link.byte_range,
                        path: link.path,
                        rev,
                    });
            }
        }
    }
    #[cfg(not(feature = "git"))]
    {
        let _ = (store, project);
        report.path_verification_skipped = Some("built without git support".to_string());
    }
}

fn list(
    store: &mut AppStore,
    repo_config: &Config,
    format: OutputFormat,
    on: String,
    project: Option<String>,
) -> Result<()> {
    let project = paths::resolve_project(project, repo_config)?;
    let target = rdm_core::ops::reviews::parse_review_target_ref(store, &project, &on)
        .context("failed to resolve --on target")?;
    let (_doc_ref, body, containing_commit) =
        rdm_core::ops::links::load_document_body(store, &project, &target)
            .context("failed to load document")?;
    let (links, _diagnostics) = rdm_core::link::extract_links(&body);

    let mut entries = Vec::with_capacity(links.len());
    for (_range, link) in &links {
        let resolved =
            rdm_core::ops::links::resolve_link(store, &project, containing_commit.as_deref(), link)
                .context("failed to resolve link")?;
        let item_path = item_path_for(store, &project, link)?;
        entries.push((
            link.to_string(),
            json::resolved_to_json(&resolved, item_path.as_deref()),
        ));
    }

    match format {
        OutputFormat::Json => {
            let arr: Vec<_> = entries
                .iter()
                .map(|(uri, resolved)| json::outgoing_link_to_json(uri, resolved.clone()))
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&arr).context("failed to serialize outgoing links")?
            );
        }
        OutputFormat::Human | OutputFormat::Table | OutputFormat::Markdown => {
            print!(
                "{}",
                display::format_outgoing_links(&target.label(), &entries)
            );
        }
    }
    Ok(())
}

fn resolve(
    store: &mut AppStore,
    repo_config: &Config,
    format: OutputFormat,
    uri: String,
    project: Option<String>,
) -> Result<()> {
    let project = paths::resolve_project(project, repo_config)?;
    let link =
        rdm_core::link::parse(&uri).with_context(|| format!("'{uri}' is not a valid rdm: link"))?;
    // A bare CLI-supplied URI has no containing document, so there is no
    // stamped phase/task commit to thread as `containing_commit` — a code
    // link's rev resolves only as far as an explicit `@rev` on the URI
    // itself. A link found inside a real document body (`link list`) can
    // resolve further, via that document's stamped commit.
    let resolved = rdm_core::ops::links::resolve_link(store, &project, None, &link)
        .context("failed to resolve link")?;
    let item_path = item_path_for(store, &project, &link)?;
    let resolved_json = json::resolved_to_json(&resolved, item_path.as_deref());

    match format {
        OutputFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(&resolved_json)
                .context("failed to serialize resolved link")?
        ),
        OutputFormat::Human | OutputFormat::Table | OutputFormat::Markdown => {
            print!("{}", display::format_resolved(&uri, &resolved_json));
        }
    }
    Ok(())
}

/// For an item link, computes the plan-repo-relative path to the target
/// document — [`rdm_core::link::Resolved::Item`] doesn't carry one (see its
/// doc comment) — via [`rdm_core::ops::links::item_ref_path`], the single
/// authoritative implementation numeric-phase-stem resolution also backs
/// for [`rdm_core::ops::links::resolve_item_link`]'s own existence check.
/// `None` for a code link (its own `path` field already covers it).
fn item_path_for(store: &AppStore, project: &str, link: &Link) -> Result<Option<String>> {
    let target = match link {
        Link::Item(target) => target,
        Link::Code { .. } => return Ok(None),
    };
    let path = rdm_core::ops::links::item_ref_path(store, project, target)
        .context("failed to resolve item link path")?;
    Ok(Some(path.as_str().to_string()))
}

#[cfg(all(test, feature = "git"))]
mod tests {
    use super::*;

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
        // Deliberately not a full URL parse: an SSH-style locator and an
        // HTTPS locator for the same underlying repo still compare unequal.
        assert_ne!(
            normalize_repo_locator("git@example.com:org/repo.git"),
            normalize_repo_locator("https://example.com/org/repo")
        );
    }

    #[test]
    fn repo_matches_source_compares_url_form_via_origin_remote() {
        let repo = TempDirGuard::new_git_repo();
        rdm_git::run_git_at(
            repo.path(),
            &[
                "remote",
                "add",
                "origin",
                "https://example.com/org/repo.git",
            ],
        )
        .unwrap();

        assert!(repo_matches_source(
            repo.path(),
            "https://example.com/org/repo"
        ));
        assert!(repo_matches_source(
            repo.path(),
            "https://example.com/org/repo.git/"
        ));
        assert!(!repo_matches_source(
            repo.path(),
            "https://example.com/org/other-repo"
        ));
    }

    #[test]
    fn repo_matches_source_false_when_origin_remote_unset() {
        let repo = TempDirGuard::new_git_repo();
        assert!(!repo_matches_source(
            repo.path(),
            "https://example.com/org/repo"
        ));
    }

    /// Minimal throwaway git checkout for these unit tests — the CLI
    /// integration tests in `cli_link.rs` cover the full `link check`
    /// path-verification behavior end to end; these cover just the two
    /// small pure/near-pure comparison helpers directly.
    struct TempDirGuard(tempfile::TempDir);

    impl TempDirGuard {
        fn new_git_repo() -> Self {
            let dir = tempfile::TempDir::new().unwrap();
            rdm_git::run_git_at(dir.path(), &["init", "-q", "-b", "main"]).unwrap();
            rdm_git::run_git_at(dir.path(), &["config", "user.email", "test@test.com"]).unwrap();
            rdm_git::run_git_at(dir.path(), &["config", "user.name", "test"]).unwrap();
            Self(dir)
        }

        fn path(&self) -> &std::path::Path {
            self.0.path()
        }
    }
}

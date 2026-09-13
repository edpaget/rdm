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
#[cfg(feature = "git")]
use crate::source_repo::repo_matches_source;
use crate::{AppStore, LinkCommand, OutputFormat};

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
///   project's configured `source.repo` (see
///   [`crate::source_repo::repo_matches_source`]) — all
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

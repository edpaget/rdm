//! `rdm backlinks` — list documents referencing a plan item, and documents
//! and review comments annotating a source-repository commit or path.
//!
//! Top-level (not nested under `link`) per the phase body's "top-level for
//! discoverability". The reference is parsed by
//! [`rdm_core::ops::links::parse_backlink_target`] — an item reference the
//! same way `rdm review --on` reads one, a commit as `rdm:src@<sha>`, or a
//! path as `rdm:src/<path>[@<rev>][#L<a>[-L<b>]]`. An item is answered by
//! the store-only [`rdm_core::ops::links::backlinks`]; a commit or a path by
//! [`rdm_core::ops::links::code_annotations`], which reads the project's
//! source checkout (found the way `rdm review --on change/…` finds it).

use anyhow::{Context, Result};
use rdm_core::config::Config;
use rdm_core::link::BacklinkTarget;
use rdm_core::{display, json};

use crate::paths;
use crate::{AppStore, OutputFormat};

/// Runs `rdm backlinks <reference>`.
///
/// # Errors
///
/// Returns an error if the project or reference cannot be resolved, the
/// backlink scan fails, or serialization fails. A commit or path reference
/// also fails when no source checkout can be found for the project, when
/// its revision or path does not exist there, when the path is not a file,
/// or on a build without git support.
pub fn run(
    store: &mut AppStore,
    repo_config: &Config,
    format: OutputFormat,
    reference: String,
    project: Option<String>,
) -> Result<()> {
    let project = paths::resolve_project(project, repo_config)?;
    let target = rdm_core::ops::links::parse_backlink_target(store, &project, &reference)
        .context("failed to resolve reference")?;
    if !matches!(target, BacklinkTarget::Item(_)) {
        return run_code(store, &project, format, &target);
    }
    let entries = rdm_core::ops::links::backlinks(store, &project, &target)
        .context("failed to find backlinks")?;
    let label = target.label();

    match format {
        OutputFormat::Json => {
            let arr: Vec<_> = entries.iter().map(json::backlink_entry_to_json).collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&arr).context("failed to serialize backlinks")?
            );
        }
        OutputFormat::Human | OutputFormat::Table | OutputFormat::Markdown => {
            print!("{}", display::format_backlinks(&label, &entries));
        }
    }
    Ok(())
}

/// `rdm backlinks` for a commit or path target: every document and review
/// comment annotating that code location, read against the project's source
/// checkout.
#[cfg(feature = "git")]
fn run_code(
    store: &AppStore,
    project: &str,
    format: OutputFormat,
    target: &BacklinkTarget,
) -> Result<()> {
    let source = crate::source_repo::discover_source_repo(store, project)?;
    let annotations = rdm_core::ops::links::code_annotations(store, &source, project, target)
        .context("failed to find code annotations")?;
    match format {
        OutputFormat::Json => {
            let arr: Vec<_> = annotations
                .iter()
                .map(json::code_annotation_to_json)
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&arr)
                    .context("failed to serialize code annotations")?
            );
        }
        OutputFormat::Human | OutputFormat::Table | OutputFormat::Markdown => {
            print!(
                "{}",
                display::format_code_annotations(&target.label(), &annotations)
            );
        }
    }
    Ok(())
}

/// A build without git support cannot read a source checkout, so a commit
/// or path target is refused outright rather than answered as empty.
#[cfg(not(feature = "git"))]
fn run_code(
    _store: &AppStore,
    _project: &str,
    _format: OutputFormat,
    _target: &BacklinkTarget,
) -> Result<()> {
    anyhow::bail!("a source-repository backlinks target (rdm:src…) needs a build with git support")
}

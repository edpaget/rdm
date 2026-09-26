//! `rdm backlinks` — list documents referencing a plan item or a
//! source-repository commit.
//!
//! Top-level (not nested under `link`) per the phase body's "top-level for
//! discoverability". Thin dispatch over
//! [`rdm_core::ops::links::backlinks`]: the reference is parsed by
//! [`rdm_core::ops::links::parse_backlink_target`] — an item reference the
//! same way `rdm review --on` reads one, or a commit as `rdm:src@<sha>`.

use anyhow::{Context, Result};
use rdm_core::config::Config;
use rdm_core::{display, json};

use crate::paths;
use crate::{AppStore, OutputFormat};

/// Runs `rdm backlinks <reference>`.
///
/// # Errors
///
/// Returns an error if the project or reference cannot be resolved, the
/// backlink scan fails, or serialization fails.
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

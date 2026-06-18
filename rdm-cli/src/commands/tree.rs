use std::path::Path;

use anyhow::{Context, Result, bail};
use rdm_core::tree;

use crate::OutputFormat;
use crate::commands;
use crate::paths;

/// Shows a hierarchical tree of a project's roadmaps, phases, and tasks.
///
/// # Errors
///
/// Returns an error if the project cannot be resolved, the tree cannot be
/// built, serialization fails, or `--format table` is requested.
pub fn run(
    root: &Path,
    repo_config: &rdm_core::config::Config,
    staging: bool,
    format: OutputFormat,
    project: Option<String>,
) -> Result<()> {
    let store = commands::make_store(root, staging)?;
    let project = paths::resolve_project(project, repo_config)?;
    let node = tree::build_tree(&store, &project).context("failed to build tree")?;
    match format {
        OutputFormat::Human => print!("{}", tree::format_tree(&node)),
        OutputFormat::Markdown => print!("{}", tree::format_tree_md(&node)),
        OutputFormat::Json => {
            println!(
                "{}",
                serde_json::to_string_pretty(&node).context("failed to serialize tree")?
            );
        }
        OutputFormat::Table => bail!(
            "--format table is not supported for 'tree'; use --format human, --format json, --format markdown, or omit --format"
        ),
    }
    commands::maybe_print_uncommitted_hint(&store, staging);
    Ok(())
}

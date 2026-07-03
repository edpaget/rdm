use std::path::Path;

use anyhow::{Context, Result};
use rdm_core::json;

use crate::OutputFormat;
use crate::commands;
use crate::commands::roadmap;
use crate::paths;

/// Lists roadmaps and their progress, optionally across all projects.
///
/// # Errors
///
/// Returns an error if the project cannot be resolved, the store cannot be
/// opened, roadmaps/phases cannot be listed, or serialization fails.
pub fn run(
    root: &Path,
    repo_config: &rdm_core::config::Config,
    format: OutputFormat,
    project: Option<String>,
    all: bool,
) -> Result<()> {
    let store = commands::make_store(root)?;
    let projects = if all {
        rdm_core::ops::project::list_projects(&store).context("failed to list projects")?
    } else {
        let p = paths::resolve_project(project, repo_config)?;
        vec![p]
    };

    // For JSON, collect all projects' summaries into one array.
    let mut all_summaries: Vec<json::RoadmapSummaryJson> = Vec::new();

    for project in &projects {
        if all && format != OutputFormat::Json {
            println!("Project: {project}");
        }
        let entries = roadmap::collect_entries(&store, project, None, None)?;
        if format == OutputFormat::Json {
            for (doc, phases) in &entries {
                all_summaries.push(json::roadmap_summary_to_json(doc, phases));
            }
        } else {
            roadmap::print_entries(&entries, format)?;
        }
    }
    if format == OutputFormat::Json {
        println!(
            "{}",
            serde_json::to_string_pretty(&all_summaries).context("failed to serialize roadmaps")?
        );
    }
    commands::maybe_print_uncommitted_hint(&store);
    Ok(())
}

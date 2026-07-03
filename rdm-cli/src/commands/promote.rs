use std::path::Path;

use anyhow::Result;

use crate::commands;
use crate::paths;

/// Promotes a task to a roadmap.
///
/// # Errors
///
/// Returns an error if the project cannot be resolved, the store cannot be
/// opened, or the promotion fails.
pub fn run(
    root: &Path,
    repo_config: &rdm_core::config::Config,
    no_index: bool,
    task_slug: String,
    roadmap_slug: String,
    project: Option<String>,
) -> Result<()> {
    let mut store = commands::make_store(root)?;
    let project = paths::resolve_project(project, repo_config)?;
    let doc = commands::commit_mutation(
        &mut store,
        &project,
        no_index,
        "failed to promote task",
        |s| rdm_core::ops::task::promote_task(s, &project, &task_slug, &roadmap_slug),
    )?;
    println!(
        "Promoted task '{task_slug}' → roadmap '{}'",
        doc.frontmatter.roadmap
    );
    Ok(())
}

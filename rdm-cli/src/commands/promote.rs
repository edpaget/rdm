use std::path::Path;

use anyhow::{Result, bail};

use crate::commands;
use crate::paths;

/// Promotes a task to a new roadmap, or consolidates it into an existing one.
///
/// Exactly one of `roadmap_slug` (create-new) or `into` (consolidate into an
/// existing roadmap) must be provided; `body`/`no_edit` only apply to the
/// `into` path. clap's `conflicts_with` already prevents both being set.
///
/// # Errors
///
/// Returns an error if the project cannot be resolved, the store cannot be
/// opened, neither `roadmap_slug` nor `into` was provided, `body`/`no_edit`
/// was passed together with `roadmap_slug`, or the promotion/consolidation
/// fails.
#[allow(clippy::too_many_arguments)]
pub fn run(
    root: &Path,
    repo_config: &rdm_core::config::Config,
    no_index: bool,
    task_slug: String,
    roadmap_slug: Option<String>,
    into: Option<String>,
    body: Option<String>,
    no_edit: bool,
    project: Option<String>,
) -> Result<()> {
    let mut store = commands::make_store(root)?;
    let project = paths::resolve_project(project, repo_config)?;

    match (roadmap_slug, into) {
        (None, None) => {
            bail!(
                "provide either --roadmap-slug <new> to create a roadmap or --into <existing> to consolidate into one"
            );
        }
        (Some(new_roadmap), None) => {
            if body.is_some() || no_edit {
                bail!("--body and --no-edit only apply to --into; omit them with --roadmap-slug");
            }
            let doc = commands::commit_mutation(
                &mut store,
                &project,
                no_index,
                "failed to promote task",
                |s| rdm_core::ops::task::promote_task(s, &project, &task_slug, &new_roadmap),
            )?;
            println!(
                "Promoted task '{task_slug}' → roadmap '{}'",
                doc.frontmatter.roadmap
            );
        }
        (None, Some(existing_roadmap)) => {
            let body_override = commands::resolve_body(body, no_edit)?;
            let (phase_doc, task_doc) = commands::commit_mutation(
                &mut store,
                &project,
                no_index,
                "failed to consolidate task",
                |s| {
                    rdm_core::ops::task::consolidate_task_into_roadmap(
                        s,
                        &project,
                        &task_slug,
                        &existing_roadmap,
                        body_override.as_deref(),
                    )
                },
            )?;
            let phase_stem = rdm_core::model::phase_stem(phase_doc.frontmatter.phase, &task_slug);
            println!(
                "Consolidated task '{task_slug}' → roadmap '{existing_roadmap}' as phase '{phase_stem}' (task status: {})",
                task_doc.frontmatter.status
            );
        }
        (Some(_), Some(_)) => unreachable!("clap conflicts_with prevents both being set"),
    }
    Ok(())
}

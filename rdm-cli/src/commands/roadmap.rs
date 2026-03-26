use anyhow::{Context, Result, bail};
use rdm_core::config::Config;
use rdm_core::display;
use rdm_core::json;

use super::{maybe_print_uncommitted_hint, maybe_regenerate_index, reject_non_human, resolve_body};
use crate::paths;
use crate::table;
use crate::{AppStore, OutputFormat, RoadmapCommand};

pub fn run(
    command: RoadmapCommand,
    store: &mut AppStore,
    repo_config: &Config,
    format: OutputFormat,
    no_index: bool,
    staging: bool,
) -> Result<()> {
    match command {
        RoadmapCommand::Create {
            slug,
            title,
            project,
            priority,
            body,
            no_edit,
        } => {
            let project = paths::resolve_project(project, repo_config)?;
            let title = title.as_deref().unwrap_or(&slug);
            let body = resolve_body(body, no_edit)?;
            rdm_core::ops::roadmap::create_roadmap(
                store,
                &project,
                &slug,
                title,
                body.as_deref(),
                priority,
            )
            .context("failed to create roadmap")?;
            println!("Created roadmap '{slug}' in project '{project}'");
            maybe_regenerate_index(store, no_index, staging, Some(&project))?;
        }
        RoadmapCommand::Show {
            slug,
            project,
            no_body,
        } => {
            let project = paths::resolve_project(project, repo_config)?;
            let mut roadmap_doc = rdm_core::io::load_roadmap(store, &project, &slug)
                .context("failed to load roadmap")?;
            let phases = rdm_core::ops::phase::list_phases(store, &project, &slug)
                .context("failed to list phases")?;
            if no_body {
                roadmap_doc.body = String::new();
            }
            match format {
                OutputFormat::Human => {
                    print!("{}", display::format_roadmap_summary(&roadmap_doc, &phases))
                }
                OutputFormat::Markdown => print!(
                    "{}",
                    display::format_roadmap_summary_md(&roadmap_doc, &phases)
                ),
                OutputFormat::Json => {
                    let j = json::roadmap_to_json(&roadmap_doc, &phases);
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&j).context("failed to serialize roadmap")?
                    );
                }
                OutputFormat::Table => bail!(
                    "--format table is not supported for 'roadmap show'; use --format human, --format json, --format markdown, or omit --format"
                ),
            }
            maybe_print_uncommitted_hint(store, staging);
        }
        RoadmapCommand::Update {
            slug,
            project,
            priority,
            clear_priority,
            body,
            no_edit,
        } => {
            let project = paths::resolve_project(project, repo_config)?;
            let body = resolve_body(body, no_edit)?;
            let priority = if clear_priority {
                Some(None)
            } else {
                priority.map(Some)
            };
            rdm_core::ops::roadmap::update_roadmap(
                store,
                &project,
                &slug,
                body.as_deref(),
                priority,
            )
            .context("failed to update roadmap")?;
            println!("Updated '{slug}'");
            maybe_regenerate_index(store, no_index, staging, Some(&project))?;
        }
        RoadmapCommand::List {
            project,
            archived,
            sort,
            priority,
        } => {
            let project = paths::resolve_project(project, repo_config)?;
            if archived && (sort.is_some() || priority.is_some()) {
                bail!("--sort and --priority are not supported with --archived");
            }
            let entries = if archived {
                let roadmaps = rdm_core::ops::roadmap::list_archived_roadmaps(store, &project)
                    .context("failed to list archived roadmaps")?;
                let mut entries = Vec::new();
                for roadmap_doc in roadmaps {
                    let slug = &roadmap_doc.frontmatter.roadmap;
                    let phases =
                        rdm_core::ops::roadmap::list_archived_phases(store, &project, slug)
                            .with_context(|| {
                                format!("failed to list phases for archived roadmap '{slug}'")
                            })?;
                    entries.push((roadmap_doc, phases));
                }
                entries
            } else {
                let roadmaps =
                    rdm_core::ops::roadmap::list_roadmaps(store, &project, sort, priority)
                        .context("failed to list roadmaps")?;
                let mut entries = Vec::new();
                for roadmap_doc in roadmaps {
                    let slug = &roadmap_doc.frontmatter.roadmap;
                    let phases = rdm_core::ops::phase::list_phases(store, &project, slug)
                        .with_context(|| format!("failed to list phases for roadmap '{slug}'"))?;
                    entries.push((roadmap_doc, phases));
                }
                entries
            };
            match format {
                OutputFormat::Human => print!("{}", display::format_roadmap_list(&entries)),
                OutputFormat::Table => print!("{}", table::format_roadmap_table(&entries)),
                OutputFormat::Markdown => {
                    print!("{}", display::format_roadmap_list_md(&entries))
                }
                OutputFormat::Json => {
                    let summaries: Vec<_> = entries
                        .iter()
                        .map(|(doc, phases)| json::roadmap_summary_to_json(doc, phases))
                        .collect();
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&summaries)
                            .context("failed to serialize roadmaps")?
                    );
                }
            }
            maybe_print_uncommitted_hint(store, staging);
        }
        RoadmapCommand::Depend { slug, on, project } => {
            let project = paths::resolve_project(project, repo_config)?;
            rdm_core::ops::roadmap::add_dependency(store, &project, &slug, &on)
                .context("failed to add dependency")?;
            println!("Added dependency: {slug} → {on}");
            maybe_regenerate_index(store, no_index, staging, Some(&project))?;
        }
        RoadmapCommand::Undepend { slug, on, project } => {
            let project = paths::resolve_project(project, repo_config)?;
            rdm_core::ops::roadmap::remove_dependency(store, &project, &slug, &on)
                .context("failed to remove dependency")?;
            println!("Removed dependency: {slug} → {on}");
            maybe_regenerate_index(store, no_index, staging, Some(&project))?;
        }
        RoadmapCommand::Deps { project } => {
            reject_non_human(format, "roadmap deps")?;
            let project = paths::resolve_project(project, repo_config)?;
            let graph = rdm_core::ops::roadmap::dependency_graph(store, &project)
                .context("failed to get dependency graph")?;
            print!("{}", display::format_dependency_graph(&graph));
            maybe_print_uncommitted_hint(store, staging);
        }
        RoadmapCommand::Delete {
            slug,
            project,
            force,
        } => {
            if !force {
                bail!(
                    "deleting a roadmap is irreversible — pass --force to confirm deletion of '{slug}'"
                );
            }
            let project = paths::resolve_project(project, repo_config)?;
            rdm_core::ops::roadmap::delete_roadmap(store, &project, &slug)
                .context("failed to delete roadmap")?;
            println!("Deleted roadmap '{slug}' from project '{project}'");
            maybe_regenerate_index(store, no_index, staging, Some(&project))?;
        }
        RoadmapCommand::Split {
            slug,
            phases,
            into,
            title,
            project,
            depends_on,
        } => {
            let project = paths::resolve_project(project, repo_config)?;
            // Resolve each phase identifier (number or stem)
            let resolved_stems: Vec<String> = phases
                .iter()
                .map(|p| rdm_core::ops::phase::resolve_phase_stem(store, &project, &slug, p))
                .collect::<std::result::Result<Vec<_>, _>>()
                .context("failed to resolve phase identifiers")?;
            let dep = if depends_on {
                Some(slug.as_str())
            } else {
                None
            };
            rdm_core::ops::roadmap::split_roadmap(
                store,
                &project,
                &slug,
                &into,
                &title,
                &resolved_stems,
                dep,
            )
            .context("failed to split roadmap")?;
            println!(
                "Split {} phase(s) from '{slug}' into new roadmap '{into}'",
                resolved_stems.len()
            );
            maybe_regenerate_index(store, no_index, staging, Some(&project))?;
        }
        RoadmapCommand::Archive {
            slug,
            project,
            force,
        } => {
            let project = paths::resolve_project(project, repo_config)?;
            rdm_core::ops::roadmap::archive_roadmap(store, &project, &slug, force)
                .context("failed to archive roadmap")?;
            println!("Archived roadmap '{slug}' from project '{project}'");
            maybe_regenerate_index(store, no_index, staging, Some(&project))?;
        }
        RoadmapCommand::Unarchive { slug, project } => {
            let project = paths::resolve_project(project, repo_config)?;
            rdm_core::ops::roadmap::unarchive_roadmap(store, &project, &slug)
                .context("failed to unarchive roadmap")?;
            println!("Restored roadmap '{slug}' to project '{project}'");
            maybe_regenerate_index(store, no_index, staging, Some(&project))?;
        }
    }
    Ok(())
}

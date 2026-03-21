use anyhow::{Context, Result, bail};
use rdm_core::config::Config;
use rdm_core::display;
use rdm_core::json;
use rdm_core::repo::PlanRepo;

use super::{maybe_print_uncommitted_hint, maybe_regenerate_index, reject_non_human, resolve_body};
use crate::paths;
use crate::table;
use crate::{AppStore, OutputFormat, RoadmapCommand};

pub fn run(
    command: RoadmapCommand,
    repo: &mut PlanRepo<AppStore>,
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
            body,
            no_edit,
        } => {
            let project = paths::resolve_project(project, repo_config)?;
            let title = title.as_deref().unwrap_or(&slug);
            let body = resolve_body(body, no_edit)?;
            repo.create_roadmap(&project, &slug, title, body.as_deref())
                .context("failed to create roadmap")?;
            println!("Created roadmap '{slug}' in project '{project}'");
            maybe_regenerate_index(repo, no_index, staging, Some(&project))?;
        }
        RoadmapCommand::Show {
            slug,
            project,
            no_body,
        } => {
            let project = paths::resolve_project(project, repo_config)?;
            let mut roadmap_doc = repo
                .load_roadmap(&project, &slug)
                .context("failed to load roadmap")?;
            let phases = repo
                .list_phases(&project, &slug)
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
            maybe_print_uncommitted_hint(repo.store(), staging);
        }
        RoadmapCommand::List { project, archived } => {
            let project = paths::resolve_project(project, repo_config)?;
            let entries = if archived {
                let roadmaps = repo
                    .list_archived_roadmaps(&project)
                    .context("failed to list archived roadmaps")?;
                let mut entries = Vec::new();
                for roadmap_doc in roadmaps {
                    let slug = &roadmap_doc.frontmatter.roadmap;
                    let phases = repo.list_archived_phases(&project, slug).with_context(|| {
                        format!("failed to list phases for archived roadmap '{slug}'")
                    })?;
                    entries.push((roadmap_doc, phases));
                }
                entries
            } else {
                let roadmaps = repo
                    .list_roadmaps(&project)
                    .context("failed to list roadmaps")?;
                let mut entries = Vec::new();
                for roadmap_doc in roadmaps {
                    let slug = &roadmap_doc.frontmatter.roadmap;
                    let phases = repo
                        .list_phases(&project, slug)
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
            maybe_print_uncommitted_hint(repo.store(), staging);
        }
        RoadmapCommand::Depend { slug, on, project } => {
            let project = paths::resolve_project(project, repo_config)?;
            repo.add_dependency(&project, &slug, &on)
                .context("failed to add dependency")?;
            println!("Added dependency: {slug} → {on}");
            maybe_regenerate_index(repo, no_index, staging, Some(&project))?;
        }
        RoadmapCommand::Undepend { slug, on, project } => {
            let project = paths::resolve_project(project, repo_config)?;
            repo.remove_dependency(&project, &slug, &on)
                .context("failed to remove dependency")?;
            println!("Removed dependency: {slug} → {on}");
            maybe_regenerate_index(repo, no_index, staging, Some(&project))?;
        }
        RoadmapCommand::Deps { project } => {
            reject_non_human(format, "roadmap deps")?;
            let project = paths::resolve_project(project, repo_config)?;
            let graph = repo
                .dependency_graph(&project)
                .context("failed to get dependency graph")?;
            print!("{}", display::format_dependency_graph(&graph));
            maybe_print_uncommitted_hint(repo.store(), staging);
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
            repo.delete_roadmap(&project, &slug)
                .context("failed to delete roadmap")?;
            println!("Deleted roadmap '{slug}' from project '{project}'");
            maybe_regenerate_index(repo, no_index, staging, Some(&project))?;
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
                .map(|p| repo.resolve_phase_stem(&project, &slug, p))
                .collect::<std::result::Result<Vec<_>, _>>()
                .context("failed to resolve phase identifiers")?;
            let dep = if depends_on {
                Some(slug.as_str())
            } else {
                None
            };
            repo.split_roadmap(&project, &slug, &into, &title, &resolved_stems, dep)
                .context("failed to split roadmap")?;
            println!(
                "Split {} phase(s) from '{slug}' into new roadmap '{into}'",
                resolved_stems.len()
            );
            maybe_regenerate_index(repo, no_index, staging, Some(&project))?;
        }
        RoadmapCommand::Archive {
            slug,
            project,
            force,
        } => {
            let project = paths::resolve_project(project, repo_config)?;
            repo.archive_roadmap(&project, &slug, force)
                .context("failed to archive roadmap")?;
            println!("Archived roadmap '{slug}' from project '{project}'");
            maybe_regenerate_index(repo, no_index, staging, Some(&project))?;
        }
        RoadmapCommand::Unarchive { slug, project } => {
            let project = paths::resolve_project(project, repo_config)?;
            repo.unarchive_roadmap(&project, &slug)
                .context("failed to unarchive roadmap")?;
            println!("Restored roadmap '{slug}' to project '{project}'");
            maybe_regenerate_index(repo, no_index, staging, Some(&project))?;
        }
    }
    Ok(())
}

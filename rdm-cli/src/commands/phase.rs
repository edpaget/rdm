use anyhow::{Context, Result, bail};
use rdm_core::config::Config;
use rdm_core::display;
use rdm_core::json;
use rdm_core::model::PhaseStatus;

use super::{maybe_print_uncommitted_hint, maybe_regenerate_index, resolve_body};
use crate::paths;
use crate::table;
use crate::{AppStore, OutputFormat, PhaseCommand};

pub fn run(
    command: PhaseCommand,
    store: &mut AppStore,
    repo_config: &Config,
    format: OutputFormat,
    no_index: bool,
    staging: bool,
) -> Result<()> {
    match command {
        PhaseCommand::Create {
            slug,
            title,
            roadmap,
            project,
            number,
            body,
            no_edit,
        } => {
            let project = paths::resolve_project(project, repo_config)?;
            let title = title.as_deref().unwrap_or(&slug);
            let body = resolve_body(body, no_edit)?;
            let doc = rdm_core::ops::phase::create_phase(
                store,
                &project,
                &roadmap,
                &slug,
                title,
                number,
                body.as_deref(),
            )
            .context("failed to create phase")?;
            let stem = doc.frontmatter.stem(&slug);
            println!("Created phase '{stem}' in roadmap '{roadmap}'");
            maybe_regenerate_index(store, no_index, staging, Some(&project))?;
        }
        PhaseCommand::List { roadmap, project } => {
            let project = paths::resolve_project(project, repo_config)?;
            let phases = rdm_core::ops::phase::list_phases(store, &project, &roadmap)
                .context("failed to list phases")?;
            match format {
                OutputFormat::Human => print!("{}", display::format_phase_list(&phases)),
                OutputFormat::Table => print!("{}", table::format_phase_table(&phases)),
                OutputFormat::Markdown => {
                    print!("{}", display::format_phase_list_md(&phases))
                }
                OutputFormat::Json => {
                    let summaries: Vec<_> = phases
                        .iter()
                        .map(|(stem, doc)| json::phase_summary_to_json(stem, doc))
                        .collect();
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&summaries)
                            .context("failed to serialize phases")?
                    );
                }
            }
            maybe_print_uncommitted_hint(store, staging);
        }
        PhaseCommand::Show {
            stem,
            roadmap,
            project,
            no_body,
        } => {
            let project = paths::resolve_project(project, repo_config)?;
            let stem = rdm_core::ops::phase::resolve_phase_stem(store, &project, &roadmap, &stem)
                .context("failed to resolve phase")?;
            let mut doc = rdm_core::io::load_phase(store, &project, &roadmap, &stem)
                .context("failed to load phase")?;
            if no_body {
                doc.body = String::new();
            }

            // Compute prev/next phase stems for navigation
            let phases = rdm_core::ops::phase::list_phases(store, &project, &roadmap)
                .context("failed to list phases")?;
            let pos = phases.iter().position(|(s, _)| s == &stem);
            let prev_stem = pos.and_then(|i| {
                if i > 0 {
                    Some(phases[i - 1].0.as_str())
                } else {
                    None
                }
            });
            let next_stem = pos.and_then(|i| phases.get(i + 1).map(|(s, _)| s.as_str()));

            let nav = display::PhaseNav {
                prev: prev_stem,
                next: next_stem,
                roadmap: &roadmap,
                project: &project,
            };

            match format {
                OutputFormat::Human => {
                    print!("{}", display::format_phase_detail(&stem, &doc, Some(&nav)))
                }
                OutputFormat::Markdown => {
                    print!(
                        "{}",
                        display::format_phase_detail_md(&stem, &doc, Some(&nav))
                    )
                }
                OutputFormat::Json => {
                    let j = json::phase_to_json(&stem, &doc, &roadmap, prev_stem, next_stem);
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&j).context("failed to serialize phase")?
                    );
                }
                OutputFormat::Table => bail!(
                    "--format table is not supported for 'phase show'; use --format human, --format json, --format markdown, or omit --format"
                ),
            }
            maybe_print_uncommitted_hint(store, staging);
        }
        PhaseCommand::Update {
            stem,
            status,
            roadmap,
            project,
            body,
            commit,
            no_edit,
        } => {
            if commit.is_some() && status != Some(PhaseStatus::Done) {
                anyhow::bail!("--commit can only be used with --status done");
            }
            let project = paths::resolve_project(project, repo_config)?;
            let stem = rdm_core::ops::phase::resolve_phase_stem(store, &project, &roadmap, &stem)
                .context("failed to resolve phase")?;
            let body = resolve_body(body, no_edit)?;
            let doc = rdm_core::ops::phase::update_phase(
                store,
                &project,
                &roadmap,
                &stem,
                status,
                body.as_deref(),
                commit,
            )
            .context("failed to update phase")?;
            println!("Updated '{stem}' → {}", doc.frontmatter.status);
            maybe_regenerate_index(store, no_index, staging, Some(&project))?;
        }
        PhaseCommand::Remove {
            stem,
            roadmap,
            project,
        } => {
            let project = paths::resolve_project(project, repo_config)?;
            let stem = rdm_core::ops::phase::resolve_phase_stem(store, &project, &roadmap, &stem)
                .context("failed to resolve phase")?;
            rdm_core::ops::phase::remove_phase(store, &project, &roadmap, &stem)
                .context("failed to remove phase")?;
            println!("Removed phase '{stem}' from roadmap '{roadmap}'");
            maybe_regenerate_index(store, no_index, staging, Some(&project))?;
        }
    }
    Ok(())
}

use anyhow::{Context, Result, bail};
use rdm_core::json;

use super::{commit_mutation, maybe_print_uncommitted_hint, reject_non_human};
use crate::{AppStore, OutputFormat, ProjectCommand};

pub fn run(command: ProjectCommand, store: &mut AppStore, format: OutputFormat) -> Result<()> {
    match command {
        ProjectCommand::Create { name, title } => {
            let title = title.as_deref().unwrap_or(&name);
            let doc = commit_mutation(store, "failed to create project", |s| {
                rdm_core::ops::project::create_project(s, &name, title)
            })?;
            println!("Created project '{}'", doc.frontmatter.name);
        }
        ProjectCommand::Update {
            name,
            source_repo,
            source_branch,
            clear_source,
        } => {
            if !clear_source && source_repo.is_none() && source_branch.is_none() {
                bail!(
                    "nothing to update for project '{name}'; pass --source-repo <locator>, --source-branch <branch>, or --clear-source"
                );
            }
            let existing = rdm_core::io::load_project(store, &name)
                .with_context(|| format!("failed to load project '{name}'"))?
                .frontmatter
                .source;
            let source = if clear_source {
                None
            } else {
                let repo = match (source_repo, existing.as_ref()) {
                    (Some(repo), _) => repo,
                    (None, Some(current)) => current.repo.clone(),
                    (None, None) => bail!(
                        "project '{name}' has no source repository configured; pass --source-repo <locator> alongside --source-branch"
                    ),
                };
                let default_branch = match source_branch {
                    Some(branch) => Some(branch),
                    None => existing.as_ref().and_then(|c| c.default_branch.clone()),
                };
                Some(rdm_core::model::Source {
                    repo,
                    default_branch,
                })
            };
            let doc = commit_mutation(store, "failed to update project", |s| {
                rdm_core::ops::project::update_project_source(s, &name, source.clone())
            })?;
            match format {
                OutputFormat::Json => {
                    let j = json::project_to_json(&doc);
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&j).context("failed to serialize project")?
                    );
                }
                _ => match &doc.frontmatter.source {
                    Some(source) => {
                        println!(
                            "Project '{}' source: {}{}",
                            doc.frontmatter.name,
                            source.repo,
                            source
                                .default_branch
                                .as_ref()
                                .map(|b| format!(" (branch: {b})"))
                                .unwrap_or_default()
                        );
                    }
                    None => println!("Project '{}' source cleared", doc.frontmatter.name),
                },
            }
        }
        ProjectCommand::Show { name } => {
            let doc = rdm_core::io::load_project(store, &name).context("failed to load project")?;
            match format {
                OutputFormat::Json => {
                    let j = json::project_to_json(&doc);
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&j).context("failed to serialize project")?
                    );
                }
                OutputFormat::Markdown => {
                    println!("# {}", doc.frontmatter.title);
                    println!();
                    println!("- **Name:** {}", doc.frontmatter.name);
                    if !doc.body.is_empty() {
                        println!();
                        println!("{}", doc.body);
                    }
                }
                OutputFormat::Table => bail!(
                    "--format table is not supported for 'project show'; use --format human, --format json, --format markdown, or omit --format"
                ),
                OutputFormat::Human => {
                    println!("{} ({})", doc.frontmatter.title, doc.frontmatter.name);
                    if !doc.body.is_empty() {
                        println!();
                        println!("{}", doc.body);
                    }
                }
            }
            maybe_print_uncommitted_hint(store);
        }
        ProjectCommand::List => {
            let projects =
                rdm_core::ops::project::list_projects(store).context("failed to list projects")?;
            match format {
                OutputFormat::Json => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&projects)
                            .context("failed to serialize projects")?
                    );
                }
                _ => {
                    reject_non_human(format, "project list")?;
                    if projects.is_empty() {
                        println!("No projects yet.");
                    } else {
                        for p in &projects {
                            println!("{p}");
                        }
                    }
                }
            }
            maybe_print_uncommitted_hint(store);
        }
    }
    Ok(())
}

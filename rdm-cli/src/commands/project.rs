use anyhow::{Context, Result, bail};
use rdm_core::json;

use super::{maybe_print_uncommitted_hint, maybe_regenerate_index, reject_non_human};
use crate::{AppStore, OutputFormat, ProjectCommand};

pub fn run(
    command: ProjectCommand,
    store: &mut AppStore,
    format: OutputFormat,
    no_index: bool,
    staging: bool,
) -> Result<()> {
    match command {
        ProjectCommand::Create { name, title } => {
            let title = title.as_deref().unwrap_or(&name);
            let doc = rdm_core::ops::project::create_project(store, &name, title)
                .context("failed to create project")?;
            println!("Created project '{}'", doc.frontmatter.name);
            maybe_regenerate_index(store, no_index, staging, Some(&name))?;
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
            maybe_print_uncommitted_hint(store, staging);
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
            maybe_print_uncommitted_hint(store, staging);
        }
    }
    Ok(())
}

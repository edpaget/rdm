use anyhow::{Context, Result, bail};
use rdm_core::config::Config;
use rdm_core::display;
use rdm_core::json;
use rdm_core::model::{TaskStatus, TaskStatusFilter};
use rdm_core::repo::PlanRepo;

use super::{maybe_print_uncommitted_hint, maybe_regenerate_index, resolve_body};
use crate::paths;
use crate::table;
use crate::{AppStore, OutputFormat, TaskCommand};

pub fn run(
    command: TaskCommand,
    repo: &mut PlanRepo<AppStore>,
    repo_config: &Config,
    format: OutputFormat,
    no_index: bool,
    staging: bool,
) -> Result<()> {
    match command {
        TaskCommand::Create {
            slug,
            title,
            project,
            priority,
            tags,
            body,
            no_edit,
        } => {
            let project = paths::resolve_project(project, repo_config)?;
            let title = title.as_deref().unwrap_or(&slug);
            let body = resolve_body(body, no_edit)?;
            repo.create_task(&project, &slug, title, priority, tags, body.as_deref())
                .context("failed to create task")?;
            println!("Created task '{slug}' in project '{project}'");
            maybe_regenerate_index(repo, no_index, staging, Some(&project))?;
        }
        TaskCommand::Show {
            slug,
            project,
            no_body,
        } => {
            let project = paths::resolve_project(project, repo_config)?;
            let mut doc = repo
                .load_task(&project, &slug)
                .context("failed to load task")?;
            if no_body {
                doc.body = String::new();
            }
            match format {
                OutputFormat::Human => {
                    print!("{}", display::format_task_detail(&slug, &doc))
                }
                OutputFormat::Markdown => {
                    print!("{}", display::format_task_detail_md(&slug, &doc))
                }
                OutputFormat::Json => {
                    let j = json::task_to_json(&slug, &doc);
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&j).context("failed to serialize task")?
                    );
                }
                OutputFormat::Table => bail!(
                    "--format table is not supported for 'task show'; use --format human, --format json, --format markdown, or omit --format"
                ),
            }
            maybe_print_uncommitted_hint(repo.store(), staging);
        }
        TaskCommand::Update {
            slug,
            project,
            status,
            priority,
            tags,
            body,
            commit,
            no_edit,
        } => {
            let project = paths::resolve_project(project, repo_config)?;
            let body = resolve_body(body, no_edit)?;
            let doc = repo
                .update_task(
                    &project,
                    &slug,
                    status,
                    priority,
                    tags,
                    body.as_deref(),
                    commit,
                )
                .context("failed to update task")?;
            println!(
                "Updated task '{slug}' → status: {}, priority: {}",
                doc.frontmatter.status, doc.frontmatter.priority
            );
            maybe_regenerate_index(repo, no_index, staging, Some(&project))?;
        }
        TaskCommand::List {
            project,
            status,
            priority,
            tag,
        } => {
            let project = paths::resolve_project(project, repo_config)?;
            let all_tasks = repo.list_tasks(&project).context("failed to list tasks")?;

            let filtered: Vec<(String, _)> = all_tasks
                .into_iter()
                .filter(|(_, doc)| match status {
                    Some(TaskStatusFilter::All) => true,
                    Some(TaskStatusFilter::Status(s)) => doc.frontmatter.status == s,
                    None => {
                        doc.frontmatter.status == TaskStatus::Open
                            || doc.frontmatter.status == TaskStatus::InProgress
                    }
                })
                .filter(|(_, doc)| priority.is_none_or(|p| doc.frontmatter.priority == p))
                .filter(|(_, doc)| {
                    tag.as_ref().is_none_or(|t| {
                        doc.frontmatter
                            .tags
                            .as_ref()
                            .is_some_and(|tags| tags.contains(t))
                    })
                })
                .collect();

            match format {
                OutputFormat::Human => print!("{}", display::format_task_list(&filtered)),
                OutputFormat::Table => print!("{}", table::format_task_table(&filtered)),
                OutputFormat::Markdown => {
                    print!("{}", display::format_task_list_md(&filtered))
                }
                OutputFormat::Json => {
                    let summaries: Vec<_> = filtered
                        .iter()
                        .map(|(slug, doc)| json::task_summary_to_json(slug, doc))
                        .collect();
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&summaries)
                            .context("failed to serialize tasks")?
                    );
                }
            }
            maybe_print_uncommitted_hint(repo.store(), staging);
        }
    }
    Ok(())
}

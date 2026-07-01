use anyhow::{Context, Result, bail};
use rdm_core::config::Config;
use rdm_core::display;
use rdm_core::json;
use rdm_core::ops::{BodyUpdate, TagsUpdate, TitleUpdate};

use super::{commit_mutation, map_body_clobber, maybe_print_uncommitted_hint, resolve_body};
use crate::paths;
use crate::table;
use crate::{AppStore, OutputFormat, TaskCommand};

pub fn run(
    command: TaskCommand,
    store: &mut AppStore,
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
            commit_mutation(
                store,
                &project,
                no_index,
                staging,
                "failed to create task",
                |s| {
                    rdm_core::ops::task::create_task(
                        s,
                        rdm_core::ops::task::CreateTask {
                            project: &project,
                            slug: &slug,
                            title,
                            priority,
                            tags,
                            body: body.as_deref(),
                        },
                    )
                },
            )?;
            println!("Created task '{slug}' in project '{project}'");
        }
        TaskCommand::Show {
            slug,
            project,
            no_body,
            at,
        } => {
            let project = paths::resolve_project(project, repo_config)?;
            let mut doc = match at.as_deref() {
                Some(sha) => rdm_core::io::load_task_at(store, &project, &slug, sha)
                    .with_context(|| format!("failed to load task '{slug}' at revision '{sha}'"))?,
                None => rdm_core::io::load_task(store, &project, &slug)
                    .context("failed to load task")?,
            };
            if no_body {
                doc.body = String::new();
            }
            let revision = at.as_deref();
            match format {
                OutputFormat::Human => {
                    print!("{}", display::format_task_detail(&slug, &doc, revision))
                }
                OutputFormat::Markdown => {
                    print!("{}", display::format_task_detail_md(&slug, &doc, revision))
                }
                OutputFormat::Json => {
                    let j = json::task_to_json(&slug, &doc, revision);
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&j).context("failed to serialize task")?
                    );
                }
                OutputFormat::Table => bail!(
                    "--format table is not supported for 'task show'; use --format human, --format json, --format markdown, or omit --format"
                ),
            }
            maybe_print_uncommitted_hint(store, staging);
        }
        TaskCommand::Update {
            slug,
            project,
            title,
            status,
            priority,
            tags,
            body,
            clear_body,
            commit,
            no_edit,
        } => {
            let project = paths::resolve_project(project, repo_config)?;
            let title = TitleUpdate::from_args(title);
            let body = if clear_body {
                BodyUpdate::Clear
            } else {
                BodyUpdate::from_args(resolve_body(body, no_edit)?, false)?
            };
            let tags = TagsUpdate::from_args(tags, false)?;
            // Stamp the source-repo HEAD SHA when entering needs-review, so the
            // review can later be scoped to the branch/worktree that produced
            // it. No commit yet (unstamped) → fail open downstream.
            #[cfg(feature = "git")]
            let review_sha = if status == Some(rdm_core::model::TaskStatus::NeedsReview) {
                rdm_git::head_commit_info_at(&std::env::current_dir()?)
                    .ok()
                    .flatten()
                    .map(|c| c.sha)
            } else {
                None
            };
            #[cfg(not(feature = "git"))]
            let review_sha = None;
            // Also stamp the firing checkout's branch so `review pending` can
            // scope by identity rather than by SHA reachability alone.
            #[cfg(feature = "git")]
            let review_branch = if status == Some(rdm_core::model::TaskStatus::NeedsReview) {
                rdm_git::current_branch_at(&std::env::current_dir()?)
                    .ok()
                    .flatten()
            } else {
                None
            };
            #[cfg(not(feature = "git"))]
            let review_branch = None;
            // Data-integrity guard: warn (non-blocking) when a task reaches
            // needs-review with no committed diff beyond the default branch.
            // Tasks have no sibling phases, so they always use this baseline.
            #[cfg(feature = "git")]
            let needs_review_warning: Option<String> = review_sha.as_deref().and_then(|sha| {
                let default_branch = repo_config.default_branch.as_deref().unwrap_or("main");
                if review_branch.as_deref() == Some(default_branch) {
                    return None;
                }
                let cwd = std::env::current_dir().ok()?;
                match rdm_git::is_ancestor_of_branch_at(&cwd, default_branch, sha) {
                    Ok(true) => Some(format!(
                        "warning: task '{slug}' is now needs-review, but HEAD has no commits beyond '{default_branch}' — there may be nothing to review. Confirm this task's work was committed before finalizing."
                    )),
                    _ => None,
                }
            });
            #[cfg(not(feature = "git"))]
            let needs_review_warning: Option<String> = None;
            let doc = commit_mutation(
                store,
                &project,
                no_index,
                staging,
                "failed to update task",
                |s| {
                    rdm_core::ops::task::update_task(
                        s,
                        &project,
                        &slug,
                        status,
                        priority,
                        tags,
                        body,
                        commit,
                        review_sha,
                        review_branch,
                        title,
                    )
                },
            )
            .map_err(map_body_clobber)?;
            println!(
                "Updated task '{slug}' → status: {}, priority: {}",
                doc.frontmatter.status, doc.frontmatter.priority
            );
            if let Some(warning) = needs_review_warning {
                eprintln!("{warning}");
            }
        }
        TaskCommand::List {
            project,
            status,
            priority,
            tag,
        } => {
            let project = paths::resolve_project(project, repo_config)?;
            let all_tasks =
                rdm_core::ops::task::list_tasks(store, &project).context("failed to list tasks")?;

            let filtered = rdm_core::ops::task::filter_tasks(
                all_tasks,
                &rdm_core::ops::task::TaskFilter {
                    status,
                    priority,
                    tags: tag.into_iter().collect(),
                },
            );

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
            maybe_print_uncommitted_hint(store, staging);
        }
    }
    Ok(())
}

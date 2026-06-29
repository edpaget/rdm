use std::path::Path;

use anyhow::{Context, Result};
use rdm_git::worktree::{self, RemoveOptions, WorktreeError, WorktreeInfo};

use crate::OutputFormat;
use crate::WorktreeCommand;
use crate::commands;
use crate::paths;
use crate::table;

/// Dispatches a `rdm worktree` subcommand.
pub fn run(
    command: WorktreeCommand,
    root: &Path,
    repo_config: &rdm_core::config::Config,
    staging: bool,
    format: OutputFormat,
) -> Result<()> {
    match command {
        WorktreeCommand::Add {
            item,
            base,
            project,
        } => add(
            root,
            repo_config,
            staging,
            format,
            &item,
            base.as_deref(),
            project,
        ),
        WorktreeCommand::List => list(format),
        WorktreeCommand::Current => current(format),
        WorktreeCommand::Remove {
            target,
            delete_branch,
            force,
            project,
        } => remove(
            root,
            repo_config,
            staging,
            &target,
            delete_branch,
            force,
            project,
        ),
    }
}

fn add(
    root: &Path,
    repo_config: &rdm_core::config::Config,
    staging: bool,
    format: OutputFormat,
    raw_item: &str,
    base: Option<&str>,
    project: Option<String>,
) -> Result<()> {
    let project = paths::resolve_project(project, repo_config)?;
    let store = commands::make_store(root, staging)?;
    let item = worktree::resolve_item(&store, &project, raw_item).map_err(map_err)?;
    let cwd = std::env::current_dir().context("cannot determine current directory")?;
    let repo = worktree::discover_distinct_project_repo(&cwd, root).map_err(map_err)?;
    let info = worktree::add(&repo, &item, &item.branch_name(), base).map_err(map_err)?;

    match format {
        OutputFormat::Json => {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "item": info.item,
                    "branch": info.branch,
                    "path": info.path.display().to_string(),
                    "created": info.created,
                }))?
            );
        }
        _ => {
            println!("{}", info.path.display());
            if !info.created {
                eprintln!("(existing worktree — nothing to create)");
            }
        }
    }
    Ok(())
}

fn list(format: OutputFormat) -> Result<()> {
    let cwd = std::env::current_dir().context("cannot determine current directory")?;
    let repo = worktree::discover_project_repo(&cwd).map_err(map_err)?;
    let worktrees = worktree::list(&repo).map_err(map_err)?;

    match format {
        OutputFormat::Json => {
            let arr: Vec<_> = worktrees
                .iter()
                .map(|w| {
                    serde_json::json!({
                        "item": w.item,
                        "branch": w.branch,
                        "path": w.path.display().to_string(),
                        "dirty": w.dirty,
                    })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&arr)?);
        }
        OutputFormat::Table => print!("{}", format_table(&worktrees)),
        _ => {
            if worktrees.is_empty() {
                println!("No rdm worktrees.");
            } else {
                for w in &worktrees {
                    let dirty = if w.dirty { " (dirty)" } else { "" };
                    println!("{}  {}  {}{}", w.item, w.branch, w.path.display(), dirty);
                }
            }
        }
    }
    Ok(())
}

fn current(format: OutputFormat) -> Result<()> {
    let cwd = std::env::current_dir().context("cannot determine current directory")?;
    let current = worktree::current(&cwd).map_err(map_err)?;

    match format {
        OutputFormat::Json => {
            let json = match &current {
                Some(c) => serde_json::json!({
                    "item": c.item,
                    "branch": c.branch,
                    "path": c.path.display().to_string(),
                    "rdm_managed": c.rdm_managed,
                }),
                None => serde_json::Value::Null,
            };
            println!("{}", serde_json::to_string_pretty(&json)?);
        }
        _ => match &current {
            Some(c) => {
                let inferred = if c.rdm_managed {
                    ""
                } else {
                    "  (inferred from branch)"
                };
                println!("{}  {}  {}{}", c.item, c.branch, c.path.display(), inferred);
            }
            None => println!("Not in an rdm worktree."),
        },
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn remove(
    root: &Path,
    repo_config: &rdm_core::config::Config,
    staging: bool,
    target: &str,
    delete_branch: bool,
    force: bool,
    project: Option<String>,
) -> Result<()> {
    let project = paths::resolve_project(project, repo_config)?;
    let store = commands::make_store(root, staging)?;
    let resolved = worktree::resolve_target(&store, &project, target);
    let cwd = std::env::current_dir().context("cannot determine current directory")?;
    let repo = worktree::discover_project_repo(&cwd).map_err(map_err)?;
    worktree::remove(
        &repo,
        &resolved,
        RemoveOptions {
            force,
            delete_branch,
        },
    )
    .map_err(map_err)?;
    println!("Removed worktree for {target}");
    Ok(())
}

fn format_table(worktrees: &[WorktreeInfo]) -> String {
    if worktrees.is_empty() {
        return "No rdm worktrees.\n".to_string();
    }
    let rows = worktrees
        .iter()
        .map(|w| {
            [
                w.item.clone(),
                w.branch.clone(),
                w.path.display().to_string(),
                if w.dirty { "yes".into() } else { "no".into() },
            ]
        })
        .collect();
    table::build_table(["Item", "Branch", "Path", "Dirty"], rows)
}

/// Maps a [`WorktreeError`] to an anyhow error, preserving its actionable
/// `Display` message.
fn map_err(err: WorktreeError) -> anyhow::Error {
    anyhow::anyhow!("{err}")
}

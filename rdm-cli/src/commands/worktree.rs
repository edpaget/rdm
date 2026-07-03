use std::path::Path;

use anyhow::{Context, Result};
use rdm_git::worktree::{
    self, PruneAction, PruneOptions, RemoveOptions, WorktreeError, WorktreeInfo,
};

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
    format: OutputFormat,
) -> Result<()> {
    match command {
        WorktreeCommand::Add {
            item,
            base,
            project,
        } => add(root, repo_config, format, &item, base.as_deref(), project),
        WorktreeCommand::List => list(format),
        WorktreeCommand::Current => current(format),
        WorktreeCommand::Remove {
            target,
            delete_branch,
            force,
            project,
        } => remove(root, repo_config, &target, delete_branch, force, project),
        WorktreeCommand::Prune {
            project,
            delete_branch,
            force,
            dry_run,
        } => prune(
            root,
            repo_config,
            format,
            delete_branch,
            force,
            dry_run,
            project,
        ),
    }
}

fn add(
    root: &Path,
    repo_config: &rdm_core::config::Config,
    format: OutputFormat,
    raw_item: &str,
    base: Option<&str>,
    project: Option<String>,
) -> Result<()> {
    let project = paths::resolve_project(project, repo_config)?;
    let store = commands::make_store(root)?;
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
    target: &str,
    delete_branch: bool,
    force: bool,
    project: Option<String>,
) -> Result<()> {
    let project = paths::resolve_project(project, repo_config)?;
    let store = commands::make_store(root)?;
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

#[allow(clippy::too_many_arguments)]
fn prune(
    root: &Path,
    repo_config: &rdm_core::config::Config,
    format: OutputFormat,
    delete_branch: bool,
    force: bool,
    dry_run: bool,
    project: Option<String>,
) -> Result<()> {
    let project = paths::resolve_project(project, repo_config)?;
    let store = commands::make_store(root)?;
    let cwd = std::env::current_dir().context("cannot determine current directory")?;
    let repo = worktree::discover_project_repo(&cwd).map_err(map_err)?;
    let results = worktree::prune(
        &repo,
        &store,
        &project,
        PruneOptions {
            delete_branch,
            force,
            dry_run,
        },
    )
    .map_err(map_err)?;

    let removed = results
        .iter()
        .filter(|r| matches!(r.action, PruneAction::Removed))
        .count();
    let would = results
        .iter()
        .filter(|r| matches!(r.action, PruneAction::WouldRemove))
        .count();
    let skipped = results
        .iter()
        .filter(|r| matches!(r.action, PruneAction::SkippedDirty))
        .count();
    let branch_kept = results
        .iter()
        .filter(|r| matches!(r.action, PruneAction::RemovedBranchKept { .. }))
        .count();
    let failed = results
        .iter()
        .filter(|r| matches!(r.action, PruneAction::Failed(_)))
        .count();

    match format {
        OutputFormat::Json => {
            let arr: Vec<_> = results
                .iter()
                .map(|r| {
                    let mut obj = serde_json::json!({
                        "item": r.item,
                        "branch": r.branch,
                        "path": r.path.display().to_string(),
                        "action": action_str(&r.action),
                    });
                    // Surface the retention reason per-result for the
                    // removed-branch-kept action so JSON is as informative as the
                    // text note; absent for every other action.
                    if let PruneAction::RemovedBranchKept { reason } = &r.action {
                        obj["reason"] = serde_json::Value::String(reason.clone());
                    }
                    obj
                })
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "results": arr,
                    "removed": removed,
                    "would_remove": would,
                    "skipped_dirty": skipped,
                    "branch_kept": branch_kept,
                    "failed": failed,
                }))?
            );
        }
        _ => {
            if results.is_empty() {
                println!("No done worktrees to prune.");
            } else {
                for r in &results {
                    let note = match &r.action {
                        PruneAction::Removed => "removed".to_string(),
                        PruneAction::WouldRemove => "would remove".to_string(),
                        PruneAction::SkippedDirty => "skipped (dirty — pass --force)".to_string(),
                        PruneAction::RemovedBranchKept { reason } => {
                            format!("removed, branch kept ({reason})")
                        }
                        PruneAction::Failed(e) => format!("failed: {e}"),
                    };
                    println!("{}  {}  {}", r.item, r.path.display(), note);
                }
                if dry_run {
                    println!("{would} would be removed, {skipped} skipped (dirty).");
                } else {
                    println!(
                        "{removed} removed, {branch_kept} branch kept, {skipped} skipped (dirty), {failed} failed."
                    );
                }
            }
        }
    }
    Ok(())
}

/// Maps a [`PruneAction`] to its stable JSON discriminator.
fn action_str(action: &PruneAction) -> &'static str {
    match action {
        PruneAction::Removed => "removed",
        PruneAction::WouldRemove => "would-remove",
        PruneAction::SkippedDirty => "skipped-dirty",
        PruneAction::RemovedBranchKept { .. } => "removed-branch-kept",
        PruneAction::Failed(_) => "failed",
    }
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

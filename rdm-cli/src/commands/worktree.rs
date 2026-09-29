use std::path::Path;

use anyhow::{Context, Result};
use rdm_core::json::WorktreeJson;
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
        WorktreeCommand::Current { project } => current(root, repo_config, format, project),
        WorktreeCommand::Remove {
            target,
            delete_branch,
            force,
            project,
        } => remove(
            root,
            repo_config,
            format,
            &target,
            delete_branch,
            force,
            project,
        ),
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

/// Creates (or reuses) a worktree for `raw_item`.
///
/// When `base` is not supplied, the new branch defaults to the invoking
/// checkout's current branch (resolved via [`rdm_git::current_branch_at`])
/// rather than always basing off `HEAD` at the main working tree. This falls
/// back to the prior `HEAD` default when the invoking checkout is in
/// detached HEAD, or when its branch is the item's own target branch (the
/// self/idempotent case). An explicit `base` always takes precedence.
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
    // Default the base to the invoking checkout's current branch rather than
    // always resolving HEAD at the main working tree. `.ok().flatten()`
    // degrades to the prior `HEAD` default on any error (never a git repo,
    // etc.) or detached HEAD; the first `.filter` drops the self-referential
    // case where the invoking branch is already the item's own target branch;
    // the second drops an unborn branch (symbolic-ref reports a branch name
    // before its first commit, but that name is not a usable base ref).
    let resolved_base: Option<String> = match base {
        Some(explicit) => Some(explicit.to_string()),
        None => rdm_git::current_branch_at(&cwd)
            .ok()
            .flatten()
            .filter(|b| b != &item.branch_name())
            .filter(|b| {
                rdm_git::run_git_at(
                    &cwd,
                    &[
                        "rev-parse",
                        "--verify",
                        "--quiet",
                        &format!("refs/heads/{b}"),
                    ],
                )
                .map(|out| out.status.success())
                .unwrap_or(false)
            }),
    };
    let info = worktree::add(&repo, &item, &item.branch_name(), resolved_base.as_deref())
        .map_err(map_err)?;

    match format {
        OutputFormat::Json => {
            let json = WorktreeJson {
                created: Some(info.created),
                ..info_json(&info)
            };
            println!("{}", serde_json::to_string_pretty(&json)?);
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
                .map(|w| WorktreeJson {
                    dirty: Some(w.dirty),
                    ..info_json(w)
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

fn current(
    root: &Path,
    repo_config: &rdm_core::config::Config,
    format: OutputFormat,
    project: Option<String>,
) -> Result<()> {
    let cwd = std::env::current_dir().context("cannot determine current directory")?;
    let current = worktree::current(&cwd).map_err(map_err)?;

    match format {
        OutputFormat::Json => {
            let json = current.as_ref().map(|c| {
                let target = c.item_ref().map(|i| i.as_review_target());
                // Best-effort: no project, no plan repo, or an unreadable
                // item leaves the state out rather than failing the command.
                let state = target.as_ref().and_then(|t| {
                    let project = paths::resolve_project(project, repo_config).ok()?;
                    let store = commands::make_store(root).ok()?;
                    rdm_core::json::item_state_json(&store, &project, t)
                        .ok()
                        .flatten()
                });
                WorktreeJson {
                    rdm_managed: Some(c.rdm_managed),
                    state,
                    ..WorktreeJson::new(&c.item, &c.branch, &c.path, target)
                }
            });
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
    format: OutputFormat,
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
    let removed = worktree::remove(
        &repo,
        &resolved,
        RemoveOptions {
            force,
            delete_branch,
        },
    )
    .map_err(map_err)?;
    match format {
        OutputFormat::Json => {
            let json = WorktreeJson {
                branch_deleted: Some(removed.branch_deleted),
                ..info_json(&removed.info)
            };
            println!("{}", serde_json::to_string_pretty(&json)?);
        }
        _ => println!("Removed worktree for {target}"),
    }
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

/// The common JSON fields of an rdm worktree.
fn info_json(info: &WorktreeInfo) -> WorktreeJson {
    WorktreeJson::new(
        &info.item,
        &info.branch,
        &info.path,
        info.item_ref().map(|i| i.as_review_target()),
    )
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

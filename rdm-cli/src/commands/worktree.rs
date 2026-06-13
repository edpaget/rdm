use std::path::Path;

use anyhow::{Context, Result};
use rdm_store_git::worktree::{self, ItemRef, RemoveOptions, WorktreeError, WorktreeInfo};

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

/// Best-effort canonicalization of a `remove` target. Item references with a
/// numeric phase are resolved to their stem against the plan repo; anything
/// that does not parse/resolve as an item (e.g. a filesystem path, or an item
/// whose phase no longer exists) is returned verbatim so it can still match by
/// path or stored canonical string.
fn canonicalize_target(
    root: &Path,
    repo_config: &rdm_core::config::Config,
    staging: bool,
    project: Option<String>,
    raw: &str,
) -> String {
    let Ok(item) = ItemRef::parse(raw) else {
        return raw.to_string();
    };
    // Only a numeric phase stem needs plan-repo resolution; everything else is
    // already canonical.
    if let ItemRef::Phase { roadmap, stem } = &item
        && stem.parse::<u32>().is_ok()
    {
        if let Ok(project) = paths::resolve_project(project, repo_config)
            && let Ok(store) = commands::make_store(root, staging)
            && let Ok(resolved) =
                rdm_core::ops::phase::resolve_phase_stem(&store, &project, roadmap, stem)
        {
            return format!("{roadmap}/{resolved}");
        }
        return raw.to_string();
    }
    item.canonical()
}

/// Discovers the project (code) repo from CWD and refuses if it is the plan
/// repo — the two must be distinct directories.
fn discover_distinct_project_repo(root: &Path) -> Result<std::path::PathBuf> {
    let cwd = std::env::current_dir().context("cannot determine current directory")?;
    let repo = worktree::discover_project_repo(&cwd).map_err(map_err)?;
    // Compare canonicalized paths so symlinks (e.g. macOS /private) don't mask a
    // match between the project repo and the plan repo.
    let repo_canon = repo.canonicalize().unwrap_or_else(|_| repo.clone());
    let root_canon = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    if repo_canon == root_canon {
        return Err(map_err(WorktreeError::IsPlanRepo(repo)));
    }
    Ok(repo)
}

/// Canonicalizes an item against the plan repo, validating that it exists
/// before any git mutation. Phase numbers are resolved to stems.
fn canonicalize_item(
    root: &Path,
    repo_config: &rdm_core::config::Config,
    staging: bool,
    project: Option<String>,
    raw: &str,
) -> Result<ItemRef> {
    let item = ItemRef::parse(raw).map_err(map_err)?;
    let project = paths::resolve_project(project, repo_config)?;
    let store = commands::make_store(root, staging)?;

    match item {
        ItemRef::Phase { roadmap, stem } => {
            let stem = rdm_core::ops::phase::resolve_phase_stem(&store, &project, &roadmap, &stem)
                .with_context(|| format!("unknown item '{roadmap}/{stem}'"))?;
            // Confirm the phase actually exists (resolve_phase_stem passes
            // non-numeric stems through unverified).
            rdm_core::io::load_phase(&store, &project, &roadmap, &stem).with_context(|| {
                format!("phase '{roadmap}/{stem}' not found — check `rdm phase list`")
            })?;
            Ok(ItemRef::Phase { roadmap, stem })
        }
        ItemRef::Task { slug } => {
            rdm_core::io::load_task(&store, &project, &slug)
                .with_context(|| format!("task '{slug}' not found — check `rdm task list`"))?;
            Ok(ItemRef::Task { slug })
        }
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
    let item = canonicalize_item(root, repo_config, staging, project, raw_item)?;
    let repo = discover_distinct_project_repo(root)?;
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
    let resolved = canonicalize_target(root, repo_config, staging, project, target);
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

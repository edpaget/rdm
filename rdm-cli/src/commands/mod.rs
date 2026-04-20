use std::io::{self, Read, Write};
use std::path::Path;

use anyhow::{Context, Result, bail};
use is_terminal::IsTerminal;
use rdm_core::model::PhaseStatus;
use rdm_core::search::ItemStatus;

use crate::paths;
use crate::{AppStore, ItemKindArg, OutputFormat};

pub mod config;
pub mod phase;
pub mod project;
pub mod roadmap;
pub mod task;

#[cfg(feature = "git")]
pub mod bootstrap;
#[cfg(feature = "git")]
pub mod hook;
#[cfg(feature = "git")]
pub mod remote;

/// Parses a status string into an `ItemStatus`, using the `--type` hint if available.
pub fn parse_status(status: &str, kind: Option<ItemKindArg>) -> Result<ItemStatus> {
    use rdm_core::model::TaskStatus;

    match kind {
        Some(ItemKindArg::Phase) => {
            let s: PhaseStatus = status.parse().map_err(|e: String| anyhow::anyhow!("{e}"))?;
            Ok(ItemStatus::Phase(s))
        }
        Some(ItemKindArg::Task) => {
            let s: TaskStatus = status.parse().map_err(|e: String| anyhow::anyhow!("{e}"))?;
            Ok(ItemStatus::Task(s))
        }
        Some(ItemKindArg::Roadmap) => {
            bail!("roadmaps do not have a status — remove --status or change --type")
        }
        None => {
            // Try both; phase first then task
            if let Ok(s) = status.parse::<PhaseStatus>() {
                Ok(ItemStatus::Phase(s))
            } else if let Ok(s) = status.parse::<TaskStatus>() {
                Ok(ItemStatus::Task(s))
            } else {
                bail!(
                    "invalid status '{status}' — use a phase status (not-started, in-progress, done, blocked) or task status (open, in-progress, done, wont-fix)"
                )
            }
        }
    }
}

/// Opens a store for an existing plan repo.
pub fn make_store(root: &Path, staging: bool) -> Result<AppStore> {
    #[cfg(feature = "git")]
    {
        Ok(rdm_store_git::GitStore::new(root)
            .context("failed to open git repository")?
            .with_staging_mode(staging))
    }
    #[cfg(not(feature = "git"))]
    {
        let _ = staging;
        Ok(rdm_store_fs::FsStore::new(root))
    }
}

/// Creates a store for initializing a new plan repo.
pub fn make_init_store(root: &Path) -> Result<AppStore> {
    #[cfg(feature = "git")]
    {
        rdm_store_git::GitStore::init(root).context("failed to initialize git repository")
    }
    #[cfg(not(feature = "git"))]
    {
        Ok(rdm_store_fs::FsStore::new(root))
    }
}

/// Resolve body content from `--body` flag, piped stdin, or interactive editor.
/// Returns an error if both `--body` and stdin are provided.
pub fn resolve_body(body_flag: Option<String>, no_edit: bool) -> Result<Option<String>> {
    let is_tty = io::stdin().is_terminal();

    let stdin_body = if !is_tty {
        let mut buf = String::new();
        io::stdin().read_to_string(&mut buf)?;
        let trimmed = buf.trim_end_matches('\n');
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    } else {
        None
    };

    match (body_flag, stdin_body) {
        (Some(_), Some(_)) => bail!("cannot use --body and piped stdin together; pick one"),
        (Some(b), None) => Ok(Some(b)),
        (None, Some(s)) => Ok(Some(s)),
        (None, None) => {
            if no_edit || !is_tty {
                Ok(None)
            } else {
                open_editor()
            }
        }
    }
}

/// Launch `$VISUAL` / `$EDITOR` / `vi` to interactively edit body content.
/// Returns `None` if the user saves an empty file.
pub fn open_editor() -> Result<Option<String>> {
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".to_string());

    let mut tmp = tempfile::Builder::new()
        .suffix(".md")
        .tempfile()
        .context("failed to create temp file for editor")?;

    writeln!(
        tmp,
        "<!-- Enter body content below. This comment will be removed. -->"
    )?;
    tmp.flush()?;

    let path = tmp.path().to_owned();

    let status = std::process::Command::new(&editor)
        .arg(&path)
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status()
        .with_context(|| format!("failed to launch editor '{editor}'"))?;

    if !status.success() {
        bail!("editor exited with non-zero status");
    }

    let content = std::fs::read_to_string(&path).context("failed to read editor temp file")?;

    let body: String = content
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            !(trimmed.starts_with("<!--") && trimmed.ends_with("-->"))
        })
        .collect::<Vec<_>>()
        .join("\n");
    let body = body.trim().to_string();

    if body.is_empty() {
        Ok(None)
    } else {
        Ok(Some(body))
    }
}

pub fn reject_non_human(format: OutputFormat, command_name: &str) -> Result<()> {
    if format != OutputFormat::Human {
        bail!(
            "--format {format} is not supported for '{command_name}'; use --format human or omit --format"
        );
    }
    Ok(())
}

pub fn maybe_regenerate_index(
    store: &mut AppStore,
    no_index: bool,
    staging: bool,
    project: Option<&str>,
) -> Result<()> {
    if !no_index {
        match project {
            Some(p) => rdm_core::ops::index::generate_index_for_project(store, p)
                .context("failed to regenerate INDEX.md")?,
            None => rdm_core::ops::index::generate_index(store)
                .context("failed to regenerate INDEX.md")?,
        }
    }
    if staging {
        println!("  (staged — run `rdm commit` to persist)");
    }
    Ok(())
}

/// Prints a hint about uncommitted changes when staging mode is active.
///
/// Called after read-only commands (list, show, search) so the user is aware
/// that the data they see includes uncommitted staged mutations.
#[cfg(feature = "git")]
pub fn maybe_print_uncommitted_hint(store: &AppStore, staging: bool) {
    if !staging {
        return;
    }
    if let Ok(statuses) = store.git_status()
        && !statuses.is_empty()
    {
        println!(
            "\n  ({} uncommitted change(s) — run `rdm status` for details)",
            statuses.len()
        );
    }
}

#[cfg(not(feature = "git"))]
pub fn maybe_print_uncommitted_hint(_store: &AppStore, _staging: bool) {}

/// Applies a list of `Done:` directives, marking matching phases/tasks as done
/// with the associated commit SHA.
///
/// Silently skips directives whose phase or task cannot be found.
#[cfg(feature = "git")]
pub fn apply_done_directives(
    root: &Path,
    staging: bool,
    directives_with_sha: &[(rdm_core::hook::DoneDirective, String)],
) -> Result<()> {
    if directives_with_sha.is_empty() {
        return Ok(());
    }

    let mut store = make_store(root, staging)?;
    let hook_global_config = paths::load_global_config();
    let hook_repo_config = paths::load_repo_config(root).with_global_defaults(&hook_global_config);
    let project = paths::resolve_project(None, &hook_repo_config)?;
    for (directive, sha) in directives_with_sha {
        match directive {
            rdm_core::hook::DoneDirective::Phase { roadmap, phase } => {
                let stem = match rdm_core::ops::phase::resolve_phase_stem(
                    &store, &project, roadmap, phase,
                ) {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                let _ = rdm_core::ops::phase::update_phase(
                    &mut store,
                    &project,
                    roadmap,
                    &stem,
                    Some(rdm_core::model::PhaseStatus::Done),
                    None,
                    Some(sha.clone()),
                );
            }
            rdm_core::hook::DoneDirective::Task { slug } => {
                let _ = rdm_core::ops::task::update_task(
                    &mut store,
                    &project,
                    slug,
                    Some(rdm_core::model::TaskStatus::Done),
                    None,
                    None,
                    None,
                    Some(sha.clone()),
                );
            }
        }
    }
    Ok(())
}

/// Runs the post-merge hook logic: parse `Done:` directives from commits
/// and mark matching phases done.
///
/// When `since` is `None`, scans commits introduced by the most recent merge
/// (using the reflog anchor `HEAD@{1}`). When `since` is `Some(ref)`, scans
/// all commits reachable from HEAD but not from the given ref.
///
/// All errors are intentionally swallowed by the caller — this must never
/// block a git merge.
#[cfg(feature = "git")]
pub fn run_post_merge_hook(root: &Path, staging: bool, since: Option<&str>) -> Result<()> {
    let cwd = std::env::current_dir().context("cannot determine current directory")?;
    let commits = rdm_store_git::commit_messages_since_at(&cwd, since)?;
    if commits.is_empty() {
        return Ok(());
    }

    // Collect directives from all commits. Commits are newest-first, so the
    // first occurrence of a directive wins (latest SHA).
    let mut seen = std::collections::HashSet::new();
    let mut directives_with_sha = Vec::new();
    for commit in &commits {
        for directive in rdm_core::hook::parse_done_directives(&commit.message) {
            if seen.insert(directive.clone()) {
                directives_with_sha.push((directive, commit.sha.clone()));
            }
        }
    }

    apply_done_directives(root, staging, &directives_with_sha)
}

/// Runs the post-commit hook logic: on the default branch, parse `Done:`
/// directives from HEAD and mark matching phases/tasks done.
///
/// Skips processing if the current branch is not the default branch
/// (configured via `default_branch` in config, falling back to `"main"`).
///
/// All errors are intentionally swallowed by the caller — this must never
/// block a git commit.
#[cfg(feature = "git")]
pub fn run_post_commit_hook(root: &Path, staging: bool) -> Result<()> {
    let cwd = std::env::current_dir().context("cannot determine current directory")?;

    // Only run on the default branch.
    let current_branch = rdm_store_git::current_branch_at(&cwd)?;
    let hook_global_config = paths::load_global_config();
    let hook_repo_config = paths::load_repo_config(root).with_global_defaults(&hook_global_config);
    let default_branch = hook_repo_config.default_branch.as_deref().unwrap_or("main");
    match current_branch.as_deref() {
        Some(branch) if branch == default_branch => {}
        _ => return Ok(()),
    }

    let commit = rdm_store_git::head_commit_info_at(&cwd)?;
    let commit = match commit {
        Some(c) => c,
        None => return Ok(()),
    };

    let directives: Vec<_> = rdm_core::hook::parse_done_directives(&commit.message)
        .into_iter()
        .map(|d| (d, commit.sha.clone()))
        .collect();

    apply_done_directives(root, staging, &directives)
}

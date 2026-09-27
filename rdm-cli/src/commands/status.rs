use std::path::Path;

use anyhow::{Context, Result};
use rdm_store_git::{FileChange, GitStore, MergeState, StatusView};

use crate::OutputFormat;
use crate::commands;

/// Shows this session's uncommitted changes and the repo's sync status.
///
/// The default view is scoped to the caller's changeset and comes from a
/// single three-way partition (`user` / `others` / `unattributed`), so what
/// is listed here and what `rdm commit` will land can never disagree. `all`
/// switches to the whole-tree view. `remote` is the repo's `remote.default`;
/// when it is set the sync state against it is reported too, after a fetch
/// when `fetch` is set.
///
/// `--format json` prints one [`rdm_core::json::StatusJson`] document; every
/// other format prints the human report. Warnings go to stderr either way.
///
/// # Errors
///
/// Returns an error if the store cannot be opened or git status/merge-state
/// queries fail.
pub fn run(
    root: &Path,
    format: OutputFormat,
    remote: Option<&str>,
    fetch: bool,
    all: bool,
) -> Result<()> {
    let mut store = commands::make_store(root)?;
    let view = status_view(&mut store, remote, fetch, all)?;
    match format {
        OutputFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(&view.to_json()).context("failed to serialize status")?
        ),
        _ => render_human(&view),
    }
    for warning in &view.warnings {
        eprintln!("{warning}");
    }
    Ok(())
}

/// Gathers everything `rdm status` reports: the merge state, the scoped or
/// whole-tree report, and the sync state against `remote` (after a fetch when
/// `fetch` is set). A failed fetch or sync query becomes a warning, not an
/// error.
fn status_view(
    store: &mut GitStore,
    remote: Option<&str>,
    fetch: bool,
    all: bool,
) -> Result<StatusView> {
    let merge = if store
        .git()
        .git_is_merge_in_progress()
        .context("failed to check merge state")?
    {
        let unmerged = store
            .git()
            .git_list_unmerged()
            .context("failed to list unmerged files")?;
        Some(MergeState {
            unmerged: unmerged.len(),
        })
    } else {
        None
    };

    let report = if all {
        store
            .git()
            .git_status_report()
            .context("failed to get git status")?
    } else {
        store
            .status_report_scoped()
            .context("failed to get git status")?
    };

    let mut warnings = Vec::new();
    let mut sync = None;
    if let Some(remote_name) = remote {
        if fetch && let Err(e) = store.git_mut().git_fetch(remote_name) {
            warnings.push(format!("warning: fetch failed: {e}"));
        }
        match store.git().git_sync_status(remote_name) {
            Ok(found) => sync = found,
            Err(e) => warnings.push(format!("warning: could not determine sync status: {e}")),
        }
    }

    Ok(StatusView {
        report,
        merge,
        sync,
        warnings,
    })
}

/// Prints `view` as the human `rdm status` report.
fn render_human(view: &StatusView) {
    if let Some(merge) = &view.merge {
        let count = merge.unmerged;
        if count > 0 {
            println!(
                "Merge in progress — {count} conflict(s) remaining. Run `rdm conflicts` for details."
            );
        } else {
            println!(
                "Merge in progress — all conflicts resolved. Run `rdm resolve <file>` or `git commit --no-edit` to complete."
            );
        }
        println!();
    }

    let report = &view.report;
    if report.user.is_empty() {
        println!("No uncommitted changes.");
    } else {
        println!("Uncommitted changes:");
        for fs in &report.user {
            let prefix = match fs.change {
                FileChange::Added => "  added:    ",
                FileChange::Modified => "  modified: ",
                FileChange::Deleted => "  deleted:  ",
            };
            println!("{prefix}{}", fs.path);
        }
        println!(
            "\n{} file(s) changed. Run `rdm commit` to persist or `rdm discard --force` to reset.",
            report.user.len()
        );
    }
    // Other sessions' work is named, never hidden and never silently swept.
    // Shared wording with `rdm commit`/`rdm discard`.
    if let Some(note) = report.others_summary() {
        println!("  {note}");
    }
    if let Some(note) = report.unattributed_summary() {
        println!("  {note}");
    }

    // Sync status, when a default remote is configured and tracked.
    if let Some(sync) = &view.sync {
        println!();
        match (sync.ahead, sync.behind) {
            (0, 0) => {
                println!("Up to date with '{}/{}'.", sync.remote, sync.branch)
            }
            (a, 0) => println!(
                "Your branch is ahead of '{}/{}' by {} commit(s).",
                sync.remote, sync.branch, a
            ),
            (0, b) => println!(
                "Your branch is behind '{}/{}' by {} commit(s).",
                sync.remote, sync.branch, b
            ),
            (a, b) => println!(
                "Your branch and '{}/{}' have diverged ({} ahead, {} behind).",
                sync.remote, sync.branch, a, b
            ),
        }
    }
}

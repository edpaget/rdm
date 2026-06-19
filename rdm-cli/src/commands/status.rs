use std::path::Path;

use anyhow::{Context, Result};

use crate::commands;

/// Shows uncommitted changes and sync status in the plan repo.
///
/// # Errors
///
/// Returns an error if the store cannot be opened or git status/merge-state
/// queries fail.
pub fn run(root: &Path, staging: bool, fetch: bool) -> Result<()> {
    let mut store = commands::make_store(root, staging)?;

    // Check for merge in progress
    if store
        .git()
        .git_is_merge_in_progress()
        .context("failed to check merge state")?
    {
        let unmerged = store
            .git()
            .git_list_unmerged()
            .context("failed to list unmerged files")?;
        let count = unmerged.len();
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

    let statuses = store
        .git()
        .git_status()
        .context("failed to get git status")?;
    if statuses.is_empty() {
        println!("No uncommitted changes.");
    } else {
        println!("Uncommitted changes:");
        for fs in &statuses {
            let prefix = match fs.change {
                rdm_store_git::FileChange::Added => "  added:    ",
                rdm_store_git::FileChange::Modified => "  modified: ",
                rdm_store_git::FileChange::Deleted => "  deleted:  ",
            };
            println!("{prefix}{}", fs.path);
        }
        println!(
            "\n{} file(s) changed. Run `rdm commit` to persist or `rdm discard --force` to reset.",
            statuses.len()
        );
    }

    // Show sync status if a default remote is configured
    let config_path = root.join("rdm.toml");
    let default_remote = std::fs::read_to_string(&config_path)
        .ok()
        .and_then(|s| rdm_core::config::Config::from_toml(&s).ok())
        .and_then(|c| c.remote)
        .and_then(|r| r.default);
    if let Some(remote_name) = default_remote {
        if fetch && let Err(e) = store.git_mut().git_fetch(&remote_name) {
            eprintln!("warning: fetch failed: {e}");
        }
        match store.git().git_sync_status(&remote_name) {
            Ok(Some(sync)) => {
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
            Ok(None) => {
                // No tracking ref — silently skip
            }
            Err(e) => {
                eprintln!("warning: could not determine sync status: {e}");
            }
        }
    }
    Ok(())
}

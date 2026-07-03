use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::commands;

/// Discards uncommitted changes, restoring the working directory to HEAD.
///
/// # Errors
///
/// Returns an error if `--force` is not passed, the store cannot be opened,
/// or git merge-abort/status/discard operations fail.
pub fn run(root: &Path, force: bool) -> Result<()> {
    if !force {
        bail!("discarding changes is irreversible — pass --force to confirm");
    }
    let mut store = commands::make_store(root)?;
    // Abort merge if one is in progress
    if store
        .git()
        .git_is_merge_in_progress()
        .context("failed to check merge state")?
    {
        store
            .git_mut()
            .git_merge_abort()
            .context("failed to abort merge")?;
        println!("Aborted in-progress merge.");
    }
    let statuses = store
        .git()
        .git_status()
        .context("failed to get git status")?;
    if statuses.is_empty() {
        println!("Nothing to discard.");
    } else {
        store
            .git()
            .git_discard()
            .context("failed to discard changes")?;
        println!("Discarded {} file(s).", statuses.len());
        for fs in &statuses {
            let prefix = match fs.change {
                rdm_store_git::FileChange::Added => "  removed:  ",
                rdm_store_git::FileChange::Modified => "  restored: ",
                rdm_store_git::FileChange::Deleted => "  restored: ",
            };
            println!("{prefix}{}", fs.path);
        }
    }
    Ok(())
}

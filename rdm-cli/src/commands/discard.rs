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
    let report = store
        .git()
        .git_status_report()
        .context("failed to get git status")?;
    // Gated on the raw truth: `git_discard` restores everything, including
    // regenerated indexes. Only the reporting below narrows to user changes.
    if report.is_clean() {
        println!("Nothing to discard.");
    } else {
        store
            .git()
            .git_discard()
            .context("failed to discard changes")?;
        // Shared with the MCP `rdm_discard` tool so the two can never disagree.
        println!("{}", report.discard_summary());
        // `git_discard` re-installs the INDEX.md merge mapping after restoring
        // the tree, so a `.gitattributes` it just deleted is back on disk.
        // Re-reading tells us which paths that actually applies to, so the
        // per-file lines below never claim a file was removed while it is
        // sitting right there.
        let still_changed: Vec<String> = store
            .git()
            .git_status_report()
            .map(|after| after.all().iter().map(|fs| fs.path.clone()).collect())
            .unwrap_or_default();
        for fs in &report.user {
            if still_changed.contains(&fs.path) {
                println!("  reinstalled: {} (rdm-managed)", fs.path);
                continue;
            }
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

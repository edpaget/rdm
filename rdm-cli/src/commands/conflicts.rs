use std::path::Path;

use anyhow::{Context, Result};

use crate::commands;

/// Lists unresolved merge conflicts with rdm item context.
///
/// # Errors
///
/// Returns an error if the store cannot be opened or git merge-state queries
/// fail.
pub fn run(root: &Path, staging: bool) -> Result<()> {
    let store = commands::make_store(root, staging)?;
    if !store
        .git()
        .git_is_merge_in_progress()
        .context("failed to check merge state")?
    {
        println!("No merge in progress.");
    } else {
        let unmerged = store
            .git()
            .git_list_unmerged()
            .context("failed to list unmerged files")?;
        if unmerged.is_empty() {
            println!("Merge in progress but all conflicts are resolved.");
            println!("Run `rdm resolve <file>` on any remaining file to complete the merge,");
            println!("or commit manually with `git commit --no-edit`.");
        } else {
            println!(
                "Merge in progress — {} conflict(s) remaining:\n",
                unmerged.len()
            );
            for path in &unmerged {
                let item = rdm_core::conflict::classify_path(path);
                println!("  {path} — {item}");
            }
            println!();
            println!("Edit each file to resolve conflicts, then run `rdm resolve <file>`.");
            println!("Run `rdm discard --force` to abort the merge.");
        }
    }
    Ok(())
}

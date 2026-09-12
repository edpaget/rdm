use std::path::Path;

use anyhow::{Context, Result};

use crate::commands;

/// Marks a conflicted file as resolved and auto-completes the merge when all
/// are resolved.
///
/// # Errors
///
/// Returns an error if the store cannot be opened or conflict resolution
/// fails.
pub fn run(root: &Path, file: String) -> Result<()> {
    let mut store = commands::make_store(root)?;
    let result = store
        .git_mut()
        .git_resolve_conflict(&file)
        .context("failed to resolve conflict")?;
    println!("Resolved: {}", result.path);
    if result.merge_completed {
        println!("All conflicts resolved — merge complete.");
    } else {
        println!(
            "{} conflict(s) remaining. Run `rdm conflicts` to see them.",
            result.remaining
        );
    }
    Ok(())
}

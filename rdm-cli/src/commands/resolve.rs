use std::path::Path;

use anyhow::{Context, Result};

use crate::commands;

/// Marks a conflicted file as resolved and auto-completes the merge when all
/// are resolved.
///
/// # Errors
///
/// Returns an error if the store cannot be opened, conflict resolution fails,
/// or post-merge index regeneration fails.
pub fn run(root: &Path, file: String) -> Result<()> {
    let mut store = commands::make_store(root)?;
    let result = store
        .git_mut()
        .git_resolve_conflict(&file)
        .context("failed to resolve conflict")?;
    println!("Resolved: {}", result.path);
    if result.merge_completed {
        println!("All conflicts resolved — merge complete.");
        // Regenerate INDEX.md after merge completion
        rdm_core::ops::index::generate_index(&mut store)
            .context("failed to regenerate INDEX.md after merge")?;
    } else {
        println!(
            "{} conflict(s) remaining. Run `rdm conflicts` to see them.",
            result.remaining
        );
    }
    Ok(())
}

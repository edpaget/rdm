use std::path::Path;

use anyhow::{Context, Result};

use crate::commands;

/// Regenerates `INDEX.md` from the current repo state.
///
/// # Errors
///
/// Returns an error if the store cannot be opened or the index generation
/// fails.
pub fn run(root: &Path, staging: bool) -> Result<()> {
    let mut store = commands::make_store(root, staging)?;
    rdm_core::ops::index::generate_index(&mut store).context("failed to generate index")?;
    println!("Generated INDEX.md");
    Ok(())
}

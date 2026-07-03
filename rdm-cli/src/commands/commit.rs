use std::path::Path;

use anyhow::{Context, Result};

use crate::commands;

/// Commits staged changes to git.
///
/// # Errors
///
/// Returns an error if the store cannot be opened, git status fails, or the
/// commit fails.
pub fn run(root: &Path, staging: bool, message: Option<String>) -> Result<()> {
    let store = commands::make_store(root, staging)?;
    let statuses = store
        .git()
        .git_status()
        .context("failed to get git status")?;
    if statuses.is_empty() {
        println!("Nothing to commit.");
    } else {
        let msg =
            message.unwrap_or_else(|| rdm_store_git::GitRepo::default_commit_message(&statuses));
        store
            .git()
            .git_commit(&msg)
            .context("failed to create git commit")?;
        println!("Committed {} file(s).", statuses.len());
    }
    Ok(())
}

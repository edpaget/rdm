use std::path::Path;

use anyhow::{Context, Result};

use crate::commands;

/// Commits staged changes to git.
///
/// # Errors
///
/// Returns an error if the store cannot be opened, git status fails, or the
/// commit fails.
pub fn run(root: &Path, message: Option<String>) -> Result<()> {
    let store = commands::make_store(root)?;
    let report = store
        .git()
        .git_status_report()
        .context("failed to get git status")?;
    // Gated on the raw truth, not on `user`: a tree holding only regenerated
    // indexes must still be committable, or it stays dirty forever and
    // `rdm remote pull` refuses to run.
    if report.is_clean() {
        println!("Nothing to commit.");
    } else {
        let all = report.all();
        let msg = message.unwrap_or_else(|| rdm_store_git::GitRepo::default_commit_message(&all));
        store
            .git()
            .git_commit(&msg)
            .context("failed to create git commit")?;
        // Shared with the MCP `rdm_commit` tool so the two can never disagree.
        println!("{}", report.commit_summary());
    }
    Ok(())
}

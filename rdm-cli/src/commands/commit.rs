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
        let msg = message.unwrap_or_else(|| {
            let summary: Vec<String> = statuses
                .iter()
                .map(|s| {
                    let kind = match s.change {
                        rdm_store_git::FileChange::Added => "add",
                        rdm_store_git::FileChange::Modified => "update",
                        rdm_store_git::FileChange::Deleted => "delete",
                    };
                    format!("{kind} {}", s.path)
                })
                .collect();
            if summary.len() == 1 {
                format!("rdm: {}", summary[0])
            } else {
                let mut msg = format!("rdm: update {} files", statuses.len());
                for s in &summary {
                    msg.push_str(&format!("\n\n- {s}"));
                }
                msg
            }
        });
        store
            .git()
            .git_commit(&msg)
            .context("failed to create git commit")?;
        println!("Committed {} file(s).", statuses.len());
    }
    Ok(())
}

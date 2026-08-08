use std::path::Path;

use anyhow::{Context, Result};

use crate::commands;

/// Commits this session's changeset to git.
///
/// By default the commit tree is HEAD plus exactly the paths this session
/// journaled, so a concurrent session's uncommitted work is structurally
/// unreachable rather than filtered out late. `--all` opts into the
/// historical whole-tree sweep; `--changeset <id>` commits a named (usually
/// orphaned) changeset instead of this session's.
///
/// # Errors
///
/// Returns an error if the store cannot be opened, git status fails, the
/// named changeset id is unusable, or the commit fails.
pub fn run(
    root: &Path,
    message: Option<String>,
    all: bool,
    changeset: Option<String>,
) -> Result<()> {
    let store = commands::make_store(root)?;

    if all {
        // The explicitly-named machine-global escape hatch.
        let report = store
            .git()
            .git_status_report()
            .context("failed to get git status")?;
        if report.is_clean() {
            println!("Nothing to commit.");
            return Ok(());
        }
        let all_paths = report.all();
        let msg =
            message.unwrap_or_else(|| rdm_store_git::GitRepo::default_commit_message(&all_paths));
        store
            .commit_whole_tree(&msg)
            .context("failed to create git commit")?;
        println!("{}", report.commit_summary());
        return Ok(());
    }

    let outcome = match changeset {
        Some(ref raw) => {
            let id = rdm_core::session::SessionId::new(raw).ok_or_else(|| {
                anyhow::anyhow!(
                    "'{raw}' is not a usable changeset id — run `rdm session list` to see them"
                )
            })?;
            store
                .commit_changeset_id(Some(&id), message.as_deref(), &[])
                .context("failed to create git commit")?
        }
        None => store
            .commit_changeset(message.as_deref(), &[])
            .context("failed to create git commit")?,
    };

    match outcome.sha {
        Some(ref sha) => {
            println!("{}", outcome.report.commit_summary());
            println!("  commit: {}", &sha[..sha.len().min(12)]);
            if let Some(id) = &outcome.changeset {
                println!("  changeset: {id}");
            }
            if !outcome.skipped_missing.is_empty() {
                println!(
                    "  skipped {} journaled path(s) no longer on disk: {}",
                    outcome.skipped_missing.len(),
                    outcome.skipped_missing.join(", ")
                );
            }
            if let Some(note) = outcome.report.others_summary() {
                println!("  {note}");
            }
        }
        // Deliberately NOT `Nothing to commit.` when the tree is dirty: those
        // paths belong to no changeset this process can see, and silently
        // saying nothing (or sweeping them) is how work gets lost.
        None if outcome.unattributed_dirt() => {
            println!("Nothing in this session's changeset to commit.");
            println!(
                "\n{} uncommitted path(s) are attributed to another changeset:",
                outcome.report.others.len()
            );
            for fs in &outcome.report.others {
                println!("  {}", fs.path);
            }
            println!("\nRecover them with one of:");
            println!("  rdm session list                 # find the owning changeset");
            println!("  rdm commit --changeset <id>      # commit that changeset");
            println!("  rdm commit --all                 # commit the whole working tree");
        }
        None => println!("Nothing to commit."),
    }
    Ok(())
}

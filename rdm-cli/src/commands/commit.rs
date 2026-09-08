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
        //
        // `commit_whole_tree` is called unconditionally — even when
        // `report.is_clean()` — because it also clears every changeset's
        // journal on disk, and a stale journal entry from an earlier no-op
        // write can outlive an already-clean tree. Short-circuiting here
        // before ever calling it would leave that journal behind forever,
        // reproducing this phase's bug one layer up.
        let report = store
            .git()
            .git_status_report()
            .context("failed to get git status")?;
        let all_paths = report.all();
        let msg =
            message.unwrap_or_else(|| rdm_store_git::GitRepo::default_commit_message(&all_paths));
        let commit_report = store
            .commit_whole_tree(&msg)
            .context("failed to create git commit")?;
        if commit_report.sha.is_none() {
            println!("Nothing to commit.");
        } else {
            println!("{}", report.commit_summary());
        }
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
            if let Some(note) = outcome.skipped_summary() {
                println!("  {note}");
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
            if let Some(note) = outcome.skipped_summary() {
                println!("\n{note}");
            }
            print_continuity_advisory(&store);
        }
        // A changeset can reduce to nothing *because* its files vanished — the
        // regenerated indexes then reconcile straight back to HEAD and the tree
        // matches. Reporting only `Nothing to commit.` there would tell a
        // session its work was a no-op when in fact the work is gone.
        None => {
            println!("Nothing to commit.");
            if let Some(note) = outcome.skipped_summary() {
                println!("{note}");
            }
            // Only when the tree actually held something: a genuinely clean
            // repo is not a symptom of anything, and advising there would fire
            // on every no-op `rdm commit` from a plain shell.
            if !outcome.report.is_clean() {
                print_continuity_advisory(&store);
            }
        }
    }
    Ok(())
}

/// Prints the harness-continuity advisory, when there is one to print.
///
/// Called only from the two branches that are *symptoms* of a fragmented
/// harness — an empty changeset over a tree that is not empty — so an ordinary
/// successful commit, and an ordinary no-op commit against a clean tree, both
/// stay silent.
///
/// The session is read back from the store rather than resolved afresh here.
/// The store memoizes exactly one resolution and the mutation path journals
/// under it, so re-resolving could describe a different identity than the one
/// the outcome above was computed against — and `lease_bootstrapped`, which
/// this advisory keys on, is true only for the invocation that actually minted
/// the lease. A store that exposes no session simply prints nothing.
fn print_continuity_advisory(store: &crate::AppStore) {
    #[cfg(feature = "git")]
    {
        let Some(resolved) = store.session() else {
            return;
        };
        if let Some(note) =
            rdm_core::session::continuity_advisory(resolved, &rdm_core::session::RealEnv)
        {
            println!("\n{note}");
        }
    }
    #[cfg(not(feature = "git"))]
    {
        let _ = store;
    }
}

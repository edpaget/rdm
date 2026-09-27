use std::path::Path;

use anyhow::{Context, Result};
use rdm_store_git::{GitStore, ScopedCommit};

use crate::OutputFormat;
use crate::commands;

/// Commits this session's changeset to git.
///
/// By default the commit tree is HEAD plus exactly the paths this session
/// journaled, so a concurrent session's uncommitted work is structurally
/// unreachable rather than filtered out late. `--all` opts into the
/// historical whole-tree sweep; `--changeset <id>` commits a named (usually
/// orphaned) changeset instead of this session's.
///
/// `--format json` prints one [`rdm_core::json::CommitJson`] document and
/// nothing else; every other format prints the human report.
///
/// # Errors
///
/// Returns an error if the store cannot be opened, git status fails, the
/// named changeset id is unusable, or the commit fails.
pub fn run(
    root: &Path,
    format: OutputFormat,
    message: Option<String>,
    all: bool,
    changeset: Option<String>,
) -> Result<()> {
    let store = commands::make_store(root)?;
    let outcome = commit(&store, message, all, changeset)?;
    // The continuity advisory is shown only on the branches that are
    // symptoms of a fragmented harness, and never for a whole-tree commit.
    let advisory = if !all && outcome.is_continuity_symptom() {
        store.continuity_advisory()
    } else {
        None
    };
    match format {
        OutputFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(&outcome.to_json(advisory.as_deref()))
                .context("failed to serialize commit")?
        ),
        _ => render_human(&outcome, all, advisory.as_deref()),
    }
    Ok(())
}

/// Commits the whole tree (`all`), a named changeset, or this session's
/// changeset, returning what happened as one [`ScopedCommit`].
fn commit(
    store: &GitStore,
    message: Option<String>,
    all: bool,
    changeset: Option<String>,
) -> Result<ScopedCommit> {
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
        return Ok(ScopedCommit::whole_tree(report, commit_report));
    }

    Ok(match changeset {
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
    })
}

/// Prints `outcome` as the human `rdm commit` report.
///
/// `advisory` is the harness-continuity advisory, present only when the
/// outcome is a symptom of a fragmented harness (see
/// [`ScopedCommit::is_continuity_symptom`]).
fn render_human(outcome: &ScopedCommit, all: bool, advisory: Option<&str>) {
    if all {
        if outcome.sha.is_none() {
            println!("Nothing to commit.");
        } else {
            println!("{}", outcome.report.commit_summary());
        }
        return;
    }

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
            if let Some(note) = outcome.report.unattributed_summary() {
                println!("  {note}");
            }
        }
        // Deliberately NOT `Nothing to commit.` when the tree is dirty: those
        // paths belong to no changeset this process can see (a real other
        // changeset, or none at all), and silently saying nothing (or
        // sweeping them) is how work gets lost.
        None if outcome.unattributed_dirt() => {
            println!("Nothing in this session's changeset to commit.");
            if !outcome.report.others.is_empty() {
                println!(
                    "\n{} uncommitted path(s) belong to another changeset:",
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
            if !outcome.report.unattributed.is_empty() {
                println!(
                    "\n{} uncommitted path(s) are not attributed to any changeset:",
                    outcome.report.unattributed.len()
                );
                for fs in &outcome.report.unattributed {
                    println!("  {}", fs.path);
                }
                println!("\nNo changeset owns them, so recover them with:");
                println!("  rdm commit --all                 # commit the whole working tree");
            }
            if let Some(note) = outcome.skipped_summary() {
                println!("\n{note}");
            }
            if let Some(note) = advisory {
                println!("\n{note}");
            }
        }
        // A changeset can reduce to nothing *because* its files vanished, so
        // every path it named was skipped and the tree still matches HEAD.
        // Reporting only `Nothing to commit.` there would tell a session its
        // work was a no-op when in fact the work is gone.
        None => {
            println!("Nothing to commit.");
            if let Some(note) = outcome.skipped_summary() {
                println!("{note}");
            }
            // Present only when the tree actually held something: a genuinely
            // clean repo is not a symptom of anything, and advising there
            // would fire on every no-op `rdm commit` from a plain shell.
            if let Some(note) = advisory {
                println!("\n{note}");
            }
        }
    }
}

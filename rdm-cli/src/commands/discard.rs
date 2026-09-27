use std::path::Path;

use anyhow::{Context, Result, bail};
use rdm_store_git::{FileChange, GitStore, ScopedDiscard};

use crate::OutputFormat;
use crate::commands;

/// Discards uncommitted changes, restoring them to HEAD.
///
/// By default this is **changeset-scoped**: only paths this session journaled
/// are restored — an `INDEX.md` this session regenerated among them, since it
/// is an ordinary claimed path and nothing recomputes one afterwards — and
/// this session's journal is cleared. `--all` opts into the historical whole-tree
/// destruction, which requires `--force` as well and first names every other
/// live changeset it is about to destroy.
///
/// `--format json` prints one [`rdm_core::json::DiscardJson`] document and
/// nothing else; every other format prints the human report.
///
/// # Errors
///
/// Returns an error if `--force` is not passed, the store cannot be opened,
/// or git merge-abort/status/discard operations fail.
pub fn run(root: &Path, format: OutputFormat, force: bool, all: bool) -> Result<()> {
    if !force {
        bail!("discarding changes is irreversible — pass --force to confirm");
    }
    let mut store = commands::make_store(root)?;
    let (merge_aborted, outcome) = discard(&mut store, all)?;
    match format {
        OutputFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(&outcome.to_json(merge_aborted))
                .context("failed to serialize discard")?
        ),
        _ => render_human(&outcome, merge_aborted, all),
    }
    Ok(())
}

/// Aborts any in-progress merge, then discards the whole tree (`all`) or this
/// session's changeset. Returns whether a merge was aborted and what the
/// discard did, as one [`ScopedDiscard`].
fn discard(store: &mut GitStore, all: bool) -> Result<(bool, ScopedDiscard)> {
    let merge_aborted = store
        .git()
        .git_is_merge_in_progress()
        .context("failed to check merge state")?;
    if merge_aborted {
        store
            .git_mut()
            .git_merge_abort()
            .context("failed to abort merge")?;
    }

    if all {
        let report = store
            .git()
            .git_status_report()
            .context("failed to get git status")?;
        if !report.is_clean() {
            // Name what is about to be destroyed *before* destroying it.
            warn_other_changesets(store);
            store
                .discard_whole_tree()
                .context("failed to discard changes")?;
        }
        return Ok((merge_aborted, ScopedDiscard::whole_tree(report)));
    }

    let report = store
        .status_report_scoped()
        .context("failed to get git status")?;
    if report.is_changeset_clean() {
        return Ok((
            merge_aborted,
            ScopedDiscard {
                report,
                ..ScopedDiscard::default()
            },
        ));
    }
    let outcome = store
        .discard_changeset()
        .context("failed to discard changes")?;
    Ok((merge_aborted, outcome))
}

/// Prints `outcome` as the human `rdm discard` report.
fn render_human(outcome: &ScopedDiscard, merge_aborted: bool, all: bool) {
    if merge_aborted {
        println!("Aborted in-progress merge.");
    }
    let report = &outcome.report;
    if all && report.is_clean() {
        println!("Nothing to discard.");
        return;
    }
    if !all && report.is_changeset_clean() {
        println!("Nothing in this session's changeset to discard.");
    } else {
        println!("{}", outcome.discard_summary());
        print_per_file(outcome);
        if let Some(note) = outcome.skipped_summary() {
            println!("  {note}");
        }
    }
    if let Some(note) = report.others_summary() {
        println!("  {note}");
    }
    if let Some(note) = report.unattributed_summary() {
        println!("  {note}");
    }
}

/// Prints the per-file discard lines.
///
/// The restore is HEAD-exact: rdm authors no file of its own into the
/// worktree, so every path the outcome restored is genuinely back at its HEAD
/// content (or gone) by the time this runs. A path the scoped discard
/// deliberately left in place (`skipped_overwritten`) gets an honest
/// `skipped:` line instead of a `removed:`/`restored:` one that would falsely
/// claim this discard reverted it; a whole-tree discard skips nothing.
fn print_per_file(outcome: &ScopedDiscard) {
    for fs in &outcome.report.user {
        if outcome.skipped_overwritten.contains(&fs.path) {
            println!(
                "  skipped:  {} (changed by another session — left in place)",
                fs.path
            );
            continue;
        }
        let prefix = match fs.change {
            FileChange::Added => "  removed:  ",
            FileChange::Modified => "  restored: ",
            FileChange::Deleted => "  restored: ",
        };
        println!("{prefix}{}", fs.path);
    }
}

/// Names every other live changeset a whole-tree discard is about to destroy.
fn warn_other_changesets(store: &GitStore) {
    let Some(paths) = store.session_paths() else {
        return;
    };
    let current = store.session().map(|s| s.id.clone());
    let procs = rdm_core::session::process::SystemProcessTable::snapshot();
    let Ok(changesets) =
        rdm_core::session::journal::list_changesets(paths, &procs, current.as_ref())
    else {
        return;
    };
    let others: Vec<&rdm_core::session::journal::ChangesetSummary> = changesets
        .iter()
        .filter(|c| current.as_ref().is_none_or(|id| id.as_str() != c.id))
        .collect();
    if others.is_empty() {
        return;
    }
    eprintln!(
        "warning: --all destroys the uncommitted work of {} other changeset(s):",
        others.len()
    );
    for c in others {
        let flag = c
            .liveness
            .label()
            .map(|l| format!(" ({l})"))
            .unwrap_or_default();
        eprintln!("  {} — {} path(s){flag}", c.id, c.paths);
    }
}

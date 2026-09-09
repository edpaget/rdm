use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::commands;

/// Discards uncommitted changes, restoring them to HEAD.
///
/// By default this is **changeset-scoped**: only paths this session journaled
/// are restored, the generated indexes are regenerated from the resulting
/// disk state (so another session's still-uncommitted rows survive), and this
/// session's journal is cleared. `--all` opts into the historical whole-tree
/// destruction, which requires `--force` as well and first names every other
/// live changeset it is about to destroy.
///
/// # Errors
///
/// Returns an error if `--force` is not passed, the store cannot be opened,
/// or git merge-abort/status/discard operations fail.
pub fn run(root: &Path, force: bool, all: bool) -> Result<()> {
    if !force {
        bail!("discarding changes is irreversible — pass --force to confirm");
    }
    let mut store = commands::make_store(root)?;
    // Abort merge if one is in progress
    if store
        .git()
        .git_is_merge_in_progress()
        .context("failed to check merge state")?
    {
        store
            .git_mut()
            .git_merge_abort()
            .context("failed to abort merge")?;
        println!("Aborted in-progress merge.");
    }

    if all {
        let report = store
            .git()
            .git_status_report()
            .context("failed to get git status")?;
        if report.is_clean() {
            println!("Nothing to discard.");
            return Ok(());
        }
        // Name what is about to be destroyed *before* destroying it.
        warn_other_changesets(&store);
        store
            .discard_whole_tree()
            .context("failed to discard changes")?;
        println!("{}", report.discard_summary());
        print_per_file(&store, &report);
        return Ok(());
    }

    let report = store
        .status_report_scoped()
        .context("failed to get git status")?;
    if report.is_changeset_clean() {
        println!("Nothing in this session's changeset to discard.");
        if let Some(note) = report.others_summary() {
            println!("  {note}");
        }
        if let Some(note) = report.unattributed_summary() {
            println!("  {note}");
        }
        return Ok(());
    }
    let outcome = store
        .discard_changeset()
        .context("failed to discard changes")?;
    println!("{}", outcome.discard_summary());
    print_per_file_scoped(&store, &outcome);
    if let Some(note) = outcome.skipped_summary() {
        println!("  {note}");
    }
    if let Some(note) = report.others_summary() {
        println!("  {note}");
    }
    if let Some(note) = report.unattributed_summary() {
        println!("  {note}");
    }
    Ok(())
}

/// Prints the per-file discard lines, re-reading status so the `reinstalled:`
/// case is honest.
///
/// `git_discard`/`discard_changeset` re-install the INDEX.md merge mapping
/// after restoring the tree, so a `.gitattributes` just deleted is back on
/// disk. Re-reading tells us which paths that actually applies to, so the
/// lines below never claim a file was removed while it is sitting right there.
///
/// Used only by the `--all` whole-tree path, which has no per-path
/// skip/overwrite guard — see [`print_per_file_scoped`] for the
/// changeset-scoped counterpart.
fn print_per_file(store: &rdm_store_git::GitStore, report: &rdm_store_git::StatusReport) {
    let still_changed: Vec<String> = store
        .git()
        .git_status_report()
        .map(|after| after.all().iter().map(|fs| fs.path.clone()).collect())
        .unwrap_or_default();
    for fs in &report.user {
        if still_changed.contains(&fs.path) {
            println!("  reinstalled: {} (rdm-managed)", fs.path);
            continue;
        }
        let prefix = match fs.change {
            rdm_store_git::FileChange::Added => "  removed:  ",
            rdm_store_git::FileChange::Modified => "  restored: ",
            rdm_store_git::FileChange::Deleted => "  restored: ",
        };
        println!("{prefix}{}", fs.path);
    }
}

/// The changeset-scoped counterpart of [`print_per_file`], reading the
/// [`rdm_store_git::ScopedDiscard`] `discard_changeset` returns so a path it
/// deliberately left in place (`skipped_overwritten`) gets an honest
/// `skipped:` line instead of a `removed:`/`restored:` one that would
/// falsely claim this discard reverted it.
fn print_per_file_scoped(store: &rdm_store_git::GitStore, outcome: &rdm_store_git::ScopedDiscard) {
    let still_changed: Vec<String> = store
        .git()
        .git_status_report()
        .map(|after| after.all().iter().map(|fs| fs.path.clone()).collect())
        .unwrap_or_default();
    for fs in &outcome.report.user {
        if outcome.skipped_overwritten.contains(&fs.path) {
            println!(
                "  skipped:  {} (changed by another session — left in place)",
                fs.path
            );
            continue;
        }
        if still_changed.contains(&fs.path) {
            println!("  reinstalled: {} (rdm-managed)", fs.path);
            continue;
        }
        let prefix = match fs.change {
            rdm_store_git::FileChange::Added => "  removed:  ",
            rdm_store_git::FileChange::Modified => "  restored: ",
            rdm_store_git::FileChange::Deleted => "  restored: ",
        };
        println!("{prefix}{}", fs.path);
    }
}

/// Names every other live changeset a whole-tree discard is about to destroy.
fn warn_other_changesets(store: &rdm_store_git::GitStore) {
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
        let orphan = if c.orphaned { " (orphaned)" } else { "" };
        eprintln!("  {} — {} path(s){orphan}", c.id, c.paths);
    }
}

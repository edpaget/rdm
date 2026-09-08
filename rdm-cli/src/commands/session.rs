//! `rdm session` — inspect this session's changeset identity and journal.
//!
//! This is the observation surface over `rdm_core::session`: it reports which
//! identity the caller resolved and which rung produced it, prints the exact
//! paths a changeset journaled, and recovers changesets orphaned by a killed
//! session. It never mutates plan data — session state lives outside the
//! committable tree, so nothing here can show up in `rdm status` or a commit.

use std::path::Path;

use anyhow::{Context, Result, bail};
use rdm_core::session::journal;
use rdm_core::session::{RealEnv, ResolvedSession, SessionId, SessionPaths};

use crate::{OutputFormat, SessionCommand};

/// Runs a `rdm session` subcommand.
///
/// # Errors
///
/// Returns an error if no session state directory can be determined, if a
/// supplied changeset id is unusable, if `discard` is called without
/// `--force`, or if the journal cannot be read or written.
pub fn run(root: &Path, format: OutputFormat, command: SessionCommand) -> Result<()> {
    let paths = resolve_paths(root)?;
    // The process table is fetched per-arm, never up front: building it is the
    // dominant cost of resolution, and hoisting it here would move that cost
    // outside the window `resolve_micros` measures — under-reporting exactly
    // the figure `rdm session id --format json` exists to report.

    match command {
        SessionCommand::Id => {
            let resolved = resolve(root, &paths)?;
            match format {
                OutputFormat::Json => println!(
                    "{}",
                    serde_json::json!({
                        "id": resolved.id.as_str(),
                        "rung": resolved.rung.number(),
                        "resolve_micros": resolved.resolve_micros,
                    })
                ),
                // Bare id on stdout, so shell callers can capture it with
                // `$(rdm session id)`.
                _ => println!("{}", resolved.id),
            }
        }
        SessionCommand::Journal { id } => {
            let id = match id {
                Some(raw) => parse_id(&raw)?,
                None => resolve(root, &paths)?.id,
            };
            let entries = journal::read_journal(&paths, &id)
                .with_context(|| format!("failed to read the journal for changeset {id}"))?;
            match format {
                OutputFormat::Json => println!(
                    "{}",
                    serde_json::json!({ "id": id.as_str(), "paths": entries })
                ),
                _ => {
                    if entries.is_empty() {
                        println!("Changeset {id} has journaled no paths.");
                    } else {
                        println!("Changeset {id} journaled {} path(s):", entries.len());
                        for entry in &entries {
                            println!("  {:<7} {}", entry.kind.as_str(), entry.path);
                        }
                    }
                }
            }
        }
        SessionCommand::List => {
            let current = resolve(root, &paths).ok().map(|r| r.id);
            let procs = rdm_core::session::system_process_table();
            let summaries = journal::list_changesets(&paths, procs, current.as_ref())
                .context("failed to list changesets")?;
            match format {
                OutputFormat::Json => println!("{}", serde_json::to_string(&summaries)?),
                _ => {
                    if summaries.is_empty() {
                        println!("No changesets recorded.");
                    } else {
                        for summary in &summaries {
                            let flag = if summary.orphaned { " (orphaned)" } else { "" };
                            println!("{}  {} path(s){flag}", summary.id, summary.paths);
                        }
                    }
                }
            }
        }
        SessionCommand::Adopt { id } => {
            let id = parse_id(&id)?;
            let procs = rdm_core::session::system_process_table();
            journal::adopt_changeset(&paths, procs, &RealEnv, &id)
                .with_context(|| format!("failed to adopt changeset {id}"))?;
            println!("Adopted changeset {id}; this session's later mutations join it.");
        }
        SessionCommand::Discard { id, force } => {
            let id = parse_id(&id)?;
            if !force {
                bail!(
                    "refusing to discard changeset {id} without --force — \
                     discarding a journal is irreversible"
                );
            }
            if journal::discard_changeset(&paths, &id)
                .with_context(|| format!("failed to discard changeset {id}"))?
            {
                println!("Discarded changeset {id}.");
            } else {
                println!("No changeset {id} to discard.");
            }
        }
        SessionCommand::Gc => {
            let procs = rdm_core::session::system_process_table();
            let removed = rdm_core::session::lease::gc(&paths, procs);
            println!("Removed {removed} stale lease(s).");
        }
    }
    Ok(())
}

/// Sanitizes a user-supplied changeset id, reporting the rule when nothing
/// usable survives.
fn parse_id(raw: &str) -> Result<SessionId> {
    SessionId::new(raw).with_context(|| {
        format!(
            "'{raw}' is not a usable changeset id — ids are limited to letters, \
             digits, '.', '_' and '-'"
        )
    })
}

/// Resolves this session's identity for `root`.
fn resolve(root: &Path, paths: &SessionPaths) -> Result<ResolvedSession> {
    // Prefer the store's own resolution so the CLI can never disagree with
    // what the mutation path journals under.
    #[cfg(feature = "git")]
    {
        if let Ok(store) = crate::commands::make_store(root)
            && let Some(resolved) = store.session()
        {
            return Ok(resolved.clone());
        }
    }
    let _ = root;
    Ok(rdm_core::session::resolve_system_session(paths))
}

/// Resolves where this repo's session state lives.
///
/// A git-backed repo hides it inside the git directory, which both of rdm's
/// whole-tree walks already skip. A plain filesystem repo falls back to an XDG
/// state directory; session state is never written under `$RDM_ROOT` outside
/// `.git`.
fn resolve_paths(root: &Path) -> Result<SessionPaths> {
    #[cfg(feature = "git")]
    {
        if let Ok(store) = crate::commands::make_store(root)
            && let Some(paths) = store.session_paths()
        {
            return Ok(paths.clone());
        }
    }
    SessionPaths::fallback_for_root(root).context(
        "cannot determine a session state directory — set XDG_STATE_HOME or HOME, \
         or pin an identity with RDM_SESSION",
    )
}

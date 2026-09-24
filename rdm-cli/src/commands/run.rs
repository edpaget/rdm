//! `rdm run` — run records: what an autonomous-lane run drove, in which
//! Claude Code session, and each dispatched unit's time window and outcome.
//!
//! The CLI owns the two inputs core never reads for itself: the clock
//! (`Utc::now()` at each call) and the session uuid (`--session-uuid`, else
//! the raw [`CLAUDE_SESSION_ENV`]).

use anyhow::{Context, Result, bail};
use chrono::Utc;
use rdm_core::config::Config;
use rdm_core::display;
use rdm_core::document::Document;
use rdm_core::json;
use rdm_core::model::{Run, RunTarget};
use rdm_core::ops::runs::{self, CreateRun, RunFilter};

use super::{CLAUDE_SESSION_ENV, commit_mutation, maybe_print_uncommitted_hint};
use crate::paths;
use crate::table;
use crate::{AppStore, OutputFormat, RunCommand};

/// The session uuid a new run is recorded against: the explicit flag, else
/// the raw `CLAUDE_CODE_SESSION_ID` when set and non-empty, else none.
///
/// Deliberately not `resolve_session`: that yields a hashed changeset id,
/// which names no transcript directory and so cannot be joined to spend.
fn session_uuid(flag: Option<String>) -> Option<String> {
    flag.filter(|s| !s.is_empty()).or_else(|| {
        std::env::var(CLAUDE_SESSION_ENV)
            .ok()
            .filter(|s| !s.is_empty())
    })
}

fn print_run_json(doc: &Document<Run>) -> Result<()> {
    println!(
        "{}",
        serde_json::to_string_pretty(&json::run_to_json(doc)).context("failed to serialize run")?
    );
    Ok(())
}

/// Runs an `rdm run` subcommand.
///
/// # Errors
///
/// Returns an error when the project cannot be resolved, the core operation
/// refuses (unknown run or target, a terminal run, an overlapping or missing
/// unit, an empty outcome), or output serialization fails.
pub fn run(
    command: RunCommand,
    store: &mut AppStore,
    repo_config: &Config,
    format: OutputFormat,
) -> Result<()> {
    match command {
        RunCommand::Record {
            driver,
            roadmap,
            task,
            session_uuid: flag,
            args,
            project,
        } => {
            let project = paths::resolve_project(project, repo_config)?;
            let target = match (roadmap, task) {
                (Some(r), None) => RunTarget::Roadmap(r),
                (None, Some(t)) => RunTarget::Task(t),
                _ => bail!("pass exactly one of --roadmap <slug> or --task <slug>"),
            };
            let session = session_uuid(flag);
            let doc = commit_mutation(store, "failed to record run", |s| {
                runs::create_run(
                    s,
                    CreateRun {
                        project: &project,
                        driver,
                        target,
                        session_uuid: session.as_deref(),
                        args: args.as_deref(),
                        now: Utc::now(),
                    },
                )
            })?;
            if doc.frontmatter.session_uuid.is_none() {
                eprintln!(
                    "  note: no session uuid recorded (pass --session-uuid or set {CLAUDE_SESSION_ENV}) — this run cannot be joined to spend"
                );
            }
            match format {
                OutputFormat::Json => print_run_json(&doc)?,
                _ => println!("{}", doc.frontmatter.id),
            }
        }
        RunCommand::UnitStart { id, unit, project } => {
            let project = paths::resolve_project(project, repo_config)?;
            let doc = commit_mutation(store, "failed to start run unit", |s| {
                runs::start_unit(s, &project, &id, &unit, Utc::now())
            })?;
            match format {
                OutputFormat::Json => print_run_json(&doc)?,
                _ => {
                    let started = doc
                        .frontmatter
                        .units
                        .last()
                        .context("started unit missing from the run")?;
                    println!(
                        "Started {} (attempt {}) in run {id}",
                        started.unit, started.attempt
                    );
                }
            }
        }
        RunCommand::UnitEnd {
            id,
            outcome,
            project,
        } => {
            let project = paths::resolve_project(project, repo_config)?;
            let now = Utc::now();
            let doc = commit_mutation(store, "failed to end run unit", |s| {
                runs::end_unit(s, &project, &id, &outcome, now)
            })?;
            match format {
                OutputFormat::Json => print_run_json(&doc)?,
                _ => {
                    // At most one unit is open at a time, so the one this
                    // call just ended is the one stamped with `now`.
                    let ended = doc
                        .frontmatter
                        .units
                        .iter()
                        .find(|u| u.ended == Some(now))
                        .context("ended unit missing from the run")?;
                    println!(
                        "Ended {} (attempt {}): {outcome}",
                        ended.unit, ended.attempt
                    );
                }
            }
        }
        RunCommand::Close {
            id,
            stop_reason,
            status,
            project,
        } => {
            let project = paths::resolve_project(project, repo_config)?;
            let doc = commit_mutation(store, "failed to close run", |s| {
                runs::close_run(s, &project, &id, status.into(), &stop_reason, Utc::now())
            })?;
            match format {
                OutputFormat::Json => print_run_json(&doc)?,
                _ => println!("Closed run {id} ({})", doc.frontmatter.status),
            }
        }
        RunCommand::List {
            roadmap,
            task,
            project,
        } => {
            let project = paths::resolve_project(project, repo_config)?;
            let all = runs::list_runs(store, &project).context("failed to list runs")?;
            let filtered = runs::filter_runs(all, &RunFilter { roadmap, task });
            match format {
                OutputFormat::Human => print!("{}", display::format_run_list(&filtered)),
                OutputFormat::Table => print!("{}", table::format_run_table(&filtered)),
                OutputFormat::Markdown => print!("{}", display::format_run_list_md(&filtered)),
                OutputFormat::Json => {
                    let items: Vec<_> = filtered
                        .iter()
                        .map(|(_, doc)| json::run_to_json(doc))
                        .collect();
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&items).context("failed to serialize runs")?
                    );
                }
            }
            maybe_print_uncommitted_hint(store);
        }
        RunCommand::Show { id, project } => {
            let project = paths::resolve_project(project, repo_config)?;
            let doc = runs::get_run(store, &project, &id).context("failed to load run")?;
            match format {
                OutputFormat::Human => print!("{}", display::format_run_detail(&doc)),
                OutputFormat::Markdown => print!("{}", display::format_run_detail_md(&doc)),
                OutputFormat::Json => print_run_json(&doc)?,
                OutputFormat::Table => bail!(
                    "--format table is not supported for 'run show'; use --format human, --format json, --format markdown, or omit --format"
                ),
            }
            maybe_print_uncommitted_hint(store);
        }
    }
    Ok(())
}

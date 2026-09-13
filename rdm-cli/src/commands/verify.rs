//! `rdm verify` — the CLI surface over the repo-only `dispatch.verify` key.
//!
//! `docs/verify-gate.md` is canonical for the key itself: one project-supplied
//! command, never decomposed or partially run, whose exit code gates a phase.
//! This module adds two ways to reach it from outside the dispatch pipeline:
//! `rdm verify resolve` (what is configured?) and `rdm verify run` (run it in
//! the item's worktree and report the result).
//!
//! Discovery — inferring a command from CI config, `docs/principles.md` or
//! `CLAUDE.md` when the key is unset — is deliberately **not** implemented
//! here. It stays an agent step in the orchestrator, whose result is written
//! into the plan document rather than into config, so a guess never silently
//! becomes a project's durable verification contract.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::OutputFormat;
use crate::VerifyCommand;
use crate::commands;
use crate::paths;

/// How many characters of merged command output `run` reports.
///
/// The **last** 4000, not the first: a failing command prints its diagnosis
/// at the end. Matches the bound `docs/verify-gate.md` § 6 states, so the two
/// surfaces cannot disagree.
const TAIL_LIMIT: usize = 4000;

/// Exit code for "no verification command is configured".
///
/// Distinct from 0/1 so a shell caller can tell an unconfigured project from a
/// failing one — but note a command that legitimately exits 2 is
/// indistinguishable by exit code alone, which is why the JSON payload's
/// `resolved` field is the contract callers should key off.
const EXIT_UNRESOLVED: i32 = 2;

/// Exit code for a command that could not produce an exit status at all
/// (killed by a signal). Fail-closed: an unrunnable verification is never a
/// pass.
const EXIT_UNRUNNABLE: i32 = 1;

/// Dispatches an `rdm verify` subcommand.
///
/// # Errors
///
/// Returns an error when the project cannot be resolved, when `--item` names
/// an item with no worktree, or when the configured command cannot be spawned.
pub fn run(
    command: VerifyCommand,
    root: &Path,
    repo_config: &rdm_core::config::Config,
    format: OutputFormat,
) -> Result<()> {
    match command {
        VerifyCommand::Resolve { project } => resolve(root, repo_config, format, project),
        VerifyCommand::Run { item, project } => {
            run_command(root, repo_config, format, item, project)
        }
    }
}

/// The configured verification command, if any.
///
/// Reads through `paths::get_config_field` — the very accessor
/// `rdm config get dispatch.verify --raw` uses — so the two surfaces can never
/// disagree about what is configured.
fn configured_command(repo_config: &rdm_core::config::Config) -> Option<String> {
    paths::get_config_field(repo_config, "dispatch.verify")
}

fn resolve(
    root: &Path,
    repo_config: &rdm_core::config::Config,
    format: OutputFormat,
    project: Option<String>,
) -> Result<()> {
    // Resolve the project so an unknown `--project` is still an error here,
    // matching every other subcommand's behavior.
    let _ = paths::resolve_project(project, repo_config)?;
    let _ = root;
    let cmd = configured_command(repo_config);
    match format {
        OutputFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "resolved": cmd.is_some(),
                "command": cmd,
            }))?
        ),
        _ => match &cmd {
            Some(c) => println!("{c}"),
            None => println!("unresolved"),
        },
    }
    Ok(())
}

fn run_command(
    root: &Path,
    repo_config: &rdm_core::config::Config,
    format: OutputFormat,
    item: Option<String>,
    project: Option<String>,
) -> Result<()> {
    let project = paths::resolve_project(project, repo_config)?;
    let Some(cmd) = configured_command(repo_config) else {
        emit(format, false, None, None, None)?;
        std::process::exit(EXIT_UNRESOLVED);
    };
    // `docs/verify-gate.md` § 6: a multi-line value is refused rather than
    // guessed at. rdm never decomposes the command — ordering, parallelism and
    // output formatting belong to whatever task runner it invokes — so a value
    // that looks like a script is a configuration mistake, not a shell to run.
    if cmd.contains('\n') {
        bail!(
            "'dispatch.verify' is multi-line, which rdm refuses to run — it executes ONE command \
             and never decomposes it. Put the steps in a script (e.g. 'bash scripts/ci.sh') and \
             set that as the value: rdm config set dispatch.verify \"bash scripts/ci.sh\""
        );
    }

    let dir = match item {
        Some(raw) => item_worktree(root, &project, &raw)?,
        None => std::env::current_dir().context("cannot determine current directory")?,
    };

    let output = Command::new("sh")
        .arg("-c")
        .arg(&cmd)
        .current_dir(&dir)
        .output()
        .with_context(|| {
            format!(
                "failed to run the configured verification command in {}",
                dir.display()
            )
        })?;

    // Merge stdout and stderr: a verification command's diagnosis routinely
    // lands on stderr, and a caller reading one stream would miss it.
    let mut merged = String::from_utf8_lossy(&output.stdout).into_owned();
    merged.push_str(&String::from_utf8_lossy(&output.stderr));
    let tail = tail_of(&merged);

    let code = output.status.code();
    emit(format, true, Some(&cmd), code, Some(&tail))?;
    std::process::exit(code.unwrap_or(EXIT_UNRUNNABLE));
}

/// The last [`TAIL_LIMIT`] characters of `text`, split on a character boundary.
fn tail_of(text: &str) -> String {
    let count = text.chars().count();
    if count <= TAIL_LIMIT {
        return text.to_string();
    }
    text.chars().skip(count - TAIL_LIMIT).collect()
}

/// Resolves the worktree directory of `raw` — the same item grammar
/// `rdm worktree add` accepts.
fn item_worktree(root: &Path, project: &str, raw: &str) -> Result<std::path::PathBuf> {
    use rdm_git::worktree;
    let store = commands::make_store(root)?;
    let item = worktree::resolve_item(&store, project, raw)
        .map_err(|e| anyhow::anyhow!("{e}"))
        .with_context(|| format!("cannot resolve item '{raw}'"))?;
    let cwd = std::env::current_dir().context("cannot determine current directory")?;
    let repo =
        worktree::discover_distinct_project_repo(&cwd, root).map_err(|e| anyhow::anyhow!("{e}"))?;
    let entries = worktree::list(&repo).map_err(|e| anyhow::anyhow!("{e}"))?;
    let canonical = item.canonical();
    entries
        .into_iter()
        .find(|w| w.item == canonical)
        .map(|w| w.path)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no rdm worktree for '{canonical}' — create one with `rdm worktree add {canonical}`, \
                 or omit --item to run in the current directory"
            )
        })
}

/// Prints the result payload in the requested format.
fn emit(
    format: OutputFormat,
    resolved: bool,
    command: Option<&str>,
    exit: Option<i32>,
    tail: Option<&str>,
) -> Result<()> {
    match format {
        OutputFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "resolved": resolved,
                "command": command,
                "exit": exit,
                "tail": tail,
            }))?
        ),
        _ => {
            if !resolved {
                println!("unresolved");
                return Ok(());
            }
            if let Some(t) = tail
                && !t.is_empty()
            {
                print!("{t}");
                if !t.ends_with('\n') {
                    println!();
                }
            }
            match exit {
                Some(0) => println!("verify: passed ({})", command.unwrap_or("")),
                Some(c) => println!("verify: FAILED with exit {c} ({})", command.unwrap_or("")),
                None => println!(
                    "verify: FAILED — the command was terminated by a signal ({})",
                    command.unwrap_or("")
                ),
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_keeps_the_last_characters_not_the_first() {
        let text = format!("START{}END", "x".repeat(TAIL_LIMIT));
        let tail = tail_of(&text);
        assert_eq!(tail.chars().count(), TAIL_LIMIT);
        assert!(tail.ends_with("END"), "the tail must be the END of output");
        assert!(!tail.contains("START"), "the head must be dropped");
    }

    #[test]
    fn tail_passes_short_output_through_unchanged() {
        assert_eq!(tail_of("hello\n"), "hello\n");
        assert_eq!(tail_of(""), "");
    }

    #[test]
    fn tail_splits_on_a_character_boundary() {
        // Multi-byte characters: truncating by bytes would panic or produce
        // invalid UTF-8; truncating by chars must not.
        let text = "é".repeat(TAIL_LIMIT + 10);
        let tail = tail_of(&text);
        assert_eq!(tail.chars().count(), TAIL_LIMIT);
    }
}

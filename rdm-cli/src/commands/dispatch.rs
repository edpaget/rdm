//! `rdm dispatch` — inspect what the autonomous dispatch lane resolves for a project.
//!
//! Read-only: nothing here writes to the plan repo, the source repo, or git.

use anyhow::{Context, Result};
use rdm_core::directives::{DirectiveRole, DirectiveSet};

use crate::OutputFormat;
use crate::cli::{DirectiveRoleArg, DispatchCommand};

/// Runs a `rdm dispatch` subcommand.
///
/// `declared` is the plan repo's `dispatch.directives` value, or `None` when no
/// plan repo was reachable or the key is unset — in which case rdm falls back to
/// discovering the known agent-instruction locations rather than erroring, so a
/// downstream repo with no plan repo still gets the feature.
///
/// # Errors
///
/// Returns an error only if the scanned directory cannot be resolved or the
/// output cannot be serialized. An absent, unreadable, or over-bound source is
/// reported in the payload's `skipped` array, never as a failure.
pub fn run(
    command: DispatchCommand,
    declared: Option<Vec<String>>,
    format: OutputFormat,
) -> Result<()> {
    match command {
        DispatchCommand::Directives {
            dir,
            role,
            format: sub_format,
        } => {
            let dir = match dir {
                Some(d) => d,
                None => {
                    std::env::current_dir().context("failed to resolve the current directory")?
                }
            };
            let set = rdm_core::directives::resolve(&dir, declared.as_deref())
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            let set = filter_role(set, role);
            // The subcommand's own --format wins when given; the global --format is
            // the fallback so `rdm --format json dispatch directives` also works.
            let chosen = if sub_format == OutputFormat::Human {
                format
            } else {
                sub_format
            };
            match chosen {
                OutputFormat::Json => print_json(&set)?,
                _ => print_text(&set),
            }
            Ok(())
        }
    }
}

/// Drops directives not addressed to `role`. `both` always survives, and no filter
/// keeps everything.
fn filter_role(mut set: DirectiveSet, role: Option<DirectiveRoleArg>) -> DirectiveSet {
    let Some(want) = role else { return set };
    let want = match want {
        DirectiveRoleArg::Implementer => DirectiveRole::Implementer,
        DirectiveRoleArg::Reviewer => DirectiveRole::Reviewer,
    };
    set.directives
        .retain(|d| d.role == want || d.role == DirectiveRole::Both);
    set
}

/// Emits the payload the workflow lane consumes.
///
/// `budget` echoes the bound so it is single-sourced in Rust — the JS helpers and
/// the harness render what `skipped` reports and never restate the numbers.
fn print_json(set: &DirectiveSet) -> Result<()> {
    let payload = serde_json::json!({
        "origin": set.origin.as_str(),
        "budget": {
            "maxBytesPerSource": set.max_bytes_per_source,
            "maxBytesTotal": set.max_bytes_total,
        },
        "directives": set
            .directives
            .iter()
            .map(|d| serde_json::json!({
                "path": d.path,
                "role": d.role.as_str(),
                "paths": d.paths,
                "text": d.text,
                "chars": d.chars,
                "bytes": d.bytes,
            }))
            .collect::<Vec<_>>(),
        "skipped": set
            .skipped
            .iter()
            .map(|s| serde_json::json!({
                "path": s.path,
                "bytes": s.bytes,
                "reason": s.reason,
            }))
            .collect::<Vec<_>>(),
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&payload).context("failed to serialize directives")?
    );
    Ok(())
}

fn print_text(set: &DirectiveSet) {
    println!("origin: {}", set.origin.as_str());
    println!(
        "budget: {} bytes per source, {} bytes total",
        set.max_bytes_per_source, set.max_bytes_total
    );
    if set.directives.is_empty() {
        println!("directives: (none)");
    } else {
        println!("directives:");
        for d in &set.directives {
            let scope = if d.paths.is_empty() {
                "unscoped".to_string()
            } else {
                d.paths.join(", ")
            };
            println!(
                "  {} [role: {}] [paths: {}] ({} bytes)",
                d.path,
                d.role.as_str(),
                scope,
                d.bytes
            );
        }
    }
    if !set.skipped.is_empty() {
        println!("skipped:");
        for s in &set.skipped {
            println!("  {} — {}", s.path, s.reason);
        }
    }
}

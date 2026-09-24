//! `rdm cost` — the tokens a Claude Code session, or one Workflow run in it,
//! spent.
//!
//! A thin adapter over `rdm_core::transcript`: it picks the selector, builds
//! the filesystem source from the environment, and renders the core report.

use anyhow::{Result, bail};
use chrono::{DateTime, SecondsFormat, Utc};
use rdm_core::transcript::{
    ReportScope, SessionReport, UsageSummary, locate_session, locate_workflow_run, reap_session,
    reap_workflow_run,
};
use rdm_transcript::FsTranscriptSource;

use super::CLAUDE_SESSION_ENV as SESSION_ENV;
use crate::OutputFormat;

enum Selector {
    Session(String),
    WorkflowRun(String),
}

fn select(session: Option<String>, workflow_run: Option<String>) -> Result<Selector> {
    if let Some(s) = session {
        return Ok(Selector::Session(s));
    }
    if let Some(r) = workflow_run {
        return Ok(Selector::WorkflowRun(r));
    }
    match std::env::var(SESSION_ENV) {
        Ok(s) if !s.is_empty() => Ok(Selector::Session(s)),
        _ => bail!(
            "no session to report: pass --session <uuid> or --workflow-run <wf_id>, or run inside a Claude Code session so {SESSION_ENV} is set"
        ),
    }
}

/// Runs `rdm cost`.
///
/// # Errors
///
/// Returns an error when no selector is given and `CLAUDE_CODE_SESSION_ID`
/// is unset, when Claude Code's data directory cannot be resolved, or when
/// the session or run cannot be located or read.
pub fn run(
    session: Option<String>,
    workflow_run: Option<String>,
    format: OutputFormat,
) -> Result<()> {
    let selector = select(session, workflow_run)?;
    let src = FsTranscriptSource::from_env()?;
    let report = match selector {
        Selector::Session(id) => reap_session(&src, &locate_session(&src, &id)?)?,
        Selector::WorkflowRun(id) => reap_workflow_run(&src, &locate_workflow_run(&src, &id)?)?,
    };
    match format {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&report)?),
        OutputFormat::Human | OutputFormat::Table | OutputFormat::Markdown => {
            print!("{}", render_human(&report));
            for w in &report.warnings {
                eprintln!("warning: {w}");
            }
        }
    }
    Ok(())
}

fn anchor_text(anchor: Option<DateTime<Utc>>) -> String {
    anchor.map_or_else(
        || "unanchored".to_owned(),
        |a| a.to_rfc3339_opts(SecondsFormat::AutoSi, true),
    )
}

/// Pads `rows` into aligned columns; columns from `numeric_from` on are
/// right-aligned.
fn table(headers: &[&str], rows: &[Vec<String>], numeric_from: usize) -> String {
    let mut widths: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
    for row in rows {
        for (w, cell) in widths.iter_mut().zip(row) {
            *w = (*w).max(cell.chars().count());
        }
    }
    let line = |cells: Vec<&str>| {
        let padded: Vec<String> = cells
            .iter()
            .zip(&widths)
            .enumerate()
            .map(|(i, (c, w))| {
                if i >= numeric_from {
                    format!("{c:>w$}")
                } else {
                    format!("{c:<w$}")
                }
            })
            .collect();
        format!("  {}\n", padded.join("  ").trim_end())
    };
    let mut out = line(headers.to_vec());
    for row in rows {
        out.push_str(&line(row.iter().map(String::as_str).collect()));
    }
    out
}

fn usage_cells(u: &UsageSummary) -> Vec<String> {
    [
        u.requests,
        u.usage.input,
        u.usage.output,
        u.usage.cache_write_5m,
        u.usage.cache_write_1h,
        u.usage.cache_read,
        u.total,
    ]
    .iter()
    .map(u64::to_string)
    .collect()
}

fn render_human(report: &SessionReport) -> String {
    let mut out = format!(
        "Session {} (project {})\n",
        report.session_id, report.project_slug
    );
    match &report.scope {
        ReportScope::Session => out.push_str("Scope: whole session\n"),
        ReportScope::WorkflowRun { run_id } => {
            out.push_str(&format!("Scope: Workflow run {run_id}\n"));
        }
    }

    out.push_str("\nTokens by model\n");
    let mut rows: Vec<Vec<String>> = report
        .models
        .iter()
        .map(|m| {
            let mut row = vec![m.model.clone()];
            row.extend(usage_cells(&m.usage));
            row
        })
        .collect();
    let mut total = vec!["total".to_owned()];
    total.extend(usage_cells(&report.totals));
    rows.push(total);
    out.push_str(&table(
        &[
            "model",
            "requests",
            "input",
            "output",
            "cache_write_5m",
            "cache_write_1h",
            "cache_read",
            "total",
        ],
        &rows,
        1,
    ));

    out.push_str("\nSources\n");
    if report.sources.is_empty() {
        out.push_str("  (none)\n");
    } else {
        let rows: Vec<Vec<String>> = report
            .sources
            .iter()
            .map(|s| {
                let label = match &s.agent_type {
                    Some(t) => format!("{} [{t}]", s.label),
                    None => s.label.clone(),
                };
                let mut flags = Vec::new();
                if s.errored {
                    flags.push("errored".to_owned());
                }
                if s.cached {
                    flags.push("cached".to_owned());
                }
                if s.duplicates_dropped > 0 {
                    flags.push(format!("{} dup dropped", s.duplicates_dropped));
                }
                vec![
                    s.kind.to_string(),
                    s.id.clone(),
                    label,
                    anchor_text(s.anchor),
                    if flags.is_empty() {
                        "-".to_owned()
                    } else {
                        flags.join(",")
                    },
                    s.totals.requests.to_string(),
                    s.totals.total.to_string(),
                ]
            })
            .collect();
        out.push_str(&table(
            &[
                "kind", "id", "label", "anchor", "flags", "requests", "total",
            ],
            &rows,
            5,
        ));
    }

    if !report.unanchored.is_empty() {
        out.push_str("\nUnanchored\n");
        for u in &report.unanchored {
            out.push_str(&format!("  {} {}: {}\n", u.kind, u.id, u.label));
        }
    }

    if !report.workflow_runs.is_empty() {
        out.push_str("\nWorkflow runs\n");
        for r in &report.workflow_runs {
            out.push_str(&format!(
                "  {} {}: {}, {} agent(s), sidecar totalTokens (not the cost basis): {}\n",
                r.run_id,
                r.workflow_name.as_deref().unwrap_or("(no name)"),
                r.anchor.map_or_else(
                    || "unanchored".to_owned(),
                    |a| format!("anchored {}", anchor_text(Some(a)))
                ),
                r.agent_count,
                r.sidecar_total_tokens
                    .map_or_else(|| "none".to_owned(), |t| t.to_string()),
            ));
        }
    }
    out
}

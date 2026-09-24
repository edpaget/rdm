//! `rdm cost report --roadmap <slug>` — what a roadmap's recorded runs spent,
//! joined to their sessions by time window.
//!
//! A thin adapter over `rdm_core::cost_report`: it loads the project's run
//! records, builds the filesystem transcript source from the environment,
//! calls core and renders the report.

use anyhow::{Context, Result};
use chrono::{DateTime, SecondsFormat, Utc};
use rdm_core::config::Config;
use rdm_core::cost_report::{RoadmapCostReport, RunJoin, cost_report};
use rdm_core::model::Run;
use rdm_core::ops::runs;
use rdm_core::transcript::UsageSummary;
use rdm_transcript::FsTranscriptSource;
use serde::Serialize;

use super::cost::{table, usage_cells};
use crate::paths;
use crate::{AppStore, OutputFormat};

/// The JSON document: the core report plus the project it was read from.
#[derive(Serialize)]
struct ReportJson<'a> {
    project: &'a str,
    #[serde(flatten)]
    report: &'a RoadmapCostReport,
}

/// Runs `rdm cost report`.
///
/// # Errors
///
/// Returns an error when the project cannot be resolved or its run records
/// cannot be read, or when Claude Code's data directory cannot be resolved.
/// A run whose session cannot be found is not an error: it is reported as
/// missing.
pub fn run(
    store: &AppStore,
    repo_config: &Config,
    roadmap: &str,
    project: Option<String>,
    format: OutputFormat,
) -> Result<()> {
    let project = paths::resolve_project(project, repo_config)?;
    let runs: Vec<Run> = runs::list_runs(store, &project)
        .context("failed to list runs")?
        .into_iter()
        .map(|(_, doc)| doc.frontmatter)
        .collect();
    let src = FsTranscriptSource::from_env()?;
    let report = cost_report(roadmap, &runs, &src);
    match format {
        OutputFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(&ReportJson {
                project: &project,
                report: &report,
            })
            .context("failed to serialize the cost report")?
        ),
        OutputFormat::Human | OutputFormat::Table | OutputFormat::Markdown => {
            print!("{}", render_human(&project, &report));
            for w in &report.warnings {
                eprintln!("warning: {w}");
            }
        }
    }
    Ok(())
}

const USAGE_HEADERS: [&str; 7] = [
    "requests",
    "input",
    "output",
    "cache_write_5m",
    "cache_write_1h",
    "cache_read",
    "total",
];

fn time(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::AutoSi, true)
}

/// Milliseconds as `1h02m03s`, `2m03s` or `3s`; sub-second remainders are
/// dropped.
fn duration(ms: u64) -> String {
    let secs = ms / 1000;
    let (h, m, s) = (secs / 3600, (secs / 60) % 60, secs % 60);
    if h > 0 {
        format!("{h}h{m:02}m{s:02}s")
    } else if m > 0 {
        format!("{m}m{s:02}s")
    } else {
        format!("{s}s")
    }
}

fn labelled(label: &str, usage: &UsageSummary) -> Vec<String> {
    let mut row = vec![label.to_owned()];
    row.extend(usage_cells(usage));
    row
}

fn join_state(join: &RunJoin) -> &'static str {
    match join {
        RunJoin::Joined => "joined",
        RunJoin::Missing { .. } => "missing",
        RunJoin::Unjoinable => "unjoinable",
    }
}

fn render_human(project: &str, report: &RoadmapCostReport) -> String {
    let mut out = format!("Roadmap {} (project {project})\n", report.roadmap);
    let c = &report.counts;
    if c.runs == 0 {
        out.push_str("No runs recorded for this roadmap.\n");
        return out;
    }
    out.push_str(&format!(
        "Runs: {} ({} joined, {} missing, {} unjoinable; {} open (incomplete))\n",
        c.runs, c.joined, c.missing, c.unjoinable, c.incomplete
    ));

    fn with_first(first: &str) -> Vec<&str> {
        let mut h = vec![first];
        h.extend(USAGE_HEADERS);
        h
    }

    out.push_str("\nTokens by model\n");
    let mut rows: Vec<Vec<String>> = report
        .models
        .iter()
        .map(|m| labelled(&m.model, &m.usage))
        .collect();
    rows.push(labelled("total", &report.totals));
    out.push_str(&table(&with_first("model"), &rows, 1));

    out.push_str("\nBuckets\n");
    let b = &report.buckets;
    let rows = vec![
        labelled("phases", &b.phases),
        labelled("run overhead", &b.overhead),
        labelled("unattributed (runs)", &b.unattributed),
        labelled("unattributed (sessions)", &b.session_unattributed),
        labelled("total", &report.totals),
    ];
    out.push_str(&table(&with_first("bucket"), &rows, 1));

    out.push_str("\nPhases, by total tokens\n");
    if report.phases.is_empty() {
        out.push_str("  (none)\n");
    } else {
        let mut headers = vec!["phase", "attempts", "wall_clock"];
        headers.extend(USAGE_HEADERS);
        let rows: Vec<Vec<String>> = report
            .phases
            .iter()
            .map(|p| {
                let mut row = vec![
                    p.stem.clone(),
                    p.attempts.to_string(),
                    format!(
                        "{}{}",
                        duration(p.wall_clock_ms),
                        if p.has_incomplete_attempt { "*" } else { "" }
                    ),
                ];
                row.extend(usage_cells(&p.usage));
                row
            })
            .collect();
        out.push_str(&table(&headers, &rows, 1));
        if report.phases.iter().any(|p| p.has_incomplete_attempt) {
            out.push_str(
                "  * an attempt never ended: its wall clock is partial and its spend is unattributed\n",
            );
        }
    }

    out.push_str("\nUnattributed in runs\n");
    if report.unattributed.is_empty() {
        out.push_str("  (none)\n");
    } else {
        let rows: Vec<Vec<String>> = report
            .unattributed
            .iter()
            .map(|u| {
                vec![
                    u.run_id.clone(),
                    u.kind.to_string(),
                    u.id.clone(),
                    u.label.clone(),
                    u.cause.to_string(),
                    u.usage.requests.to_string(),
                    u.usage.total.to_string(),
                ]
            })
            .collect();
        out.push_str(&table(
            &["run", "kind", "id", "label", "cause", "requests", "total"],
            &rows,
            5,
        ));
    }

    out.push_str("\nUnattributed in sessions (no anchor or timestamp; counted once per session)\n");
    if report.session_unattributed.is_empty() {
        out.push_str("  (none)\n");
    } else {
        let rows: Vec<Vec<String>> = report
            .session_unattributed
            .iter()
            .map(|u| {
                vec![
                    u.session_uuid.clone(),
                    u.kind.to_string(),
                    u.id.clone(),
                    u.label.clone(),
                    u.cause.to_string(),
                    u.usage.requests.to_string(),
                    u.usage.total.to_string(),
                ]
            })
            .collect();
        out.push_str(&table(
            &[
                "session", "kind", "id", "label", "cause", "requests", "total",
            ],
            &rows,
            5,
        ));
    }

    let e = &report.excluded;
    out.push_str(&format!(
        "\nExcluded (outside this roadmap's runs; not in any total): {} source(s), {} main request(s), {} tokens\n",
        e.sources, e.main_requests, e.usage.total
    ));

    out.push_str("\nRuns\n");
    let rows: Vec<Vec<String>> = report
        .runs
        .iter()
        .map(|r| {
            vec![
                r.id.clone(),
                r.driver.to_string(),
                r.status_label.clone(),
                r.session_uuid.clone().unwrap_or_else(|| "-".to_owned()),
                join_state(&r.join).to_owned(),
                time(r.started),
                r.ended.map_or_else(|| "-".to_owned(), time),
                r.wall_clock_ms.map_or_else(|| "-".to_owned(), duration),
                r.totals.total.to_string(),
            ]
        })
        .collect();
    out.push_str(&table(
        &[
            "id",
            "driver",
            "status",
            "session",
            "join",
            "started",
            "ended",
            "wall_clock",
            "total",
        ],
        &rows,
        7,
    ));

    let missing: Vec<(&str, &str)> = report
        .runs
        .iter()
        .filter_map(|r| match &r.join {
            RunJoin::Missing { reason } => Some((r.id.as_str(), reason.as_str())),
            _ => None,
        })
        .collect();
    if !missing.is_empty() {
        out.push_str("\nMissing sessions\n");
        for (id, reason) in missing {
            out.push_str(&format!("  {id}: {}\n", reason.replace('\n', "\n    ")));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::duration;

    #[test]
    fn durations_render_compactly() {
        assert_eq!(duration(0), "0s");
        assert_eq!(duration(59_999), "59s");
        assert_eq!(duration(90_000), "1m30s");
        assert_eq!(duration(3_723_000), "1h02m03s");
    }
}

//! `rdm-measure lane-tokens`: token usage across Workflow runs, broken out by
//! token class and grouped by agent class, full label, model and workflow,
//! with the sidecar-vs-deduped `totalTokens` discrepancy always reported and
//! never reconciled, plus a per-agent-class first-request floor.
//!
//! Replaces `scripts/measure-lane-tokens.mjs`; the JSON schema is unchanged.

use std::path::Path;

use serde::Serialize;

use super::jsnum::{fmt_number, serialize_js_number};
use super::sidecar::{
    AgentRecord, Bucket, Floor, RunFilters, aggregate, build_records, find_workflow_run_files,
    floor_by_agent_class, locate_session_dirs,
};

/// The sidecar `totalTokens` sum next to the independently deduped sum.
/// Neither is a cost basis; both sides and the delta are always reported.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TotalsDiscrepancy {
    /// Sum of every in-scope run's own `totalTokens`.
    #[serde(serialize_with = "serialize_js_number")]
    pub sidecar_total_tokens: f64,
    /// Sum of every record's four token classes.
    #[serde(serialize_with = "serialize_js_number")]
    pub deduped_total_tokens: f64,
    /// `sidecar - deduped`.
    #[serde(serialize_with = "serialize_js_number")]
    pub delta: f64,
}

/// The lane-tokens report (`--format json` adds `warnings`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaneReport {
    /// The sidecar root searched, as given.
    pub projects_root: String,
    /// In-scope runs.
    pub runs_considered: usize,
    /// Agent records.
    pub record_count: usize,
    /// By agent class.
    pub by_agent_class: Vec<Bucket>,
    /// By full label.
    pub by_label: Vec<Bucket>,
    /// By model.
    pub by_model: Vec<Bucket>,
    /// By workflow.
    pub by_workflow: Vec<Bucket>,
    /// First-request floor per agent class.
    pub floor_by_agent_class: Vec<Floor>,
    /// The never-reconciled discrepancy line.
    pub totals_discrepancy: TotalsDiscrepancy,
    /// Degradation warnings, in the order they arose.
    pub warnings: Vec<String>,
}

/// Builds the report over `root`.
///
/// # Errors
///
/// When `root` cannot be read or `filters.since` is not a date.
pub fn build_report(
    root: &Path,
    root_display: &str,
    filters: &RunFilters,
) -> Result<LaneReport, String> {
    let mut warnings = Vec::new();
    let sessions = locate_session_dirs(root)?;
    let runs = find_workflow_run_files(&sessions, filters, &mut |w| warnings.push(w))?;
    let records: Vec<AgentRecord> = build_records(&runs, &mut |w| warnings.push(w));
    let sidecar_total_tokens: f64 = runs.iter().map(|r| r.run.total_tokens).sum();
    let deduped_total_tokens: f64 = records.iter().map(AgentRecord::all_tokens).sum();
    Ok(LaneReport {
        projects_root: root_display.to_owned(),
        runs_considered: runs.len(),
        record_count: records.len(),
        by_agent_class: aggregate(&records, |r| r.agent_class.clone()),
        by_label: aggregate(&records, |r| r.label.clone()),
        by_model: aggregate(&records, |r| r.model.clone()),
        by_workflow: aggregate(&records, |r| r.workflow_name.clone()),
        floor_by_agent_class: floor_by_agent_class(&records),
        totals_discrepancy: TotalsDiscrepancy {
            sidecar_total_tokens,
            deduped_total_tokens,
            delta: sidecar_total_tokens - deduped_total_tokens,
        },
        warnings,
    })
}

fn group(title: &str, rows: &[Bucket]) -> String {
    let mut lines = vec![format!("-- {title} --")];
    if rows.is_empty() {
        lines.push("  (none)".to_owned());
    }
    for r in rows {
        lines.push(format!(
            "  {}  agents={} requests={} output={} uncachedInput={} cacheWrite={} cacheRead={}",
            r.key,
            fmt_number(r.agent_count),
            fmt_number(r.deduped_request_count),
            fmt_number(r.output),
            fmt_number(r.uncached_input),
            fmt_number(r.cache_write),
            fmt_number(r.cache_read)
        ));
    }
    lines.join("\n")
}

fn floor_group(title: &str, rows: &[Floor]) -> String {
    let mut lines = vec![format!("-- {title} --")];
    if rows.is_empty() {
        lines.push("  (none)".to_owned());
    }
    for r in rows {
        lines.push(format!(
            "  {}  n={} min={} p10={} median={} mean={}",
            r.key,
            fmt_number(r.n),
            fmt_number(r.min_tokens),
            fmt_number(r.p10_tokens),
            fmt_number(r.median_tokens),
            fmt_number(r.mean_tokens)
        ));
    }
    lines.join("\n")
}

/// The `--format text` rendering (no trailing newline).
pub fn format_text(report: &LaneReport) -> String {
    let d = &report.totals_discrepancy;
    let mut lines = vec![
        format!(
            "Runs considered: {}  Agent records: {}",
            report.runs_considered, report.record_count
        ),
        String::new(),
        format!(
            "Sidecar totalTokens vs deduped-sum discrepancy: {} (sidecar={}, deduped={})",
            fmt_number(d.delta),
            fmt_number(d.sidecar_total_tokens),
            fmt_number(d.deduped_total_tokens)
        ),
        String::new(),
        group("By agent class", &report.by_agent_class),
        String::new(),
        group("By label", &report.by_label),
        String::new(),
        group("By model", &report.by_model),
        String::new(),
        group("By workflow", &report.by_workflow),
        String::new(),
        floor_group(
            "Per-agent-class first-request floor",
            &report.floor_by_agent_class,
        ),
    ];
    if !report.warnings.is_empty() {
        lines.push(String::new());
        lines.push("-- Warnings --".to_owned());
        for w in &report.warnings {
            lines.push(format!("  {w}"));
        }
    }
    lines.join("\n")
}

/// The `--format json` rendering (no trailing newline).
///
/// # Errors
///
/// Serialization failure (not expected for this data).
pub fn format_json(report: &LaneReport) -> Result<String, String> {
    serde_json::to_string_pretty(report).map_err(|e| e.to_string())
}

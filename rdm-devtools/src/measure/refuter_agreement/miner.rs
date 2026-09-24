//! `rdm-measure mine-refuter-corpus`: recover real historical refuter
//! findings, verbatim, from the full-fidelity agent transcripts.
//!
//! Every refuter's `subagents/workflows/<runId>/agent-<agentId>.jsonl`
//! transcript opens with the COMPLETE prompt it was given and carries the
//! COMPLETE `StructuredOutput` verdict it returned (the 401-character
//! truncation applies only to the sidecars' previews). The historical verdict
//! is NOT ground truth — every run was refuted by an opus-class model, so
//! scoring opus against it would be circular — and is recorded only as
//! `provenance.historicalVerdict`, with `groundTruth: null`.
//!
//! `promptSha256` is the sha256 of the recovered prompt text; mining never
//! calls `refutePrompt`, so it needs no JavaScript runtime.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

use super::corpus::{CORPUS_SCHEMA_VERSION, sha256};
use super::prompt::reconstruct_refute_inputs;
use super::trials::{BatchPower, group_corpus_for_batching};
use crate::measure::jsjson::{JsMap, JsValue, js_len, js_str_cmp, obj};
use crate::measure::refuter_severity::LANE_WORKFLOWS;
use crate::measure::refuter_severity::transcripts::structured_outputs;
use crate::measure::sidecar::{
    AgentRecord, RunFilters, build_records, filter_until, find_workflow_run_files, io_message,
    locate_session_dirs, read_text, transcript_entries, transcript_path_for,
};

/// This repo's own project slugs (their `--worktrees-` variants share the
/// prefix), so another project's source text is never mined into this repo.
pub const DEFAULT_PROJECT_SLUG_PREFIXES: [&str; 1] = ["-Users-edward-Projects-rdm"];

/// The `instrument` value the JSON format writes.
pub const INSTRUMENT: &str = "rdm-measure mine-refuter-corpus";

/// A refuter transcript's prompt and verdict, or why it has none.
#[derive(Debug, Clone, PartialEq)]
pub enum RefuterRecord {
    /// The initiating prompt and the last boolean verdict (if any).
    Found {
        /// The verbatim prompt.
        prompt: String,
        /// `{refuted, confidence, rationale}`.
        verdict: Option<JsValue>,
    },
    /// `no-transcript` or `no-prompt`.
    Skipped(&'static str),
}

/// Reads one refuter transcript.
pub fn read_refuter_record(path: &Path) -> RefuterRecord {
    let Ok(raw) = read_text(path) else {
        return RefuterRecord::Skipped("no-transcript");
    };
    let mut prompt: Option<String> = None;
    let mut verdict = None;
    for entry in transcript_entries(&raw) {
        if prompt.is_none() && entry.get("type").and_then(Value::as_str) == Some("user") {
            if let Some(p) = entry
                .get("message")
                .and_then(|m| m.get("content"))
                .and_then(Value::as_str)
            {
                prompt = Some(p.to_owned());
            }
            continue;
        }
        for input in structured_outputs(&entry) {
            if let Some(refuted) = input.get("refuted").and_then(Value::as_bool) {
                verdict = Some(obj([
                    ("refuted", JsValue::Bool(refuted)),
                    (
                        "confidence",
                        input.get("confidence").and_then(Value::as_f64).into(),
                    ),
                    (
                        "rationale",
                        input
                            .get("rationale")
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                            .into(),
                    ),
                ]));
            }
        }
    }
    match prompt {
        Some(prompt) => RefuterRecord::Found { prompt, verdict },
        None => RefuterRecord::Skipped("no-prompt"),
    }
}

/// Turns one recovered refuter into a candidate record, or names the skip
/// reason. Never assigns ground truth.
///
/// # Errors
///
/// The skip reason: `unparseable-finding`, `unrecoverable-mode`,
/// `unrecoverable-dim` or `no-verdict` (checked in that order).
pub fn build_candidate(
    record: &AgentRecord,
    prompt: &str,
    verdict: Option<&JsValue>,
    agent_index: Option<usize>,
) -> Result<JsValue, &'static str> {
    let inputs = reconstruct_refute_inputs(prompt);
    let finding = inputs.finding.ok_or("unparseable-finding")?;
    let mode = inputs.mode.ok_or("unrecoverable-mode")?;
    let dim_key = inputs
        .dim_key
        .filter(|d| !d.is_empty())
        .ok_or("unrecoverable-dim")?;
    let verdict = verdict.ok_or("no-verdict")?;
    let mut provenance = vec![
        ("kind", JsValue::from("mined")),
        ("projectSlug", record.project_slug.clone().into()),
        ("sessionId", record.session_id.clone().into()),
        ("runId", record.run_id.clone().into()),
        ("agentId", record.agent_id.clone().into()),
        ("workflow", record.workflow_name.clone().into()),
        ("historicalVerdict", verdict.clone()),
        ("historicalModel", record.model.clone().into()),
    ];
    if let Some(i) = agent_index {
        provenance.push(("agentIndex", i.into()));
    }
    Ok(obj([
        (
            "id",
            format!(
                "mined-{}-{}",
                record.run_id,
                record.agent_id.as_deref().unwrap_or("undefined")
            )
            .into(),
        ),
        ("schemaVersion", JsValue::Number(CORPUS_SCHEMA_VERSION)),
        ("mode", mode.into()),
        ("dim", obj([("key", JsValue::String(dim_key))])),
        ("target", inputs.target.unwrap_or_default().into()),
        ("finding", finding),
        ("promptSha256", sha256(prompt).into()),
        ("promptDrift", false.into()),
        ("provenance", obj(provenance)),
        ("groundTruth", JsValue::Null),
        ("promptLength", js_len(prompt).into()),
    ]))
}

/// Mining options.
#[derive(Debug, Clone, Default)]
pub struct MineOptions {
    /// The sidecar root.
    pub root: PathBuf,
    /// The root as reported.
    pub root_display: String,
    /// Project-slug prefixes (empty: the defaults).
    pub project_slug_prefixes: Vec<String>,
    /// Ignore runs starting after this instant.
    pub until: Option<String>,
    /// Keep only these finding severities (empty: all).
    pub severities: Vec<String>,
    /// Stop after this many recovered records.
    pub limit: Option<usize>,
    /// Keep only candidates whose unit-scoped group has at least this many
    /// members.
    pub min_group_size: Option<usize>,
    /// Ids already adjudicated into the corpus.
    pub exclude_ids: Vec<String>,
}

/// The `--min-group-size` grouping diagnostic.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BatchGrouping {
    /// The floor.
    pub min_group_size: usize,
    /// Groups formed.
    pub group_count: usize,
    /// size → groups.
    pub size_histogram: BTreeMap<usize, usize>,
    /// mode → size → groups.
    pub size_histogram_by_mode: JsMap<BTreeMap<usize, usize>>,
    /// Groups at or above the floor.
    pub qualifying_groups: usize,
    /// Their items.
    pub qualifying_items: usize,
    /// Candidates with no recoverable unit.
    pub unrecoverable_unit_excluded: usize,
    /// Non-gating candidates.
    pub non_gating_excluded: usize,
}

impl From<&BatchPower> for BatchGrouping {
    fn from(p: &BatchPower) -> Self {
        Self {
            min_group_size: p.min_group_size,
            group_count: p.group_count,
            size_histogram: p.size_histogram.clone(),
            size_histogram_by_mode: p.size_histogram_by_mode.clone(),
            qualifying_groups: p.qualifying_groups,
            qualifying_items: p.qualifying_items,
            unrecoverable_unit_excluded: p.unrecoverable_unit_excluded,
            non_gating_excluded: p.non_gating_excluded,
        }
    }
}

/// The mining result.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MineResult {
    /// The sidecar root.
    pub corpus_root: String,
    /// The slug prefixes applied.
    pub project_slug_prefixes: Vec<String>,
    /// The window.
    pub until: Option<String>,
    /// Refuter records found.
    pub refuter_record_count: usize,
    /// Candidates emitted.
    pub recovered: usize,
    /// Skip reason → count; `recovered + Σ skips == refuter_record_count`.
    pub skips: JsMap<usize>,
    /// The grouping diagnostic, with `--min-group-size`.
    pub batch_grouping: Option<BatchGrouping>,
    /// The candidates, in stable order.
    pub items: Vec<JsValue>,
}

/// `{ instrument, ...result }` for `--format json`.
#[derive(Debug, Serialize)]
pub struct InstrumentMine<'a> {
    /// The producing command.
    pub instrument: &'static str,
    /// The result.
    #[serde(flatten)]
    pub result: &'a MineResult,
}

fn bump(skips: &mut JsMap<usize>, reason: &str, by: usize) {
    *skips.entry(reason, || 0) += by;
}

/// Mines candidate records.
///
/// # Errors
///
/// When the root cannot be read or `until` is not a date.
pub fn mine(options: &MineOptions) -> Result<MineResult, String> {
    let prefixes: Vec<String> = if options.project_slug_prefixes.is_empty() {
        DEFAULT_PROJECT_SLUG_PREFIXES
            .iter()
            .map(|s| (*s).to_owned())
            .collect()
    } else {
        options.project_slug_prefixes.clone()
    };
    let sessions: Vec<_> = locate_session_dirs(&options.root)?
        .into_iter()
        .filter(|s| {
            prefixes
                .iter()
                .any(|p| s.project_slug.starts_with(p.as_str()))
        })
        .collect();
    let filters = RunFilters {
        since: None,
        workflow_names: LANE_WORKFLOWS.iter().map(|s| (*s).to_owned()).collect(),
    };
    let mut runs = find_workflow_run_files(&sessions, &filters, &mut |_| {})?;
    if let Some(until) = options.until.as_deref().filter(|u| !u.is_empty()) {
        runs = filter_until(runs, until)?;
    }
    let mut session_dir_of: Vec<(String, PathBuf)> = Vec::new();
    let mut agent_index_of: Vec<(String, usize)> = Vec::new();
    for rf in &runs {
        let key = rf.run_key();
        match session_dir_of.iter_mut().find(|(k, _)| *k == key) {
            Some(slot) => slot.1 = rf.session_dir.clone(),
            None => session_dir_of.push((key.clone(), rf.session_dir.clone())),
        }
        for (i, agent) in rf.run.agents.iter().enumerate() {
            if crate::measure::sidecar::truthy(agent.get("agentId")) {
                let id = crate::measure::sidecar::js_display(agent.get("agentId"));
                let k = format!("{key}|{id}");
                match agent_index_of.iter_mut().find(|(x, _)| *x == k) {
                    Some(slot) => slot.1 = i,
                    None => agent_index_of.push((k, i)),
                }
            }
        }
    }
    let mut records: Vec<AgentRecord> = build_records(&runs, &mut |_| {})
        .into_iter()
        .filter(|r| r.agent_class == "refute")
        .collect();
    let sort_key = |r: &AgentRecord| {
        format!(
            "{}|{}",
            r.run_key(),
            r.agent_id.as_deref().unwrap_or("undefined")
        )
    };
    records.sort_by(|a, b| js_str_cmp(&sort_key(a), &sort_key(b)));

    let mut items: Vec<JsValue> = Vec::new();
    let mut skips: JsMap<usize> = JsMap::new();
    for r in &records {
        if options.limit.is_some_and(|l| items.len() >= l) {
            break;
        }
        let dir = session_dir_of
            .iter()
            .find(|(k, _)| *k == r.run_key())
            .map(|(_, d)| d);
        let (Some(dir), Some(agent_id)) = (dir, r.agent_id.as_deref()) else {
            bump(&mut skips, "no-transcript", 1);
            continue;
        };
        let (prompt, verdict) =
            match read_refuter_record(&transcript_path_for(dir, &r.run_id, agent_id)) {
                RefuterRecord::Found { prompt, verdict } => (prompt, verdict),
                RefuterRecord::Skipped(reason) => {
                    bump(&mut skips, reason, 1);
                    continue;
                }
            };
        let index = agent_index_of
            .iter()
            .find(|(k, _)| *k == format!("{}|{agent_id}", r.run_key()))
            .map(|(_, i)| *i);
        let item = match build_candidate(r, &prompt, verdict.as_ref(), index) {
            Ok(item) => item,
            Err(reason) => {
                bump(&mut skips, reason, 1);
                continue;
            }
        };
        if !options.severities.is_empty() {
            let sev = item
                .get("finding")
                .and_then(|f| f.get("severity"))
                .and_then(JsValue::as_str);
            if !sev.is_some_and(|s| options.severities.iter().any(|x| x == s)) {
                bump(&mut skips, "severity-filtered", 1);
                continue;
            }
        }
        items.push(item);
    }

    // Grouping runs over EVERY recovered candidate, already-adjudicated ones
    // included: they still occupy their slot in the real dispatch.
    let mut batch_grouping = None;
    if let Some(min) = options.min_group_size {
        let refs: Vec<&JsValue> = items.iter().collect();
        let summary = group_corpus_for_batching(&refs, min)?;
        let keep: Vec<&String> = summary
            .groups
            .iter()
            .filter(|g| g.size >= min)
            .flat_map(|g| g.ids.iter())
            .collect();
        let before = items.len();
        items.retain(|i| {
            let id = i.get("id").map(JsValue::js_string).unwrap_or_default();
            keep.contains(&&id)
        });
        bump(&mut skips, "below-min-group-size", before - items.len());
        batch_grouping = Some(BatchGrouping::from(&summary));
    }
    if !options.exclude_ids.is_empty() {
        let before = items.len();
        items.retain(|i| {
            let id = i.get("id").map(JsValue::js_string).unwrap_or_default();
            !options.exclude_ids.contains(&id)
        });
        let dropped = before - items.len();
        if dropped > 0 {
            bump(&mut skips, "already-adjudicated", dropped);
        }
    }

    Ok(MineResult {
        corpus_root: options.root_display.clone(),
        project_slug_prefixes: prefixes,
        until: options.until.clone().filter(|u| !u.is_empty()),
        refuter_record_count: records.len(),
        recovered: items.len(),
        skips,
        batch_grouping,
        items,
    })
}

/// The ids already present in a checked-in corpus (lines that do not parse
/// contribute nothing).
///
/// # Errors
///
/// When the file cannot be read.
pub fn read_corpus_ids(path: &Path) -> Result<Vec<String>, String> {
    let raw = read_text(path).map_err(|e| {
        format!(
            "--exclude-corpus could not read \"{}\": {}",
            path.display(),
            io_message(&e)
        )
    })?;
    let mut ids = Vec::new();
    for entry in transcript_entries(&raw) {
        if let Some(id) = entry.get("id").and_then(Value::as_str)
            && !ids.iter().any(|x| x == id)
        {
            ids.push(id.to_owned());
        }
    }
    Ok(ids)
}

/// The JSONL body: one record per line, newline-terminated when non-empty.
pub fn jsonl(items: &[JsValue]) -> String {
    let mut body = items
        .iter()
        .map(JsValue::stringify)
        .collect::<Vec<_>>()
        .join("\n");
    if !items.is_empty() {
        body.push('\n');
    }
    body
}

/// The stderr accounting line.
pub fn summary_line(result: &MineResult) -> String {
    let mut skips: Vec<(&String, &usize)> = result.skips.iter().collect();
    skips.sort_by(|a, b| js_str_cmp(a.0, b.0));
    let joined = skips
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "mine-refuter-corpus: {} recovered of {} refuter record(s){}",
        result.recovered,
        result.refuter_record_count,
        if joined.is_empty() {
            String::new()
        } else {
            format!(" — skipped: {joined}")
        }
    )
}

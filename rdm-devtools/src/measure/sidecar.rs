//! Claude Code Workflow session sidecars: locate them, parse them, join each
//! agent with its transcript's deduped usage, and aggregate by token class.
//!
//! This is the read side every measurement instrument shares (it replaces
//! `scripts/lib/token-report.mjs`). The on-disk facts it relies on:
//!
//! - A `workflows/wf_*.json` sidecar's `runId` equals its filename stem, and
//!   its transcripts live at `subagents/workflows/<runId>/agent-<agentId>.jsonl`
//!   under the same session directory, using that full runId.
//! - A `workflow_agent` entry served from cache (`cached: true`) carries no
//!   `tokens` at all: it is zero-cost, not unmeasured.
//! - A sidecar's `startTime` is an epoch-millisecond number (its sibling
//!   `timestamp` is the ISO string); `--since`/`--until` compare against it.
//! - Inside a transcript the SAME `requestId` appears on several consecutive
//!   `assistant` lines while a request streams; only the last carries the final
//!   `output_tokens`. Usage is therefore deduped by `requestId`,
//!   **last write wins**, keeping the request's first position.
//! - `user` lines carry no `requestId`/`usage` and are skipped.
//!
//! Transcript parsing, `requestId` dedupe and the all-zero-usage exclusion are
//! owned by [`rdm_core::usage`]; this module reads the file and only adapts the
//! core's five integer token classes to the four `f64` report classes
//! (`cache_write` is the core's 5m + 1h cache write).
//!
//! Directory listings are sorted by name, so warnings and "first/last"
//! selections never depend on filesystem enumeration order.

use std::fs;
use std::path::{Path, PathBuf};

use rdm_core::{ParsedTranscript, TokenUsage};
use serde::Serialize;
use serde_json::Value;

use super::jsdate;
use super::jsjson::js_str_cmp;
use super::jsnum::{fmt_number, percentile, serialize_js_number};

/// The default sidecar root: `$HOME/.claude/projects`.
pub fn default_projects_root() -> PathBuf {
    let home = std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .unwrap_or_default();
    home.join(".claude").join("projects")
}

/// One session directory that carries a `workflows/` subdirectory.
#[derive(Debug, Clone)]
pub struct SessionDir {
    /// The project-slug directory name (for example `-Users-me-Projects-rdm`).
    pub project_slug: String,
    /// The session directory name.
    pub session_id: String,
    /// The session directory.
    pub session_dir: PathBuf,
}

fn sorted_entries(dir: &Path) -> std::io::Result<Vec<fs::DirEntry>> {
    let mut entries: Vec<fs::DirEntry> = fs::read_dir(dir)?.filter_map(Result::ok).collect();
    entries.sort_by_key(fs::DirEntry::file_name);
    Ok(entries)
}

/// Every session directory under `root` with a `workflows/` subdirectory,
/// across every project-slug directory (including `--worktrees-` ones: a
/// worktree gets its own project slug, so the session cannot be derived from
/// a working directory and must be found by walking).
///
/// # Errors
///
/// When `root` itself cannot be read, naming it.
pub fn locate_session_dirs(root: &Path) -> Result<Vec<SessionDir>, String> {
    let projects = sorted_entries(root).map_err(|e| {
        format!(
            "cannot read projects root \"{}\": {}",
            root.display(),
            io_message(&e)
        )
    })?;
    let mut sessions = Vec::new();
    for project in projects {
        if !project.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let project_slug = project.file_name().to_string_lossy().into_owned();
        let Ok(children) = sorted_entries(&project.path()) else {
            continue;
        };
        for session in children {
            if !session.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            let session_dir = session.path();
            if session_dir.join("workflows").is_dir() {
                sessions.push(SessionDir {
                    project_slug: project_slug.clone(),
                    session_id: session.file_name().to_string_lossy().into_owned(),
                    session_dir,
                });
            }
        }
    }
    Ok(sessions)
}

/// A readable I/O error message without Rust's `(os error N)` suffix.
pub fn io_message(e: &std::io::Error) -> String {
    let text = e.to_string();
    match text.find(" (os error") {
        Some(at) => text[..at].to_owned(),
        None => text,
    }
}

/// `String(v)` for a JSON value (`undefined` for an absent one).
pub fn js_display(v: Option<&Value>) -> String {
    match v {
        None => "undefined".to_owned(),
        Some(Value::Null) => "null".to_owned(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Number(n)) => fmt_number(n.as_f64().unwrap_or(f64::NAN)),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(a)) => a
            .iter()
            .map(|x| match x {
                Value::Null => String::new(),
                other => js_display(Some(other)),
            })
            .collect::<Vec<_>>()
            .join(","),
        Some(Value::Object(_)) => "[object Object]".to_owned(),
    }
}

/// JavaScript truthiness of an optional JSON value.
pub fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|x| x != 0.0 && !x.is_nan()),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(_) | Value::Object(_)) => true,
    }
}

/// One parsed `wf_*.json` sidecar.
#[derive(Debug, Clone)]
pub struct WorkflowRun {
    /// The sidecar file.
    pub file_path: PathBuf,
    /// `String(runId)`.
    pub run_id: String,
    /// The `workflowName` field, when present.
    pub workflow_name: Option<Value>,
    /// `totalTokens` when it is a number, else 0.
    pub total_tokens: f64,
    /// The run's start instant in epoch milliseconds, when recoverable.
    pub start_time_ms: Option<f64>,
    /// The `workflowProgress` entries whose `type` is `workflow_agent`.
    pub agents: Vec<Value>,
}

/// Parses one sidecar. A corrupt file (a partial write mid-session) is an
/// `Err` naming why, so the caller can skip it rather than abort the pass.
///
/// # Errors
///
/// When the file cannot be read or is not JSON.
pub fn parse_workflow_run(path: &Path) -> Result<WorkflowRun, String> {
    let bytes = fs::read(path).map_err(|e| format!("read failed: {}", io_message(&e)))?;
    let raw = String::from_utf8_lossy(&bytes);
    let data: Value = serde_json::from_str(&raw).map_err(|e| format!("JSON parse failed: {e}"))?;
    let start = match data.get("startTime") {
        Some(Value::Number(n)) => n.as_f64(),
        other => {
            let source = match other {
                Some(v) if !v.is_null() => Some(v),
                _ => data.get("timestamp").filter(|v| !v.is_null()),
            };
            source.and_then(|v| jsdate::parse(&js_display(Some(v))))
        }
    };
    let agents = data
        .get("workflowProgress")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter(|e| e.get("type").and_then(Value::as_str) == Some("workflow_agent"))
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    Ok(WorkflowRun {
        file_path: path.to_owned(),
        run_id: js_display(data.get("runId")),
        workflow_name: data.get("workflowName").cloned(),
        total_tokens: data
            .get("totalTokens")
            .and_then(Value::as_f64)
            .unwrap_or(0.0),
        start_time_ms: start.filter(|t| t.is_finite()),
        agents,
    })
}

/// One sidecar found under a session directory.
#[derive(Debug, Clone)]
pub struct RunFile {
    /// The project slug.
    pub project_slug: String,
    /// The session id.
    pub session_id: String,
    /// The session directory (where its transcripts live).
    pub session_dir: PathBuf,
    /// The parsed run.
    pub run: WorkflowRun,
}

impl RunFile {
    /// `projectSlug|sessionId|runId`, the run's identity across the corpus.
    pub fn run_key(&self) -> String {
        format!(
            "{}|{}|{}",
            self.project_slug, self.session_id, self.run.run_id
        )
    }
}

/// Run filters. `workflow_names` is OR'd: a run matches when its
/// `workflowName` is any one of them (unlike rdm's AND-ed `--tag`).
#[derive(Debug, Clone, Default)]
pub struct RunFilters {
    /// Keep only runs starting at or after this date (`Date.parse` string).
    pub since: Option<String>,
    /// Keep only runs with one of these workflow names (empty: all).
    pub workflow_names: Vec<String>,
}

/// Every `workflows/wf_*.json` under `sessions`, parsed and filtered. An
/// unparsable sidecar is skipped with a warning.
///
/// # Errors
///
/// When `since` is not a parseable date.
pub fn find_workflow_run_files(
    sessions: &[SessionDir],
    filters: &RunFilters,
    warn: &mut dyn FnMut(String),
) -> Result<Vec<RunFile>, String> {
    let since_ms = match filters.since.as_deref().filter(|s| !s.is_empty()) {
        Some(since) => Some(
            jsdate::parse(since)
                .ok_or_else(|| format!("--since value is not a parseable date: \"{since}\""))?,
        ),
        None => None,
    };
    let mut out = Vec::new();
    for s in sessions {
        let Ok(entries) = sorted_entries(&s.session_dir.join("workflows")) else {
            continue;
        };
        for entry in entries {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.starts_with("wf_") || !name.ends_with(".json") {
                continue;
            }
            let path = entry.path();
            let run = match parse_workflow_run(&path) {
                Ok(run) => run,
                Err(e) => {
                    warn(format!(
                        "skipping unparsable workflow run {}: {e}",
                        path.display()
                    ));
                    continue;
                }
            };
            if !filters.workflow_names.is_empty() {
                let matches = run
                    .workflow_name
                    .as_ref()
                    .and_then(Value::as_str)
                    .is_some_and(|n| filters.workflow_names.iter().any(|w| w == n));
                if !matches {
                    continue;
                }
            }
            if let Some(since) = since_ms
                && run.start_time_ms.is_none_or(|t| t < since)
            {
                continue;
            }
            out.push(RunFile {
                project_slug: s.project_slug.clone(),
                session_id: s.session_id.clone(),
                session_dir: s.session_dir.clone(),
                run,
            });
        }
    }
    Ok(out)
}

/// Keeps only runs starting at or before `until` (a `Date.parse` string).
///
/// # Errors
///
/// When `until` is not a parseable date.
pub fn filter_until(runs: Vec<RunFile>, until: &str) -> Result<Vec<RunFile>, String> {
    let until_ms = jsdate::parse(until)
        .ok_or_else(|| format!("--until value is not a parseable date: \"{until}\""))?;
    Ok(runs
        .into_iter()
        .filter(|rf| rf.run.start_time_ms.is_some_and(|t| t <= until_ms))
        .collect())
}

/// The agent-class rollup key: the label's first colon segment
/// (`refute:plan:coherence-1` → `refute`); a colon-less label is its own
/// class.
pub fn agent_class_from_label(label: &str) -> String {
    match label.find(':') {
        Some(at) => label[..at].to_owned(),
        None => label.to_owned(),
    }
}

/// The transcript file for one agent of one run (full runId, `wf_` included).
pub fn transcript_path_for(session_dir: &Path, run_id: &str, agent_id: &str) -> PathBuf {
    session_dir
        .join("subagents")
        .join("workflows")
        .join(run_id)
        .join(format!("agent-{agent_id}.jsonl"))
}

/// Splits a transcript into its non-blank, JSON-parseable lines.
pub fn transcript_entries(raw: &str) -> impl Iterator<Item = Value> + '_ {
    raw.split('\n').filter_map(|line| {
        let trimmed = super::jsnum::js_trim(line);
        if trimmed.is_empty() {
            None
        } else {
            serde_json::from_str(trimmed).ok()
        }
    })
}

/// Reads a file as UTF-8, replacing invalid sequences (as Node does).
///
/// # Errors
///
/// The read error.
pub fn read_text(path: &Path) -> std::io::Result<String> {
    fs::read(path).map(|b| String::from_utf8_lossy(&b).into_owned())
}

/// Reads and parses one `agent-*.jsonl` transcript with
/// [`rdm_core::parse_transcript`]: deduped requests (last write wins, first
/// position kept), with all-zero requests excluded and reported as warnings.
///
/// # Errors
///
/// The read error's message when the file cannot be read.
pub fn parse_agent_transcript(path: &Path) -> Result<ParsedTranscript, String> {
    let raw = read_text(path).map_err(|e| io_message(&e))?;
    Ok(rdm_core::parse_transcript(&raw))
}

/// One agent of one run, joined with its transcript's deduped usage.
#[derive(Debug, Clone)]
pub struct AgentRecord {
    /// The project slug.
    pub project_slug: String,
    /// The session id.
    pub session_id: String,
    /// The run id.
    pub run_id: String,
    /// `String(agentId)` when truthy.
    pub agent_id: Option<String>,
    /// The full label.
    pub label: String,
    /// The label's first colon segment.
    pub agent_class: String,
    /// The model, `unknown` when absent.
    pub model: String,
    /// The run's workflow name, `unknown` when absent.
    pub workflow_name: String,
    /// Always 1 (summed by aggregation).
    pub agent_count: f64,
    /// Output tokens.
    pub output: f64,
    /// Uncached input tokens.
    pub uncached_input: f64,
    /// Cache-write tokens.
    pub cache_write: f64,
    /// Cache-read tokens.
    pub cache_read: f64,
    /// Distinct requests in the transcript (0 for cached/sidecar-only).
    pub deduped_request_count: f64,
    /// The record fell back to the sidecar's `tokens` scalar.
    pub sidecar_only: bool,
    /// The agent was served from cache (zero-cost).
    pub cached: bool,
    /// The first request's input + cache-write + cache-read (output excluded);
    /// `None` for cached and sidecar-only records, which have no first request
    /// to measure (never 0, which would drag a floor down).
    pub first_request_tokens: Option<f64>,
}

impl AgentRecord {
    /// `projectSlug|sessionId|runId`.
    pub fn run_key(&self) -> String {
        format!("{}|{}|{}", self.project_slug, self.session_id, self.run_id)
    }

    /// All four token classes summed.
    pub fn all_tokens(&self) -> f64 {
        self.output + self.uncached_input + self.cache_write + self.cache_read
    }
}

fn value_or_unknown(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => "unknown".to_owned(),
        other => js_display(other),
    }
}

/// Joins every `workflow_agent` entry of `runs` with its transcript. Three
/// cases per agent, in order: cached (zero-cost); a transcript with at least
/// one deduped request (per-class breakdown); otherwise a sidecar-only
/// fallback whose `tokens` scalar is attributed to `output`, with a warning
/// that distinguishes a missing agent id, a missing transcript, an unreadable
/// one and an empty one.
pub fn build_records(runs: &[RunFile], warn: &mut dyn FnMut(String)) -> Vec<AgentRecord> {
    let mut records = Vec::new();
    for rf in runs {
        let run = &rf.run;
        for agent in &run.agents {
            let label = match agent.get("label") {
                Some(Value::String(s)) => s.clone(),
                other => value_or_unknown(other),
            };
            let agent_id = truthy(agent.get("agentId")).then(|| js_display(agent.get("agentId")));
            let mut rec = AgentRecord {
                project_slug: rf.project_slug.clone(),
                session_id: rf.session_id.clone(),
                run_id: run.run_id.clone(),
                agent_id: agent_id.clone(),
                agent_class: agent_class_from_label(&label),
                label: label.clone(),
                model: value_or_unknown(agent.get("model")),
                workflow_name: value_or_unknown(run.workflow_name.as_ref()),
                agent_count: 1.0,
                output: 0.0,
                uncached_input: 0.0,
                cache_write: 0.0,
                cache_read: 0.0,
                deduped_request_count: 0.0,
                sidecar_only: false,
                cached: false,
                first_request_tokens: None,
            };
            if truthy(agent.get("cached")) {
                rec.cached = true;
                records.push(rec);
                continue;
            }
            let path = agent_id
                .as_deref()
                .map(|id| transcript_path_for(&rf.session_dir, &run.run_id, id));
            let transcript = path
                .as_deref()
                .filter(|p| p.exists())
                .map(parse_agent_transcript);
            if let (Some(id), Some(Ok(t))) = (&agent_id, &transcript) {
                for w in &t.warnings {
                    warn(format!(
                        "transcript for agent {id} in run {}: {w}",
                        run.run_id
                    ));
                }
            }
            match &transcript {
                Some(Ok(t)) if !t.requests.is_empty() => {
                    let first = t.requests[0].usage;
                    let sum: TokenUsage = t.requests.iter().map(|r| r.usage).sum();
                    // u64 -> f64 is exact below 2^53, far above any token count.
                    #[allow(clippy::cast_precision_loss)]
                    {
                        rec.first_request_tokens = Some(
                            first
                                .input
                                .saturating_add(first.cache_write())
                                .saturating_add(first.cache_read)
                                as f64,
                        );
                        rec.output = sum.output as f64;
                        rec.uncached_input = sum.input as f64;
                        rec.cache_write = sum.cache_write() as f64;
                        rec.cache_read = sum.cache_read as f64;
                        rec.deduped_request_count = t.requests.len() as f64;
                    }
                }
                _ => {
                    match (&agent_id, &transcript) {
                        (None, _) => warn(format!(
                            "agent at label \"{label}\" in run {} has no agentId; cannot locate a transcript",
                            run.run_id
                        )),
                        (Some(id), None) => warn(format!(
                            "no transcript found for agent {id} in run {}; falling back to sidecar tokens",
                            run.run_id
                        )),
                        (Some(id), Some(Err(e))) => warn(format!(
                            "transcript for agent {id} in run {} failed to read: {e}; falling back to sidecar tokens",
                            run.run_id
                        )),
                        (Some(id), Some(Ok(_))) => warn(format!(
                            "transcript for agent {id} in run {} contained no usable assistant/usage lines (empty transcript); falling back to sidecar tokens",
                            run.run_id
                        )),
                    }
                    rec.output = agent.get("tokens").and_then(Value::as_f64).unwrap_or(0.0);
                    rec.sidecar_only = true;
                }
            }
            records.push(rec);
        }
    }
    records
}

/// One aggregation bucket: counts and the four token classes.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Bucket {
    /// The grouping key.
    pub key: String,
    /// Agents in the bucket.
    #[serde(serialize_with = "serialize_js_number")]
    pub agent_count: f64,
    /// Deduped requests in the bucket.
    #[serde(serialize_with = "serialize_js_number")]
    pub deduped_request_count: f64,
    /// Output tokens.
    #[serde(serialize_with = "serialize_js_number")]
    pub output: f64,
    /// Uncached input tokens.
    #[serde(serialize_with = "serialize_js_number")]
    pub uncached_input: f64,
    /// Cache-write tokens.
    #[serde(serialize_with = "serialize_js_number")]
    pub cache_write: f64,
    /// Cache-read tokens.
    #[serde(serialize_with = "serialize_js_number")]
    pub cache_read: f64,
}

impl Bucket {
    /// An empty bucket.
    pub fn empty(key: &str) -> Self {
        Self {
            key: key.to_owned(),
            agent_count: 0.0,
            deduped_request_count: 0.0,
            output: 0.0,
            uncached_input: 0.0,
            cache_write: 0.0,
            cache_read: 0.0,
        }
    }

    /// All four token classes summed.
    pub fn all_tokens(&self) -> f64 {
        self.output + self.uncached_input + self.cache_write + self.cache_read
    }

    /// Output + uncached input + cache write ("fresh": cache reads excluded).
    pub fn fresh_tokens(&self) -> f64 {
        self.output + self.uncached_input + self.cache_write
    }
}

/// Buckets `records` by `key`, summing every count and token class; sorted by
/// key (JavaScript string order) for deterministic output.
pub fn aggregate<'a, I>(records: I, key: impl Fn(&AgentRecord) -> String) -> Vec<Bucket>
where
    I: IntoIterator<Item = &'a AgentRecord>,
{
    aggregate_pairs(records.into_iter().map(|r| (key(r), r)))
}

/// [`aggregate`] over precomputed `(key, record)` pairs.
pub fn aggregate_pairs<'a, I>(pairs: I) -> Vec<Bucket>
where
    I: IntoIterator<Item = (String, &'a AgentRecord)>,
{
    let mut buckets: Vec<Bucket> = Vec::new();
    for (k, r) in pairs {
        let at = match buckets.iter().position(|b| b.key == k) {
            Some(at) => at,
            None => {
                buckets.push(Bucket::empty(&k));
                buckets.len() - 1
            }
        };
        let b = &mut buckets[at];
        b.agent_count += r.agent_count;
        b.deduped_request_count += r.deduped_request_count;
        b.output += r.output;
        b.uncached_input += r.uncached_input;
        b.cache_write += r.cache_write;
        b.cache_read += r.cache_read;
    }
    buckets.sort_by(|a, b| js_str_cmp(&a.key, &b.key));
    buckets
}

/// A per-agent-class first-request floor.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Floor {
    /// The agent class.
    pub key: String,
    /// Eligible records.
    #[serde(serialize_with = "serialize_js_number")]
    pub n: f64,
    /// Minimum first-request tokens.
    #[serde(serialize_with = "serialize_js_number")]
    pub min_tokens: f64,
    /// 10th percentile.
    #[serde(serialize_with = "serialize_js_number")]
    pub p10_tokens: f64,
    /// Median.
    #[serde(serialize_with = "serialize_js_number")]
    pub median_tokens: f64,
    /// Mean.
    #[serde(serialize_with = "serialize_js_number")]
    pub mean_tokens: f64,
}

/// The first-request floor per agent class over records that HAVE a
/// `first_request_tokens`; a class whose every record is cached or
/// sidecar-only is omitted entirely (no `n: 0` row).
pub fn floor_by_agent_class<'a, I>(records: I) -> Vec<Floor>
where
    I: IntoIterator<Item = &'a AgentRecord>,
{
    let mut by_class: Vec<(String, Vec<f64>)> = Vec::new();
    for r in records {
        let Some(tokens) = r.first_request_tokens else {
            continue;
        };
        match by_class.iter_mut().find(|(k, _)| *k == r.agent_class) {
            Some(slot) => slot.1.push(tokens),
            None => by_class.push((r.agent_class.clone(), vec![tokens])),
        }
    }
    let mut out: Vec<Floor> = by_class
        .into_iter()
        .map(|(key, values)| {
            let mut sorted = values;
            sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            #[allow(clippy::cast_precision_loss)]
            let n = sorted.len() as f64;
            let sum: f64 = sorted.iter().sum();
            Floor {
                key,
                n,
                min_tokens: sorted[0],
                p10_tokens: percentile(&sorted, 0.1),
                median_tokens: percentile(&sorted, 0.5),
                mean_tokens: sum / n,
            }
        })
        .collect();
    out.sort_by(|a, b| js_str_cmp(&a.key, &b.key));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, body: &str) {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::write(path, body);
    }

    #[test]
    fn request_id_dedupe_last_write_wins() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("tempdir: {e}"));
        let path = dir.path().join("agent-x.jsonl");
        write(
            &path,
            concat!(
                r#"{"type":"assistant","requestId":"req-A","message":{"usage":{"input_tokens":100,"output_tokens":10}}}"#,
                "\n",
                r#"{"type":"user","message":{"content":"x"}}"#,
                "\n",
                r#"{"type":"assistant","requestId":"req-B","message":{"usage":{"input_tokens":80,"output_tokens":120}}}"#,
                "\n",
                r#"{"type":"assistant","requestId":"req-A","message":{"usage":{"input_tokens":100,"output_tokens":50}}}"#,
                "\nnot json\n\n"
            ),
        );
        let t = parse_agent_transcript(&path).unwrap_or_default();
        assert_eq!(
            t.requests.len(),
            2,
            "two distinct requestIds, not three lines"
        );
        assert_eq!(
            t.requests[0].request_id, "req-A",
            "req-A keeps its first position"
        );
        assert_eq!(
            t.requests[0].usage.output, 50,
            "req-A resolves to the LAST line's output (50), not the first (10) or a sum (60)"
        );
        assert_eq!(t.requests[1].usage.output, 120);
    }

    fn run(session_dir: &Path, agents: Value) -> RunFile {
        RunFile {
            project_slug: "synthetic".to_owned(),
            session_id: "sess".to_owned(),
            session_dir: session_dir.to_owned(),
            run: WorkflowRun {
                file_path: PathBuf::new(),
                run_id: "run-synth".to_owned(),
                workflow_name: Some(Value::String("wf".to_owned())),
                total_tokens: 0.0,
                start_time_ms: None,
                agents: agents.as_array().cloned().unwrap_or_default(),
            },
        }
    }

    #[test]
    fn unreadable_and_empty_transcripts_warn_distinctly() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("tempdir: {e}"));
        let tdir = dir.path().join("subagents/workflows/run-synth");
        let unreadable = tdir.join("agent-unreadable.jsonl");
        write(
            &unreadable,
            r#"{"type":"assistant","requestId":"r","message":{"usage":{"output_tokens":1}}}"#,
        );
        let _ = fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o000));
        write(
            &tdir.join("agent-empty.jsonl"),
            r#"{"type":"user","message":{"role":"user","content":"no assistant lines here"}}"#,
        );
        let runs = [run(
            dir.path(),
            serde_json::json!([
                {"label": "find:unreadable", "agentId": "unreadable", "model": "m", "tokens": 77},
                {"label": "find:empty", "agentId": "empty", "model": "m", "tokens": 55},
                {"label": "find:absent", "agentId": "absent", "model": "m", "tokens": 33},
                {"label": "find:anon", "model": "m", "tokens": 11}
            ]),
        )];
        let mut warnings = Vec::new();
        let records = build_records(&runs, &mut |w| warnings.push(w));
        let _ = fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o600));
        assert_eq!(records.len(), 4);
        assert!(
            records.iter().all(|r| r.sidecar_only),
            "every case degrades, none throws"
        );
        assert_eq!(
            records[0].output, 77.0,
            "the fallback uses the sidecar tokens scalar"
        );
        assert_eq!(records[1].output, 55.0);
        assert!(warnings[0].contains("failed to read"), "{warnings:?}");
        assert!(warnings[1].contains("empty transcript"), "{warnings:?}");
        assert!(warnings[2].contains("no transcript found"), "{warnings:?}");
        assert!(warnings[3].contains("has no agentId"), "{warnings:?}");
        let distinct: std::collections::BTreeSet<&str> = warnings
            .iter()
            .map(|w| w.split(" in run ").nth(1).unwrap_or(w))
            .collect();
        assert_eq!(
            distinct.len(),
            4,
            "each degraded case has its own wording: {warnings:?}"
        );
    }

    #[test]
    fn all_zero_only_transcript_falls_back_to_sidecar_tokens() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("tempdir: {e}"));
        let tdir = dir.path().join("subagents/workflows/run-synth");
        write(
            &tdir.join("agent-zero.jsonl"),
            concat!(
                r#"{"type":"assistant","requestId":"z1","message":{"model":"m","usage":{"input_tokens":0,"output_tokens":0,"cache_creation_input_tokens":0,"cache_read_input_tokens":0}}}"#,
                "\n",
                r#"{"type":"assistant","requestId":"z2","message":{"model":"<synthetic>","usage":{}}}"#,
                "\n"
            ),
        );
        let runs = [run(
            dir.path(),
            serde_json::json!([
                {"label": "find:zero", "agentId": "zero", "model": "m", "tokens": 42}
            ]),
        )];
        let mut warnings = Vec::new();
        let records = build_records(&runs, &mut |w| warnings.push(w));
        assert_eq!(records.len(), 1);
        assert!(records[0].sidecar_only, "no measured request: sidecar-only");
        assert_eq!(records[0].output, 42.0, "falls back to the sidecar tokens");
        assert_eq!(records[0].first_request_tokens, None);
        assert_eq!(records[0].deduped_request_count, 0.0);
        assert!(
            warnings.iter().any(|w| w.contains("empty transcript")),
            "{warnings:?}"
        );
        assert_eq!(
            warnings.iter().filter(|w| w.contains("all-zero")).count(),
            2,
            "each all-zero request is reported: {warnings:?}"
        );
    }

    #[test]
    fn first_request_tokens_null_for_cached_and_sidecar_only() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("tempdir: {e}"));
        let tdir = dir.path().join("subagents/workflows/run-synth");
        write(
            &tdir.join("agent-multi.jsonl"),
            concat!(
                r#"{"type":"assistant","requestId":"a","message":{"usage":{"input_tokens":100,"cache_creation_input_tokens":500,"cache_read_input_tokens":200,"output_tokens":9}}}"#,
                "\n",
                r#"{"type":"assistant","requestId":"b","message":{"usage":{"input_tokens":80,"cache_read_input_tokens":300,"output_tokens":9}}}"#,
                "\n"
            ),
        );
        let runs = [run(
            dir.path(),
            serde_json::json!([
                {"label": "fetch:x", "agentId": "multi", "model": "m"},
                {"label": "stamp:x", "agentId": "c", "model": "m", "cached": true},
                {"label": "gate:x", "agentId": "gone", "model": "m", "tokens": 400}
            ]),
        )];
        let records = build_records(&runs, &mut |_| {});
        assert_eq!(
            records[0].first_request_tokens,
            Some(800.0),
            "the FIRST request only (100+500+200), not the second (380) or the sum"
        );
        assert_eq!(records[1].first_request_tokens, None, "cached: None, not 0");
        assert_eq!(
            records[2].first_request_tokens, None,
            "sidecar-only: None, not 0 or 400"
        );
        let floor = floor_by_agent_class(&records);
        assert_eq!(
            floor.len(),
            1,
            "cached/sidecar-only classes are omitted, not n:0"
        );
        assert_eq!(floor[0].key, "fetch");
    }

    #[test]
    fn floor_interpolates_across_a_multi_record_class() {
        let base = AgentRecord {
            project_slug: String::new(),
            session_id: String::new(),
            run_id: String::new(),
            agent_id: None,
            label: "synthtest".to_owned(),
            agent_class: "synthtest".to_owned(),
            model: String::new(),
            workflow_name: String::new(),
            agent_count: 1.0,
            output: 0.0,
            uncached_input: 0.0,
            cache_write: 0.0,
            cache_read: 0.0,
            deduped_request_count: 0.0,
            sidecar_only: false,
            cached: false,
            first_request_tokens: None,
        };
        let records: Vec<AgentRecord> = [40.0, 10.0, 30.0, 20.0]
            .into_iter()
            .map(|v| AgentRecord {
                first_request_tokens: Some(v),
                ..base.clone()
            })
            .collect();
        let floor = floor_by_agent_class(&records);
        assert_eq!(floor[0].n, 4.0);
        assert_eq!(floor[0].min_tokens, 10.0);
        assert_eq!(floor[0].p10_tokens, 13.0, "10 + (20-10)*0.3");
        assert_eq!(floor[0].median_tokens, 25.0, "20 + (30-20)*0.5");
        assert_eq!(floor[0].mean_tokens, 25.0);
        assert_eq!(agent_class_from_label("malformed"), "malformed");
        assert_eq!(agent_class_from_label(""), "");
    }
}

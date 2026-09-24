//! Locating a Claude Code session on disk and building its anchored token
//! ledger.
//!
//! This module holds every domain rule about the Claude Code projects tree
//! (`<config>/projects/`): which files make up a session, how each spend
//! source is anchored to the main transcript, and how requests are deduped
//! across sources. It performs no I/O itself. Reading goes through the
//! [`TranscriptSource`] trait, whose implementations only know how to list a
//! directory and read a file: [`MemoryTranscriptSource`] here, and the
//! filesystem implementation in the `rdm-transcript` crate.
//!
//! # Layout
//!
//! ```text
//! <projects root>/<project-slug>/<session>.jsonl                       main transcript
//! <projects root>/<project-slug>/<session>/subagents/agent-<id>.jsonl  Agent subagent
//! <projects root>/<project-slug>/<session>/subagents/agent-<id>.meta.json
//! <projects root>/<project-slug>/<session>/workflows/<runId>.json      Workflow sidecar
//! <projects root>/<project-slug>/<session>/subagents/workflows/<runId>/agent-<id>.jsonl
//! ```
//!
//! A worktree gets its own project-slug directory, so a session is located by
//! walking every slug ([`locate_session`]), never by deriving a slug from a
//! working directory.
//!
//! # Spend sources and anchors
//!
//! [`reap_session`] builds one [`SourceReport`] per spend source, each
//! anchored to a timestamp from the main transcript:
//!
//! - **main**: the main transcript's own requests; the source's anchor is its
//!   first timestamped line.
//! - **agent**: an `Agent` subagent, anchored to the main-transcript line
//!   holding the `tool_use` block whose `id` is the subagent's `toolUseId`. A
//!   nested subagent (its `toolUseId` names a `tool_use` in its parent's
//!   transcript) follows `parentAgentId` through the sibling `.meta.json`
//!   files to the first ancestor that is anchored. The walk stops on a cycle
//!   or a missing parent.
//! - **workflow_agent**: an agent of a Workflow run, anchored to the
//!   main-transcript `tool_result` line whose `toolUseResult.runId` names the
//!   run. Its label comes from the sidecar's `workflowProgress[]`.
//!
//! A source whose anchor cannot be found is kept and counted, flagged
//! `anchored: false`, and listed in [`SessionReport::unanchored`].
//!
//! # What is counted
//!
//! Every transcript is parsed with [`parse_transcript`], so the usage module's
//! rules apply per transcript (last-write-wins dedupe by `requestId`, all-zero
//! requests excluded with a warning). Across sources, a `requestId` is owned by
//! the first source that counts it in canonical order (main, then `Agent`
//! subagents by id, then Workflow runs by run id and each run's agents by id);
//! a later copy is dropped and recorded in [`SourceReport::duplicates_dropped`].
//! A `cached: true` Workflow agent costs zero. The sidecar `tokens` and
//! `totalTokens` figures are never counted.
//!
//! Nesting below a Workflow agent has never been observed on disk, so a
//! session with any Workflow run carries the [`WORKFLOW_NESTED_UNVERIFIED`]
//! warning.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Serialize, Serializer};
use serde_json::Value;

use crate::usage::{ModelUsage, RequestUsage, TokenUsage, UsageLedger, parse_transcript};

/// The prefix of every Workflow run id (`wf_<…>`).
pub const WORKFLOW_RUN_PREFIX: &str = "wf_";

/// The leading text of the warning every report of a session with a Workflow
/// run carries: nesting below a Workflow agent is unobserved, so its spend is
/// not located.
pub const WORKFLOW_NESTED_UNVERIFIED: &str = "workflow-nested agents unverified";

/// A validated path relative to a [`TranscriptSource`]'s root.
///
/// Every segment is non-empty, is not `.` or `..`, and holds no `/`, `\` or
/// NUL. That keeps a user-supplied session uuid or run id from escaping the
/// projects tree.
///
/// # Examples
///
/// ```
/// use rdm_core::transcript::TranscriptPath;
///
/// let p = TranscriptPath::parse("-proj/abc.jsonl")?;
/// assert_eq!(p.to_string(), "-proj/abc.jsonl");
/// assert!(TranscriptPath::root().join("../etc").is_err());
/// assert!(TranscriptPath::root().join("a/b").is_err());
/// # Ok::<(), rdm_core::transcript::TranscriptError>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TranscriptPath {
    segments: Vec<String>,
}

fn check_segment(segment: &str) -> Result<(), TranscriptError> {
    let reason = if segment.is_empty() {
        "it is empty"
    } else if segment == "." || segment == ".." {
        "it is a relative path component"
    } else if segment.contains(['/', '\\', '\0']) {
        "it contains a path separator"
    } else {
        return Ok(());
    };
    Err(TranscriptError::InvalidId {
        id: segment.to_owned(),
        reason: reason.to_owned(),
    })
}

impl TranscriptPath {
    /// The root of the source: the projects directory itself.
    #[must_use]
    pub fn root() -> Self {
        Self::default()
    }

    /// Parses a `/`-separated relative path. The empty string is the root.
    ///
    /// # Errors
    ///
    /// Returns [`TranscriptError::InvalidId`] when a segment is empty, `.`,
    /// `..`, or holds a `\` or NUL.
    pub fn parse(path: &str) -> Result<Self, TranscriptError> {
        if path.is_empty() {
            return Ok(Self::root());
        }
        let mut out = Self::root();
        for segment in path.split('/') {
            out = out.join(segment)?;
        }
        Ok(out)
    }

    /// This path with one more segment appended.
    ///
    /// # Errors
    ///
    /// Returns [`TranscriptError::InvalidId`] when `segment` is empty, `.`,
    /// `..`, or holds a `/`, `\` or NUL.
    pub fn join(&self, segment: &str) -> Result<Self, TranscriptError> {
        check_segment(segment)?;
        let mut segments = self.segments.clone();
        segments.push(segment.to_owned());
        Ok(Self { segments })
    }

    /// The path's segments, outermost first.
    #[must_use]
    pub fn segments(&self) -> &[String] {
        &self.segments
    }
}

impl fmt::Display for TranscriptPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.segments.join("/"))
    }
}

/// Whether a [`TranscriptEntry`] is a file or a directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    /// A regular file.
    File,
    /// A directory.
    Dir,
}

/// One entry of a directory listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptEntry {
    /// The entry's name within its directory.
    pub name: String,
    /// File or directory.
    pub kind: EntryKind,
}

/// A read-only view of a Claude Code projects tree.
///
/// Implementations know only how to list a directory and read a file; the
/// layout rules live in this module's functions.
pub trait TranscriptSource {
    /// The projects root as a user would recognise it, used in errors and
    /// warnings.
    fn root_display(&self) -> String;

    /// The entries of the directory `dir`, sorted by name.
    ///
    /// # Errors
    ///
    /// A missing directory is not an error: it lists as empty. Returns
    /// [`TranscriptError::Io`] when an existing directory cannot be read.
    fn list(&self, dir: &TranscriptPath) -> Result<Vec<TranscriptEntry>, TranscriptError>;

    /// The text of the file `file`, with invalid UTF-8 replaced lossily.
    ///
    /// # Errors
    ///
    /// Returns [`TranscriptError::Io`] when the file is missing or cannot be
    /// read.
    fn read(&self, file: &TranscriptPath) -> Result<String, TranscriptError>;
}

/// A complete in-memory [`TranscriptSource`]: a map of file path to contents,
/// with directories derived from the file paths.
///
/// # Examples
///
/// ```
/// use rdm_core::transcript::{MemoryTranscriptSource, TranscriptPath, TranscriptSource};
///
/// let src = MemoryTranscriptSource::new()
///     .with_file("-proj/s1.jsonl", "")
///     .with_file("-proj/s1/subagents/agent-a.jsonl", "");
/// let names: Vec<String> = src
///     .list(&TranscriptPath::parse("-proj")?)?
///     .into_iter()
///     .map(|e| e.name)
///     .collect();
/// assert_eq!(names, ["s1", "s1.jsonl"]);
/// # Ok::<(), rdm_core::transcript::TranscriptError>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryTranscriptSource {
    files: BTreeMap<TranscriptPath, String>,
}

impl MemoryTranscriptSource {
    /// An empty projects tree.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Stores `contents` at `path`, replacing any previous contents.
    pub fn insert(&mut self, path: TranscriptPath, contents: impl Into<String>) {
        self.files.insert(path, contents.into());
    }

    /// Builder form of [`insert`](Self::insert) taking a `/`-separated path.
    ///
    /// # Panics
    ///
    /// Panics when `path` is not a valid [`TranscriptPath`]; this is a
    /// fixture builder, and a bad literal path is a bug in the fixture.
    #[must_use]
    pub fn with_file(mut self, path: &str, contents: impl Into<String>) -> Self {
        match TranscriptPath::parse(path) {
            Ok(p) => self.insert(p, contents),
            Err(e) => panic!("invalid fixture path {path:?}: {e}"),
        }
        self
    }
}

impl TranscriptSource for MemoryTranscriptSource {
    fn root_display(&self) -> String {
        "memory:".to_owned()
    }

    fn list(&self, dir: &TranscriptPath) -> Result<Vec<TranscriptEntry>, TranscriptError> {
        let depth = dir.segments.len();
        let mut entries: BTreeMap<&str, EntryKind> = BTreeMap::new();
        for path in self.files.keys() {
            if path.segments.len() > depth && path.segments[..depth] == dir.segments[..] {
                let kind = if path.segments.len() == depth + 1 {
                    EntryKind::File
                } else {
                    EntryKind::Dir
                };
                entries.insert(path.segments[depth].as_str(), kind);
            }
        }
        Ok(entries
            .into_iter()
            .map(|(name, kind)| TranscriptEntry {
                name: name.to_owned(),
                kind,
            })
            .collect())
    }

    fn read(&self, file: &TranscriptPath) -> Result<String, TranscriptError> {
        self.files
            .get(file)
            .cloned()
            .ok_or_else(|| TranscriptError::Io {
                path: format!("{}/{file}", self.root_display()),
                message: "no such file".to_owned(),
            })
    }
}

/// An error locating or reading a session.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TranscriptError {
    /// No project-slug directory holds the session.
    SessionNotFound {
        /// The session uuid that was looked up.
        session_id: String,
        /// The projects root that was searched.
        root: String,
        /// Every project-slug directory searched, in order.
        searched: Vec<String>,
    },
    /// No session holds a sidecar for the Workflow run.
    WorkflowRunNotFound {
        /// The run id that was looked up, with its `wf_` prefix.
        run_id: String,
        /// The projects root that was searched.
        root: String,
        /// Every project-slug directory searched, in order.
        searched: Vec<String>,
    },
    /// The run's sidecar was found but could not be read as a run.
    WorkflowRunUnreadable {
        /// The run id, with its `wf_` prefix.
        run_id: String,
        /// The session whose `workflows/` directory holds the sidecar.
        session_id: String,
    },
    /// A session uuid, run id or path segment that would not stay inside the
    /// projects tree.
    InvalidId {
        /// The rejected value.
        id: String,
        /// Why it was rejected.
        reason: String,
    },
    /// A directory or file that exists could not be read.
    Io {
        /// The path, including the projects root.
        path: String,
        /// The underlying error.
        message: String,
    },
}

fn write_searched(f: &mut fmt::Formatter<'_>, root: &str, searched: &[String]) -> fmt::Result {
    if searched.is_empty() {
        write!(
            f,
            "; the projects directory {root} holds no project directories"
        )
    } else {
        write!(
            f,
            "; searched {} project director{} under {root}:",
            searched.len(),
            if searched.len() == 1 { "y" } else { "ies" }
        )?;
        for slug in searched {
            write!(f, "\n  {slug}")?;
        }
        Ok(())
    }
}

const ROOT_HINT: &str = "check the id, or set CLAUDE_CONFIG_DIR if Claude Code keeps its data somewhere other than ~/.claude";

impl fmt::Display for TranscriptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TranscriptError::SessionNotFound {
                session_id,
                root,
                searched,
            } => {
                write!(f, "session {session_id} was not found")?;
                write_searched(f, root, searched)?;
                write!(f, "\n{ROOT_HINT}")
            }
            TranscriptError::WorkflowRunNotFound {
                run_id,
                root,
                searched,
            } => {
                write!(f, "Workflow run {run_id} was not found in any session")?;
                write_searched(f, root, searched)?;
                write!(f, "\n{ROOT_HINT}")
            }
            TranscriptError::WorkflowRunUnreadable { run_id, session_id } => write!(
                f,
                "Workflow run {run_id} has a sidecar in session {session_id}, but it could not be read as a run; `rdm cost --session {session_id}` shows the warning naming it"
            ),
            TranscriptError::InvalidId { id, reason } => write!(
                f,
                "invalid id {id:?}: {reason}; pass a bare session uuid or Workflow run id"
            ),
            TranscriptError::Io { path, message } => {
                write!(f, "could not read {path}: {message}")
            }
        }
    }
}

impl std::error::Error for TranscriptError {}

/// Where a session lives: the result of [`locate_session`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionLocation {
    /// The session uuid.
    pub session_id: String,
    /// The project-slug directory holding it.
    pub project_slug: String,
    /// Anything the lookup noticed, such as the uuid appearing under more
    /// than one slug.
    pub warnings: Vec<String>,
}

/// Where a Workflow run lives: the result of [`locate_workflow_run`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowRunLocation {
    /// The session whose `workflows/` directory holds the run's sidecar.
    pub session: SessionLocation,
    /// The run id, with its `wf_` prefix.
    pub run_id: String,
}

fn slugs<S: TranscriptSource + ?Sized>(src: &S) -> Result<Vec<String>, TranscriptError> {
    Ok(src
        .list(&TranscriptPath::root())?
        .into_iter()
        .filter(|e| e.kind == EntryKind::Dir)
        .map(|e| e.name)
        .collect())
}

fn child(dir: &TranscriptPath, name: &str) -> TranscriptPath {
    let mut segments = dir.segments.clone();
    segments.push(name.to_owned());
    TranscriptPath { segments }
}

/// Finds session `session_id` under every project-slug directory.
///
/// A session is present in a slug when `<slug>/<uuid>.jsonl` or
/// `<slug>/<uuid>/` exists. When it is present under several slugs, the first
/// in name order wins and the location carries a warning naming the others.
///
/// # Errors
///
/// - [`TranscriptError::InvalidId`] when `session_id` is not a single path
///   segment.
/// - [`TranscriptError::SessionNotFound`] when no slug holds the session.
/// - [`TranscriptError::Io`] when a directory cannot be listed.
///
/// # Examples
///
/// ```
/// use rdm_core::transcript::{MemoryTranscriptSource, locate_session};
///
/// let src = MemoryTranscriptSource::new()
///     .with_file("-proj/s1.jsonl", "")
///     .with_file("-proj--worktrees-demo/s2.jsonl", "");
/// assert_eq!(locate_session(&src, "s2")?.project_slug, "-proj--worktrees-demo");
/// assert!(locate_session(&src, "nope").is_err());
/// # Ok::<(), rdm_core::transcript::TranscriptError>(())
/// ```
pub fn locate_session<S: TranscriptSource + ?Sized>(
    src: &S,
    session_id: &str,
) -> Result<SessionLocation, TranscriptError> {
    check_segment(session_id)?;
    let transcript = format!("{session_id}.jsonl");
    let searched = slugs(src)?;
    let mut found = Vec::new();
    for slug in &searched {
        let present = src
            .list(&child(&TranscriptPath::root(), slug))?
            .iter()
            .any(|e| {
                (e.kind == EntryKind::File && e.name == transcript)
                    || (e.kind == EntryKind::Dir && e.name == session_id)
            });
        if present {
            found.push(slug.clone());
        }
    }
    let Some(first) = found.first().cloned() else {
        return Err(TranscriptError::SessionNotFound {
            session_id: session_id.to_owned(),
            root: src.root_display(),
            searched,
        });
    };
    let mut warnings = Vec::new();
    if found.len() > 1 {
        warnings.push(format!(
            "session {session_id} is present under several project directories; using {first}, ignoring {}",
            found[1..].join(", ")
        ));
    }
    Ok(SessionLocation {
        session_id: session_id.to_owned(),
        project_slug: first,
        warnings,
    })
}

/// `run_id` with the `wf_` prefix added when it is missing.
fn normalize_run_id(run_id: &str) -> String {
    if run_id.starts_with(WORKFLOW_RUN_PREFIX) {
        run_id.to_owned()
    } else {
        format!("{WORKFLOW_RUN_PREFIX}{run_id}")
    }
}

/// Finds the session holding Workflow run `run_id`'s sidecar,
/// `<slug>/<session>/workflows/<runId>.json`. The id is accepted with or
/// without its `wf_` prefix.
///
/// # Errors
///
/// - [`TranscriptError::InvalidId`] when `run_id` is not a single path
///   segment.
/// - [`TranscriptError::WorkflowRunNotFound`] when no session holds the run.
/// - [`TranscriptError::Io`] when a directory cannot be listed.
///
/// # Examples
///
/// ```
/// use rdm_core::transcript::{MemoryTranscriptSource, locate_workflow_run};
///
/// let src = MemoryTranscriptSource::new().with_file("-proj/s1/workflows/wf_r1.json", "{}");
/// let loc = locate_workflow_run(&src, "r1")?;
/// assert_eq!((loc.run_id.as_str(), loc.session.session_id.as_str()), ("wf_r1", "s1"));
/// # Ok::<(), rdm_core::transcript::TranscriptError>(())
/// ```
pub fn locate_workflow_run<S: TranscriptSource + ?Sized>(
    src: &S,
    run_id: &str,
) -> Result<WorkflowRunLocation, TranscriptError> {
    check_segment(run_id)?;
    let run_id = normalize_run_id(run_id);
    let sidecar = format!("{run_id}.json");
    let searched = slugs(src)?;
    let mut found: Vec<(String, String)> = Vec::new();
    for slug in &searched {
        let slug_dir = child(&TranscriptPath::root(), slug);
        for session in src.list(&slug_dir)? {
            if session.kind != EntryKind::Dir {
                continue;
            }
            let workflows = child(&child(&slug_dir, &session.name), "workflows");
            if src
                .list(&workflows)?
                .iter()
                .any(|e| e.kind == EntryKind::File && e.name == sidecar)
            {
                found.push((slug.clone(), session.name));
            }
        }
    }
    let Some((slug, session_id)) = found.first().cloned() else {
        return Err(TranscriptError::WorkflowRunNotFound {
            run_id,
            root: src.root_display(),
            searched,
        });
    };
    let mut warnings = Vec::new();
    if found.len() > 1 {
        let others: Vec<String> = found[1..]
            .iter()
            .map(|(s, id)| format!("{s}/{id}"))
            .collect();
        warnings.push(format!(
            "Workflow run {run_id} is present in several sessions; using {slug}/{session_id}, ignoring {}",
            others.join(", ")
        ));
    }
    Ok(WorkflowRunLocation {
        session: SessionLocation {
            session_id,
            project_slug: slug,
            warnings,
        },
        run_id,
    })
}

/// A request count and the five token classes, plus their sum.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct UsageSummary {
    /// Deduped requests counted.
    pub requests: u64,
    /// The five token classes.
    #[serde(flatten)]
    pub usage: TokenUsage,
    /// All five classes summed.
    pub total: u64,
}

impl From<ModelUsage> for UsageSummary {
    fn from(m: ModelUsage) -> Self {
        UsageSummary {
            requests: m.requests,
            usage: m.usage,
            total: m.usage.total(),
        }
    }
}

/// One model's row of a per-model breakdown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModelRow {
    /// The raw `message.model` string.
    pub model: String,
    /// That model's usage.
    #[serde(flatten)]
    pub usage: UsageSummary,
}

fn model_rows(ledger: &UsageLedger) -> Vec<ModelRow> {
    ledger
        .iter()
        .map(|(model, m)| ModelRow {
            model: model.to_owned(),
            usage: (*m).into(),
        })
        .collect()
}

/// Totals and per-model rows summed over `sources`' own per-model rows.
fn aggregate(sources: &[SourceReport]) -> (UsageSummary, Vec<ModelRow>) {
    let mut per_model: BTreeMap<&str, ModelUsage> = BTreeMap::new();
    for row in sources.iter().flat_map(|s| &s.models) {
        let m = per_model.entry(row.model.as_str()).or_default();
        m.requests = m.requests.saturating_add(row.usage.requests);
        m.usage += row.usage.usage;
    }
    let models: Vec<ModelRow> = per_model
        .into_iter()
        .map(|(model, m)| ModelRow {
            model: model.to_owned(),
            usage: m.into(),
        })
        .collect();
    let total = models
        .iter()
        .fold(ModelUsage::default(), |acc, r| ModelUsage {
            requests: acc.requests.saturating_add(r.usage.requests),
            usage: acc.usage + r.usage.usage,
        });
    (total.into(), models)
}

/// The kind of a spend source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    /// The main session transcript.
    Main,
    /// An `Agent` subagent.
    Agent,
    /// An agent of a Workflow run.
    WorkflowAgent,
}

impl fmt::Display for SourceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            SourceKind::Main => "main",
            SourceKind::Agent => "agent",
            SourceKind::WorkflowAgent => "workflow_agent",
        })
    }
}

/// One spend source of a session and what it counted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SourceReport {
    /// Main transcript, `Agent` subagent, or Workflow agent.
    pub kind: SourceKind,
    /// The session uuid for the main source, otherwise the agent id.
    pub id: String,
    /// A human label: the `Agent` description, or the Workflow name and the
    /// agent's progress label. Falls back to the id.
    pub label: String,
    /// The `Agent` subagent's `agentType`.
    pub agent_type: Option<String>,
    /// The `Agent` subagent's `parentAgentId`, for a nested subagent.
    pub parent_agent_id: Option<String>,
    /// The `Agent` subagent's `spawnDepth`.
    pub spawn_depth: Option<u64>,
    /// The Workflow run a Workflow agent belongs to.
    pub run_id: Option<String>,
    /// The Workflow run's `workflowName`.
    pub workflow_name: Option<String>,
    /// The main-transcript timestamp this source is anchored to.
    pub anchor: Option<DateTime<Utc>>,
    /// `false` when no anchor was found; the source is still counted.
    pub anchored: bool,
    /// The Workflow agent's `workflowProgress[]` `state` was `"error"`.
    pub errored: bool,
    /// The Workflow agent reused a cached result and cost nothing.
    pub cached: bool,
    /// Requests dropped because an earlier source already counted their
    /// `requestId`.
    pub duplicates_dropped: u64,
    /// The source's usage summed over every model.
    pub totals: UsageSummary,
    /// The source's usage per model, in model-string order.
    pub models: Vec<ModelRow>,
    /// The deduped requests counted, each with its own timestamp. Not
    /// serialized.
    #[serde(skip)]
    pub requests: Vec<RequestUsage>,
}

/// What an [`UnanchoredEntry`] names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnanchoredKind {
    /// A main transcript with no timestamped line.
    Main,
    /// An `Agent` subagent whose ancestry reaches no `tool_use` in the main
    /// transcript.
    Agent,
    /// A Workflow run that no main-transcript `tool_result` names. Its agents
    /// carry `anchored: false` but are not listed one by one.
    WorkflowRun,
}

impl fmt::Display for UnanchoredKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            UnanchoredKind::Main => "main",
            UnanchoredKind::Agent => "agent",
            UnanchoredKind::WorkflowRun => "workflow_run",
        })
    }
}

/// A source or Workflow run with no anchor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnanchoredEntry {
    /// What is unanchored.
    pub kind: UnanchoredKind,
    /// The session uuid, agent id, or run id.
    pub id: String,
    /// Its label.
    pub label: String,
}

/// One Workflow run of the session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkflowRunReport {
    /// The run id, with its `wf_` prefix.
    pub run_id: String,
    /// The sidecar's `workflowName`.
    pub workflow_name: Option<String>,
    /// The timestamp of the main-transcript `tool_result` naming the run.
    pub anchor: Option<DateTime<Utc>>,
    /// `false` when no `tool_result` names the run.
    pub anchored: bool,
    /// How many of the report's sources belong to the run.
    pub agent_count: u64,
    /// The sidecar's own `totalTokens` figure. Shown for reference only: it
    /// is not the cost basis and is never added to any total.
    pub sidecar_total_tokens: Option<u64>,
}

/// Which part of a session a [`ReportWarning`] is about, so a narrowed report
/// keeps only the warnings that apply to it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum WarningScope {
    /// The whole session: kept in every narrowed view.
    Session,
    /// A source outside every Workflow run, or a sidecar that could not be
    /// read as a run.
    Source,
    /// One Workflow run or one of its agents.
    WorkflowRun(String),
}

/// A warning in a [`SessionReport`]. It serializes as its message string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportWarning {
    /// What the warning is about.
    pub scope: WarningScope,
    /// The user-facing message, including its source context.
    pub message: String,
}

impl fmt::Display for ReportWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Serialize for ReportWarning {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.message)
    }
}

/// What a [`SessionReport`] covers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReportScope {
    /// The whole session.
    Session,
    /// One Workflow run's agents.
    WorkflowRun {
        /// The run id, with its `wf_` prefix.
        run_id: String,
    },
}

/// The anchored token ledger of one session, or of one Workflow run in it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SessionReport {
    /// The session uuid.
    pub session_id: String,
    /// The project-slug directory holding the session.
    pub project_slug: String,
    /// The whole session, or one Workflow run.
    pub scope: ReportScope,
    /// Every counted source summed.
    pub totals: UsageSummary,
    /// Every counted source summed per model; sums to [`totals`](Self::totals).
    pub models: Vec<ModelRow>,
    /// The spend sources, in canonical order.
    pub sources: Vec<SourceReport>,
    /// Sources and runs with no anchor.
    pub unanchored: Vec<UnanchoredEntry>,
    /// The session's Workflow runs, by run id.
    pub workflow_runs: Vec<WorkflowRunReport>,
    /// Everything the reap tolerated, in a deterministic order.
    pub warnings: Vec<ReportWarning>,
}

impl SessionReport {
    /// This report narrowed to Workflow run `run_id` (with or without its
    /// `wf_` prefix), or `None` when the report has no such run.
    ///
    /// Narrowing keeps that run's agents in `sources`, its entry in
    /// `unanchored` and `workflow_runs`, and recomputes `totals` and `models`
    /// from those sources alone, so the per-model breakdown covers only the
    /// run. Dedupe already happened at session scope, so the numbers match
    /// the run's rows in the full report.
    ///
    /// `warnings` is filtered to the run: it keeps the warnings about the
    /// session as a whole (including [`WORKFLOW_NESTED_UNVERIFIED`] and
    /// location warnings) and those about this run or its agents, and drops
    /// the ones about the main transcript, `Agent` subagents, other runs and
    /// unreadable sidecars.
    ///
    /// # Examples
    ///
    /// ```
    /// use rdm_core::transcript::{MemoryTranscriptSource, locate_session, reap_session};
    ///
    /// let src = MemoryTranscriptSource::new()
    ///     .with_file("-p/s.jsonl", r#"{"type":"assistant","requestId":"m","message":{"model":"opus","usage":{"output_tokens":5}}}"#)
    ///     .with_file("-p/s/workflows/wf_r.json", r#"{"runId":"wf_r","workflowName":"w","workflowProgress":[{"type":"workflow_agent","agentId":"a","label":"find"}]}"#)
    ///     .with_file("-p/s/subagents/workflows/wf_r/agent-a.jsonl", r#"{"type":"assistant","requestId":"x","message":{"model":"haiku","usage":{"output_tokens":2}}}"#);
    /// let full = reap_session(&src, &locate_session(&src, "s")?)?;
    /// assert_eq!(full.totals.total, 7);
    /// let run = full.for_workflow_run("r").unwrap_or_else(|| unreachable!());
    /// assert_eq!(run.totals.total, 2);
    /// assert_eq!(run.models.iter().map(|m| m.model.as_str()).collect::<Vec<_>>(), ["haiku"]);
    /// # Ok::<(), rdm_core::transcript::TranscriptError>(())
    /// ```
    #[must_use]
    pub fn for_workflow_run(&self, run_id: &str) -> Option<SessionReport> {
        let run_id = normalize_run_id(run_id);
        let run = self.workflow_runs.iter().find(|r| r.run_id == run_id)?;
        let sources: Vec<SourceReport> = self
            .sources
            .iter()
            .filter(|s| s.run_id.as_deref() == Some(run_id.as_str()))
            .cloned()
            .collect();
        let (totals, models) = aggregate(&sources);
        Some(SessionReport {
            session_id: self.session_id.clone(),
            project_slug: self.project_slug.clone(),
            scope: ReportScope::WorkflowRun {
                run_id: run_id.clone(),
            },
            totals,
            models,
            sources,
            unanchored: self
                .unanchored
                .iter()
                .filter(|u| u.kind == UnanchoredKind::WorkflowRun && u.id == run_id)
                .cloned()
                .collect(),
            workflow_runs: vec![run.clone()],
            warnings: self
                .warnings
                .iter()
                .filter(|w| match &w.scope {
                    WarningScope::Session => true,
                    WarningScope::WorkflowRun(r) => *r == run_id,
                    WarningScope::Source => false,
                })
                .cloned()
                .collect(),
        })
    }
}

fn timestamp_of(line: &Value) -> Option<DateTime<Utc>> {
    line.get("timestamp")
        .and_then(Value::as_str)
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|t| t.with_timezone(&Utc))
}

/// The anchors the main transcript offers.
#[derive(Default)]
struct MainIndex {
    first_timestamp: Option<DateTime<Utc>>,
    tool_uses: BTreeMap<String, DateTime<Utc>>,
    run_results: BTreeMap<String, DateTime<Utc>>,
}

fn index_main(raw: &str) -> MainIndex {
    let mut index = MainIndex::default();
    for line in raw.lines() {
        let Ok(entry) = serde_json::from_str::<Value>(line.trim()) else {
            continue;
        };
        let Some(ts) = timestamp_of(&entry) else {
            continue;
        };
        index.first_timestamp.get_or_insert(ts);
        if let Some(blocks) = entry
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(Value::as_array)
        {
            for block in blocks {
                if block.get("type").and_then(Value::as_str) == Some("tool_use")
                    && let Some(id) = block.get("id").and_then(Value::as_str)
                {
                    index.tool_uses.entry(id.to_owned()).or_insert(ts);
                }
            }
        }
        if let Some(run_id) = entry
            .get("toolUseResult")
            .and_then(|r| r.get("runId"))
            .and_then(Value::as_str)
        {
            index.run_results.entry(run_id.to_owned()).or_insert(ts);
        }
    }
    index
}

fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_owned)
}

/// The fields of an `agent-<id>.meta.json` the reaper uses.
#[derive(Default)]
struct AgentMeta {
    agent_type: Option<String>,
    description: Option<String>,
    tool_use_id: Option<String>,
    spawn_depth: Option<u64>,
    parent_agent_id: Option<String>,
}

impl AgentMeta {
    fn from_json(v: &Value) -> Self {
        AgentMeta {
            agent_type: str_field(v, "agentType"),
            description: str_field(v, "description"),
            tool_use_id: str_field(v, "toolUseId"),
            spawn_depth: v.get("spawnDepth").and_then(Value::as_u64),
            parent_agent_id: str_field(v, "parentAgentId"),
        }
    }
}

/// Follows `parentAgentId` from `id` until an ancestor's `toolUseId` is in
/// the main transcript. Stops on a cycle or a missing parent.
fn agent_anchor(
    id: &str,
    metas: &BTreeMap<String, AgentMeta>,
    index: &MainIndex,
) -> Option<DateTime<Utc>> {
    let mut seen = BTreeSet::new();
    let mut current = id;
    while seen.insert(current) {
        let meta = metas.get(current)?;
        if let Some(ts) = meta
            .tool_use_id
            .as_ref()
            .and_then(|t| index.tool_uses.get(t))
        {
            return Some(*ts);
        }
        current = meta.parent_agent_id.as_deref()?;
    }
    None
}

/// A source before cross-source dedupe.
struct Pending {
    report: SourceReport,
    requests: Vec<RequestUsage>,
    context: String,
    scope: WarningScope,
}

impl Pending {
    fn new(report: SourceReport, context: String, scope: WarningScope) -> Self {
        Pending {
            report,
            requests: Vec::new(),
            context,
            scope,
        }
    }
}

fn blank_source(kind: SourceKind, id: &str, label: String) -> SourceReport {
    SourceReport {
        kind,
        id: id.to_owned(),
        label,
        agent_type: None,
        parent_agent_id: None,
        spawn_depth: None,
        run_id: None,
        workflow_name: None,
        anchor: None,
        anchored: false,
        errored: false,
        cached: false,
        duplicates_dropped: 0,
        totals: UsageSummary::default(),
        models: Vec::new(),
        requests: Vec::new(),
    }
}

/// A Workflow run found in the session: from a readable sidecar, or from a
/// transcript directory alone.
#[derive(Default)]
struct RunSidecar {
    workflow_name: Option<String>,
    total_tokens: Option<u64>,
    progress: Vec<Value>,
}

/// Collects warnings in order, with their scope.
#[derive(Default)]
struct Warnings(Vec<ReportWarning>);

impl Warnings {
    fn push(&mut self, scope: WarningScope, message: String) {
        self.0.push(ReportWarning { scope, message });
    }
}

/// `agent-<id>.<suffix>` → `<id>`.
fn agent_id_of<'a>(name: &'a str, suffix: &str) -> Option<&'a str> {
    name.strip_prefix("agent-")
        .and_then(|rest| rest.strip_suffix(suffix))
        .filter(|id| !id.is_empty())
}

fn parse_into(pending: &mut Pending, raw: &str, warnings: &mut Warnings) {
    let parsed = parse_transcript(raw);
    for w in parsed.warnings {
        warnings.push(pending.scope.clone(), format!("{}: {w}", pending.context));
    }
    pending.requests = parsed.requests;
}

/// Builds the anchored ledger of the session at `location`: every spend
/// source, anchored, deduped across sources, and summed per model.
///
/// See the [module docs](self) for the anchoring and dedupe rules. A
/// subagent or Workflow agent whose transcript cannot be read, a meta file or
/// sidecar that is not valid JSON, and a sidecar without a string `runId` are
/// tolerated: each yields a warning and the rest of the session is still
/// reported. A missing main transcript is tolerated too, leaving every other
/// source unanchored.
///
/// # Errors
///
/// - [`TranscriptError::InvalidId`] when the location's session id or slug is
///   not a single path segment.
/// - [`TranscriptError::Io`] when a directory cannot be listed or the main
///   transcript cannot be read.
///
/// # Examples
///
/// ```
/// use rdm_core::transcript::{MemoryTranscriptSource, locate_session, reap_session};
///
/// let src = MemoryTranscriptSource::new()
///     .with_file("-p/s.jsonl", concat!(
///         r#"{"type":"assistant","requestId":"m1","timestamp":"2026-01-01T00:00:00Z","message":{"model":"opus","content":[{"type":"tool_use","id":"tu1","name":"Agent"}],"usage":{"output_tokens":5}}}"#, "\n",
///     ))
///     .with_file("-p/s/subagents/agent-a.jsonl", r#"{"type":"assistant","requestId":"a1","message":{"model":"haiku","usage":{"output_tokens":2}}}"#)
///     .with_file("-p/s/subagents/agent-a.meta.json", r#"{"agentType":"Explore","description":"look around","toolUseId":"tu1"}"#);
/// let report = reap_session(&src, &locate_session(&src, "s")?)?;
/// assert_eq!(report.totals.total, 7);
/// assert_eq!(report.sources[1].label, "look around");
/// assert_eq!(report.sources[1].anchor, report.sources[0].anchor);
/// assert!(report.unanchored.is_empty());
/// # Ok::<(), rdm_core::transcript::TranscriptError>(())
/// ```
pub fn reap_session<S: TranscriptSource + ?Sized>(
    src: &S,
    location: &SessionLocation,
) -> Result<SessionReport, TranscriptError> {
    let root_display = src.root_display();
    let full = |p: &TranscriptPath| format!("{}/{p}", root_display.trim_end_matches('/'));
    let slug_dir = TranscriptPath::root().join(&location.project_slug)?;
    let session_id = location.session_id.as_str();
    let session_dir = slug_dir.join(session_id)?;
    let main_name = format!("{session_id}.jsonl");
    let main_path = slug_dir.join(&main_name)?;

    let mut warnings = Warnings::default();
    for w in &location.warnings {
        warnings.push(WarningScope::Session, w.clone());
    }
    let mut pending: Vec<Pending> = Vec::new();

    // 1. The main transcript.
    let has_main = src
        .list(&slug_dir)?
        .iter()
        .any(|e| e.kind == EntryKind::File && e.name == main_name);
    let index = if has_main {
        let raw = src.read(&main_path)?;
        let index = index_main(&raw);
        let mut report = blank_source(SourceKind::Main, session_id, "main session".to_owned());
        report.anchor = index.first_timestamp;
        report.anchored = report.anchor.is_some();
        let mut p = Pending::new(report, "main transcript".to_owned(), WarningScope::Source);
        parse_into(&mut p, &raw, &mut warnings);
        pending.push(p);
        index
    } else {
        warnings.push(
            WarningScope::Session,
            format!(
                "session {session_id} has no main transcript at {}; every subagent and Workflow run in it is unanchored",
                full(&main_path)
            ),
        );
        MainIndex::default()
    };

    // 2. `Agent` subagents, by id.
    let subagents = child(&session_dir, "subagents");
    let entries = src.list(&subagents)?;
    let mut metas: BTreeMap<String, AgentMeta> = BTreeMap::new();
    let mut agent_ids: BTreeSet<String> = BTreeSet::new();
    for e in entries.iter().filter(|e| e.kind == EntryKind::File) {
        if let Some(id) = agent_id_of(&e.name, ".meta.json") {
            let path = child(&subagents, &e.name);
            match src
                .read(&path)
                .map_err(|e| e.to_string())
                .and_then(|raw| serde_json::from_str::<Value>(&raw).map_err(|e| e.to_string()))
            {
                Ok(v) => {
                    metas.insert(id.to_owned(), AgentMeta::from_json(&v));
                }
                Err(err) => warnings.push(
                    WarningScope::Source,
                    format!("agent {id}: could not read {}: {err}", full(&path)),
                ),
            }
        } else if let Some(id) = agent_id_of(&e.name, ".jsonl") {
            agent_ids.insert(id.to_owned());
        }
    }
    for id in &agent_ids {
        let meta = metas.get(id);
        let label = meta
            .and_then(|m| m.description.clone())
            .unwrap_or_else(|| id.clone());
        let mut report = blank_source(SourceKind::Agent, id, label.clone());
        if let Some(m) = meta {
            report.agent_type = m.agent_type.clone();
            report.parent_agent_id = m.parent_agent_id.clone();
            report.spawn_depth = m.spawn_depth;
        }
        report.anchor = agent_anchor(id, &metas, &index);
        report.anchored = report.anchor.is_some();
        let mut p = Pending::new(
            report,
            format!("agent {id} ({label})"),
            WarningScope::Source,
        );
        let path = child(&subagents, &format!("agent-{id}.jsonl"));
        match src.read(&path) {
            Ok(raw) => parse_into(&mut p, &raw, &mut warnings),
            Err(err) => warnings.push(
                WarningScope::Source,
                format!("{}: counted as zero: {err}", p.context),
            ),
        }
        pending.push(p);
    }

    // 3. Workflow runs, by run id, and each run's agents, by agent id.
    let workflows = child(&session_dir, "workflows");
    let mut runs: BTreeMap<String, RunSidecar> = BTreeMap::new();
    for e in src.list(&workflows)? {
        if e.kind != EntryKind::File
            || !e.name.starts_with(WORKFLOW_RUN_PREFIX)
            || !e.name.ends_with(".json")
        {
            continue;
        }
        let path = child(&workflows, &e.name);
        let skip = |warnings: &mut Warnings, why: String| {
            warnings.push(
                WarningScope::Source,
                format!("skipped Workflow sidecar {}: {why}", full(&path)),
            );
        };
        let value = match src.read(&path) {
            Ok(raw) => match serde_json::from_str::<Value>(&raw) {
                Ok(v) => v,
                Err(err) => {
                    skip(&mut warnings, format!("not valid JSON ({err})"));
                    continue;
                }
            },
            Err(err) => {
                skip(&mut warnings, err.to_string());
                continue;
            }
        };
        let Some(run_id) = value.get("runId").and_then(Value::as_str) else {
            skip(&mut warnings, "it has no string runId".to_owned());
            continue;
        };
        if check_segment(run_id).is_err() {
            skip(
                &mut warnings,
                format!("its runId {run_id:?} is not a usable id"),
            );
            continue;
        }
        if runs.contains_key(run_id) {
            skip(
                &mut warnings,
                format!("another sidecar already declares runId {run_id}"),
            );
            continue;
        }
        runs.insert(
            run_id.to_owned(),
            RunSidecar {
                workflow_name: str_field(&value, "workflowName"),
                total_tokens: value.get("totalTokens").and_then(Value::as_u64),
                progress: value
                    .get("workflowProgress")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default(),
            },
        );
    }
    let run_transcripts = child(&subagents, "workflows");
    for e in src.list(&run_transcripts)? {
        if e.kind == EntryKind::Dir && !runs.contains_key(&e.name) {
            warnings.push(
                WarningScope::WorkflowRun(e.name.clone()),
                format!(
                    "Workflow run {} has agent transcripts but no readable sidecar; its agents are labelled by id",
                    e.name
                ),
            );
            runs.insert(e.name, RunSidecar::default());
        }
    }

    let mut run_reports = Vec::new();
    for (run_id, sidecar) in &runs {
        let scope = WarningScope::WorkflowRun(run_id.clone());
        let anchor = index.run_results.get(run_id).copied();
        let run_dir = child(&run_transcripts, run_id);
        let transcripts: BTreeSet<String> = src
            .list(&run_dir)?
            .iter()
            .filter(|e| e.kind == EntryKind::File)
            .filter_map(|e| agent_id_of(&e.name, ".jsonl").map(str::to_owned))
            .collect();
        let name = sidecar.workflow_name.as_deref().unwrap_or(run_id);
        let mut agents: BTreeMap<String, Pending> = BTreeMap::new();
        for entry in &sidecar.progress {
            if entry.get("type").and_then(Value::as_str) != Some("workflow_agent") {
                continue;
            }
            let Some(id) = entry
                .get("agentId")
                .and_then(Value::as_str)
                .filter(|id| check_segment(id).is_ok())
            else {
                continue;
            };
            if agents.contains_key(id) {
                continue;
            }
            let label = match entry.get("label").and_then(Value::as_str) {
                Some(l) => format!("{name} / {l}"),
                None => format!("{name} / {id}"),
            };
            let mut report = blank_source(SourceKind::WorkflowAgent, id, label.clone());
            report.errored = entry.get("state").and_then(Value::as_str) == Some("error");
            report.cached = entry.get("cached").and_then(Value::as_bool) == Some(true);
            let mut p = Pending::new(
                report,
                format!("Workflow agent {run_id}/{id} ({label})"),
                scope.clone(),
            );
            if !p.report.cached {
                let path = child(&run_dir, &format!("agent-{id}.jsonl"));
                let read = if transcripts.contains(id) {
                    src.read(&path).map_err(|e| e.to_string())
                } else {
                    Err(format!("no transcript at {}", full(&path)))
                };
                match read {
                    Ok(raw) => parse_into(&mut p, &raw, &mut warnings),
                    Err(err) => warnings.push(
                        scope.clone(),
                        format!(
                            "{}: counted as zero (its sidecar tokens figure is not used): {err}",
                            p.context
                        ),
                    ),
                }
            }
            agents.insert(id.to_owned(), p);
        }
        for id in &transcripts {
            if agents.contains_key(id) {
                continue;
            }
            let label = format!("{name} / {id}");
            let report = blank_source(SourceKind::WorkflowAgent, id, label.clone());
            let mut p = Pending::new(
                report,
                format!("Workflow agent {run_id}/{id} ({label})"),
                scope.clone(),
            );
            match src.read(&child(&run_dir, &format!("agent-{id}.jsonl"))) {
                Ok(raw) => parse_into(&mut p, &raw, &mut warnings),
                Err(err) => warnings.push(
                    scope.clone(),
                    format!("{}: counted as zero: {err}", p.context),
                ),
            }
            agents.insert(id.clone(), p);
        }
        run_reports.push(WorkflowRunReport {
            run_id: run_id.clone(),
            workflow_name: sidecar.workflow_name.clone(),
            anchor,
            anchored: anchor.is_some(),
            agent_count: agents.len() as u64,
            sidecar_total_tokens: sidecar.total_tokens,
        });
        for (_, mut p) in agents {
            p.report.run_id = Some(run_id.clone());
            p.report.workflow_name = sidecar.workflow_name.clone();
            p.report.anchor = anchor;
            p.report.anchored = anchor.is_some();
            pending.push(p);
        }
    }

    // 4. Cross-source dedupe by requestId, in canonical order.
    let mut owners: BTreeMap<String, String> = BTreeMap::new();
    let mut sources = Vec::with_capacity(pending.len());
    for p in pending {
        let Pending {
            mut report,
            requests,
            context,
            scope,
        } = p;
        let mut kept = Vec::with_capacity(requests.len());
        let mut owned_by: BTreeSet<String> = BTreeSet::new();
        for r in requests {
            if let Some(owner) = owners.get(&r.request_id) {
                report.duplicates_dropped += 1;
                owned_by.insert(owner.clone());
            } else {
                owners.insert(r.request_id.clone(), context.clone());
                kept.push(r);
            }
        }
        if report.duplicates_dropped > 0 {
            warnings.push(
                scope,
                format!(
                    "{context}: dropped {} request(s) already counted by {}",
                    report.duplicates_dropped,
                    owned_by.into_iter().collect::<Vec<_>>().join(", ")
                ),
            );
        }
        let ledger = UsageLedger::from_requests(&kept);
        report.totals = ledger.total().into();
        report.models = model_rows(&ledger);
        report.requests = kept;
        sources.push(report);
    }

    if !runs.is_empty() {
        warnings.push(
            WarningScope::Session,
            format!(
                "{WORKFLOW_NESTED_UNVERIFIED}: an agent spawned below a Workflow agent has never been observed on disk, so any spend it made is not located or counted"
            ),
        );
    }

    let mut unanchored: Vec<UnanchoredEntry> = sources
        .iter()
        .filter(|s| !s.anchored && s.kind != SourceKind::WorkflowAgent)
        .map(|s| UnanchoredEntry {
            kind: match s.kind {
                SourceKind::Main => UnanchoredKind::Main,
                _ => UnanchoredKind::Agent,
            },
            id: s.id.clone(),
            label: s.label.clone(),
        })
        .collect();
    unanchored.extend(
        run_reports
            .iter()
            .filter(|r| !r.anchored)
            .map(|r| UnanchoredEntry {
                kind: UnanchoredKind::WorkflowRun,
                id: r.run_id.clone(),
                label: r.workflow_name.clone().unwrap_or_else(|| r.run_id.clone()),
            }),
    );

    let (totals, models) = aggregate(&sources);
    Ok(SessionReport {
        session_id: session_id.to_owned(),
        project_slug: location.project_slug.clone(),
        scope: ReportScope::Session,
        totals,
        models,
        sources,
        unanchored,
        workflow_runs: run_reports,
        warnings: warnings.0,
    })
}

/// The report of the Workflow run at `location`: the session is reaped
/// whole (so cross-source dedupe sees every source) and then narrowed with
/// [`SessionReport::for_workflow_run`].
///
/// # Errors
///
/// - Everything [`reap_session`] returns.
/// - [`TranscriptError::WorkflowRunUnreadable`] when the run's sidecar could
///   not be read as a run (for example, it has no string `runId`).
///
/// # Examples
///
/// ```
/// use rdm_core::transcript::{MemoryTranscriptSource, locate_workflow_run, reap_workflow_run};
///
/// let src = MemoryTranscriptSource::new()
///     .with_file("-p/s/workflows/wf_r.json", r#"{"runId":"wf_r","workflowProgress":[]}"#);
/// let report = reap_workflow_run(&src, &locate_workflow_run(&src, "wf_r")?)?;
/// assert_eq!(report.workflow_runs.len(), 1);
/// # Ok::<(), rdm_core::transcript::TranscriptError>(())
/// ```
pub fn reap_workflow_run<S: TranscriptSource + ?Sized>(
    src: &S,
    location: &WorkflowRunLocation,
) -> Result<SessionReport, TranscriptError> {
    reap_session(src, &location.session)?
        .for_workflow_run(&location.run_id)
        .ok_or_else(|| TranscriptError::WorkflowRunUnreadable {
            run_id: location.run_id.clone(),
            session_id: location.session.session_id.clone(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: &str = "sess-1";

    fn ts(s: &str) -> DateTime<Utc> {
        s.parse()
            .unwrap_or_else(|e| panic!("bad test timestamp {s}: {e}"))
    }

    fn asst(request_id: &str, model: &str, output: u64) -> String {
        format!(
            r#"{{"type":"assistant","requestId":"{request_id}","message":{{"model":"{model}","usage":{{"output_tokens":{output}}}}}}}"#
        )
    }

    fn asst_at(request_id: &str, at: &str, output: u64) -> String {
        format!(
            r#"{{"type":"assistant","requestId":"{request_id}","timestamp":"{at}","message":{{"model":"opus","usage":{{"output_tokens":{output}}}}}}}"#
        )
    }

    fn tool_use(at: &str, id: &str) -> String {
        format!(
            r#"{{"type":"assistant","timestamp":"{at}","message":{{"content":[{{"type":"tool_use","id":"{id}","name":"Agent"}}]}}}}"#
        )
    }

    fn run_result(at: &str, run_id: &str) -> String {
        format!(
            r#"{{"type":"user","timestamp":"{at}","message":{{"content":[{{"type":"tool_result","tool_use_id":"x"}}]}},"toolUseResult":{{"runId":"{run_id}"}}}}"#
        )
    }

    fn meta(tool_use_id: &str, parent: Option<&str>, description: &str) -> String {
        match parent {
            Some(p) => format!(
                r#"{{"agentType":"general-purpose","description":"{description}","toolUseId":"{tool_use_id}","parentAgentId":"{p}","spawnDepth":2}}"#
            ),
            None => format!(
                r#"{{"agentType":"general-purpose","description":"{description}","toolUseId":"{tool_use_id}","spawnDepth":1}}"#
            ),
        }
    }

    fn lines(ls: &[String]) -> String {
        ls.join("\n")
    }

    fn reap(src: &MemoryTranscriptSource) -> SessionReport {
        let loc = locate_session(src, S).unwrap_or_else(|e| panic!("locate: {e}"));
        reap_session(src, &loc).unwrap_or_else(|e| panic!("reap: {e}"))
    }

    fn source<'a>(report: &'a SessionReport, id: &str) -> &'a SourceReport {
        report
            .sources
            .iter()
            .find(|s| s.id == id)
            .unwrap_or_else(|| panic!("no source {id} in {:?}", report.sources))
    }

    fn has_warning(report: &SessionReport, needle: &str) -> bool {
        report.warnings.iter().any(|w| w.message.contains(needle))
    }

    fn main_with(extra: &[String]) -> String {
        let mut ls = vec![asst_at("m1", "2026-01-01T00:00:00Z", 10)];
        ls.extend_from_slice(extra);
        lines(&ls)
    }

    #[test]
    fn memory_source_lists_sorted_and_missing_dir_is_empty() {
        let src = MemoryTranscriptSource::new()
            .with_file("b/z.jsonl", "")
            .with_file("b/a/x", "")
            .with_file("a/y", "");
        let listed = src
            .list(&TranscriptPath::parse("b").unwrap_or_default())
            .unwrap_or_default();
        assert_eq!(
            listed,
            [
                TranscriptEntry {
                    name: "a".into(),
                    kind: EntryKind::Dir
                },
                TranscriptEntry {
                    name: "z.jsonl".into(),
                    kind: EntryKind::File
                },
            ]
        );
        let missing = src.list(&TranscriptPath::parse("nope/deeper").unwrap_or_default());
        assert_eq!(missing, Ok(Vec::new()));
        assert!(matches!(
            src.read(&TranscriptPath::parse("a/missing").unwrap_or_default()),
            Err(TranscriptError::Io { .. })
        ));
    }

    #[test]
    fn locate_finds_session_under_worktree_slug() {
        let src = MemoryTranscriptSource::new()
            .with_file("-proj/other.jsonl", "")
            .with_file("-proj--worktrees-roadmap-demo/sess-1.jsonl", "");
        let loc = locate_session(&src, S).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(loc.project_slug, "-proj--worktrees-roadmap-demo");
        assert!(loc.warnings.is_empty());

        // A session directory alone (no main transcript) is also a hit.
        let src = MemoryTranscriptSource::new().with_file("-p/sess-1/subagents/agent-a.jsonl", "");
        assert_eq!(
            locate_session(&src, S).map(|l| l.project_slug),
            Ok("-p".to_owned())
        );
    }

    #[test]
    fn locate_under_two_slugs_picks_first_and_warns() {
        let src = MemoryTranscriptSource::new()
            .with_file("-b/sess-1.jsonl", "")
            .with_file("-a/sess-1.jsonl", "")
            .with_file("-c/sess-1/subagents/agent-x.jsonl", "");
        let loc = locate_session(&src, S).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(loc.project_slug, "-a");
        assert_eq!(loc.warnings.len(), 1);
        assert!(loc.warnings[0].contains("-b, -c"), "{:?}", loc.warnings);
        let report = reap_session(&src, &loc).unwrap_or_else(|e| panic!("{e}"));
        assert!(has_warning(&report, "ignoring -b, -c"));
    }

    #[test]
    fn missing_session_error_names_uuid_root_and_every_slug() {
        let src = MemoryTranscriptSource::new()
            .with_file("-proj/a.jsonl", "")
            .with_file("-proj--worktrees-x/b.jsonl", "");
        let err = locate_session(&src, "deadbeef").err();
        assert_eq!(
            err,
            Some(TranscriptError::SessionNotFound {
                session_id: "deadbeef".into(),
                root: "memory:".into(),
                searched: vec!["-proj".into(), "-proj--worktrees-x".into()],
            })
        );
        let msg = err.map(|e| e.to_string()).unwrap_or_default();
        for needle in [
            "deadbeef",
            "memory:",
            "-proj\n",
            "-proj--worktrees-x",
            "CLAUDE_CONFIG_DIR",
        ] {
            assert!(msg.contains(needle), "{needle:?} missing from {msg}");
        }

        let empty = locate_session(&MemoryTranscriptSource::new(), "deadbeef")
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(empty.contains("holds no project directories"), "{empty}");
    }

    #[test]
    fn path_escaping_ids_are_invalid() {
        let src = MemoryTranscriptSource::new().with_file("-p/s.jsonl", "");
        for bad in ["../x", "a/b", "..", "", "a\\b"] {
            assert!(
                matches!(
                    locate_session(&src, bad),
                    Err(TranscriptError::InvalidId { .. })
                ),
                "session {bad:?}"
            );
            assert!(
                matches!(
                    locate_workflow_run(&src, bad),
                    Err(TranscriptError::InvalidId { .. })
                ),
                "run {bad:?}"
            );
        }
        assert!(TranscriptPath::parse("a//b").is_err());
    }

    #[test]
    fn agent_is_anchored_to_its_tool_use_line() {
        let src = MemoryTranscriptSource::new()
            .with_file(
                "-p/sess-1.jsonl",
                main_with(&[tool_use("2026-01-01T00:05:00Z", "tu-a")]),
            )
            .with_file("-p/sess-1/subagents/agent-a.jsonl", asst("a1", "haiku", 3))
            .with_file(
                "-p/sess-1/subagents/agent-a.meta.json",
                meta("tu-a", None, "Plan the thing"),
            );
        let report = reap(&src);
        let a = source(&report, "a");
        assert_eq!(a.anchor, Some(ts("2026-01-01T00:05:00Z")));
        assert!(a.anchored);
        assert_eq!(a.label, "Plan the thing");
        assert_eq!(a.agent_type.as_deref(), Some("general-purpose"));
        let main = source(&report, S);
        assert_eq!(main.kind, SourceKind::Main);
        assert_eq!(main.anchor, Some(ts("2026-01-01T00:00:00Z")));
        assert!(report.unanchored.is_empty());
    }

    #[test]
    fn nested_chain_is_anchored_to_the_root_ancestor() {
        // root <- mid <- leaf: depth 3; only root's toolUseId is in main.
        let src = MemoryTranscriptSource::new()
            .with_file(
                "-p/sess-1.jsonl",
                main_with(&[tool_use("2026-01-01T00:07:00Z", "tu-root")]),
            )
            .with_file("-p/sess-1/subagents/agent-root.jsonl", asst("r", "opus", 1))
            .with_file(
                "-p/sess-1/subagents/agent-root.meta.json",
                meta("tu-root", None, "root"),
            )
            .with_file("-p/sess-1/subagents/agent-mid.jsonl", asst("m", "opus", 1))
            .with_file(
                "-p/sess-1/subagents/agent-mid.meta.json",
                meta("tu-in-root", Some("root"), "mid"),
            )
            .with_file("-p/sess-1/subagents/agent-leaf.jsonl", asst("l", "opus", 1))
            .with_file(
                "-p/sess-1/subagents/agent-leaf.meta.json",
                meta("tu-in-mid", Some("mid"), "leaf"),
            );
        let report = reap(&src);
        for id in ["root", "mid", "leaf"] {
            assert_eq!(
                source(&report, id).anchor,
                Some(ts("2026-01-01T00:07:00Z")),
                "{id}"
            );
        }
        assert_eq!(
            source(&report, "leaf").parent_agent_id.as_deref(),
            Some("mid")
        );
    }

    #[test]
    fn orphan_and_cyclic_agents_are_unanchored_but_counted() {
        let src = MemoryTranscriptSource::new()
            .with_file("-p/sess-1.jsonl", main_with(&[]))
            .with_file(
                "-p/sess-1/subagents/agent-orphan.jsonl",
                asst("o", "opus", 100),
            )
            .with_file(
                "-p/sess-1/subagents/agent-orphan.meta.json",
                meta("tu-nowhere", None, "Orphan work"),
            )
            .with_file(
                "-p/sess-1/subagents/agent-c1.jsonl",
                asst("c1", "opus", 1000),
            )
            .with_file(
                "-p/sess-1/subagents/agent-c1.meta.json",
                meta("tu-x", Some("c2"), "cycle one"),
            )
            .with_file(
                "-p/sess-1/subagents/agent-c2.jsonl",
                asst("c2", "opus", 10000),
            )
            .with_file(
                "-p/sess-1/subagents/agent-c2.meta.json",
                meta("tu-y", Some("c1"), "cycle two"),
            );
        let report = reap(&src);
        let unanchored: Vec<(UnanchoredKind, &str, &str)> = report
            .unanchored
            .iter()
            .map(|u| (u.kind, u.id.as_str(), u.label.as_str()))
            .collect();
        assert_eq!(
            unanchored,
            [
                (UnanchoredKind::Agent, "c1", "cycle one"),
                (UnanchoredKind::Agent, "c2", "cycle two"),
                (UnanchoredKind::Agent, "orphan", "Orphan work"),
            ]
        );
        assert!(!source(&report, "orphan").anchored);
        assert_eq!(report.totals.usage.output, 10 + 100 + 1000 + 10000);
    }

    fn workflow_src() -> MemoryTranscriptSource {
        MemoryTranscriptSource::new()
            .with_file(
                "-p/sess-1.jsonl",
                main_with(&[
                    run_result("2026-01-01T01:00:00Z", "wf_r1"),
                    asst_at("m2", "2026-01-01T02:00:00Z", 20),
                ]),
            )
            .with_file("-p/sess-1/subagents/agent-a.jsonl", asst("a1", "haiku", 3))
            .with_file(
                "-p/sess-1/workflows/wf_r1.json",
                r#"{"runId":"wf_r1","workflowName":"review","totalTokens":999999,"workflowProgress":[
                    {"type":"workflow_phase","index":1},
                    {"type":"workflow_agent","label":"find","agentId":"w1","state":"done","tokens":777777},
                    {"type":"workflow_agent","label":"refute","agentId":"w2","state":"error"},
                    {"type":"workflow_agent","label":"fetch","agentId":"w3","state":"done","cached":true,"tokens":5},
                    {"type":"workflow_agent","label":"lost","agentId":"w4","state":"done","tokens":123}
                ]}"#,
            )
            .with_file(
                "-p/sess-1/subagents/workflows/wf_r1/agent-w1.jsonl",
                asst("w1r", "sonnet", 40),
            )
            .with_file(
                "-p/sess-1/subagents/workflows/wf_r1/agent-w2.jsonl",
                asst("w2r", "opus", 50),
            )
            .with_file(
                "-p/sess-1/subagents/workflows/wf_r1/agent-w9.jsonl",
                asst("w9r", "sonnet", 60),
            )
            .with_file(
                "-p/sess-1/workflows/wf_r2.json",
                r#"{"runId":"wf_r2","workflowName":"estimate","workflowProgress":[{"type":"workflow_agent","label":"rate","agentId":"x1","state":"done"}]}"#,
            )
            .with_file(
                "-p/sess-1/subagents/workflows/wf_r2/agent-x1.jsonl",
                asst("x1r", "haiku", 7),
            )
            .with_file("-p/sess-1/workflows/wf_bad.json", r#"{"workflowName":"broken"}"#)
            .with_file("-p/sess-1/workflows/wf_num.json", r#"{"runId":42}"#)
            .with_file("-p/sess-1/workflows/wf_junk.json", "not json")
    }

    #[test]
    fn workflow_runs_are_anchored_by_tool_use_result_and_flagged() {
        let report = reap(&workflow_src());
        let runs: Vec<(&str, bool, u64)> = report
            .workflow_runs
            .iter()
            .map(|r| (r.run_id.as_str(), r.anchored, r.agent_count))
            .collect();
        assert_eq!(runs, [("wf_r1", true, 5), ("wf_r2", false, 1)]);
        let w1 = source(&report, "w1");
        assert_eq!(w1.anchor, Some(ts("2026-01-01T01:00:00Z")));
        assert_eq!(w1.label, "review / find");
        assert_eq!(w1.run_id.as_deref(), Some("wf_r1"));
        assert!(source(&report, "w2").errored);
        assert!(!w1.errored);
        let w3 = source(&report, "w3");
        assert!(w3.cached);
        assert_eq!(w3.totals, UsageSummary::default());
        // A missing transcript is zero, and its sidecar tokens are not used.
        assert_eq!(source(&report, "w4").totals, UsageSummary::default());
        assert!(has_warning(&report, "Workflow agent wf_r1/w4"));
        // A transcript with no progress entry is still counted.
        assert_eq!(source(&report, "w9").totals.usage.output, 60);
        assert_eq!(source(&report, "w9").label, "review / w9");
        // wf_r2 is named by no tool_result.
        assert!(!source(&report, "x1").anchored);
        assert!(report.unanchored.contains(&UnanchoredEntry {
            kind: UnanchoredKind::WorkflowRun,
            id: "wf_r2".into(),
            label: "estimate".into(),
        }));
        // Sidecar totalTokens and per-agent tokens are never counted.
        assert_eq!(
            report.totals.usage.output,
            10 + 20 + 3 + 40 + 50 + 60 + 7,
            "counted from transcripts only"
        );
        assert_eq!(report.workflow_runs[0].sidecar_total_tokens, Some(999_999));
    }

    #[test]
    fn bad_sidecars_are_skipped_with_their_path() {
        let report = reap(&workflow_src());
        for name in ["wf_bad.json", "wf_num.json", "wf_junk.json"] {
            let path = format!("memory:/-p/sess-1/workflows/{name}");
            assert!(
                report
                    .warnings
                    .iter()
                    .any(|w| w.message.contains(&path) && w.scope == WarningScope::Source),
                "no warning naming {path}: {:?}",
                report.warnings
            );
        }
        assert_eq!(report.workflow_runs.len(), 2, "the good runs still report");
        assert!(report.sources.len() > 1);
    }

    fn warnings_naming<'a>(report: &'a SessionReport, path: &str) -> Vec<&'a ReportWarning> {
        report
            .warnings
            .iter()
            .filter(|w| w.message.contains(path))
            .collect()
    }

    #[test]
    fn a_second_sidecar_declaring_the_same_run_id_is_skipped() {
        let report = reap(
            &MemoryTranscriptSource::new()
                .with_file(
                    "-p/sess-1.jsonl",
                    main_with(&[run_result("2026-01-01T01:00:00Z", "wf_r1")]),
                )
                .with_file(
                    "-p/sess-1/workflows/wf_a.json",
                    r#"{"runId":"wf_r1","workflowName":"first","totalTokens":11,"workflowProgress":[
                        {"type":"workflow_agent","label":"find","agentId":"w1","state":"done"}
                    ]}"#,
                )
                .with_file(
                    "-p/sess-1/workflows/wf_b.json",
                    r#"{"runId":"wf_r1","workflowName":"second","totalTokens":22,"workflowProgress":[
                        {"type":"workflow_agent","label":"other","agentId":"w1","state":"error"}
                    ]}"#,
                )
                .with_file(
                    "-p/sess-1/subagents/workflows/wf_r1/agent-w1.jsonl",
                    asst("w1r", "sonnet", 40),
                ),
        );
        let skipped = warnings_naming(&report, "memory:/-p/sess-1/workflows/wf_b.json");
        assert_eq!(skipped.len(), 1, "{:?}", report.warnings);
        assert_eq!(
            skipped[0].message,
            "skipped Workflow sidecar memory:/-p/sess-1/workflows/wf_b.json: another sidecar already declares runId wf_r1"
        );
        assert_eq!(skipped[0].scope, WarningScope::Source);
        assert!(warnings_naming(&report, "wf_a.json").is_empty());
        // The first sidecar's run and progress are kept, not overwritten.
        assert_eq!(report.workflow_runs.len(), 1);
        let run = &report.workflow_runs[0];
        assert_eq!(run.workflow_name.as_deref(), Some("first"));
        assert_eq!(run.sidecar_total_tokens, Some(11));
        assert_eq!(run.agent_count, 1);
        let w1 = source(&report, "w1");
        assert_eq!(w1.label, "first / find");
        assert!(!w1.errored);
        assert_eq!(report.totals.usage.output, 10 + 40);
    }

    #[test]
    fn a_sidecar_with_a_path_escaping_run_id_is_skipped() {
        let report = reap(
            &MemoryTranscriptSource::new()
                .with_file("-p/sess-1.jsonl", main_with(&[]))
                .with_file(
                    "-p/sess-1/workflows/wf_esc.json",
                    r#"{"runId":"../x","workflowName":"escape","workflowProgress":[
                        {"type":"workflow_agent","label":"e","agentId":"e1","state":"done"}
                    ]}"#,
                )
                .with_file(
                    "-p/sess-1/subagents/x/agent-e1.jsonl",
                    asst("e1r", "opus", 5),
                ),
        );
        let skipped = warnings_naming(&report, "memory:/-p/sess-1/workflows/wf_esc.json");
        assert_eq!(skipped.len(), 1, "{:?}", report.warnings);
        assert_eq!(
            skipped[0].message,
            r#"skipped Workflow sidecar memory:/-p/sess-1/workflows/wf_esc.json: its runId "../x" is not a usable id"#
        );
        assert_eq!(skipped[0].scope, WarningScope::Source);
        // No run, source, or path is built from the escaping id.
        assert!(report.workflow_runs.is_empty());
        assert!(
            report
                .sources
                .iter()
                .all(|s| s.kind == SourceKind::Main && s.run_id.is_none()),
            "{:?}",
            report.sources
        );
        assert!(!has_warning(&report, "e1"));
        assert!(!has_warning(&report, WORKFLOW_NESTED_UNVERIFIED));
        assert_eq!(report.totals.usage.output, 10);
    }

    #[test]
    fn an_invalid_agent_meta_warns_and_the_agent_is_still_counted() {
        let report = reap(
            &MemoryTranscriptSource::new()
                .with_file(
                    "-p/sess-1.jsonl",
                    main_with(&[tool_use("2026-01-01T00:05:00Z", "tu-b")]),
                )
                .with_file("-p/sess-1/subagents/agent-a.meta.json", "not json")
                .with_file("-p/sess-1/subagents/agent-a.jsonl", asst("a1", "haiku", 3))
                .with_file(
                    "-p/sess-1/subagents/agent-b.meta.json",
                    meta("tu-b", None, "the b agent"),
                )
                .with_file("-p/sess-1/subagents/agent-b.jsonl", asst("b1", "opus", 7)),
        );
        let path = "memory:/-p/sess-1/subagents/agent-a.meta.json";
        let bad = warnings_naming(&report, path);
        assert_eq!(bad.len(), 1, "{:?}", report.warnings);
        assert!(
            bad[0]
                .message
                .starts_with(&format!("agent a: could not read {path}: ")),
            "{}",
            bad[0].message
        );
        assert_eq!(bad[0].scope, WarningScope::Source);
        // Agent a is counted, labelled by id, and unanchored.
        let a = source(&report, "a");
        assert_eq!(a.label, "a");
        assert_eq!(a.totals.usage.output, 3);
        assert!(!a.anchored);
        assert_eq!(a.agent_type, None);
        // Agent b's valid meta is unaffected.
        let b = source(&report, "b");
        assert_eq!(b.label, "the b agent");
        assert_eq!(b.anchor, Some(ts("2026-01-01T00:05:00Z")));
        assert!(warnings_naming(&report, "agent-b.meta.json").is_empty());
        assert_eq!(report.totals.usage.output, 10 + 3 + 7);
    }

    #[test]
    fn locate_workflow_run_in_several_sessions_picks_first_and_warns() {
        let sidecar = |name: &str| {
            format!(r#"{{"runId":"wf_r","workflowName":"{name}","workflowProgress":[]}}"#)
        };
        let src = MemoryTranscriptSource::new()
            .with_file("-p/s2/workflows/wf_r.json", sidecar("second"))
            .with_file("-p/s1/workflows/wf_r.json", sidecar("first"))
            .with_file("-q/s3/workflows/wf_r.json", sidecar("third"))
            .with_file("-p/s4/workflows/wf_other.json", sidecar("other"));
        let loc = locate_workflow_run(&src, "r").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(loc.run_id, "wf_r");
        assert_eq!(
            (
                loc.session.project_slug.as_str(),
                loc.session.session_id.as_str()
            ),
            ("-p", "s1")
        );
        assert_eq!(
            loc.session.warnings,
            [
                "Workflow run wf_r is present in several sessions; using -p/s1, ignoring -p/s2, -q/s3"
            ]
        );
        let report = reap_workflow_run(&src, &loc).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(report.session_id, "s1");
        assert_eq!(report.workflow_runs.len(), 1);
        assert_eq!(
            report.workflow_runs[0].workflow_name.as_deref(),
            Some("first")
        );
        let located = warnings_naming(&report, "ignoring -p/s2, -q/s3");
        assert_eq!(located.len(), 1, "{:?}", report.warnings);
        assert_eq!(located[0].scope, WarningScope::Session);
    }

    #[test]
    fn nested_unverified_warning_iff_a_workflow_run_exists() {
        let with = reap(&workflow_src());
        assert!(has_warning(&with, WORKFLOW_NESTED_UNVERIFIED));
        let without = reap(
            &MemoryTranscriptSource::new()
                .with_file("-p/sess-1.jsonl", main_with(&[]))
                .with_file("-p/sess-1/subagents/agent-a.jsonl", asst("a1", "opus", 1)),
        );
        assert!(!has_warning(&without, WORKFLOW_NESTED_UNVERIFIED));
        // A run found only by its transcript directory still counts as a run.
        let dir_only = reap(
            &MemoryTranscriptSource::new()
                .with_file("-p/sess-1.jsonl", main_with(&[]))
                .with_file(
                    "-p/sess-1/subagents/workflows/wf_z/agent-q.jsonl",
                    asst("q", "opus", 4),
                ),
        );
        assert!(has_warning(&dir_only, WORKFLOW_NESTED_UNVERIFIED));
        assert!(has_warning(&dir_only, "no readable sidecar"));
        assert_eq!(source(&dir_only, "q").run_id.as_deref(), Some("wf_z"));
    }

    #[test]
    fn workflow_run_filter_narrows_every_section() {
        let full = reap(&workflow_src());
        let run = full
            .for_workflow_run("r1")
            .unwrap_or_else(|| panic!("wf_r1 missing"));
        assert_eq!(
            run.scope,
            ReportScope::WorkflowRun {
                run_id: "wf_r1".into()
            }
        );
        assert!(
            run.sources
                .iter()
                .all(|s| s.run_id.as_deref() == Some("wf_r1"))
        );
        assert_eq!(run.sources.len(), 5);
        assert_eq!(run.totals.usage.output, 40 + 50 + 60);
        let models: Vec<&str> = run.models.iter().map(|m| m.model.as_str()).collect();
        assert_eq!(models, ["opus", "sonnet"], "main and Agent models excluded");
        let summed = run
            .models
            .iter()
            .fold(UsageSummary::default(), |acc, m| UsageSummary {
                requests: acc.requests + m.usage.requests,
                usage: acc.usage + m.usage.usage,
                total: acc.total + m.usage.total,
            });
        assert_eq!(summed, run.totals, "narrowed models sum to narrowed totals");
        assert_eq!(run.workflow_runs.len(), 1);
        assert!(run.unanchored.is_empty(), "wf_r1 is anchored");
        // Warnings: session-wide and this run's are kept; others dropped.
        assert!(has_warning(&run, WORKFLOW_NESTED_UNVERIFIED));
        assert!(has_warning(&run, "wf_r1/w4"));
        assert!(!has_warning(&run, "wf_bad.json"));

        let r2 = full
            .for_workflow_run("wf_r2")
            .unwrap_or_else(|| panic!("wf_r2 missing"));
        assert_eq!(r2.unanchored.len(), 1);
        assert_eq!(r2.totals.usage.output, 7);
        assert!(full.for_workflow_run("wf_nope").is_none());
    }

    #[test]
    fn reap_workflow_run_reports_an_unreadable_sidecar() {
        let src = workflow_src();
        let loc = locate_workflow_run(&src, "bad").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(loc.run_id, "wf_bad");
        assert!(matches!(
            reap_workflow_run(&src, &loc),
            Err(TranscriptError::WorkflowRunUnreadable { .. })
        ));
        let ok = locate_workflow_run(&src, "wf_r1")
            .and_then(|loc| reap_workflow_run(&src, &loc))
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(ok.sources.len(), 5);
        assert!(matches!(
            locate_workflow_run(&src, "wf_missing"),
            Err(TranscriptError::WorkflowRunNotFound { .. })
        ));
    }

    #[test]
    fn a_request_in_main_and_a_subagent_is_owned_by_main() {
        let src = MemoryTranscriptSource::new()
            .with_file("-p/sess-1.jsonl", main_with(&[asst("shared", "opus", 500)]))
            .with_file(
                "-p/sess-1/subagents/agent-a.jsonl",
                lines(&[asst("shared", "opus", 500), asst("own", "opus", 1)]),
            );
        let report = reap(&src);
        assert_eq!(report.totals.requests, 3);
        assert_eq!(report.totals.usage.output, 10 + 500 + 1);
        assert_eq!(source(&report, S).duplicates_dropped, 0);
        let a = source(&report, "a");
        assert_eq!(a.duplicates_dropped, 1);
        assert_eq!(a.totals.usage.output, 1);
        assert!(has_warning(
            &report,
            "agent a (a): dropped 1 request(s) already counted by main transcript"
        ));
    }

    #[test]
    fn all_zero_request_in_a_subagent_warns_with_context() {
        let src = MemoryTranscriptSource::new()
            .with_file("-p/sess-1.jsonl", main_with(&[]))
            .with_file(
                "-p/sess-1/subagents/agent-a.jsonl",
                lines(&[asst("z", "opus", 0), asst("real", "opus", 2)]),
            )
            .with_file(
                "-p/sess-1/subagents/agent-a.meta.json",
                meta("t", None, "Zero"),
            );
        let report = reap(&src);
        assert_eq!(source(&report, "a").totals.requests, 1);
        assert!(has_warning(
            &report,
            "agent a (Zero): request z (model opus) reported all-zero usage"
        ));
    }

    #[test]
    fn missing_main_transcript_leaves_everything_unanchored() {
        let src = MemoryTranscriptSource::new()
            .with_file("-p/sess-1/subagents/agent-a.jsonl", asst("a1", "opus", 3))
            .with_file(
                "-p/sess-1/subagents/agent-a.meta.json",
                meta("t", None, "A"),
            );
        let report = reap(&src);
        assert_eq!(report.sources.len(), 1);
        assert!(!report.sources[0].anchored);
        assert_eq!(report.unanchored.len(), 1);
        assert!(has_warning(&report, "has no main transcript"));
        assert_eq!(report.totals.usage.output, 3);
    }

    #[test]
    fn report_serializes_snake_case_with_string_warnings() {
        let report = reap(&workflow_src());
        let json = serde_json::to_value(&report).unwrap_or_default();
        assert_eq!(json["scope"]["kind"], "session");
        assert_eq!(json["sources"][0]["kind"], "main");
        assert_eq!(json["sources"][0]["anchor"], "2026-01-01T00:00:00Z");
        assert!(json["sources"][0].get("requests").is_none());
        assert!(json["totals"]["cache_write_5m"].is_u64());
        assert!(json["warnings"][0].is_string());
        let kinds: Vec<&str> = json["unanchored"]
            .as_array()
            .map(|a| a.iter().filter_map(|u| u["kind"].as_str()).collect())
            .unwrap_or_default();
        assert_eq!(kinds, ["agent", "workflow_run"]);
    }
}

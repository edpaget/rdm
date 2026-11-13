//! Splits an rdm-lane checkpoint's cost and tokens by lane stage, from the
//! Claude Code trace files SCBench copies into the checkpoint.
//!
//! # Where the usage comes from
//!
//! The checkpoint's stream ([`super::outputs::STREAM_FILE`]) cannot
//! attribute cost: it repeats one event per content block with a stale
//! `output_tokens`, and never shows workflow agents. Its final `result` stays
//! the recorded total. The trace files under [`PROJECTS_DIR`] hold every
//! assistant message with its final usage:
//!
//! - `<project>/<session>.jsonl`: a main transcript;
//! - `<project>/<session>/subagents/agent-<id>.jsonl`, with
//!   `agent-<id>.meta.json` naming the spawning `Agent` call (`toolUseId`);
//! - `<project>/<session>/subagents/workflows/<runId>/agent-<id>.jsonl`: one
//!   workflow agent of run `<runId>`;
//! - `<project>/<session>/workflows/<runId>.json`: the run's `workflowName`.
//!
//! Only `assistant` lines with a `message.id` and `message.usage` count, and
//! a message counts once across the checkpoint: the last line with its id
//! wins, both its usage and its stage. Lines that are not JSON or not
//! assistant messages are skipped. A meta or workflow JSON that exists but
//! does not parse is an error naming it.
//!
//! # The attribution rule
//!
//! Stages are [`Stage::ALL`]: planning, plan review, implementation,
//! verification, code review, triage, other, and unattributed.
//!
//! 1. **Main thread.** Each main transcript is read in file order with a
//!    current stage that starts as `other`. Each `tool_use` in a line moves
//!    it:
//!    - `Agent` whose `subagent_type`, after a `<plugin>:` prefix, starts
//!      with `rdm-effort-`: its role stage, `planning` when its `prompt`
//!      contains `plan create` (the dispatch-phase planner brief), else
//!      `triage` when a code-review `Workflow` call came earlier in the same
//!      transcript, else `implementation`;
//!    - `Workflow` whose `name`, after the last `:`, is `rdm-wf-plan-review`:
//!      `plan_review`; `rdm-wf-review-refute-fix`: `code_review`; any other:
//!      `other`;
//!    - `Bash` whose `command` contains `verify run`: `verification`;
//!    - `Skill` whose `skill`, after the last `:`, is `land` or `rdm-land`:
//!      `other`;
//!    - any other `tool_use` leaves it unchanged.
//!
//!    A line counts toward the stage current after its own `tool_use`s, so a
//!    dispatching turn counts toward the stage it opens, and every turn
//!    between openers (reading results, persisting, status writes, triage
//!    replies) counts toward the stage most recently opened.
//! 2. **Agent subagents** take the stage recorded for their meta's
//!    `toolUseId`: an `rdm-effort-*` call's role stage; any other subagent
//!    type dispatched from the main thread, `other`; an `Agent` called
//!    inside a subagent or workflow agent, that agent's stage. A missing
//!    meta, or a `toolUseId` no transcript called, is `unattributed`.
//! 3. **Workflow agents** take their run's `workflowName`, mapped as in 1. A
//!    run without a readable `workflows/<runId>.json` naming it is
//!    `unattributed`.
//!
//! # Dollars and the remainder
//!
//! No trace records a per-message cost, so a message's dollars are its
//! tokens times the run's model-file prices ([`Pricing`]); a message whose
//! model the file does not name gets tokens but no dollars. Against the
//! checkpoint's recorded totals ([`Breakdown::reconcile`]) every named
//! stage keeps what it was attributed and `unattributed` takes the rest:
//! recorded cost minus the named stages' dollars, recorded tokens minus
//! their tokens. That picks up models no trace shows (Claude Code's own
//! haiku side calls), unpriced models, unplaceable messages and any trace
//! the export missed. It is signed and never clamped: a negative remainder
//! means the traces hold usage the final result does not. Without a model
//! file every stage's dollars are `None`; without a trace directory the
//! whole checkpoint is unattributed.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Where SCBench copies Claude Code's `~/.claude/projects`, under a
/// checkpoint directory.
pub const PROJECTS_DIR: &str = "agent/workspace/projects";
/// A 1-hour cache write's price as a multiple of the input price (Anthropic's
/// 1-hour write rate); the model file's `cache_write` is the 5-minute rate.
pub const HOUR_CACHE_WRITE_INPUT_MULTIPLIER: f64 = 2.0;

/// A lane stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    /// The dispatch-phase planner.
    Planning,
    /// The plan-review workflow.
    PlanReview,
    /// The implementer.
    Implementation,
    /// `rdm verify run`.
    Verification,
    /// The code-review workflow.
    CodeReview,
    /// Fix dispatches after a code review.
    Triage,
    /// Everything else the rule places (setup, landing, other subagents).
    Other,
    /// What the rule cannot place, and the remainder against the recorded
    /// totals.
    Unattributed,
}

impl Stage {
    /// Every stage, in report order.
    pub const ALL: [Stage; 8] = [
        Stage::Planning,
        Stage::PlanReview,
        Stage::Implementation,
        Stage::Verification,
        Stage::CodeReview,
        Stage::Triage,
        Stage::Other,
        Stage::Unattributed,
    ];

    /// The stage's key (`plan_review`).
    pub fn as_str(self) -> &'static str {
        match self {
            Stage::Planning => "planning",
            Stage::PlanReview => "plan_review",
            Stage::Implementation => "implementation",
            Stage::Verification => "verification",
            Stage::CodeReview => "code_review",
            Stage::Triage => "triage",
            Stage::Other => "other",
            Stage::Unattributed => "unattributed",
        }
    }
}

/// Token counts by class.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Tokens {
    /// Uncached input.
    pub input: f64,
    /// Output.
    pub output: f64,
    /// Cache reads.
    pub cache_read: f64,
    /// Cache writes, both durations.
    pub cache_write: f64,
}

impl Tokens {
    /// Every class summed.
    pub fn total(&self) -> f64 {
        self.input + self.output + self.cache_read + self.cache_write
    }

    fn add(&mut self, o: &Tokens) {
        self.input += o.input;
        self.output += o.output;
        self.cache_read += o.cache_read;
        self.cache_write += o.cache_write;
    }

    fn minus(&self, o: &Tokens) -> Tokens {
        Tokens {
            input: self.input - o.input,
            output: self.output - o.output,
            cache_read: self.cache_read - o.cache_read,
            cache_write: self.cache_write - o.cache_write,
        }
    }
}

/// One message's usage, its cache writes split by duration.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Usage {
    /// `input_tokens`.
    pub input: f64,
    /// `output_tokens`.
    pub output: f64,
    /// `cache_read_input_tokens`.
    pub cache_read: f64,
    /// 5-minute cache writes: `cache_creation_input_tokens` less the 1-hour
    /// ones.
    pub cache_write_5m: f64,
    /// `cache_creation.ephemeral_1h_input_tokens`, 0 when not split.
    pub cache_write_1h: f64,
}

impl Usage {
    /// The usage as token classes.
    pub fn tokens(&self) -> Tokens {
        Tokens {
            input: self.input,
            output: self.output,
            cache_read: self.cache_read,
            cache_write: self.cache_write_5m + self.cache_write_1h,
        }
    }
}

/// One `tool_use` block.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolUse {
    /// The block's `id`.
    pub id: String,
    /// The tool (`Agent`, `Workflow`, `Bash`, `Skill`, ...).
    pub name: String,
    /// Its `input`.
    pub input: serde_json::Value,
}

/// One assistant line of a transcript.
#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    /// `message.id`; a line without one changes stages but never counts.
    pub id: Option<String>,
    /// `message.model`.
    pub model: Option<String>,
    /// `message.usage`; a line without one never counts.
    pub usage: Option<Usage>,
    /// The line's `tool_use` blocks, in order.
    pub tool_uses: Vec<ToolUse>,
}

/// Which kind of transcript a file is.
#[derive(Debug, Clone, PartialEq)]
pub enum Origin {
    /// A main-thread `<session>.jsonl`.
    Main,
    /// An `Agent` subagent, with its meta's `toolUseId` (`None` without a
    /// readable meta).
    Subagent {
        /// The spawning `Agent` call's id.
        tool_use_id: Option<String>,
    },
    /// A workflow agent, with its run's `workflowName` (`None` without one).
    Workflow {
        /// The run's workflow name.
        name: Option<String>,
    },
}

/// One trace file, parsed.
#[derive(Debug, Clone, PartialEq)]
pub struct Transcript {
    /// Its kind.
    pub origin: Origin,
    /// Its assistant lines, in file order.
    pub messages: Vec<Message>,
}

/// The last `:`-separated segment of `s` (`rdm:rdm-effort-medium` ->
/// `rdm-effort-medium`).
fn bare(s: &str) -> &str {
    s.rsplit(':').next().unwrap_or(s)
}

fn input_str<'a>(t: &'a ToolUse, key: &str) -> &'a str {
    t.input
        .get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
}

/// The stage of workflow `name` (with or without a plugin prefix).
fn workflow_stage(name: &str) -> Stage {
    match bare(name) {
        "rdm-wf-plan-review" => Stage::PlanReview,
        "rdm-wf-review-refute-fix" => Stage::CodeReview,
        _ => Stage::Other,
    }
}

fn is_rdm_effort(t: &ToolUse) -> bool {
    bare(input_str(t, "subagent_type")).starts_with("rdm-effort-")
}

/// Every counted message with its stage, by the module's rule: one entry per
/// `message.id`, the last line with that id winning, in id order.
pub fn attribute(transcripts: &[Transcript]) -> Vec<(Stage, &Message)> {
    // The stage of every `Agent` call, by its tool_use id.
    let mut agent_stage: HashMap<&str, Stage> = HashMap::new();
    // Per transcript: the stage of each main-thread line, or the whole
    // transcript's stage once resolved.
    let mut line_stages: Vec<Vec<Stage>> = vec![Vec::new(); transcripts.len()];
    let mut whole: Vec<Option<Stage>> = vec![None; transcripts.len()];
    for (i, t) in transcripts.iter().enumerate() {
        match &t.origin {
            Origin::Main => {
                let mut current = Stage::Other;
                let mut reviewed = false;
                for m in &t.messages {
                    for tu in &m.tool_uses {
                        match tu.name.as_str() {
                            "Agent" if is_rdm_effort(tu) => {
                                let stage = if input_str(tu, "prompt").contains("plan create") {
                                    Stage::Planning
                                } else if reviewed {
                                    Stage::Triage
                                } else {
                                    Stage::Implementation
                                };
                                agent_stage.insert(&tu.id, stage);
                                current = stage;
                            }
                            "Agent" => {
                                agent_stage.insert(&tu.id, Stage::Other);
                            }
                            "Workflow" => {
                                let stage = workflow_stage(input_str(tu, "name"));
                                if stage == Stage::CodeReview {
                                    reviewed = true;
                                }
                                current = stage;
                            }
                            "Bash" if input_str(tu, "command").contains("verify run") => {
                                current = Stage::Verification;
                            }
                            "Skill"
                                if matches!(bare(input_str(tu, "skill")), "land" | "rdm-land") =>
                            {
                                current = Stage::Other;
                            }
                            _ => {}
                        }
                    }
                    line_stages[i].push(current);
                }
            }
            Origin::Workflow { name } => {
                whole[i] = Some(name.as_deref().map_or(Stage::Unattributed, workflow_stage));
            }
            Origin::Subagent { .. } => {}
        }
    }
    // Resolve subagents through their spawning call, and pass each resolved
    // agent's stage to the agents it spawned, until nothing changes.
    loop {
        let mut changed = false;
        for (i, t) in transcripts.iter().enumerate() {
            if whole[i].is_none()
                && let Origin::Subagent {
                    tool_use_id: Some(id),
                } = &t.origin
                && let Some(&s) = agent_stage.get(id.as_str())
            {
                whole[i] = Some(s);
                changed = true;
            }
            if let Some(s) = whole[i] {
                for tu in t.messages.iter().flat_map(|m| &m.tool_uses) {
                    if tu.name == "Agent" && !agent_stage.contains_key(tu.id.as_str()) {
                        agent_stage.insert(&tu.id, s);
                        changed = true;
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
    let mut out: BTreeMap<&str, (Stage, &Message)> = BTreeMap::new();
    for (i, t) in transcripts.iter().enumerate() {
        for (j, m) in t.messages.iter().enumerate() {
            let (Some(id), Some(_)) = (&m.id, &m.usage) else {
                continue;
            };
            let stage = match t.origin {
                Origin::Main => line_stages[i][j],
                _ => whole[i].unwrap_or(Stage::Unattributed),
            };
            out.insert(id, (stage, m));
        }
    }
    out.into_values().collect()
}

/// The prices of the run's model file (USD per million tokens).
#[derive(Debug, Clone, PartialEq)]
pub struct Pricing {
    names: Vec<String>,
    input: f64,
    output: f64,
    cache_read: f64,
    cache_write: f64,
}

#[derive(Deserialize)]
struct ModelFileYaml {
    #[serde(default)]
    internal_name: Option<String>,
    #[serde(default)]
    aliases: Vec<String>,
    pricing: PricingYaml,
}

#[derive(Deserialize)]
struct PricingYaml {
    input: f64,
    output: f64,
    cache_read: f64,
    cache_write: f64,
}

impl Pricing {
    /// Parses an SCBench model file's `internal_name`, `aliases` and
    /// `pricing`.
    ///
    /// # Errors
    ///
    /// When it is not YAML of that shape.
    pub fn parse(text: &str) -> Result<Self, String> {
        let y: ModelFileYaml = serde_saphyr::from_str(text).map_err(|e| e.to_string())?;
        Ok(Self {
            names: y.internal_name.into_iter().chain(y.aliases).collect(),
            input: y.pricing.input,
            output: y.pricing.output,
            cache_read: y.pricing.cache_read,
            cache_write: y.pricing.cache_write,
        })
    }

    /// Reads the model file at `path`; `None` when it does not exist.
    ///
    /// # Errors
    ///
    /// When it exists but cannot be read or parsed, naming it.
    pub fn read(path: &Path) -> Result<Option<Self>, String> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => {
                return Err(format!(
                    "cannot read the model file {}: {e}",
                    path.display()
                ));
            }
        };
        Self::parse(&text)
            .map(Some)
            .map_err(|e| format!("cannot parse the model file {}: {e}", path.display()))
    }

    /// `usage`'s dollars at these prices; `None` when `model` is neither the
    /// file's `internal_name` nor one of its aliases.
    pub fn cost(&self, model: Option<&str>, usage: &Usage) -> Option<f64> {
        let model = model?;
        self.names.iter().any(|n| n == model).then(|| {
            (usage.input * self.input
                + usage.output * self.output
                + usage.cache_read * self.cache_read
                + usage.cache_write_5m * self.cache_write
                + usage.cache_write_1h * self.input * HOUR_CACHE_WRITE_INPUT_MULTIPLIER)
                / 1_000_000.0
        })
    }
}

/// One stage's share of a checkpoint.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct StageUsage {
    /// Its dollars; `None` without a model file.
    pub cost_usd: Option<f64>,
    /// Its tokens.
    pub tokens: Tokens,
}

/// A checkpoint's usage per [`Stage`]; every stage is present.
#[derive(Debug, Clone, PartialEq)]
pub struct Breakdown {
    stages: BTreeMap<Stage, StageUsage>,
}

impl Breakdown {
    /// The usage of `stage`.
    pub fn get(&self, stage: Stage) -> StageUsage {
        self.stages.get(&stage).copied().unwrap_or_default()
    }

    /// The attributed sums of `transcripts` ([`attribute`]), priced with
    /// `pricing`.
    pub fn attributed(transcripts: &[Transcript], pricing: Option<&Pricing>) -> Self {
        let mut sums: BTreeMap<Stage, (f64, Tokens)> = BTreeMap::new();
        for (stage, m) in attribute(transcripts) {
            let Some(usage) = &m.usage else { continue };
            let e = sums.entry(stage).or_default();
            e.1.add(&usage.tokens());
            if let Some(c) = pricing.and_then(|p| p.cost(m.model.as_deref(), usage)) {
                e.0 += c;
            }
        }
        Self {
            stages: Stage::ALL
                .iter()
                .map(|&s| {
                    let (cost, tokens) = sums.get(&s).copied().unwrap_or_default();
                    (
                        s,
                        StageUsage {
                            cost_usd: pricing.map(|_| cost),
                            tokens,
                        },
                    )
                })
                .collect(),
        }
    }

    /// The checkpoint's breakdown against its recorded cost and tokens: every
    /// named stage keeps what `attributed` gave it (nothing without a trace),
    /// and [`Stage::Unattributed`] is the recorded totals minus the named
    /// stages, signed. Without `priced`, every stage's dollars are `None`.
    pub fn reconcile(
        attributed: Option<&Breakdown>,
        recorded_cost: f64,
        recorded: &Tokens,
        priced: bool,
    ) -> Self {
        let mut stages = BTreeMap::new();
        let (mut cost, mut tokens) = (0.0, Tokens::default());
        for s in Stage::ALL.into_iter().filter(|&s| s != Stage::Unattributed) {
            let u = attributed.map(|b| b.get(s)).unwrap_or_default();
            cost += u.cost_usd.unwrap_or(0.0);
            tokens.add(&u.tokens);
            stages.insert(
                s,
                StageUsage {
                    cost_usd: priced.then_some(u.cost_usd.unwrap_or(0.0)),
                    tokens: u.tokens,
                },
            );
        }
        stages.insert(
            Stage::Unattributed,
            StageUsage {
                cost_usd: priced.then_some(recorded_cost - cost),
                tokens: recorded.minus(&tokens),
            },
        );
        Self { stages }
    }
}

// --- Reading the trace tree -------------------------------------------------

#[derive(Deserialize)]
struct LineJson {
    #[serde(rename = "type", default)]
    kind: Option<String>,
    #[serde(default)]
    message: Option<MessageJson>,
}

#[derive(Deserialize)]
struct MessageJson {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    usage: Option<UsageJson>,
    #[serde(default)]
    content: serde_json::Value,
}

#[derive(Deserialize)]
struct UsageJson {
    #[serde(default)]
    input_tokens: f64,
    #[serde(default)]
    output_tokens: f64,
    #[serde(default)]
    cache_read_input_tokens: f64,
    #[serde(default)]
    cache_creation_input_tokens: f64,
    #[serde(default)]
    cache_creation: Option<CacheCreationJson>,
}

#[derive(Deserialize)]
struct CacheCreationJson {
    #[serde(default)]
    ephemeral_1h_input_tokens: f64,
}

#[derive(Deserialize)]
struct MetaJson {
    #[serde(rename = "toolUseId", default)]
    tool_use_id: Option<String>,
}

#[derive(Deserialize)]
struct WorkflowRunJson {
    #[serde(rename = "workflowName", default)]
    workflow_name: Option<String>,
}

impl From<MessageJson> for Message {
    fn from(m: MessageJson) -> Self {
        let tool_uses = m
            .content
            .as_array()
            .into_iter()
            .flatten()
            .filter(|b| b.get("type").and_then(serde_json::Value::as_str) == Some("tool_use"))
            .map(|b| ToolUse {
                id: b
                    .get("id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_owned(),
                name: b
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_owned(),
                input: b.get("input").cloned().unwrap_or(serde_json::Value::Null),
            })
            .collect();
        Message {
            id: m.id,
            model: m.model,
            usage: m.usage.map(|u| {
                let hour = u
                    .cache_creation
                    .map_or(0.0, |c| c.ephemeral_1h_input_tokens);
                Usage {
                    input: u.input_tokens,
                    output: u.output_tokens,
                    cache_read: u.cache_read_input_tokens,
                    cache_write_5m: u.cache_creation_input_tokens - hour,
                    cache_write_1h: hour,
                }
            }),
            tool_uses,
        }
    }
}

/// The assistant lines of the transcript at `path`.
fn read_messages(path: &Path) -> Result<Vec<Message>, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    Ok(text
        .lines()
        .filter_map(|l| serde_json::from_str::<LineJson>(l).ok())
        .filter(|l| l.kind.as_deref() == Some("assistant"))
        .filter_map(|l| l.message.map(Message::from))
        .collect())
}

/// Reads `path` as `T`; `None` when it does not exist.
fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<Option<T>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|e| format!("cannot parse {}: {e}", path.display()))
}

/// The entries of `dir`, sorted by name, that are directories (`dirs`) or
/// files ending in `suffix`.
fn entries(dir: &Path, dirs: bool, suffix: &str) -> Result<Vec<std::path::PathBuf>, String> {
    let read = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("cannot read {}: {e}", dir.display())),
    };
    let mut out = Vec::new();
    for entry in read {
        let path = entry
            .map_err(|e| format!("cannot read {}: {e}", dir.display()))?
            .path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let keep = if dirs {
            path.is_dir()
        } else {
            path.is_file() && name.ends_with(suffix)
        };
        if keep {
            out.push(path);
        }
    }
    out.sort();
    Ok(out)
}

/// The `agent-*.jsonl` transcripts of `dir`.
fn agent_files(dir: &Path) -> Result<Vec<std::path::PathBuf>, String> {
    Ok(entries(dir, false, ".jsonl")?
        .into_iter()
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("agent-"))
        })
        .collect())
}

/// Every transcript under the projects directory `projects`: per project,
/// its main transcripts, then per session its subagents and its workflow
/// agents, each set in name order.
///
/// # Errors
///
/// When a directory or transcript cannot be read, or a meta or workflow JSON
/// exists but does not parse.
pub fn read_transcripts(projects: &Path) -> Result<Vec<Transcript>, String> {
    let mut out = Vec::new();
    for project in entries(projects, true, "")? {
        for main in entries(&project, false, ".jsonl")? {
            out.push(Transcript {
                origin: Origin::Main,
                messages: read_messages(&main)?,
            });
        }
        for session in entries(&project, true, "")? {
            let subagents = session.join("subagents");
            for path in agent_files(&subagents)? {
                let meta: Option<MetaJson> = read_json(&path.with_extension("meta.json"))?;
                out.push(Transcript {
                    origin: Origin::Subagent {
                        tool_use_id: meta.and_then(|m| m.tool_use_id),
                    },
                    messages: read_messages(&path)?,
                });
            }
            for run in entries(&subagents.join("workflows"), true, "")? {
                let id = run.file_name().and_then(|n| n.to_str()).unwrap_or("");
                let info: Option<WorkflowRunJson> =
                    read_json(&session.join("workflows").join(format!("{id}.json")))?;
                let name = info.and_then(|w| w.workflow_name);
                for path in agent_files(&run)? {
                    out.push(Transcript {
                        origin: Origin::Workflow { name: name.clone() },
                        messages: read_messages(&path)?,
                    });
                }
            }
        }
    }
    Ok(out)
}

/// The attributed usage of the checkpoint at `dir` ([`Breakdown::attributed`]
/// over [`read_transcripts`]); `None` when it has no [`PROJECTS_DIR`].
///
/// # Errors
///
/// As [`read_transcripts`].
pub fn read_checkpoint(dir: &Path, pricing: Option<&Pricing>) -> Result<Option<Breakdown>, String> {
    let projects = dir.join(PROJECTS_DIR);
    if !projects.is_dir() {
        return Ok(None);
    }
    let transcripts = read_transcripts(&projects)?;
    Ok(Some(Breakdown::attributed(&transcripts, pricing)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const MODEL: &str = "claude-test-1";

    /// A model file pricing input 4, output 20, cache read 0.2, 5-minute
    /// cache write 5 USD/MTok.
    const MODEL_FILE: &str = "internal_name: claude-test-1\nprovider: anthropic\naliases:\n  - \
                              test-1\npricing:\n  input: 4\n  output: 20\n  cache_read: 0.20\n  \
                              cache_write: 5.00\n";

    fn pricing() -> Pricing {
        Pricing::parse(MODEL_FILE).expect("model file")
    }

    /// An assistant line `id` with `input` input tokens and the tool uses
    /// `(id, name, input)`.
    fn msg(id: &str, input: f64, tools: &[(&str, &str, serde_json::Value)]) -> Message {
        Message {
            id: Some(id.to_owned()),
            model: Some(MODEL.to_owned()),
            usage: Some(Usage {
                input,
                ..Usage::default()
            }),
            tool_uses: tools
                .iter()
                .map(|(id, name, input)| ToolUse {
                    id: (*id).to_owned(),
                    name: (*name).to_owned(),
                    input: input.clone(),
                })
                .collect(),
        }
    }

    /// An `rdm-effort` `Agent` call `id` with `prompt`.
    fn rdm_agent(
        id: &'static str,
        prompt: &str,
    ) -> (&'static str, &'static str, serde_json::Value) {
        (
            id,
            "Agent",
            json!({ "subagent_type": "rdm:rdm-effort-medium", "prompt": prompt }),
        )
    }

    fn main(messages: Vec<Message>) -> Transcript {
        Transcript {
            origin: Origin::Main,
            messages,
        }
    }

    fn sub(tool_use_id: Option<&str>, messages: Vec<Message>) -> Transcript {
        Transcript {
            origin: Origin::Subagent {
                tool_use_id: tool_use_id.map(str::to_owned),
            },
            messages,
        }
    }

    fn wf(name: Option<&str>, messages: Vec<Message>) -> Transcript {
        Transcript {
            origin: Origin::Workflow {
                name: name.map(str::to_owned),
            },
            messages,
        }
    }

    fn stages_by_id(ts: &[Transcript]) -> BTreeMap<String, Stage> {
        attribute(ts)
            .into_iter()
            .map(|(s, m)| (m.id.clone().expect("id"), s))
            .collect()
    }

    /// The dispatch-phase sequence of the main thread, one line each.
    fn dispatch_main() -> Transcript {
        main(vec![
            msg("m01", 1.0, &[]),
            msg(
                "m02",
                1.0,
                &[("s1", "Skill", json!({ "skill": "rdm:dispatch-phase" }))],
            ),
            msg(
                "m03",
                1.0,
                &[rdm_agent(
                    "a-plan",
                    "Write the plan; run rdm plan create ...",
                )],
            ),
            msg("m04", 1.0, &[]),
            msg(
                "m05",
                1.0,
                &[(
                    "w1",
                    "Workflow",
                    json!({ "name": "rdm:rdm-wf-plan-review" }),
                )],
            ),
            msg(
                "m06",
                1.0,
                &[rdm_agent("a-impl", "Implement the approved plan.")],
            ),
            msg(
                "m07",
                1.0,
                &[(
                    "b1",
                    "Bash",
                    json!({ "command": "rdm verify run --item phase/x/phase-1 --project p" }),
                )],
            ),
            msg(
                "m08",
                1.0,
                &[("b2", "Bash", json!({ "command": "git log --oneline" }))],
            ),
            msg(
                "m09",
                1.0,
                &[(
                    "w2",
                    "Workflow",
                    json!({ "name": "rdm-wf-review-refute-fix" }),
                )],
            ),
            msg("m10", 1.0, &[]),
            msg(
                "m11",
                1.0,
                &[rdm_agent("a-fix", "Fix the review comments.")],
            ),
            msg(
                "m12",
                1.0,
                &[(
                    "w3",
                    "Workflow",
                    json!({ "name": "rdm:rdm-wf-review-refute-fix" }),
                )],
            ),
            msg(
                "m13",
                1.0,
                &[("s2", "Skill", json!({ "skill": "rdm:land" }))],
            ),
            msg(
                "m14",
                1.0,
                &[(
                    "a-explore",
                    "Agent",
                    json!({ "subagent_type": "Explore", "prompt": "look" }),
                )],
            ),
        ])
    }

    #[test]
    fn main_thread_lines_take_the_most_recently_opened_stage() {
        use Stage::*;
        let got = stages_by_id(&[dispatch_main()]);
        let want: BTreeMap<String, Stage> = [
            ("m01", Other),
            ("m02", Other),
            ("m03", Planning),
            ("m04", Planning),
            ("m05", PlanReview),
            ("m06", Implementation),
            ("m07", Verification),
            ("m08", Verification),
            ("m09", CodeReview),
            ("m10", CodeReview),
            ("m11", Triage),
            ("m12", CodeReview),
            ("m13", Other),
            ("m14", Other),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect();
        assert_eq!(got, want);
    }

    #[test]
    fn subagents_take_their_spawning_calls_stage() {
        use Stage::*;
        let ts = [
            dispatch_main(),
            sub(Some("a-plan"), vec![msg("p1", 1.0, &[])]),
            sub(Some("a-impl"), vec![msg("i1", 1.0, &[])]),
            sub(
                Some("a-fix"),
                vec![msg(
                    "f1",
                    1.0,
                    &[("a-nested", "Agent", json!({ "subagent_type": "Explore" }))],
                )],
            ),
            // Listed before its parent resolves nothing differently.
            sub(Some("a-nested"), vec![msg("n1", 1.0, &[])]),
            sub(Some("a-explore"), vec![msg("e1", 1.0, &[])]),
            sub(Some("a-unknown"), vec![msg("u1", 1.0, &[])]),
            sub(None, vec![msg("u2", 1.0, &[])]),
        ];
        let got = stages_by_id(&ts);
        for (id, want) in [
            ("p1", Planning),
            ("i1", Implementation),
            ("f1", Triage),
            ("n1", Triage),
            ("e1", Other),
            ("u1", Unattributed),
            ("u2", Unattributed),
        ] {
            assert_eq!(got[id], want, "{id}");
        }
    }

    #[test]
    fn workflow_agents_take_their_runs_workflow() {
        use Stage::*;
        let ts = [
            wf(Some("rdm-wf-plan-review"), vec![msg("w1", 1.0, &[])]),
            wf(Some("rdm-wf-review-refute-fix"), vec![msg("w2", 1.0, &[])]),
            wf(Some("rdm-wf-estimate"), vec![msg("w3", 1.0, &[])]),
            wf(None, vec![msg("w4", 1.0, &[])]),
        ];
        let got = stages_by_id(&ts);
        assert_eq!(
            [&got["w1"], &got["w2"], &got["w3"], &got["w4"]],
            [&PlanReview, &CodeReview, &Other, &Unattributed]
        );
    }

    #[test]
    fn a_repeated_message_id_counts_once_and_the_last_line_wins() {
        let t = main(vec![
            msg("x", 10.0, &[]),
            msg("x", 30.0, &[rdm_agent("a1", "Implement.")]),
            msg("y", 5.0, &[]),
        ]);
        let b = Breakdown::attributed(&[t], None);
        assert_eq!(b.get(Stage::Implementation).tokens.input, 35.0);
        assert_eq!(b.get(Stage::Other).tokens.input, 0.0);
        assert_eq!(b.get(Stage::Implementation).cost_usd, None);
    }

    #[test]
    fn dollars_price_both_cache_write_durations_and_skip_unknown_models() {
        let p = pricing();
        let u = Usage {
            input: 1_000_000.0,
            output: 100_000.0,
            cache_read: 10_000_000.0,
            cache_write_5m: 200_000.0,
            cache_write_1h: 500_000.0,
        };
        // 4 + 2 + 2 + 1 + 0.5 * 4 * 2 = 13.
        assert_eq!(p.cost(Some("claude-test-1"), &u), Some(13.0));
        assert_eq!(p.cost(Some("test-1"), &u), Some(13.0));
        assert_eq!(p.cost(Some("claude-other"), &u), None);
        assert_eq!(p.cost(None, &u), None);

        let mut unknown = msg("z", 1_000_000.0, &[]);
        unknown.model = Some("claude-other".to_owned());
        let b = Breakdown::attributed(&[main(vec![msg("a", 1_000_000.0, &[]), unknown])], Some(&p));
        let other = b.get(Stage::Other);
        assert_eq!(other.tokens.input, 2_000_000.0);
        assert_eq!(other.cost_usd, Some(4.0));
        assert_eq!(b.get(Stage::Planning).cost_usd, Some(0.0));
    }

    #[test]
    fn the_remainder_against_the_recorded_totals_is_unattributed() {
        let p = pricing();
        // 1M input in planning ($4), 1M input in other ($4).
        let t = main(vec![
            msg("a", 1_000_000.0, &[]),
            msg("b", 1_000_000.0, &[rdm_agent("a1", "rdm plan create")]),
        ]);
        let attributed = Breakdown::attributed(&[t], Some(&p));
        // The recorded totals add an untraced model's 500 input, 300 output
        // tokens and $0.25.
        let recorded = Tokens {
            input: 2_000_500.0,
            output: 300.0,
            cache_read: 0.0,
            cache_write: 0.0,
        };
        let b = Breakdown::reconcile(Some(&attributed), 8.25, &recorded, true);
        assert_eq!(b.get(Stage::Planning).cost_usd, Some(4.0));
        assert_eq!(b.get(Stage::Other).cost_usd, Some(4.0));
        let u = b.get(Stage::Unattributed);
        assert_eq!(u.cost_usd, Some(0.25));
        assert_eq!(
            u.tokens,
            Tokens {
                input: 500.0,
                output: 300.0,
                cache_read: 0.0,
                cache_write: 0.0
            }
        );
        let total: f64 = Stage::ALL
            .iter()
            .map(|&s| b.get(s).cost_usd.expect("priced"))
            .sum();
        assert_eq!(total, 8.25);

        // No trace: everything is unattributed. No prices: no dollars.
        let none = Breakdown::reconcile(None, 8.25, &recorded, true);
        assert_eq!(none.get(Stage::Unattributed).cost_usd, Some(8.25));
        assert_eq!(none.get(Stage::Unattributed).tokens, recorded);
        assert_eq!(none.get(Stage::Planning).cost_usd, Some(0.0));
        let unpriced = Breakdown::reconcile(Some(&attributed), 8.25, &recorded, false);
        assert!(
            Stage::ALL
                .iter()
                .all(|&s| unpriced.get(s).cost_usd.is_none())
        );
        assert_eq!(unpriced.get(Stage::Planning).tokens.input, 1_000_000.0);
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, text).expect("write");
    }

    fn line(id: &str, input: u64, hour: u64, tools: serde_json::Value) -> String {
        json!({
            "type": "assistant",
            "message": {
                "id": id, "model": MODEL, "content": tools,
                "usage": {
                    "input_tokens": input, "output_tokens": 0,
                    "cache_read_input_tokens": 0, "cache_creation_input_tokens": hour + 100,
                    "cache_creation": { "ephemeral_1h_input_tokens": hour,
                                        "ephemeral_5m_input_tokens": 100 },
                },
            },
        })
        .to_string()
    }

    #[test]
    fn a_trace_tree_is_read_and_attributed() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let project = tmp.path().join(PROJECTS_DIR).join("-workspace");
        let session = project.join("s1");
        let main_lines = [
            json!({ "type": "user", "message": { "content": "go" } }).to_string(),
            "not json".to_owned(),
            line(
                "m1",
                10,
                0,
                json!([{ "type": "tool_use", "id": "t-plan", "name": "Agent",
                         "input": { "subagent_type": "rdm-effort-low", "prompt": "rdm plan create" } }]),
            ),
            line(
                "m2",
                20,
                0,
                json!([{ "type": "tool_use", "id": "t-wf", "name": "Workflow",
                         "input": { "name": "rdm:rdm-wf-plan-review" } }]),
            ),
        ];
        write(&project.join("s1.jsonl"), &main_lines.join("\n"));
        write(
            &session.join("subagents/agent-a1.jsonl"),
            &line("p1", 100, 1000, json!([])),
        );
        write(
            &session.join("subagents/agent-a1.meta.json"),
            r#"{"agentType":"rdm:rdm-effort-low","toolUseId":"t-plan"}"#,
        );
        write(
            &session.join("subagents/workflows/wf_1/agent-b1.jsonl"),
            &line("r1", 1000, 0, json!([])),
        );
        write(
            &session.join("subagents/workflows/wf_1/agent-b1.meta.json"),
            r#"{"agentType":"workflow-subagent","description":"find:plan:x"}"#,
        );
        write(
            &session.join("workflows/wf_1.json"),
            r#"{"runId":"wf_1","workflowName":"rdm-wf-plan-review"}"#,
        );
        write(
            &session.join("subagents/workflows/wf_2/agent-c1.jsonl"),
            &line("q1", 7, 0, json!([])),
        );
        write(&project.join("rdm-plan/000001.tar"), "tar");

        let b = read_checkpoint(tmp.path(), Some(&pricing()))
            .expect("read")
            .expect("trace");
        let planning = b.get(Stage::Planning);
        assert_eq!(planning.tokens.input, 110.0);
        assert_eq!(planning.tokens.cache_write, 1200.0);
        // (110 * 4 + 200 * 5 + 1000 * 8) / 1e6.
        assert_eq!(planning.cost_usd, Some(9440.0 / 1_000_000.0));
        assert_eq!(b.get(Stage::PlanReview).tokens.input, 1020.0);
        assert_eq!(b.get(Stage::Unattributed).tokens.input, 7.0);

        assert_eq!(read_checkpoint(&tmp.path().join("none"), None), Ok(None));

        let meta = session.join("subagents/agent-a1.meta.json");
        write(&meta, "{not json");
        let e = read_checkpoint(tmp.path(), None).expect_err("bad meta");
        assert!(e.contains(&meta.display().to_string()), "{e}");
    }

    #[test]
    fn a_model_file_that_does_not_parse_is_named() {
        let tmp = tempfile::tempdir().expect("tempdir");
        assert_eq!(Pricing::read(&tmp.path().join("missing.yaml")), Ok(None));
        let bad = tmp.path().join("m.yaml");
        std::fs::write(&bad, "internal_name: x\n").expect("write");
        let e = Pricing::read(&bad).expect_err("no pricing");
        assert!(e.contains(&bad.display().to_string()), "{e}");
    }
}

//! Splits an rdm-lane checkpoint's cost and tokens by lane stage, from the
//! Claude Code trace files SCBench copies into the checkpoint.
//!
//! # Where the usage comes from
//!
//! The checkpoint's stream ([`super::outputs::STREAM_FILE`]) cannot
//! attribute cost: it repeats one event per content block with a stale
//! `output_tokens`, and never shows workflow agents. Its final `result` stays
//! the recorded total. The trace files under [`PROJECTS_DIR`] are a Claude
//! Code projects tree holding every request with its final usage, and
//! reading them is [`rdm_core::transcript`]'s job: every session it lists
//! ([`rdm_core::transcript::list_sessions`]) is reaped with
//! [`rdm_core::transcript::reap_session`]. Which files make up a session,
//! how requests are deduped (by `requestId`, the first source in canonical
//! order owning a request), how a request's tokens split into classes, and
//! which subagent hangs off which main-thread `tool_use` are therefore core's
//! rules, the same ones `rdm cost` uses; whatever core tolerates and cannot
//! place (an unreadable meta file, a run without a sidecar) reaches this
//! module as a source without an anchor or a workflow name.
//!
//! # The attribution rule
//!
//! Stages are [`Stage::ALL`]: planning, plan review, implementation,
//! verification, code review, triage, other, and unattributed.
//!
//! 1. **Main thread.** The main source's requests are read in order with a
//!    current stage that starts as `other`. Each `tool_use` of a request
//!    moves it:
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
//!    A request counts toward the stage current after its own `tool_use`s,
//!    so a dispatching turn counts toward the stage it opens, and every turn
//!    between openers (reading results, persisting, status writes, triage
//!    replies) counts toward the stage most recently opened.
//! 2. **Agent subagents** take the stage of the main-thread `Agent` call
//!    core anchors them to ([`SourceReport::anchor_tool_use_id`]): an
//!    `rdm-effort-*` call's role stage; any other subagent type, `other`. A
//!    nested subagent is anchored through its `parentAgentId` chain, so it
//!    takes its top-level ancestor's stage. A subagent core cannot anchor (no
//!    readable meta, a `toolUseId` the main thread never called, a parent
//!    below a workflow agent) is `unattributed`.
//! 3. **Workflow agents** take their run's `workflowName`, mapped as in 1. A
//!    run without a readable sidecar naming it is `unattributed`.
//!
//! # Dollars and the remainder
//!
//! No trace records a per-request cost, so a request's dollars are its
//! tokens times the run's model-file prices ([`Pricing`]); a request whose
//! model the file does not name gets tokens but no dollars. Against the
//! checkpoint's recorded totals ([`Breakdown::reconcile`]) every named
//! stage keeps what it was attributed and `unattributed` takes the rest:
//! recorded cost minus the named stages' dollars, recorded tokens minus
//! their tokens. That picks up models no trace shows (Claude Code's own
//! haiku side calls), unpriced models, unplaceable requests and any trace
//! the export missed. It is signed and never clamped: a negative remainder
//! means the traces hold usage the final result does not. Without a model
//! file every stage's dollars are `None`; without a trace directory the
//! whole checkpoint is unattributed.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use rdm_core::transcript::{SessionReport, SourceKind, SourceReport, list_sessions, reap_session};
use rdm_core::usage::{RequestUsage, TokenUsage, ToolUse};
use rdm_transcript::FsTranscriptSource;
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

impl From<&TokenUsage> for Tokens {
    #[allow(clippy::cast_precision_loss)]
    fn from(u: &TokenUsage) -> Self {
        Tokens {
            input: u.input as f64,
            output: u.output as f64,
            cache_read: u.cache_read as f64,
            cache_write: u.cache_write() as f64,
        }
    }
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

/// Every request `report` counts, with its stage by the module's rule: the
/// main source's requests first, in order, then every other source's.
pub fn attribute(report: &SessionReport) -> Vec<(Stage, &RequestUsage)> {
    // The stage of every main-thread `Agent` call, by its tool_use id.
    let mut agent_stage: HashMap<&str, Stage> = HashMap::new();
    let mut out = Vec::new();
    for source in report.sources.iter().filter(|s| s.kind == SourceKind::Main) {
        let mut current = Stage::Other;
        let mut reviewed = false;
        for request in &source.requests {
            for tu in &request.tool_uses {
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
                    "Skill" if matches!(bare(input_str(tu, "skill")), "land" | "rdm-land") => {
                        current = Stage::Other;
                    }
                    _ => {}
                }
            }
            out.push((current, request));
        }
    }
    let stage_of = |source: &SourceReport| match source.kind {
        SourceKind::Main => None,
        SourceKind::Agent => Some(
            source
                .anchor_tool_use_id
                .as_deref()
                .and_then(|id| agent_stage.get(id).copied())
                .unwrap_or(Stage::Unattributed),
        ),
        SourceKind::WorkflowAgent => Some(
            source
                .workflow_name
                .as_deref()
                .map_or(Stage::Unattributed, workflow_stage),
        ),
    };
    for source in &report.sources {
        if let Some(stage) = stage_of(source) {
            out.extend(source.requests.iter().map(|r| (stage, r)));
        }
    }
    out
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
    #[allow(clippy::cast_precision_loss)]
    pub fn cost(&self, model: &str, usage: &TokenUsage) -> Option<f64> {
        self.names.iter().any(|n| n == model).then(|| {
            (usage.input as f64 * self.input
                + usage.output as f64 * self.output
                + usage.cache_read as f64 * self.cache_read
                + usage.cache_write_5m as f64 * self.cache_write
                + usage.cache_write_1h as f64 * self.input * HOUR_CACHE_WRITE_INPUT_MULTIPLIER)
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

    /// The attributed sums of the sessions `reports` ([`attribute`]), priced
    /// with `pricing`.
    pub fn attributed(reports: &[SessionReport], pricing: Option<&Pricing>) -> Self {
        let mut sums: BTreeMap<Stage, (f64, Tokens)> = BTreeMap::new();
        for (stage, r) in reports.iter().flat_map(attribute) {
            let e = sums.entry(stage).or_default();
            e.1.add(&Tokens::from(&r.usage));
            if let Some(c) = pricing.and_then(|p| p.cost(&r.model, &r.usage)) {
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

/// The attributed usage of the checkpoint at `dir`
/// ([`Breakdown::attributed`] over every session core reaps from its
/// [`PROJECTS_DIR`]); `None` when it has no [`PROJECTS_DIR`].
///
/// # Errors
///
/// When core cannot list a directory of the tree or read a main transcript,
/// naming the projects directory.
pub fn read_checkpoint(dir: &Path, pricing: Option<&Pricing>) -> Result<Option<Breakdown>, String> {
    let projects = dir.join(PROJECTS_DIR);
    if !projects.is_dir() {
        return Ok(None);
    }
    let src = FsTranscriptSource::new(projects.clone());
    let reports = list_sessions(&src)
        .and_then(|sessions| {
            sessions
                .iter()
                .map(|location| reap_session(&src, location))
                .collect::<Result<Vec<_>, _>>()
        })
        .map_err(|e| format!("cannot read the traces under {}: {e}", projects.display()))?;
    Ok(Some(Breakdown::attributed(&reports, pricing)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rdm_core::transcript::{MemoryTranscriptSource, locate_session};
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

    /// A tool call: `(id, name, input)`.
    type Call = (&'static str, &'static str, serde_json::Value);

    /// An assistant line of request `id` on `model`, with `input` input
    /// tokens and the calls `calls`.
    fn model_line(id: &str, model: &str, input: u64, calls: &[Call]) -> String {
        let content: Vec<serde_json::Value> = calls
            .iter()
            .map(|(id, name, input)| {
                json!({ "type": "tool_use", "id": id, "name": name, "input": input })
            })
            .collect();
        json!({
            "type": "assistant",
            "requestId": id,
            "timestamp": "2026-01-01T00:00:00Z",
            "message": {
                "id": format!("msg_{id}"), "model": model, "content": content,
                "usage": { "input_tokens": input },
            },
        })
        .to_string()
    }

    /// [`model_line`] on the priced model.
    fn line(id: &str, input: u64, calls: &[Call]) -> String {
        model_line(id, MODEL, input, calls)
    }

    /// An `rdm-effort` `Agent` call `id` with `prompt`.
    fn rdm_agent(id: &'static str, prompt: &str) -> Call {
        (
            id,
            "Agent",
            json!({ "subagent_type": "rdm:rdm-effort-medium", "prompt": prompt }),
        )
    }

    /// An in-memory projects tree holding session `s` under `-w`.
    struct Tree(MemoryTranscriptSource);

    impl Tree {
        fn new(main: &[String]) -> Self {
            Tree(MemoryTranscriptSource::new().with_file("-w/s.jsonl", main.join("\n")))
        }

        /// Adds `Agent` subagent `id` with its meta (none when `None`).
        fn agent(self, id: &str, meta: Option<serde_json::Value>, lines: &[String]) -> Self {
            let mut src = self.0.with_file(
                &format!("-w/s/subagents/agent-{id}.jsonl"),
                lines.join("\n"),
            );
            if let Some(meta) = meta {
                src = src.with_file(
                    &format!("-w/s/subagents/agent-{id}.meta.json"),
                    meta.to_string(),
                );
            }
            Tree(src)
        }

        /// Adds agent `id` of workflow run `wf_<run>`, with a sidecar naming
        /// the run `name` (no sidecar when `None`).
        fn workflow(self, run: &str, name: Option<&str>, id: &str, lines: &[String]) -> Self {
            let mut src = self.0.with_file(
                &format!("-w/s/subagents/workflows/wf_{run}/agent-{id}.jsonl"),
                lines.join("\n"),
            );
            if let Some(name) = name {
                src = src.with_file(
                    &format!("-w/s/workflows/wf_{run}.json"),
                    json!({ "runId": format!("wf_{run}"), "workflowName": name }).to_string(),
                );
            }
            Tree(src)
        }

        fn reap(&self) -> SessionReport {
            let location = locate_session(&self.0, "s").expect("locate");
            reap_session(&self.0, &location).expect("reap")
        }
    }

    fn meta(tool_use_id: &str, parent: Option<&str>) -> Option<serde_json::Value> {
        Some(json!({
            "agentType": "rdm:rdm-effort-medium",
            "toolUseId": tool_use_id,
            "parentAgentId": parent,
        }))
    }

    fn stages_by_id(report: &SessionReport) -> BTreeMap<String, Stage> {
        attribute(report)
            .into_iter()
            .map(|(s, r)| (r.request_id.clone(), s))
            .collect()
    }

    /// The dispatch-phase sequence of the main thread, one request each.
    fn dispatch_main() -> Vec<String> {
        vec![
            line("m01", 1, &[]),
            line(
                "m02",
                1,
                &[("s1", "Skill", json!({ "skill": "rdm:dispatch-phase" }))],
            ),
            line(
                "m03",
                1,
                &[rdm_agent(
                    "a-plan",
                    "Write the plan; run rdm plan create ...",
                )],
            ),
            line("m04", 1, &[]),
            line(
                "m05",
                1,
                &[(
                    "w1",
                    "Workflow",
                    json!({ "name": "rdm:rdm-wf-plan-review" }),
                )],
            ),
            line(
                "m06",
                1,
                &[rdm_agent("a-impl", "Implement the approved plan.")],
            ),
            line(
                "m07",
                1,
                &[(
                    "b1",
                    "Bash",
                    json!({ "command": "rdm verify run --item phase/x/phase-1 --project p" }),
                )],
            ),
            line(
                "m08",
                1,
                &[("b2", "Bash", json!({ "command": "git log --oneline" }))],
            ),
            line(
                "m09",
                1,
                &[(
                    "w2",
                    "Workflow",
                    json!({ "name": "rdm-wf-review-refute-fix" }),
                )],
            ),
            line("m10", 1, &[]),
            line("m11", 1, &[rdm_agent("a-fix", "Fix the review comments.")]),
            line(
                "m12",
                1,
                &[(
                    "w3",
                    "Workflow",
                    json!({ "name": "rdm:rdm-wf-review-refute-fix" }),
                )],
            ),
            line("m13", 1, &[("s2", "Skill", json!({ "skill": "rdm:land" }))]),
            line(
                "m14",
                1,
                &[(
                    "a-explore",
                    "Agent",
                    json!({ "subagent_type": "Explore", "prompt": "look" }),
                )],
            ),
        ]
    }

    #[test]
    fn main_thread_requests_take_the_most_recently_opened_stage() {
        use Stage::*;
        let got = stages_by_id(&Tree::new(&dispatch_main()).reap());
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
    fn subagents_take_their_anchoring_calls_stage() {
        use Stage::*;
        let report = Tree::new(&dispatch_main())
            .agent("plan", meta("a-plan", None), &[line("p1", 1, &[])])
            .agent("impl", meta("a-impl", None), &[line("i1", 1, &[])])
            .agent(
                "fix",
                meta("a-fix", None),
                &[line(
                    "f1",
                    1,
                    &[("a-nested", "Agent", json!({ "subagent_type": "Explore" }))],
                )],
            )
            // Nested: anchored through its parent to the main-thread call.
            .agent(
                "nested",
                meta("a-nested", Some("fix")),
                &[line("n1", 1, &[])],
            )
            .agent("explore", meta("a-explore", None), &[line("e1", 1, &[])])
            .agent("unknown", meta("a-unknown", None), &[line("u1", 1, &[])])
            .agent("nometa", None, &[line("u2", 1, &[])])
            .reap();
        let got = stages_by_id(&report);
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
        let report = Tree::new(&[line("m", 1, &[])])
            .workflow("1", Some("rdm-wf-plan-review"), "a", &[line("w1", 1, &[])])
            .workflow(
                "2",
                Some("rdm-wf-review-refute-fix"),
                "a",
                &[line("w2", 1, &[])],
            )
            .workflow("3", Some("rdm-wf-estimate"), "a", &[line("w3", 1, &[])])
            .workflow("4", None, "a", &[line("w4", 1, &[])])
            .reap();
        let got = stages_by_id(&report);
        assert_eq!(
            [&got["w1"], &got["w2"], &got["w3"], &got["w4"]],
            [&PlanReview, &CodeReview, &Other, &Unattributed]
        );
    }

    #[test]
    fn a_request_streamed_over_several_lines_counts_once_at_its_last_usage() {
        let report = Tree::new(&[
            line("x", 10, &[]),
            line("x", 30, &[rdm_agent("a1", "Implement.")]),
            line("y", 5, &[]),
        ])
        .reap();
        let b = Breakdown::attributed(&[report], None);
        assert_eq!(b.get(Stage::Implementation).tokens.input, 35.0);
        assert_eq!(b.get(Stage::Other).tokens.input, 0.0);
        assert_eq!(b.get(Stage::Implementation).cost_usd, None);
    }

    #[test]
    fn dollars_price_both_cache_write_durations_and_skip_unknown_models() {
        let p = pricing();
        let u = TokenUsage {
            input: 1_000_000,
            output: 100_000,
            cache_read: 10_000_000,
            cache_write_5m: 200_000,
            cache_write_1h: 500_000,
        };
        // 4 + 2 + 2 + 1 + 0.5 * 4 * 2 = 13.
        assert_eq!(p.cost("claude-test-1", &u), Some(13.0));
        assert_eq!(p.cost("test-1", &u), Some(13.0));
        assert_eq!(p.cost("claude-other", &u), None);

        let report = Tree::new(&[
            line("a", 1_000_000, &[]),
            model_line("z", "claude-other", 1_000_000, &[]),
        ])
        .reap();
        let b = Breakdown::attributed(&[report], Some(&p));
        let other = b.get(Stage::Other);
        assert_eq!(other.tokens.input, 2_000_000.0);
        assert_eq!(other.cost_usd, Some(4.0));
        assert_eq!(b.get(Stage::Planning).cost_usd, Some(0.0));
    }

    #[test]
    fn the_remainder_against_the_recorded_totals_is_unattributed() {
        let p = pricing();
        // 1M input in other ($4), 1M input in planning ($4).
        let report = Tree::new(&[
            line("a", 1_000_000, &[]),
            line("b", 1_000_000, &[rdm_agent("a1", "rdm plan create")]),
        ])
        .reap();
        let attributed = Breakdown::attributed(&[report], Some(&p));
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

    /// An assistant line with `hour` 1-hour and 100 5-minute cache writes.
    fn cached_line(id: &str, input: u64, hour: u64, tools: serde_json::Value) -> String {
        json!({
            "type": "assistant",
            "requestId": id,
            "timestamp": "2026-01-01T00:00:00Z",
            "message": {
                "id": format!("msg_{id}"), "model": MODEL, "content": tools,
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
            cached_line(
                "m1",
                10,
                0,
                json!([{ "type": "tool_use", "id": "t-plan", "name": "Agent",
                         "input": { "subagent_type": "rdm-effort-low", "prompt": "rdm plan create" } }]),
            ),
            cached_line(
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
            &cached_line("p1", 100, 1000, json!([])),
        );
        let meta = session.join("subagents/agent-a1.meta.json");
        write(
            &meta,
            r#"{"agentType":"rdm:rdm-effort-low","toolUseId":"t-plan"}"#,
        );
        write(
            &session.join("subagents/workflows/wf_1/agent-b1.jsonl"),
            &cached_line("r1", 1000, 0, json!([])),
        );
        write(
            &session.join("workflows/wf_1.json"),
            r#"{"runId":"wf_1","workflowName":"rdm-wf-plan-review"}"#,
        );
        write(
            &session.join("subagents/workflows/wf_2/agent-c1.jsonl"),
            &cached_line("q1", 7, 0, json!([])),
        );
        // Not a session's trace: the plan-repo snapshots beside it.
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

        // A meta that does not parse leaves its subagent unanchored, so its
        // usage is unattributed instead of failing the checkpoint.
        write(&meta, "{not json");
        let b = read_checkpoint(tmp.path(), None)
            .expect("read")
            .expect("trace");
        assert_eq!(b.get(Stage::Planning).tokens.input, 10.0);
        assert_eq!(b.get(Stage::Unattributed).tokens.input, 107.0);
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

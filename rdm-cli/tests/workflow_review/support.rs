//! Shared scaffolding for the review workflow tests: where the real sources
//! live, how a scenario reports failure, mutant libraries, and a recording
//! fake agent whose replies are scripted in Rust.
//!
//! A scenario is a `fn(&Lib) -> Outcome`. The normal test runs it against the
//! real sources and requires `Ok`; a mutant test runs it against an isolated
//! copy with one planted logic change and requires a *behavioural* failure
//! (a failed check or a JavaScript exception) — an infrastructure failure
//! never counts as catching a mutant.

#![allow(dead_code)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use rdm_devtools::workflow::{Host, Invocation, JsError, MutantTree, Outbox, WorkflowError};
use serde_json::{Value, json};

/// The canonical review core.
pub const REVIEW_LIB: &str = ".claude/workflows/lib/review.mjs";
/// The plan-review driver module (imports the review core).
pub const PLAN_LIB: &str = ".claude/workflows/lib/plan-review.mjs";
/// The standalone review engine.
pub const REVIEW_ENGINE: &str = ".claude/workflows/rdm-wf-review-refute-fix.js";
/// The engine's shipped (embedded template) copy.
pub const REVIEW_ENGINE_TEMPLATE: &str =
    "rdm-core/src/templates/workflows/rdm-wf-review-refute-fix.js";

/// The workspace root.
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("rdm-cli has a parent directory")
        .to_path_buf()
}

/// Why a scenario failed.
pub enum Failure {
    /// A scenario check did not hold.
    Check(String),
    /// Workflow JavaScript threw where the scenario expected a value.
    Js(JsError),
    /// The test infrastructure failed (runtime, protocol, setup).
    Infra(String),
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Check(m) => write!(f, "check failed: {m}"),
            Self::Js(e) => write!(f, "workflow JavaScript threw {e}\n{}", e.stack),
            Self::Infra(m) => write!(f, "infrastructure failure: {m}"),
        }
    }
}

impl fmt::Debug for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl From<WorkflowError> for Failure {
    fn from(e: WorkflowError) -> Self {
        match e {
            WorkflowError::Js(js) => Self::Js(js),
            other => Self::Infra(other.to_string()),
        }
    }
}

/// A scenario's result.
pub type Outcome = Result<(), Failure>;

/// Fails the scenario unless `cond` holds.
macro_rules! check {
    ($cond:expr, $($msg:tt)+) => {
        if !$cond {
            return Err($crate::support::Failure::Check(format!($($msg)+)));
        }
    };
}

/// Fails the scenario unless `left == right`.
macro_rules! check_eq {
    ($left:expr, $right:expr, $($msg:tt)+) => {{
        let (l, r) = (&$left, &$right);
        if l != r {
            return Err($crate::support::Failure::Check(format!(
                "{}\n  left: {:?}\n right: {:?}",
                format!($($msg)+),
                l,
                r
            )));
        }
    }};
}

/// The workflow sources a scenario runs against: the real tree, or an
/// isolated mutant copy.
pub struct Lib {
    root: PathBuf,
    _tree: Option<MutantTree>,
}

impl Lib {
    /// The real sources in this checkout (read only).
    pub fn real() -> Self {
        Self {
            root: repo_root(),
            _tree: None,
        }
    }

    /// A copy of the review core and the plan-review module with each
    /// `(file, from, to)` edit applied exactly once.
    ///
    /// # Panics
    ///
    /// If a file cannot be copied or an anchor does not occur exactly once —
    /// a mutant that was not planted must fail its test, not pass it.
    pub fn mutant(name: &str, edits: &[(&str, &str, &str)]) -> Self {
        Self::mutant_of(name, &[REVIEW_LIB, PLAN_LIB], edits)
    }

    /// Like [`Lib::mutant`] over an explicit file list.
    pub fn mutant_of(name: &str, files: &[&str], edits: &[(&str, &str, &str)]) -> Self {
        let tree = MutantTree::copy(&repo_root(), files)
            .unwrap_or_else(|e| panic!("mutant `{name}`: setup failed: {e}"));
        for (file, from, to) in edits {
            tree.replace_once(file, name, from, to)
                .unwrap_or_else(|e| panic!("{e}"));
        }
        Self {
            root: tree.root().to_owned(),
            _tree: Some(tree),
        }
    }

    /// A source path under this library's root.
    pub fn path(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    /// The text of a source file.
    pub fn read(&self, rel: &str) -> Result<String, Failure> {
        std::fs::read_to_string(self.path(rel))
            .map_err(|e| Failure::Infra(format!("reading {rel}: {e}")))
    }
}

/// Runs `scenario` against the real sources and panics with its failure.
pub fn run_real(scenario: fn(&Lib) -> Outcome) {
    if let Err(f) = scenario(&Lib::real()) {
        panic!("{f}");
    }
}

/// Runs `scenario` against a mutant and requires a behavioural failure.
pub fn run_mutant(lib: Lib, scenario: fn(&Lib) -> Outcome) {
    match scenario(&lib) {
        Ok(()) => {
            panic!("the planted mutant survived: the scenario passed against mutated sources")
        }
        Err(Failure::Infra(e)) => {
            panic!("the mutant run failed for an infrastructure reason, not the planted logic: {e}")
        }
        Err(caught) => eprintln!("mutant caught: {caught}"),
    }
}

/// A started host with the review core imported.
pub struct Js {
    /// The underlying host (for registering callbacks and raw calls).
    pub host: Host,
    review: Value,
    plan: Option<Value>,
    plan_path: PathBuf,
}

impl Js {
    /// Starts a host and imports `lib`'s review core.
    pub fn open(lib: &Lib) -> Result<Self, Failure> {
        let mut host = Host::start_default()?;
        let review = host.import(&lib.path(REVIEW_LIB))?;
        Ok(Self {
            host,
            review,
            plan: None,
            plan_path: lib.path(PLAN_LIB),
        })
    }

    /// An export of the review core.
    pub fn get(&mut self, name: &str) -> Result<Value, Failure> {
        Ok(self.host.export(&self.review, name)?)
    }

    /// Calls an export of the review core; a JavaScript throw is a failure.
    pub fn call(&mut self, name: &str, args: Vec<Value>) -> Result<Value, Failure> {
        Ok(self.host.call_export(&self.review, name, args)?)
    }

    /// Calls an export of the review core, returning a JavaScript throw as a
    /// value for the scenario to inspect.
    pub fn try_call(
        &mut self,
        name: &str,
        args: Vec<Value>,
    ) -> Result<Result<Value, JsError>, Failure> {
        split(self.host.call_export(&self.review, name, args))
    }

    fn plan_module(&mut self) -> Result<Value, Failure> {
        if self.plan.is_none() {
            self.plan = Some(self.host.import(&self.plan_path)?);
        }
        Ok(self.plan.clone().unwrap_or(Value::Null))
    }

    /// Calls an export of the plan-review module.
    pub fn plan_call(&mut self, name: &str, args: Vec<Value>) -> Result<Value, Failure> {
        let m = self.plan_module()?;
        Ok(self.host.call_export(&m, name, args)?)
    }

    /// Like [`Js::plan_call`], returning a JavaScript throw as a value.
    pub fn plan_try_call(
        &mut self,
        name: &str,
        args: Vec<Value>,
    ) -> Result<Result<Value, JsError>, Failure> {
        let m = self.plan_module()?;
        split(self.host.call_export(&m, name, args))
    }

    /// Runs `buildReviewPipeline(mode, deps)(ctx)` with `agent` installed as
    /// the fake agent and a recording `log`.
    pub fn review(
        &mut self,
        mode: &str,
        agent: &Agent,
        ctx: Value,
    ) -> Result<Result<Value, JsError>, Failure> {
        let deps = agent.install(&mut self.host);
        let run = match self.try_call("buildReviewPipeline", vec![json!(mode), deps])? {
            Ok(run) => run,
            Err(e) => return Ok(Err(e)),
        };
        split(self.host.call(&run, vec![ctx]))
    }

    /// [`Js::review`], treating a rejection as a failure.
    pub fn review_ok(&mut self, mode: &str, agent: &Agent, ctx: Value) -> Result<Value, Failure> {
        self.review(mode, agent, ctx)?.map_err(Failure::Js)
    }
}

/// Separates a JavaScript throw (returned as a value) from an infrastructure
/// failure.
pub fn split(r: Result<Value, WorkflowError>) -> Result<Result<Value, JsError>, Failure> {
    match r {
        Ok(v) => Ok(Ok(v)),
        Err(WorkflowError::Js(e)) => Ok(Err(e)),
        Err(e) => Err(e.into()),
    }
}

/// One recorded `agent()` call.
#[derive(Debug, Clone)]
pub struct AgentCall {
    /// Arrival order across every callback of the host.
    pub seq: usize,
    /// `opts.label`.
    pub label: String,
    /// The prompt text.
    pub prompt: String,
    /// The whole `opts` object (label, phase, schema, model).
    pub opts: Value,
}

impl AgentCall {
    /// `opts.model` (`None` when absent or undefined).
    pub fn model(&self) -> Option<&Value> {
        rdm_devtools::workflow::member(&self.opts, "model")
    }
}

/// A scripted reply.
pub enum Reply {
    /// Resolve with this value.
    Value(Value),
    /// Resolve with `null` (what `agent()` does for an unknown model).
    Null,
    /// Reject with this message.
    Throw(String),
}

type ScriptFn = Box<dyn FnMut(&AgentCall) -> Reply>;

struct Hold {
    prefix: String,
    count: usize,
    order: Vec<usize>,
    held: Vec<(u64, Result<Value, String>)>,
}

struct AgentState {
    calls: Vec<AgentCall>,
    logs: Vec<String>,
    script: ScriptFn,
    hold: Option<Hold>,
}

/// A recording fake agent whose replies are scripted in Rust.
#[derive(Clone)]
pub struct Agent {
    state: Rc<RefCell<AgentState>>,
}

impl Agent {
    /// An agent answering every call with `script`.
    pub fn scripted(script: impl FnMut(&AgentCall) -> Reply + 'static) -> Self {
        Self {
            state: Rc::new(RefCell::new(AgentState {
                calls: Vec::new(),
                logs: Vec::new(),
                script: Box::new(script),
                hold: None,
            })),
        }
    }

    /// The common fixture: finder `find:<mode>:<dim>` returns
    /// `{ findings: findings[dim] or [] }`; refuter `refute:<mode>:<id>`
    /// returns `verdicts[id]` or `{ refuted: false, confidence: 90 }`.
    pub fn planted(findings: Value, verdicts: Value) -> Self {
        Self::scripted(move |call| planted_reply(&findings, &verdicts, call))
    }

    /// What this agent's script would answer to `call`, without recording
    /// it — for composing an override on top of a planted agent.
    pub fn reply_for(&self, call: &AgentCall) -> Reply {
        (self.state.borrow_mut().script)(call)
    }

    /// Holds replies to calls whose label starts with `prefix` until `count`
    /// are held, then releases them in `order` (indices into arrival order),
    /// so a fixed permutation of completion order is planted.
    pub fn hold_then_release(self, prefix: &str, count: usize, order: Vec<usize>) -> Self {
        self.state.borrow_mut().hold = Some(Hold {
            prefix: prefix.to_owned(),
            count,
            order,
            held: Vec::new(),
        });
        self
    }

    /// Registers this agent (and a recording `log`) with `host`, returning
    /// the `deps` object for `buildReviewPipeline`.
    pub fn install(&self, host: &mut Host) -> Value {
        let state = Rc::clone(&self.state);
        let agent = host.register(move |inv: &Invocation, out: &mut Outbox| {
            let call = AgentCall {
                seq: inv.seq,
                label: inv
                    .args
                    .get(1)
                    .and_then(|o| o.get("label"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                prompt: inv
                    .args
                    .first()
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                opts: inv.args.get(1).cloned().unwrap_or(Value::Null),
            };
            let mut st = state.borrow_mut();
            let reply = (st.script)(&call);
            let result = match reply {
                Reply::Value(v) => Ok(v),
                Reply::Null => Ok(Value::Null),
                Reply::Throw(m) => Err(m),
            };
            let label = call.label.clone();
            st.calls.push(call);
            let holding = st
                .hold
                .as_ref()
                .is_some_and(|h| label.starts_with(&h.prefix));
            if holding {
                let hold = st.hold.as_mut().expect("hold present");
                hold.held.push((inv.call_id, result));
                if hold.held.len() == hold.count {
                    let held = std::mem::take(&mut hold.held);
                    for &i in &hold.order {
                        if let Some((id, r)) = held.get(i) {
                            out.reply(*id, r.clone());
                        }
                    }
                }
            } else {
                out.reply(inv.call_id, result);
            }
        });
        let state = Rc::clone(&self.state);
        let log = host.register_fn(move |args| {
            let text = args
                .first()
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            state.borrow_mut().logs.push(text);
            Ok(Value::Null)
        });
        json!({
            "agent": agent,
            "pipeline": Host::primitive("pipeline"),
            "parallel": Host::primitive("parallel"),
            "log": log,
        })
    }

    /// Every recorded call, in arrival order.
    pub fn calls(&self) -> Vec<AgentCall> {
        self.state.borrow().calls.clone()
    }

    /// Recorded calls whose label starts with `prefix`.
    pub fn calls_with(&self, prefix: &str) -> Vec<AgentCall> {
        self.calls()
            .into_iter()
            .filter(|c| c.label.starts_with(prefix))
            .collect()
    }

    /// Finding ids of every refuter dispatched, in arrival order.
    pub fn refuted_ids(&self) -> Vec<String> {
        self.calls()
            .into_iter()
            .filter_map(|c| match parse_label(&c.label) {
                Label::Refute { id, .. } => Some(id),
                _ => None,
            })
            .collect()
    }

    /// Every `log` message.
    pub fn logs(&self) -> Vec<String> {
        self.state.borrow().logs.clone()
    }
}

/// The [`Agent::planted`] reply rule.
pub fn planted_reply(findings: &Value, verdicts: &Value, call: &AgentCall) -> Reply {
    match parse_label(&call.label) {
        Label::Find { dim, .. } => Reply::Value(json!({
            "findings": findings.get(&dim).cloned().unwrap_or_else(|| json!([]))
        })),
        Label::Refute { id, .. } => Reply::Value(
            verdicts
                .get(&id)
                .cloned()
                .unwrap_or_else(|| json!({ "refuted": false, "confidence": 90 })),
        ),
        Label::Other => Reply::Throw(format!("unexpected agent label: {}", call.label)),
    }
}

/// A parsed agent label.
pub enum Label {
    /// `find:<mode>:<dim>[:retry]`.
    Find {
        /// The dimension key.
        dim: String,
        /// Whether this is the `:retry` attempt.
        retry: bool,
    },
    /// `refute:<mode>:<id…>`.
    Refute {
        /// The finding id (may contain `:`).
        id: String,
    },
    /// Anything else.
    Other,
}

/// Splits an agent label into its kind and subject.
pub fn parse_label(label: &str) -> Label {
    let parts: Vec<&str> = label.split(':').collect();
    match parts.as_slice() {
        ["find", _mode, dim] => Label::Find {
            dim: (*dim).to_owned(),
            retry: false,
        },
        ["find", _mode, dim, "retry"] => Label::Find {
            dim: (*dim).to_owned(),
            retry: true,
        },
        ["refute", _mode, rest @ ..] if !rest.is_empty() => Label::Refute { id: rest.join(":") },
        _ => Label::Other,
    }
}

/// Survivor ids of a review result, in order.
pub fn ids(result: &Value, key: &str) -> Vec<String> {
    result[key]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|f| f["id"].as_str().unwrap_or_default().to_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// The survivor with `id`.
pub fn find<'a>(result: &'a Value, key: &str, id: &str) -> Option<&'a Value> {
    result[key]
        .as_array()
        .and_then(|a| a.iter().find(|f| f["id"] == json!(id)))
}

/// A map from dimension/id to value, for [`Agent::planted`].
pub fn plant(pairs: &[(&str, Value)]) -> Value {
    let map: HashMap<&str, Value> = pairs.iter().cloned().collect();
    json!(map)
}

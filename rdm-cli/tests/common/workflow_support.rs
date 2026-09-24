//! Generic scaffolding shared by the Rust-driven workflow component-test
//! binaries (`workflow_review`, `workflow_passes`, `distribution`; the
//! `cli_loops` and `golden_json` binaries use only its failure model): where
//! the real sources live — this checkout, or a tree `rdm agent-config`
//! emitted ([`Lib::at`]) — how a scenario reports failure, mutant libraries,
//! loading a workflow driver, and a recording fake agent whose replies are
//! scripted in Rust.
//!
//! Included with `#[macro_use] #[path = "../common/workflow_support.rs"] mod
//! workflow_support;` from each binary's `main.rs`. It lives under
//! `tests/common/` rather than beside `git_test_support.rs` because it is not
//! self-contained (its macros name `crate::workflow_support`), and cargo
//! builds every top-level `tests/*.rs` as a test target of its own. Nothing
//! here is specific to one workflow.
//!
//! A scenario is a function or closure `(&Lib) -> Outcome`. The normal test runs it against the
//! real sources and requires `Ok`; a mutant test runs it against an isolated
//! copy with one planted logic change and requires a *behavioural* failure
//! (a failed check or a JavaScript exception thrown while the scenario
//! runs) — an infrastructure failure, or JavaScript that fails to import or
//! compile, never counts as catching a mutant.

#![allow(dead_code)]

use std::cell::RefCell;
use std::fmt;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use rdm_devtools::workflow::{Host, Invocation, JsError, MutantTree, Outbox, WorkflowError};
use serde_json::{Value, json};

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
    /// Workflow JavaScript failed to load: a module import, a driver
    /// compile or a helper extraction threw (for example a `SyntaxError`),
    /// so no scenario code ran.
    Load(JsError),
    /// The test infrastructure failed (runtime, protocol, setup).
    Infra(String),
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Check(m) => write!(f, "check failed: {m}"),
            Self::Js(e) => write!(f, "workflow JavaScript threw {e}\n{}", e.stack),
            Self::Load(e) => write!(f, "workflow JavaScript failed to load: {e}\n{}", e.stack),
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

/// Maps a load-time result (import, compile, helper extraction): a
/// JavaScript throw there is [`Failure::Load`], not a scenario failure.
pub fn load<T>(r: Result<T, WorkflowError>) -> Result<T, Failure> {
    r.map_err(|e| match e {
        WorkflowError::Js(js) => Failure::Load(js),
        other => Failure::Infra(other.to_string()),
    })
}

/// Maps any displayable setup error to [`Failure::Infra`].
pub fn infra(e: impl fmt::Display) -> Failure {
    Failure::Infra(e.to_string())
}

/// Fails the scenario unless `cond` holds.
#[allow(unused_macros)]
macro_rules! check {
    ($cond:expr, $($msg:tt)+) => {
        if !$cond {
            return Err($crate::workflow_support::Failure::Check(format!($($msg)+)));
        }
    };
}

/// Fails the scenario unless `left == right`.
#[allow(unused_macros)]
macro_rules! check_eq {
    ($left:expr, $right:expr, $($msg:tt)+) => {{
        let (l, r) = (&$left, &$right);
        if l != r {
            return Err($crate::workflow_support::Failure::Check(format!(
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

    /// A copy of `files` with each `(file, from, to)` edit applied exactly
    /// once.
    ///
    /// # Panics
    ///
    /// If a file cannot be copied or an anchor does not occur exactly once —
    /// a mutant that was not planted must fail its test, not pass it.
    pub fn mutant_of(name: &str, files: &[&str], edits: &[(&str, &str, &str)]) -> Self {
        Self::mutant_at(&repo_root(), name, files, edits)
    }

    /// Sources rooted at `root` instead of this checkout — for example a
    /// tree `rdm agent-config` emitted into a test's temp directory (read
    /// only).
    pub fn at(root: &Path) -> Self {
        Self {
            root: root.to_owned(),
            _tree: None,
        }
    }

    /// Like [`Lib::mutant_of`], copying `files` from `root` instead of this
    /// checkout, so a mutant can be planted in emitted bytes.
    ///
    /// # Panics
    ///
    /// As for [`Lib::mutant_of`].
    pub fn mutant_at(
        root: &Path,
        name: &str,
        files: &[&str],
        edits: &[(&str, &str, &str)],
    ) -> Self {
        let tree = MutantTree::copy(root, files)
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
pub fn run_real(scenario: impl FnOnce(&Lib) -> Outcome) {
    if let Err(f) = scenario(&Lib::real()) {
        panic!("{f}");
    }
}

/// How a mutant run ended.
#[derive(Debug)]
pub enum MutantVerdict {
    /// The scenario passed against the mutated sources.
    Survived,
    /// The run failed before or outside the scenario's logic.
    NotRun(Failure),
    /// A check failed or the scenario's JavaScript threw: the mutant is
    /// caught.
    Caught(Failure),
}

/// Runs `scenario` against `lib` and classifies the result: only a failed
/// check or a throw during the scenario counts as caught; an infrastructure
/// or load failure (the mutated source did not even import or compile) does
/// not.
pub fn mutant_verdict(lib: &Lib, scenario: impl FnOnce(&Lib) -> Outcome) -> MutantVerdict {
    match scenario(lib) {
        Ok(()) => MutantVerdict::Survived,
        Err(f @ (Failure::Infra(_) | Failure::Load(_))) => MutantVerdict::NotRun(f),
        Err(f @ (Failure::Check(_) | Failure::Js(_))) => MutantVerdict::Caught(f),
    }
}

/// Runs `scenario` against a mutant and requires a behavioural failure.
pub fn run_mutant(lib: Lib, scenario: impl FnOnce(&Lib) -> Outcome) {
    match mutant_verdict(&lib, scenario) {
        MutantVerdict::Survived => {
            panic!("the planted mutant survived: the scenario passed against mutated sources")
        }
        MutantVerdict::NotRun(f) => {
            panic!("the mutant run failed before exercising the planted logic: {f}")
        }
        MutantVerdict::Caught(caught) => eprintln!("mutant caught: {caught}"),
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

/// A started host with one ES module imported.
pub struct Module {
    /// The underlying host (for registering callbacks and raw calls).
    pub host: Host,
    module: Value,
}

impl Module {
    /// Starts a host and imports `lib`'s module at `rel`.
    pub fn open(lib: &Lib, rel: &str) -> Result<Self, Failure> {
        let mut host = Host::start_default()?;
        let module = load(host.import(&lib.path(rel)))?;
        Ok(Self { host, module })
    }

    /// An export of the module.
    pub fn get(&mut self, name: &str) -> Result<Value, Failure> {
        Ok(self.host.export(&self.module, name)?)
    }

    /// Calls an export; a JavaScript throw is a failure.
    pub fn call(&mut self, name: &str, args: Vec<Value>) -> Result<Value, Failure> {
        Ok(self.host.call_export(&self.module, name, args)?)
    }

    /// Calls an export, returning a JavaScript throw as a value for the
    /// scenario to inspect.
    pub fn try_call(
        &mut self,
        name: &str,
        args: Vec<Value>,
    ) -> Result<Result<Value, JsError>, Failure> {
        split(self.host.call_export(&self.module, name, args))
    }

    /// Calls an arbitrary function value (for example one an export
    /// returned), returning a JavaScript throw as a value.
    pub fn try_invoke(
        &mut self,
        f: &Value,
        args: Vec<Value>,
    ) -> Result<Result<Value, JsError>, Failure> {
        split(self.host.call(f, args))
    }
}

/// Loads the workflow script at `rel` as a driver and calls it with `args`
/// and `agent`'s installed primitives (`agent`, `pipeline`, `parallel`,
/// `log`). A driver that fails to compile is [`Failure::Load`]; a throw
/// while it runs is returned as a value.
pub fn run_driver(
    lib: &Lib,
    rel: &str,
    args: Value,
    agent: &Agent,
) -> Result<Result<Value, JsError>, Failure> {
    let script = lib.read(rel)?;
    let mut host = Host::start_default()?;
    let driver = load(host.load_driver(&script))?;
    let deps = agent.install(&mut host);
    split(host.call(
        &driver,
        vec![
            args,
            deps["agent"].clone(),
            deps["pipeline"].clone(),
            deps["parallel"].clone(),
            deps["log"].clone(),
        ],
    ))
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

    /// What this agent's script would answer to `call`, without recording
    /// it — for composing an override on top of another agent.
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
    /// the `deps` object a workflow core takes: `agent`, `pipeline`,
    /// `parallel` and `log`.
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

    /// Every recorded label, in arrival order.
    pub fn labels(&self) -> Vec<String> {
        self.calls().into_iter().map(|c| c.label).collect()
    }

    /// Every `log` message.
    pub fn logs(&self) -> Vec<String> {
        self.state.borrow().logs.clone()
    }
}

/// The string array at `v`, or empty.
pub fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|s| s.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

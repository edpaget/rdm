//! Repository-only binding (tests and measurement tools) that executes the real
//! Claude Workflow JavaScript sources under Node, so Rust tests own the
//! scenarios, fake-agent responses and assertions while the code under test is
//! the shipped JavaScript itself, and so the measurement tools in
//! [`measure`](crate::measure) can call canonical workflow decisions instead of
//! copying them.
//!
//! # Mechanism
//!
//! A [`Host`] is one Node child process driven through a
//! [`Session`](crate::process::Session): it runs in its own process group,
//! under one overall deadline, with signal interception, and it is killed and
//! reaped on every exit path (including [`Drop`] and every protocol failure).
//! Node is an explicit prerequisite, resolved by [`resolve_node`]; a missing
//! runtime is an error naming the fix, never a skip.
//!
//! The only JavaScript on the test side is `workflow_host.mjs` (embedded in
//! this crate and written into each host's private temp directory). It is
//! generic execution glue: it imports modules, compiles function bodies,
//! encodes and decodes values, keeps a handle table, forwards callbacks,
//! serializes thrown errors (name, message, stack) and routes every `console`
//! method to stderr so workflow logging cannot corrupt the protocol. It holds
//! no tests, scenarios, expected values, assertions, fixture branches or
//! review logic.
//!
//! # Protocol
//!
//! One JSON object per line. Rust sends requests, each with a fresh `id`:
//!
//! - `{"op":"import","id":n,"path":p}` — `import()` the module at `p`; the
//!   reply value is a `{"$ref":h}` handle to its namespace.
//! - `{"op":"compile","id":n,"params":[…],"source":s}` — `new
//!   AsyncFunction(...params, s)`; the reply is a `{"$fn":h}` handle.
//! - `{"op":"get","id":n,"handle":h,"member":m}` — read one member of a
//!   handle; with `"keep":true` the member is not encoded but kept, and the
//!   reply is a `{"$ref":h}` handle to it (so a platform object such as an
//!   `AbortSignal` can travel back into arguments by reference).
//! - `{"op":"new","id":n,"global":name,"args":[…]}` — `new
//!   globalThis[name](...args)`; the reply is a `{"$ref":h}` handle.
//! - `{"op":"invoke","id":n,"handle":h,"member":m,"args":[…]}` — call method
//!   `m` of a handle with the handle as its receiver, await and encode the
//!   result.
//! - `{"op":"call","id":n,"target":v,"args":[…]}` — call a function value and
//!   await its result.
//! - `{"op":"ping","id":n}` — answered with `null` (see [`Host::flush`]).
//! - `{"op":"reply","callId":c,"value":v}`, `{"op":"reply","callId":c,"error":msg}`
//!   or `{"op":"reply","callId":c,"reject":v}` — answer a callback; an
//!   `error` rejects the JavaScript promise with an `Error(msg)`, a `reject`
//!   rejects it with exactly the decoded value `v` (for example `null`).
//! - `{"op":"shutdown"}` — exit.
//!
//! Node answers with `{"op":"ok","id":n,"value":v}`, `{"op":"err","id":n,"error":{name,message,stack}}`,
//! or, while a request is outstanding, `{"op":"callback","callId":c,"fn":f,"args":[…]}`.
//!
//! # Detached calls
//!
//! [`Host::call`] waits for its result. [`Host::start_call`] only sends the
//! call and returns a [`PendingCall`], so a test can act while it is in
//! flight — hold callback replies, abort a signal, signal the runtime — and
//! collect the result later with [`Host::await_call`]. While any request is
//! outstanding, a result for a started call that is not being awaited is
//! buffered ([`Host::is_settled`]); a result for an id that was never started
//! is still a protocol error. [`Host::service`] handles one incoming message,
//! [`Host::flush`] waits until JavaScript has drained everything the lines
//! already sent set in motion (two consecutive pings, the second sent only
//! after the first is answered, so it starts a fresh macrotask), and
//! [`Host::reply`] answers a held invocation outside a handler.
//!
//! Values are JSON plus these single-key tagged forms:
//!
//! - `{"$fn":h}` — a JavaScript function held by the glue; pass it back as a
//!   call target or inside arguments.
//! - `{"$ref":h}` — any other opaque value held by the glue (a module
//!   namespace, a constructed object, a kept member).
//! - `{"$callback":f}` — a Rust callback registered with [`Host::register`];
//!   JavaScript sees an async function that emits `callback` and awaits the
//!   `reply`.
//! - `{"$host":"parallel"}` / `{"$host":"pipeline"}` — a fake host primitive
//!   (below).
//! - `{"$undefined":true}` — `undefined`, including as an object property, so
//!   "key present with value undefined" survives the round trip.
//!
//! Concurrent callbacks (for example parallel finder agents) arrive in the
//! order JavaScript emits them; each [`Invocation`] carries that arrival
//! sequence, so barrier and fan-out properties are observed, not inferred.
//! A handler may answer at once or hold a reply and answer later through the
//! [`Outbox`] (for example to resolve refuters in a fixed permuted order).
//!
//! # Fake host primitives
//!
//! Only what the existing component tests need, and nothing more:
//!
//! - `parallel(thunks)` — order-preserving `Promise.all`; a thunk that throws
//!   or rejects resolves to `null` in the result array (the Workflow runtime's
//!   documented contract).
//! - `pipeline` — deliberately **not** implemented: no ported review case
//!   reaches a `pipeline()` call (the review core requires the dependency to
//!   be present but composes with `parallel` only). Calling it throws
//!   `unsupported host primitive: pipeline`.
//! - `log` is not a primitive: tests register it as a Rust callback, which
//!   records every message.
//!
//! Limits: there is no schema validation, no model and no Claude host. Tests
//! built on this binding are component tests of the JavaScript logic; they do
//! not establish behaviour inside Claude's real Workflow host.
//!
//! # Loading workflow scripts
//!
//! A workflow script (`.claude/workflows/*.js`) is a function body with a
//! column-0 `export const meta`. All text transforms happen here in Rust;
//! the glue only compiles:
//!
//! - [`driver_source`] drops that one `export ` so the whole body can be
//!   compiled with [`DRIVER_PARAMS`] and called with the primitives — this
//!   executes the driver.
//! - [`helper_source`] applies the same transform and injects
//!   `return { <names> }` before the unique `// --- Driver` sentinel, so the
//!   stamped helpers are returned and the driver never runs.
//! - [`invert_helper_source`] is its exact inverse, so a test can show that
//!   the helpers it executed came from the bytes it read.
//!
//! All three fail with [`WorkflowError::Transform`] when the meta line or the
//! sentinel is missing or duplicated.
//!
//! # Mutants
//!
//! [`MutantTree`] copies real workflow sources into a private temp directory,
//! keeping their relative layout so sibling imports still resolve, and
//! [`MutantTree::replace_once`] plants one logic change, refusing a missing or
//! ambiguous anchor so an unapplied mutant can never report a pass.

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use serde_json::{Value, json};
use tempfile::TempDir;

use crate::process::{ProcessSpec, Session, SessionError};

/// The glue script, written into every host's private temp directory.
const HOST_GLUE: &str = include_str!("workflow_host.mjs");

/// File name the glue is written under.
const HOST_GLUE_NAME: &str = "workflow_host.mjs";

/// Default overall deadline for one [`Host`].
pub const DEFAULT_HOST_TIMEOUT: Duration = Duration::from_secs(60);

/// Parameter names a workflow driver body is compiled with, in call order.
pub const DRIVER_PARAMS: [&str; 5] = ["args", "agent", "pipeline", "parallel", "log"];

/// The column-0 sentinel that starts a workflow script's driver.
pub const DRIVER_SENTINEL: &str = "// --- Driver";

/// Environment variable naming an explicit Node executable for tests. Read,
/// never written.
pub const NODE_ENV: &str = "RDM_TEST_NODE";

const FIX_HINT: &str = "install Node (`mise install` provisions the version pinned in .mise.toml), put `node` on PATH, or set RDM_TEST_NODE to its path";

/// A JavaScript exception, as serialized by the glue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsError {
    /// The error's `name` (for example `TypeError`).
    pub name: String,
    /// The error's `message`.
    pub message: String,
    /// The error's `stack`, when it had one.
    pub stack: String,
    /// For a failed `import`, the module path being loaded (Node does not put
    /// it on an ES-module syntax error); empty otherwise.
    pub file: String,
}

impl fmt::Display for JsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.name, self.message)?;
        if !self.file.is_empty() {
            write!(f, " (in {})", self.file)?;
        }
        Ok(())
    }
}

/// Why a workflow-host operation failed.
#[derive(Debug)]
pub enum WorkflowError {
    /// No Node executable could be resolved.
    RuntimeNotFound,
    /// The runtime could not be started.
    Runtime(SessionError),
    /// The host's private temp directory or glue file could not be written.
    Setup(io::Error),
    /// JavaScript threw or rejected; the host is still usable.
    Js(JsError),
    /// The child broke the protocol (for example printed a non-JSON line).
    /// The host was torn down.
    Protocol {
        /// What was wrong.
        detail: String,
        /// The offending line, truncated.
        line: String,
        /// The tail of the child's stderr.
        stderr: String,
    },
    /// The host's session failed while a request was outstanding (timeout,
    /// exit, signal). The host was torn down.
    Session {
        /// The request that was waiting for a reply.
        pending: String,
        /// The underlying session failure.
        source: SessionError,
    },
    /// The host was already torn down by an earlier failure.
    Dead,
    /// A workflow script could not be transformed for loading.
    Transform(String),
    /// A requested module member was `undefined`.
    MissingMember(String),
}

impl WorkflowError {
    /// The JavaScript exception, when this error is one.
    pub fn as_js(&self) -> Option<&JsError> {
        match self {
            Self::Js(e) => Some(e),
            _ => None,
        }
    }
}

impl fmt::Display for WorkflowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RuntimeNotFound => {
                write!(
                    f,
                    "no Node runtime found for the workflow test host; {FIX_HINT}"
                )
            }
            Self::Runtime(e) => write!(
                f,
                "could not start the workflow test host's JavaScript runtime: {e}; {FIX_HINT}"
            ),
            Self::Setup(e) => write!(f, "could not prepare the workflow test host: {e}"),
            Self::Js(e) => {
                write!(f, "JavaScript threw {e}")?;
                if !e.stack.is_empty() {
                    write!(f, "\n{}", e.stack)?;
                }
                Ok(())
            }
            Self::Protocol {
                detail,
                line,
                stderr,
            } => {
                write!(
                    f,
                    "workflow host protocol error: {detail}; offending line: {line:?}; the host was killed"
                )?;
                let tail = stderr.trim_end();
                if !tail.is_empty() {
                    write!(f, "; runtime stderr (tail):\n{tail}")?;
                }
                Ok(())
            }
            Self::Session { pending, source } => {
                write!(
                    f,
                    "workflow host failed while waiting for {pending}: {source}"
                )
            }
            Self::Dead => write!(
                f,
                "the workflow host was already torn down by an earlier failure; start a new one"
            ),
            Self::Transform(msg) => write!(f, "cannot load workflow script: {msg}"),
            Self::MissingMember(name) => write!(
                f,
                "the module has no export named `{name}` (it read as undefined)"
            ),
        }
    }
}

impl std::error::Error for WorkflowError {}

/// Resolves the Node executable once per process: the `RDM_TEST_NODE`
/// environment variable, else `node` on `PATH`, else `mise which node`.
///
/// # Errors
///
/// [`WorkflowError::RuntimeNotFound`] when none of those yields an
/// executable file.
pub fn resolve_node() -> Result<PathBuf, WorkflowError> {
    static NODE: OnceLock<Option<PathBuf>> = OnceLock::new();
    NODE.get_or_init(find_node)
        .clone()
        .ok_or(WorkflowError::RuntimeNotFound)
}

fn find_node() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os(NODE_ENV).filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(explicit));
    }
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            let candidate = dir.join("node");
            if is_executable(&candidate) {
                return Some(candidate);
            }
        }
    }
    let spec = ProcessSpec::new("mise")
        .args(["which", "node"])
        .timeout(Duration::from_secs(30));
    let out = crate::process::run_bounded(&spec, crate::process::Hooks::new()).ok()?;
    let text = String::from_utf8(out.stdout).ok()?;
    let candidate = PathBuf::from(text.trim());
    is_executable(&candidate).then_some(candidate)
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// How to start a [`Host`].
#[derive(Debug, Clone)]
pub struct HostConfig {
    runtime: Option<PathBuf>,
    runtime_args: Vec<OsString>,
    timeout: Duration,
    env_set: Vec<(OsString, OsString)>,
    env_remove: Vec<OsString>,
}

impl Default for HostConfig {
    fn default() -> Self {
        Self {
            runtime: None,
            runtime_args: Vec::new(),
            timeout: DEFAULT_HOST_TIMEOUT,
            env_set: Vec::new(),
            env_remove: Vec::new(),
        }
    }
}

impl HostConfig {
    /// Defaults: the resolved Node, [`DEFAULT_HOST_TIMEOUT`].
    pub fn new() -> Self {
        Self::default()
    }

    /// Uses `program` as the runtime instead of the resolved Node.
    pub fn runtime(mut self, program: impl Into<PathBuf>) -> Self {
        self.runtime = Some(program.into());
        self
    }

    /// Arguments passed to the runtime before the glue script's path.
    pub fn runtime_args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.runtime_args = args.into_iter().map(Into::into).collect();
        self
    }

    /// The host's overall deadline.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Sets `key` in the runtime child's environment only. It is applied
    /// after [`Host::start`]'s built-in removals (`NODE_OPTIONS`, every
    /// `RDM_*`), so an explicit set wins; a later [`HostConfig::env_remove`]
    /// of the same key cancels it. The calling process's environment is
    /// never touched.
    pub fn env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        let key = key.into();
        self.env_set.retain(|(k, _)| *k != key);
        self.env_remove.retain(|k| *k != key);
        self.env_set.push((key, value.into()));
        self
    }

    /// Removes `key` from the runtime child's environment only, cancelling
    /// any earlier [`HostConfig::env`] of the same key.
    pub fn env_remove(mut self, key: impl Into<OsString>) -> Self {
        let key = key.into();
        self.env_set.retain(|(k, _)| *k != key);
        self.env_remove.push(key);
        self
    }
}

/// One callback invocation from JavaScript.
#[derive(Debug, Clone)]
pub struct Invocation {
    /// Identifies this invocation for [`Outbox::reply`].
    pub call_id: u64,
    /// Arrival order across every callback of this host, starting at 0.
    pub seq: usize,
    /// The decoded-as-JSON arguments.
    pub args: Vec<Value>,
}

/// How a callback's JavaScript promise is settled.
#[derive(Debug, Clone)]
enum Settle {
    /// Resolve with the decoded value.
    Value(Value),
    /// Reject with `Error(message)`.
    Error(String),
    /// Reject with exactly the decoded value.
    Reject(Value),
}

impl Settle {
    fn from_result(result: Result<Value, String>) -> Self {
        match result {
            Ok(value) => Self::Value(value),
            Err(message) => Self::Error(message),
        }
    }

    fn line(&self, call_id: u64) -> String {
        match self {
            Self::Value(value) => json!({ "op": "reply", "callId": call_id, "value": value }),
            Self::Error(message) => json!({ "op": "reply", "callId": call_id, "error": message }),
            Self::Reject(value) => json!({ "op": "reply", "callId": call_id, "reject": value }),
        }
        .to_string()
    }
}

/// Replies a callback handler queues; the host sends them after the handler
/// returns, in queue order.
#[derive(Debug, Default)]
pub struct Outbox {
    replies: Vec<(u64, Settle)>,
}

impl Outbox {
    /// Answers invocation `call_id`: `Ok` resolves the JavaScript promise with
    /// the value, `Err` rejects it with `Error(message)`. A reply may answer
    /// an invocation delivered to an earlier handler call.
    pub fn reply(&mut self, call_id: u64, result: Result<Value, String>) {
        self.replies.push((call_id, Settle::from_result(result)));
    }

    /// Rejects invocation `call_id` with exactly the decoded `value` — not an
    /// `Error` — so a falsy rejection such as `null` reaches JavaScript as
    /// itself.
    pub fn reject_with(&mut self, call_id: u64, value: Value) {
        self.replies.push((call_id, Settle::Reject(value)));
    }
}

/// A call sent with [`Host::start_call`] whose result has not been collected.
#[derive(Debug)]
#[must_use = "a started call's result is collected with Host::await_call"]
pub struct PendingCall {
    id: u64,
}

impl PendingCall {
    /// The request id the call was sent with.
    pub fn id(&self) -> u64 {
        self.id
    }
}

type Handler = Box<dyn FnMut(&Invocation, &mut Outbox)>;

/// One Node workflow host. See the [module documentation](self).
pub struct Host {
    session: Option<Session>,
    dir: Option<TempDir>,
    dir_path: PathBuf,
    pid: u32,
    next_id: u64,
    next_seq: usize,
    handlers: HashMap<u64, Handler>,
    next_callback: u64,
    /// Request ids sent and not yet answered.
    started: HashSet<u64>,
    /// Answers received for started ids nobody was waiting on yet.
    settled: HashMap<u64, Result<Value, JsError>>,
}

impl fmt::Debug for Host {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Host")
            .field("pid", &self.pid)
            .field("dir", &self.dir_path)
            .finish_non_exhaustive()
    }
}

impl Host {
    /// Starts a host with `config`.
    ///
    /// The child runs with its private temp directory as working directory,
    /// with `NODE_OPTIONS` and every `RDM_*` variable removed from its
    /// environment, and then `config`'s own [`HostConfig::env`] /
    /// [`HostConfig::env_remove`] applied.
    ///
    /// # Errors
    ///
    /// [`WorkflowError::RuntimeNotFound`], [`WorkflowError::Setup`], or
    /// [`WorkflowError::Runtime`] if the runtime cannot be started.
    pub fn start(config: &HostConfig) -> Result<Self, WorkflowError> {
        let runtime = match &config.runtime {
            Some(path) => path.clone(),
            None => resolve_node()?,
        };
        let dir = tempfile::Builder::new()
            .prefix("rdm-workflow-host-")
            .tempdir()
            .map_err(WorkflowError::Setup)?;
        let glue = dir.path().join(HOST_GLUE_NAME);
        std::fs::write(&glue, HOST_GLUE).map_err(WorkflowError::Setup)?;

        let mut spec = ProcessSpec::new(&runtime)
            .args(&config.runtime_args)
            .arg(&glue)
            .cwd(dir.path())
            .timeout(config.timeout)
            .env_remove("NODE_OPTIONS");
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("RDM_") {
                spec = spec.env_remove(key);
            }
        }
        // ProcessSpec applies removals before sets, so an explicit set wins
        // over the built-in removals above; HostConfig keeps its own set and
        // remove lists disjoint.
        for key in &config.env_remove {
            spec = spec.env_remove(key);
        }
        for (key, value) in &config.env_set {
            spec = spec.env(key, value);
        }
        let session = Session::spawn(&spec).map_err(WorkflowError::Runtime)?;
        let pid = session.pid();
        let dir_path = dir.path().to_owned();
        Ok(Self {
            session: Some(session),
            dir: Some(dir),
            dir_path,
            pid,
            next_id: 1,
            next_seq: 0,
            handlers: HashMap::new(),
            next_callback: 1,
            started: HashSet::new(),
            settled: HashMap::new(),
        })
    }

    /// Starts a host with the default configuration.
    ///
    /// # Errors
    ///
    /// As for [`Host::start`].
    pub fn start_default() -> Result<Self, WorkflowError> {
        Self::start(&HostConfig::new())
    }

    /// The runtime child's pid.
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// The host's private temp directory (removed on teardown).
    pub fn temp_dir(&self) -> &Path {
        &self.dir_path
    }

    /// Registers a callback handler and returns the `{"$callback":f}` value
    /// to pass into JavaScript.
    pub fn register(&mut self, handler: impl FnMut(&Invocation, &mut Outbox) + 'static) -> Value {
        let id = self.next_callback;
        self.next_callback += 1;
        self.handlers.insert(id, Box::new(handler));
        json!({ "$callback": id })
    }

    /// Registers a callback that answers every invocation at once.
    pub fn register_fn(
        &mut self,
        mut f: impl FnMut(&[Value]) -> Result<Value, String> + 'static,
    ) -> Value {
        self.register(move |inv, out| out.reply(inv.call_id, f(&inv.args)))
    }

    /// The `{"$host":name}` value for a fake host primitive.
    pub fn primitive(name: &str) -> Value {
        json!({ "$host": name })
    }

    /// The `{"$undefined":true}` value.
    pub fn undefined() -> Value {
        json!({ "$undefined": true })
    }

    /// Imports the ES module at `path`, returning a `{"$ref":h}` handle.
    ///
    /// # Errors
    ///
    /// [`WorkflowError::Js`] if the import throws (for example a syntax
    /// error); any other variant tears the host down.
    pub fn import(&mut self, path: &Path) -> Result<Value, WorkflowError> {
        let desc = format!("import of {}", path.display());
        self.request(json!({ "op": "import", "path": path }), desc)
    }

    /// Reads `member` of a handle.
    ///
    /// # Errors
    ///
    /// As for [`Host::import`].
    pub fn get(&mut self, handle: &Value, member: &str) -> Result<Value, WorkflowError> {
        let id = handle_id(handle)?;
        self.request(
            json!({ "op": "get", "handle": id, "member": member }),
            format!("read of member `{member}`"),
        )
    }

    /// Reads an export of a module handle, refusing an `undefined` one.
    ///
    /// # Errors
    ///
    /// [`WorkflowError::MissingMember`], or as for [`Host::import`].
    pub fn export(&mut self, module: &Value, name: &str) -> Result<Value, WorkflowError> {
        let value = self.get(module, name)?;
        if is_undefined(&value) {
            return Err(WorkflowError::MissingMember(name.to_owned()));
        }
        Ok(value)
    }

    /// Compiles `source` as the body of an async function with `params`.
    ///
    /// # Errors
    ///
    /// As for [`Host::import`].
    pub fn compile(&mut self, params: &[&str], source: &str) -> Result<Value, WorkflowError> {
        self.request(
            json!({ "op": "compile", "params": params, "source": source }),
            "compile of a function body".to_owned(),
        )
    }

    /// Calls a function value with `args`, servicing callbacks until it
    /// settles.
    ///
    /// # Errors
    ///
    /// As for [`Host::import`].
    pub fn call(&mut self, target: &Value, args: Vec<Value>) -> Result<Value, WorkflowError> {
        self.request(
            json!({ "op": "call", "target": target, "args": args }),
            "a function call".to_owned(),
        )
    }

    /// Calls a function value with arguments given as JSON text (an array).
    ///
    /// The text is spliced into the request verbatim, so object key order
    /// survives to JavaScript exactly as written — which a `serde_json::Value`
    /// argument (sorted keys) cannot guarantee. Use this when the callee's
    /// output depends on key order (for example `JSON.stringify` of an
    /// argument). The tagged forms (`$fn`, `$undefined`, …) are decoded as for
    /// [`Host::call`].
    ///
    /// # Errors
    ///
    /// [`WorkflowError::Transform`] when `args_json` is not a JSON array, or as
    /// for [`Host::call`].
    pub fn call_with_json_args(
        &mut self,
        target: &Value,
        args_json: &str,
    ) -> Result<Value, WorkflowError> {
        // One request per line: compact JSON never needs a raw line break.
        if args_json.contains(['\n', '\r'])
            || !matches!(
                serde_json::from_str::<Value>(args_json),
                Ok(Value::Array(_))
            )
        {
            return Err(WorkflowError::Transform(
                "call arguments must be a single-line JSON array".to_owned(),
            ));
        }
        let id = self.next_id;
        self.next_id += 1;
        let line = format!(r#"{{"op":"call","id":{id},"target":{target},"args":{args_json}}}"#);
        self.exchange(id, &line, "a function call")
    }

    /// Calls export `name` of a module handle.
    ///
    /// # Errors
    ///
    /// As for [`Host::export`] and [`Host::call`].
    pub fn call_export(
        &mut self,
        module: &Value,
        name: &str,
        args: Vec<Value>,
    ) -> Result<Value, WorkflowError> {
        let f = self.export(module, name)?;
        self.call(&f, args)
    }

    /// Constructs `new globalThis[global](...args)` (for example an
    /// `AbortController`), returning a `{"$ref":h}` handle.
    ///
    /// # Errors
    ///
    /// [`WorkflowError::Js`] if `global` is not a constructor or construction
    /// throws; otherwise as for [`Host::import`].
    pub fn construct(&mut self, global: &str, args: Vec<Value>) -> Result<Value, WorkflowError> {
        self.request(
            json!({ "op": "new", "global": global, "args": args }),
            format!("construction of `{global}`"),
        )
    }

    /// Reads `member` of a handle without encoding it, returning a
    /// `{"$ref":h}` handle to the member itself, so an object that does not
    /// survive JSON (an `AbortSignal`) can be passed back by reference.
    ///
    /// # Errors
    ///
    /// As for [`Host::import`].
    pub fn get_ref(&mut self, handle: &Value, member: &str) -> Result<Value, WorkflowError> {
        let id = handle_id(handle)?;
        self.request(
            json!({ "op": "get", "handle": id, "member": member, "keep": true }),
            format!("kept read of member `{member}`"),
        )
    }

    /// Calls method `member` of a handle with the handle as its receiver,
    /// servicing callbacks until it settles.
    ///
    /// # Errors
    ///
    /// [`WorkflowError::Js`] if the member is not a function or the call
    /// throws; otherwise as for [`Host::import`].
    pub fn invoke(
        &mut self,
        handle: &Value,
        member: &str,
        args: Vec<Value>,
    ) -> Result<Value, WorkflowError> {
        let id = handle_id(handle)?;
        self.request(
            json!({ "op": "invoke", "handle": id, "member": member, "args": args }),
            format!("invocation of member `{member}`"),
        )
    }

    /// Sends a call of `target` with `args` without waiting for it. Collect
    /// its result with [`Host::await_call`]; meanwhile every other request,
    /// [`Host::service`] and [`Host::flush`] keep delivering callbacks.
    ///
    /// # Errors
    ///
    /// [`WorkflowError::Dead`] or [`WorkflowError::Session`] if the line
    /// cannot be sent (the host is torn down).
    pub fn start_call(
        &mut self,
        target: &Value,
        args: Vec<Value>,
    ) -> Result<PendingCall, WorkflowError> {
        let id = self.next_id;
        self.next_id += 1;
        let line = json!({ "op": "call", "id": id, "target": target, "args": args }).to_string();
        self.send_request(id, &line, &format!("request #{id} (a started call)"))?;
        Ok(PendingCall { id })
    }

    /// Whether `call`'s result has already arrived (it is then returned by
    /// [`Host::await_call`] without waiting).
    pub fn is_settled(&self, call: &PendingCall) -> bool {
        self.settled.contains_key(&call.id)
    }

    /// Waits for `call`'s result, servicing callbacks meanwhile.
    ///
    /// # Errors
    ///
    /// [`WorkflowError::Js`] if the call threw or rejected; otherwise as for
    /// [`Host::import`].
    pub fn await_call(&mut self, call: PendingCall) -> Result<Value, WorkflowError> {
        let pending = format!("request #{} (a started call)", call.id);
        self.wait_for(call.id, &pending)
    }

    /// Receives and handles one incoming message: a callback is dispatched to
    /// its handler, a started call's result is buffered. Blocks until a
    /// message arrives, bounded by the host's deadline — so call it only while
    /// JavaScript has something outstanding that will emit one.
    ///
    /// # Errors
    ///
    /// As for [`Host::import`].
    pub fn service(&mut self) -> Result<(), WorkflowError> {
        self.receive_one("the next message")
    }

    /// Returns once JavaScript has handled every line sent before it and
    /// drained the promise work those lines set in motion: two `ping`s, the
    /// second sent only after the first is answered, so it is read in a
    /// fresh macrotask after every microtask the earlier lines queued.
    /// Callbacks emitted meanwhile are dispatched to their handlers. Work
    /// waiting on I/O or timers is not covered.
    ///
    /// # Errors
    ///
    /// As for [`Host::import`].
    pub fn flush(&mut self) -> Result<(), WorkflowError> {
        for _ in 0..2 {
            self.request(json!({ "op": "ping" }), "a flush ping".to_owned())?;
        }
        Ok(())
    }

    /// Answers a held callback invocation outside a handler, as
    /// [`Outbox::reply`] does inside one.
    ///
    /// # Errors
    ///
    /// [`WorkflowError::Dead`] or [`WorkflowError::Session`] if the line
    /// cannot be sent (the host is torn down).
    pub fn reply(
        &mut self,
        call_id: u64,
        result: Result<Value, String>,
    ) -> Result<(), WorkflowError> {
        self.send_settle(call_id, &Settle::from_result(result))
    }

    /// Compiles a workflow script's whole body (see [`driver_source`]) with
    /// [`DRIVER_PARAMS`], returning the function to call with
    /// `[args, agent, pipeline, parallel, log]`.
    ///
    /// # Errors
    ///
    /// [`WorkflowError::Transform`], or as for [`Host::compile`].
    pub fn load_driver(&mut self, script: &str) -> Result<Value, WorkflowError> {
        let source = driver_source(script)?;
        self.compile(&DRIVER_PARAMS, &source)
    }

    /// Extracts the named helpers from a workflow script without running its
    /// driver (see [`helper_source`]), returning an object of `$fn` values.
    ///
    /// # Errors
    ///
    /// [`WorkflowError::Transform`], or as for [`Host::compile`].
    pub fn extract_helpers(
        &mut self,
        script: &str,
        names: &[&str],
    ) -> Result<Value, WorkflowError> {
        let source = helper_source(script, names)?;
        let f = self.compile(&[], &source)?;
        self.call(&f, Vec::new())
    }

    /// Ends the host: asks the runtime to exit, reaps it and removes the temp
    /// directory.
    ///
    /// # Errors
    ///
    /// [`WorkflowError::Dead`] if the host was already torn down, or
    /// [`WorkflowError::Session`] if the runtime did not exit cleanly (it is
    /// killed and reaped regardless, and the directory removed).
    pub fn shutdown(mut self) -> Result<(), WorkflowError> {
        let Some(mut session) = self.session.take() else {
            return Err(WorkflowError::Dead);
        };
        let _ = session.send_line(r#"{"op":"shutdown"}"#);
        let result = session.shutdown();
        self.dir = None;
        result.map(|_| ()).map_err(|source| WorkflowError::Session {
            pending: "shutdown".to_owned(),
            source,
        })
    }

    fn teardown(&mut self) -> String {
        let stderr = match self.session.as_mut() {
            Some(session) => session.abort(),
            None => String::new(),
        };
        self.session = None;
        self.dir = None;
        stderr
    }

    fn request(&mut self, mut req: Value, desc: String) -> Result<Value, WorkflowError> {
        let id = self.next_id;
        self.next_id += 1;
        req["id"] = json!(id);
        self.exchange(id, &req.to_string(), &desc)
    }

    /// Sends request line `line` (carrying `id`) and services callbacks until
    /// its reply arrives.
    fn exchange(&mut self, id: u64, line: &str, desc: &str) -> Result<Value, WorkflowError> {
        let pending = format!("request #{id} ({desc})");
        self.send_request(id, line, &pending)?;
        self.wait_for(id, &pending)
    }

    /// Sends request line `line` and records `id` as started.
    fn send_request(&mut self, id: u64, line: &str, pending: &str) -> Result<(), WorkflowError> {
        let Some(session) = self.session.as_mut() else {
            return Err(WorkflowError::Dead);
        };
        if let Err(source) = session.send_line(line) {
            self.teardown();
            return Err(WorkflowError::Session {
                pending: pending.to_owned(),
                source,
            });
        }
        self.started.insert(id);
        Ok(())
    }

    /// Services messages until the result for started request `id` is in.
    fn wait_for(&mut self, id: u64, pending: &str) -> Result<Value, WorkflowError> {
        loop {
            if let Some(result) = self.settled.remove(&id) {
                return result.map_err(WorkflowError::Js);
            }
            if !self.started.contains(&id) {
                return Err(WorkflowError::Transform(format!(
                    "{pending} was never started on this host or was already collected"
                )));
            }
            self.receive_one(pending)?;
        }
    }

    /// Receives one message: dispatches a callback, or buffers the result of
    /// a started request. Anything else is a protocol error.
    fn receive_one(&mut self, pending: &str) -> Result<(), WorkflowError> {
        let Some(session) = self.session.as_mut() else {
            return Err(WorkflowError::Dead);
        };
        let line = match session.recv_line() {
            Ok(line) => line,
            Err(source) => {
                self.teardown();
                return Err(WorkflowError::Session {
                    pending: pending.to_owned(),
                    source,
                });
            }
        };
        let msg: Value = match serde_json::from_str(&line) {
            Ok(v @ Value::Object(_)) => v,
            Ok(_) | Err(_) => {
                return Err(self.protocol_error("expected one JSON object per line", &line));
            }
        };
        let op = msg.get("op").and_then(Value::as_str);
        if op == Some("callback") {
            return self.dispatch(&msg, &line);
        }
        let started = msg
            .get("id")
            .and_then(Value::as_u64)
            .filter(|id| self.started.contains(id));
        match (op, started) {
            (Some("ok"), Some(id)) => {
                self.started.remove(&id);
                let value = msg.get("value").cloned().unwrap_or(Value::Null);
                self.settled.insert(id, Ok(value));
                Ok(())
            }
            (Some("err"), Some(id)) => {
                self.started.remove(&id);
                let e = msg.get("error").cloned().unwrap_or(Value::Null);
                let field = |k: &str| e.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
                self.settled.insert(
                    id,
                    Err(JsError {
                        name: field("name"),
                        message: field("message"),
                        stack: field("stack"),
                        file: field("file"),
                    }),
                );
                Ok(())
            }
            _ => Err(self.protocol_error(
                &format!("unexpected message while waiting for {pending}"),
                &line,
            )),
        }
    }

    fn dispatch(&mut self, msg: &Value, line: &str) -> Result<(), WorkflowError> {
        let (Some(call_id), Some(fn_id)) = (
            msg.get("callId").and_then(Value::as_u64),
            msg.get("fn").and_then(Value::as_u64),
        ) else {
            return Err(self.protocol_error("callback without callId/fn", line));
        };
        let args = match msg.get("args") {
            Some(Value::Array(a)) => a.clone(),
            _ => Vec::new(),
        };
        let inv = Invocation {
            call_id,
            seq: self.next_seq,
            args,
        };
        self.next_seq += 1;
        let mut outbox = Outbox::default();
        match self.handlers.get_mut(&fn_id) {
            Some(handler) => handler(&inv, &mut outbox),
            None => return Err(self.protocol_error("callback for an unregistered function", line)),
        }
        for (call_id, settle) in outbox.replies {
            self.send_settle(call_id, &settle)?;
        }
        Ok(())
    }

    /// Sends the reply that settles callback invocation `call_id`.
    fn send_settle(&mut self, call_id: u64, settle: &Settle) -> Result<(), WorkflowError> {
        let Some(session) = self.session.as_mut() else {
            return Err(WorkflowError::Dead);
        };
        if let Err(source) = session.send_line(&settle.line(call_id)) {
            self.teardown();
            return Err(WorkflowError::Session {
                pending: format!("reply to callback {call_id}"),
                source,
            });
        }
        Ok(())
    }

    fn protocol_error(&mut self, detail: &str, line: &str) -> WorkflowError {
        let stderr = self.teardown();
        let mut shown: String = line.chars().take(200).collect();
        if shown.len() < line.len() {
            shown.push('…');
        }
        WorkflowError::Protocol {
            detail: detail.to_owned(),
            line: shown,
            stderr,
        }
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        // Session's own Drop kills and reaps the group; the TempDir's Drop
        // removes the directory afterwards (fields drop in declaration order).
        self.session = None;
        self.dir = None;
    }
}

fn handle_id(handle: &Value) -> Result<u64, WorkflowError> {
    handle
        .get("$ref")
        .or_else(|| handle.get("$fn"))
        .and_then(Value::as_u64)
        .ok_or_else(|| WorkflowError::Transform(format!("not a handle value: {handle}")))
}

/// Whether `value` is the `{"$undefined":true}` tag.
pub fn is_undefined(value: &Value) -> bool {
    value
        .as_object()
        .is_some_and(|o| o.len() == 1 && o.contains_key("$undefined"))
}

/// Whether `value` is a `{"$fn":h}` function handle.
pub fn is_fn(value: &Value) -> bool {
    value
        .as_object()
        .is_some_and(|o| o.len() == 1 && o.get("$fn").is_some_and(Value::is_u64))
}

/// Object member `key`, treating an absent key and an `undefined` value alike
/// as `None`.
pub fn member<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    value.get(key).filter(|v| !is_undefined(v))
}

/// Makes a workflow script's body compilable as a function: drops the
/// `export ` of the single column-0 `export const meta` line.
///
/// # Errors
///
/// [`WorkflowError::Transform`] if there is no such line, or more than one.
pub fn driver_source(script: &str) -> Result<String, WorkflowError> {
    let hits: Vec<usize> = line_starts(script)
        .filter(|&i| script[i..].starts_with("export const meta"))
        .collect();
    match hits.as_slice() {
        [at] => {
            let mut out = String::with_capacity(script.len());
            out.push_str(&script[..*at]);
            out.push_str(&script[at + "export ".len()..]);
            Ok(out)
        }
        [] => Err(WorkflowError::Transform(
            "no column-0 `export const meta` line; is this a workflow script?".to_owned(),
        )),
        _ => Err(WorkflowError::Transform(format!(
            "{} column-0 `export const meta` lines; expected exactly one",
            hits.len()
        ))),
    }
}

/// [`driver_source`] plus `return { <names> };` injected before the unique
/// column-0 [`DRIVER_SENTINEL`] line, so calling the compiled body returns the
/// named helpers and never reaches the driver.
///
/// # Errors
///
/// [`WorkflowError::Transform`] if the meta line or the sentinel is missing
/// or duplicated, or a name is not a plain identifier.
pub fn helper_source(script: &str, names: &[&str]) -> Result<String, WorkflowError> {
    for name in names {
        let ok = !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
            && !name.starts_with(|c: char| c.is_ascii_digit());
        if !ok {
            return Err(WorkflowError::Transform(format!(
                "helper name {name:?} is not a plain identifier"
            )));
        }
    }
    let body = driver_source(script)?;
    let hits: Vec<usize> = line_starts(&body)
        .filter(|&i| body[i..].starts_with(DRIVER_SENTINEL))
        .collect();
    match hits.as_slice() {
        [at] => {
            let mut out = String::with_capacity(body.len() + 64);
            out.push_str(&body[..*at]);
            out.push_str("return { ");
            out.push_str(&names.join(", "));
            out.push_str(" };\n");
            out.push_str(&body[*at..]);
            Ok(out)
        }
        [] => Err(WorkflowError::Transform(format!(
            "no column-0 `{DRIVER_SENTINEL}` sentinel line"
        ))),
        _ => Err(WorkflowError::Transform(format!(
            "{} column-0 `{DRIVER_SENTINEL}` sentinel lines; expected exactly one",
            hits.len()
        ))),
    }
}

/// The exact inverse of [`helper_source`]: removes the injected
/// `return { <names> };` line that sits immediately before the column-0
/// [`DRIVER_SENTINEL`], and restores `export ` on the single column-0
/// `const meta` line, so `invert_helper_source(helper_source(s, n)?, n)`
/// reproduces `s` byte for byte. A caller uses it as provenance: the text it
/// executed through [`Host::extract_helpers`] is the text it read.
///
/// # Errors
///
/// [`WorkflowError::Transform`] if the sentinel is missing or duplicated, the
/// line before it is not the injected return for `names`, or the column-0
/// `const meta` line is missing or duplicated.
pub fn invert_helper_source(transformed: &str, names: &[&str]) -> Result<String, WorkflowError> {
    let injected = format!("return {{ {} }};\n", names.join(", "));
    let sentinels: Vec<usize> = line_starts(transformed)
        .filter(|&i| transformed[i..].starts_with(DRIVER_SENTINEL))
        .collect();
    let at = match sentinels.as_slice() {
        [at] => *at,
        [] => {
            return Err(WorkflowError::Transform(format!(
                "no column-0 `{DRIVER_SENTINEL}` sentinel line to invert around"
            )));
        }
        _ => {
            return Err(WorkflowError::Transform(format!(
                "{} column-0 `{DRIVER_SENTINEL}` sentinel lines; expected exactly one",
                sentinels.len()
            )));
        }
    };
    let injected_here = at
        .checked_sub(injected.len())
        .filter(|&start| start == 0 || transformed.as_bytes()[start - 1] == b'\n')
        .filter(|&start| transformed.get(start..at) == Some(injected.as_str()));
    let Some(start) = injected_here else {
        return Err(WorkflowError::Transform(format!(
            "the line before the driver sentinel is not the injected {:?}",
            injected.trim_end()
        )));
    };
    let mut body = String::with_capacity(transformed.len());
    body.push_str(&transformed[..start]);
    body.push_str(&transformed[at..]);
    let metas: Vec<usize> = line_starts(&body)
        .filter(|&i| body[i..].starts_with("const meta"))
        .collect();
    match metas.as_slice() {
        [m] => {
            let mut out = String::with_capacity(body.len() + "export ".len());
            out.push_str(&body[..*m]);
            out.push_str("export ");
            out.push_str(&body[*m..]);
            Ok(out)
        }
        [] => Err(WorkflowError::Transform(
            "no column-0 `const meta` line to restore `export` on".to_owned(),
        )),
        _ => Err(WorkflowError::Transform(format!(
            "{} column-0 `const meta` lines; expected exactly one",
            metas.len()
        ))),
    }
}

fn line_starts(text: &str) -> impl Iterator<Item = usize> + '_ {
    std::iter::once(0).chain(
        text.char_indices()
            .filter(|&(_, c)| c == '\n')
            .map(|(i, _)| i + 1)
            .filter(move |&i| i < text.len()),
    )
}

/// A private, mutable copy of real workflow sources for planting logic
/// mutants. See the [module documentation](self).
#[derive(Debug)]
pub struct MutantTree {
    dir: TempDir,
}

impl MutantTree {
    /// Copies each of `files` (paths relative to `root`) into a fresh temp
    /// directory under the same relative path.
    ///
    /// # Errors
    ///
    /// Any I/O error, annotated with the file.
    pub fn copy(root: &Path, files: &[&str]) -> io::Result<Self> {
        let dir = tempfile::Builder::new().prefix("rdm-mutant-").tempdir()?;
        for rel in files {
            let from = root.join(rel);
            let to = dir.path().join(rel);
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(&from, &to).map_err(|e| {
                io::Error::new(e.kind(), format!("copying {}: {e}", from.display()))
            })?;
        }
        Ok(Self { dir })
    }

    /// The tree's root; copied files keep their relative paths under it.
    pub fn root(&self) -> &Path {
        self.dir.path()
    }

    /// Replaces the single occurrence of `from` with `to` in `file`
    /// (relative to the root).
    ///
    /// # Errors
    ///
    /// A setup error naming `mutant` when `from` does not occur exactly once,
    /// so an unapplied mutant can never report a pass.
    pub fn replace_once(&self, file: &str, mutant: &str, from: &str, to: &str) -> io::Result<()> {
        let path = self.dir.path().join(file);
        let text = std::fs::read_to_string(&path)?;
        let count = text.matches(from).count();
        if count != 1 {
            return Err(io::Error::other(format!(
                "mutant `{mutant}` not applied: its anchor occurs {count} times in {file} (expected exactly once): {from:?}"
            )));
        }
        std::fs::write(&path, text.replacen(from, to, 1))
    }
}

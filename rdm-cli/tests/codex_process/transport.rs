//! `scripts/lib/codex-process.mjs` — the Codex subprocess transport the
//! runtime imports — against [`FAKE_CODEX`](crate::support::FAKE_CODEX):
//! strict schemas, safe argv, stream validation, private redacted evidence,
//! timeouts, cancellation and `boundedParallel`. Ported from
//! `scripts/lib/codex-spike-process.test.mjs`.
//!
//! Every `runCodex` call gets an explicit `bin`, an explicit `env`
//! (`PATH=/usr/bin:/bin` plus the fake's two locations) and an explicit
//! `cwd` (the test root). A rejection must never carry the provider's
//! diagnostics (the fixture plants `secret` in stream errors and a bearer
//! token on stderr).

use std::cell::RefCell;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use rdm_devtools::workflow::{Host, WorkflowError};
use serde_json::{Value, json};

use crate::support::{
    Call, FAKE_CODEX, Reaper, Root, alive, calls, gone_within, host, js_err, js_ok, read_pid,
    until, write_exec,
};

const TRANSPORT: &str = "scripts/lib/codex-process.mjs";
const SPIKE: &str = "scripts/lib/codex-spike-process.mjs";

/// A response the fake replays.
#[derive(Default)]
struct Response {
    stdout: Option<String>,
    output: Option<Value>,
    stderr: Option<&'static str>,
    exit: Option<i32>,
    behaviour: Option<&'static str>,
}

/// A JSONL stream: `thread.started` (`thread-one`), `turn.started`, the
/// `extra` lines verbatim, then `turn.completed` unless `complete` is false.
fn stream(extra: &[String], complete: bool) -> String {
    let mut lines = vec![
        json!({"type":"thread.started","thread_id":"thread-one"}).to_string(),
        json!({"type":"turn.started"}).to_string(),
    ];
    lines.extend(extra.iter().cloned());
    if complete {
        lines.push(
            json!({"type":"turn.completed","usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3}})
                .to_string(),
        );
    }
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

/// A well-formed successful turn answering `{ok:true, note:null}`.
fn answer(extra: &[Value]) -> Response {
    Response {
        stdout: Some(stream(
            &extra.iter().map(Value::to_string).collect::<Vec<_>>(),
            true,
        )),
        output: Some(json!({"ok": true, "note": null})),
        ..Response::default()
    }
}

/// The provider failing with a bearer token on stderr and exit 1.
fn provider_failure() -> Response {
    Response {
        stderr: Some("Authorization: Bearer sk-fake-secret-token\nauthentication failed\n"),
        exit: Some(1),
        ..Response::default()
    }
}

/// The closed schema every call uses.
fn schema() -> Value {
    json!({"type":"object","additionalProperties":false,"required":["ok"],
           "properties":{"ok":{"type":"boolean"},"note":{"type":"string"}}})
}

struct Transport {
    host: Host,
    module: Value,
    _reaper: Reaper,
    fake: PathBuf,
    root: Root,
}

impl Transport {
    fn new() -> Self {
        Self::importing(TRANSPORT)
    }

    fn importing(rel: &str) -> Self {
        let root = Root::new();
        let fake = root.join("fake-codex");
        write_exec(&fake, FAKE_CODEX);
        let reaper = Reaper::new(&[&root.path]);
        let mut host = host(&root, &[]);
        let module = js_ok(host.import(&crate::support::repo().join(rel)), "import");
        Self {
            host,
            module,
            _reaper: reaper,
            fake,
            root,
        }
    }

    /// Writes `r` as response directory `name`; returns its path.
    fn respond(&self, name: &str, r: Response) -> PathBuf {
        let dir = self.root.mkdir(&format!("responses/{name}"));
        if let Some(s) = r.stdout {
            fs::write(dir.join("stdout"), s).unwrap();
        }
        if let Some(v) = r.output {
            fs::write(dir.join("output"), v.to_string()).unwrap();
        }
        if let Some(s) = r.stderr {
            fs::write(dir.join("stderr"), s).unwrap();
        }
        if let Some(code) = r.exit {
            fs::write(dir.join("exit"), code.to_string()).unwrap();
        }
        if let Some(b) = r.behaviour {
            fs::write(dir.join("behaviour"), b).unwrap();
        }
        dir
    }

    /// `runCodex` options replaying response `name`.
    fn opts(&self, name: &str, r: Response) -> Value {
        let response = self.respond(name, r);
        json!({
            "bin": self.fake,
            "cwd": self.root.path,
            "prompt": "Say ok",
            "schema": schema(),
            "model": "test-model",
            "effort": "medium",
            "env": {
                "PATH": "/usr/bin:/bin",
                "FAKE_ROOT": self.root.path,
                "FAKE_RESPONSE": response,
            },
        })
    }

    fn export(&mut self, name: &str) -> Value {
        js_ok(self.host.export(&self.module, name), name)
    }

    fn run(&mut self, opts: Value) -> Result<Value, WorkflowError> {
        let f = self.export("runCodex");
        self.host.call(&f, vec![opts])
    }

    fn calls(&self) -> Vec<Call> {
        calls(&self.root.path)
    }

    fn only_call(&self) -> Call {
        let calls = self.calls();
        assert_eq!(calls.len(), 1, "exactly one fake invocation: {calls:?}");
        calls.into_iter().next().unwrap()
    }
}

/// A rejection that carries none of the provider's planted diagnostics.
fn assert_rejects_without_diagnostics(r: Response) {
    let mut t = Transport::new();
    let opts = t.opts("case", r);
    let e = js_err(t.run(opts), "runCodex");
    assert!(
        !e.message.contains("secret") && !e.message.contains("sensitive"),
        "the rejection leaks provider diagnostics: {e}"
    );
}

#[test]
fn spike_entrypoint_runs_the_runtime_transport() {
    let mut t = Transport::importing(SPIKE);
    let opts = t.opts("ok", answer(&[]));
    let out = js_ok(t.run(opts), "runCodex via the spike entrypoint");
    assert_eq!(out["value"], json!({"ok": true}));
    assert_eq!(out["threadId"], "thread-one");
    let call = t.only_call();
    assert_eq!(call.after("--sandbox"), Some("read-only"));
    assert_eq!(call.prompt, "Say ok");

    let order = Rc::new(RefCell::new(Vec::new()));
    let thunks: Vec<Value> = ["a", "b"]
        .into_iter()
        .map(|v| {
            let order = Rc::clone(&order);
            t.host.register(move |inv, out| {
                order.borrow_mut().push(v);
                out.reply(inv.call_id, Ok(json!(v)));
            })
        })
        .collect();
    let bounded = t.export("boundedParallel");
    let values = js_ok(
        t.host.call(&bounded, vec![json!(thunks), json!(1)]),
        "boundedParallel via the spike entrypoint",
    );
    assert_eq!(values, json!(["a", "b"]));
    assert_eq!(*order.borrow(), ["a", "b"], "limit 1 runs them in order");
}

#[test]
fn validate_schema_accepts_the_closed_subset_and_fails_closed() {
    let mut t = Transport::new();
    let validate = t.export("validateSchema");
    let mut check = |value: Value, schema: Value| t.host.call(&validate, vec![value, schema]);
    assert_eq!(
        js_ok(check(json!({"ok": true}), schema()), "valid"),
        json!(true)
    );
    for (value, schema, expect) in [
        (json!({"ok": "yes"}), schema(), "schema"),
        (json!({"ok": true, "extra": 1}), schema(), "schema"),
        (
            json!("x"),
            json!({"type":"string","pattern":"x"}),
            "unsupported",
        ),
        (
            json!(101),
            json!({"type":"integer","minimum":0,"maximum":100}),
            "schema",
        ),
    ] {
        let e = js_err(check(value.clone(), schema.clone()), "validateSchema");
        assert!(
            e.message.to_lowercase().contains(expect),
            "{value} against {schema}: {e}"
        );
    }
}

#[test]
fn run_codex_uses_strict_nullable_schema_stdin_and_safe_argv() {
    let mut t = Transport::new();
    let opts = t.opts("ok", answer(&[]));
    let out = js_ok(t.run(opts), "runCodex");
    assert_eq!(
        out["value"],
        json!({"ok": true}),
        "the optional null is dropped"
    );
    assert_eq!(out["threadId"], "thread-one");
    assert_eq!(out["usage"]["input_tokens"], 10);
    let call = t.only_call();
    assert_eq!(call.prompt, "Say ok", "the prompt travels on stdin");
    assert!(!call.has("Say ok"), "and never in argv: {:?}", call.argv);
    assert!(call.has("--ephemeral"), "{:?}", call.argv);
    assert_eq!(call.after("--sandbox"), Some("read-only"));
    assert!(!call.has("--ignore-rules"), "{:?}", call.argv);
    assert_eq!(call.argv.last().map(String::as_str), Some("-"));
    let strict = call.schema.expect("the schema file was passed");
    // Every property becomes required (key order follows the object the
    // runtime received, which serde_json sorts).
    let mut required = crate::workflow_support::strings(&strict["required"]);
    required.sort();
    assert_eq!(required, ["note", "ok"], "{strict}");
    let any_of = strict["properties"]["note"]["anyOf"].as_array().unwrap();
    assert!(any_of.iter().any(|s| s["type"] == "null"), "{strict}");
}

#[test]
fn run_codex_disables_optional_capabilities_and_escalation() {
    let mut t = Transport::new();
    let opts = t.opts("ok", answer(&[]));
    js_ok(t.run(opts), "runCodex");
    let call = t.only_call();
    let configs = call.configs();
    for config in [
        "approval_policy=\"never\"",
        "web_search=\"disabled\"",
        "features.apps=false",
        "features.plugins=false",
        "features.remote_plugin=false",
        "features.browser_use=false",
        "features.computer_use=false",
        "features.hooks=false",
        "features.multi_agent=false",
        "features.multi_agent_v2=false",
        "features.workspace_dependencies=false",
        "features.image_generation=false",
    ] {
        assert!(
            configs.contains(&config),
            "missing role restriction {config}: {configs:?}"
        );
    }
    assert!(call.has("--ignore-user-config"), "{:?}", call.argv);
    assert!(!call.has("--ignore-rules"), "{:?}", call.argv);
    assert!(
        !call.has("--dangerously-bypass-approvals-and-sandbox"),
        "{:?}",
        call.argv
    );
}

#[test]
fn run_codex_accepts_a_recovered_nonzero_shell_command() {
    let mut t = Transport::new();
    let opts = t.opts(
        "recovered",
        answer(&[
            json!({"type":"item.completed","item":{"type":"command_execution","status":"failed","exit_code":1}}),
        ]),
    );
    assert_eq!(js_ok(t.run(opts), "runCodex")["value"], json!({"ok": true}));
}

fn command_item(exit_code: Value) -> Value {
    json!({"type":"item.completed","item":{"type":"command_execution","status":"failed","exit_code":exit_code}})
}

#[test]
fn run_codex_rejects_inconsistent_command_without_diagnostics() {
    assert_rejects_without_diagnostics(answer(&[command_item(json!(0))]));
}

#[test]
fn run_codex_rejects_interrupted_command_without_diagnostics() {
    assert_rejects_without_diagnostics(answer(&[command_item(Value::Null)]));
}

#[test]
fn run_codex_rejects_failed_tool_without_diagnostics() {
    assert_rejects_without_diagnostics(answer(&[
        json!({"type":"item.completed","item":{"type":"mcp_tool_call","status":"failed"}}),
    ]));
}

#[test]
fn run_codex_rejects_killed_child_without_diagnostics() {
    assert_rejects_without_diagnostics(Response {
        behaviour: Some("kill-self"),
        ..Response::default()
    });
}

#[test]
fn run_codex_rejects_malformed_stream_without_diagnostics() {
    let mut r = answer(&[]);
    r.stdout = Some(stream(&["{".to_owned()], true));
    assert_rejects_without_diagnostics(r);
}

#[test]
fn run_codex_rejects_error_event_without_diagnostics() {
    assert_rejects_without_diagnostics(answer(&[json!({"type":"error","message":"secret"})]));
}

#[test]
fn run_codex_rejects_failed_turn_without_diagnostics() {
    assert_rejects_without_diagnostics(answer(&[
        json!({"type":"turn.failed","error":{"message":"secret"}}),
    ]));
}

#[test]
fn run_codex_rejects_duplicate_thread_without_diagnostics() {
    assert_rejects_without_diagnostics(answer(&[
        json!({"type":"thread.started","thread_id":"thread-two"}),
    ]));
}

#[test]
fn run_codex_rejects_truncated_stream_without_diagnostics() {
    let mut r = answer(&[]);
    r.stdout = Some(stream(&[], false));
    assert_rejects_without_diagnostics(r);
}

#[test]
fn run_codex_rejects_invalid_response_without_diagnostics() {
    let mut r = answer(&[]);
    r.output = Some(json!({"ok": "yes"}));
    assert_rejects_without_diagnostics(r);
}

#[test]
fn run_codex_rejects_output_flood_without_diagnostics() {
    let mut r = answer(&[]);
    r.stdout = Some(stream(&["x".repeat(9 * 1024 * 1024)], true));
    assert_rejects_without_diagnostics(r);
}

/// Also stands for the legacy `rate`, `unknown-model` and `nonzero` modes,
/// whose fake ran this exact branch (same stderr, exit 1).
#[test]
fn run_codex_rejects_nonzero_exit_without_diagnostics() {
    let mut t = Transport::new();
    let opts = t.opts("auth", provider_failure());
    let e = js_err(t.run(opts), "runCodex");
    assert!(e.message.contains("subprocess failed"), "{e}");
    assert!(
        !e.message.contains("secret") && !e.message.contains("sensitive"),
        "{e}"
    );
}

#[test]
fn run_codex_timeout_rejects_a_hung_child() {
    let mut t = Transport::new();
    let mut opts = t.opts(
        "hang",
        Response {
            behaviour: Some("hang"),
            ..Response::default()
        },
    );
    opts["timeoutMs"] = json!(1000);
    let e = js_err(t.run(opts), "runCodex");
    assert!(e.message.contains("timed out"), "{e}");
    let call = t.only_call();
    assert!(
        gone_within(call.pid, Duration::from_secs(3)),
        "the hung child {} survived its timeout",
        call.pid
    );
}

#[test]
fn run_codex_abort_rejects_a_hung_child() {
    let mut t = Transport::new();
    let controller = js_ok(t.host.construct("AbortController", Vec::new()), "new");
    let signal = js_ok(t.host.get_ref(&controller, "signal"), "signal");
    let mut opts = t.opts(
        "hang",
        Response {
            behaviour: Some("hang"),
            ..Response::default()
        },
    );
    opts["signal"] = signal;
    let run = t.export("runCodex");
    let pending = t.host.start_call(&run, vec![opts]).expect("start runCodex");
    let pid_file = || t.calls().first().map(|c| c.dir.join("pid"));
    assert!(
        until(Duration::from_secs(10), || pid_file()
            .and_then(|p| read_pid(&p))
            .is_some()),
        "the fake never started"
    );
    js_ok(t.host.invoke(&controller, "abort", Vec::new()), "abort");
    let e = js_err(t.host.await_call(pending), "aborted runCodex");
    assert!(e.message.contains("cancelled"), "{e}");
    let call = t.only_call();
    assert!(
        gone_within(call.pid, Duration::from_secs(3)),
        "the hung child {} survived the abort",
        call.pid
    );
}

#[test]
fn run_codex_resume_requires_the_same_thread() {
    let mut t = Transport::new();
    let mut opts = t.opts("ok", answer(&[]));
    opts["resumeThreadId"] = json!("different");
    let e = js_err(t.run(opts.clone()), "resume of another thread");
    assert!(e.message.to_lowercase().contains("thread"), "{e}");
    let call = t.only_call();
    assert!(
        call.has("resume") && call.has("different"),
        "{:?}",
        call.argv
    );
    assert!(!call.has("--ephemeral"), "{:?}", call.argv);
    opts["resumeThreadId"] = json!("thread-one");
    let out = js_ok(t.run(opts), "resume of the same thread");
    assert_eq!(out["threadId"], "thread-one");
}

#[test]
fn run_codex_failure_evidence_is_private_and_redacted() {
    let mut t = Transport::new();
    let evidence = t.root.join("evidence");
    let mut opts = t.opts("auth", provider_failure());
    opts["label"] = json!("find:code:ac");
    opts["evidenceDir"] = json!(evidence);
    let e = js_err(t.run(opts), "runCodex");
    assert!(e.message.contains("subprocess failed"), "{e}");
    let stderr = evidence.join("find:code:ac.stderr.txt");
    let diagnostic = fs::read_to_string(&stderr).expect("stderr evidence under the label");
    assert!(diagnostic.contains("authentication failed"), "{diagnostic}");
    assert!(diagnostic.contains("[REDACTED]"), "{diagnostic}");
    assert!(!diagnostic.contains("sk-fake-secret-token"), "{diagnostic}");
    assert!(evidence.join("find:code:ac.jsonl").is_file());
    let mode = |p: &std::path::Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&evidence), 0o700, "private evidence directory");
    assert_eq!(mode(&stderr), 0o600, "private evidence file");
}

/// Held thunk invocations, as `(index, call_id)` in arrival order.
type Held = Rc<RefCell<Vec<(usize, u64)>>>;

/// Registers `n` thunks whose invocations are held; each records
/// `(index, call_id)` in the returned list.
fn held_thunks(host: &mut Host, n: usize) -> (Vec<Value>, Held) {
    let pending = Rc::new(RefCell::new(Vec::new()));
    let thunks = (0..n)
        .map(|i| {
            let sink = Rc::clone(&pending);
            host.register(move |inv, _out| sink.borrow_mut().push((i, inv.call_id)))
        })
        .collect();
    (thunks, pending)
}

#[test]
fn bounded_parallel_preserves_order_under_the_limit() {
    let mut t = Transport::new();
    let (thunks, pending) = held_thunks(&mut t.host, 4);
    let bounded = t.export("boundedParallel");
    let call = t
        .host
        .start_call(&bounded, vec![json!(thunks), json!(2)])
        .expect("start");
    let (mut peak, mut answered) = (0, Vec::new());
    loop {
        t.host.flush().expect("flush");
        let live = pending.borrow().len();
        assert!(live <= 2, "{live} thunks in flight under a limit of 2");
        peak = peak.max(live);
        if t.host.is_settled(&call) {
            break;
        }
        // Answer the newest first, so completion order differs from input
        // order.
        let (index, id) = pending
            .borrow_mut()
            .pop()
            .expect("an unsettled call has a thunk in flight");
        t.host.reply(id, Ok(json!(index))).expect("reply");
        answered.push(index);
    }
    assert_eq!(peak, 2, "the limit was reached");
    assert_eq!(answered, [1, 2, 3, 0], "completion order was permuted");
    assert_eq!(
        js_ok(t.host.await_call(call), "boundedParallel"),
        json!([0, 1, 2, 3])
    );
}

#[test]
fn bounded_parallel_failure_waits_for_in_flight_work() {
    let mut t = Transport::new();
    let (thunks, pending) = held_thunks(&mut t.host, 2);
    let bounded = t.export("boundedParallel");
    let call = t
        .host
        .start_call(&bounded, vec![json!(thunks), json!(2)])
        .expect("start");
    t.host.flush().expect("flush");
    let started: Vec<(usize, u64)> = pending.borrow().clone();
    assert_eq!(started.len(), 2, "both thunks started: {started:?}");
    let id_of = |i: usize| started.iter().find(|(x, _)| *x == i).unwrap().1;
    t.host
        .reply(id_of(0), Err("failed".to_owned()))
        .expect("reply");
    t.host.flush().expect("flush");
    assert!(
        !t.host.is_settled(&call),
        "the failure must wait for the in-flight thunk"
    );
    t.host.reply(id_of(1), Ok(Value::Null)).expect("reply");
    let e = js_err(t.host.await_call(call), "boundedParallel");
    assert_eq!(e.message, "failed", "{e}");
}

#[test]
fn bounded_parallel_falsy_rejection_is_a_failure() {
    let mut t = Transport::new();
    let thunk = t
        .host
        .register(|inv, out| out.reject_with(inv.call_id, Value::Null));
    let bounded = t.export("boundedParallel");
    let e = js_err(
        t.host.call(&bounded, vec![json!([thunk])]),
        "boundedParallel over a null rejection",
    );
    assert_eq!(e.message, "null", "the rejection value itself: {e:?}");
}

#[test]
fn run_codex_cancellation_kills_descendants() {
    let mut t = Transport::new();
    let controller = js_ok(t.host.construct("AbortController", Vec::new()), "new");
    let signal = js_ok(t.host.get_ref(&controller, "signal"), "signal");
    let mut opts = t.opts(
        "descendant",
        Response {
            behaviour: Some("spawn-and-hang"),
            ..Response::default()
        },
    );
    opts["signal"] = signal;
    let run = t.export("runCodex");
    let pending = t.host.start_call(&run, vec![opts]).expect("start runCodex");
    let mut descendant = None;
    assert!(
        until(Duration::from_secs(10), || {
            descendant = t.calls().first().and_then(|c| c.descendant);
            descendant.is_some()
        }),
        "the fake never started its descendant"
    );
    let descendant = descendant.unwrap();
    assert!(alive(descendant));
    js_ok(t.host.invoke(&controller, "abort", Vec::new()), "abort");
    let e = js_err(t.host.await_call(pending), "cancelled runCodex");
    assert!(e.message.contains("cancelled"), "{e}");
    assert!(
        gone_within(descendant, Duration::from_secs(3)),
        "descendant {descendant} survived cancellation"
    );
}

//! Binding tests for `rdm_devtools::workflow`, exercised against the real
//! review core (`.claude/workflows/lib/review.mjs`) — no JavaScript fixture
//! is authored here — plus the failure modes a test author will hit: a
//! missing runtime, an invalid module, a malformed protocol stream and a hung
//! child, each of which must fail actionably and leave nothing behind.

mod common;

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use common::{fixture, gone_within};
use rdm_devtools::process::{ProcessSpec, Session, SessionError};
use rdm_devtools::workflow::{Host, HostConfig, MutantTree, WorkflowError, member};
use serde_json::{Value, json};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("rdm-devtools has a parent directory")
        .to_path_buf()
}

fn review_lib() -> PathBuf {
    repo_root().join(".claude/workflows/lib/review.mjs")
}

/// One recorded agent call.
#[derive(Debug, Clone)]
struct Call {
    seq: usize,
    label: String,
}

type Script = Box<dyn FnMut(&str) -> Result<Value, String>>;

/// Registers a recording fake agent whose replies come from `script(label)`.
fn fake_agent(host: &mut Host, mut script: Script) -> (Value, Rc<RefCell<Vec<Call>>>) {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let sink = Rc::clone(&calls);
    let agent = host.register(move |inv, out| {
        let label = inv
            .args
            .get(1)
            .and_then(|o| o.get("label"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        sink.borrow_mut().push(Call {
            seq: inv.seq,
            label: label.clone(),
        });
        out.reply(inv.call_id, script(&label));
    });
    (agent, calls)
}

fn deps(host: &mut Host, agent: Value) -> Value {
    let log = host.register_fn(|_| Ok(Value::Null));
    json!({
        "agent": agent,
        "pipeline": Host::primitive("pipeline"),
        "parallel": Host::primitive("parallel"),
        "log": log,
    })
}

/// A clean finder reply for `label`; `ac` gets an (empty) AC table.
fn clean_finder(label: &str) -> Value {
    if label.starts_with("find:code:ac") {
        json!({ "ac": [], "findings": [] })
    } else {
        json!({ "findings": [] })
    }
}

#[test]
fn async_calls_return_functions_and_drive_rust_callbacks() {
    let mut host = Host::start_default().expect("start host");
    let module = host.import(&review_lib()).expect("import review.mjs");
    let (agent, calls) = fake_agent(&mut host, Box::new(|label| Ok(clean_finder(label))));
    let d = deps(&mut host, agent);
    let run = host
        .call_export(&module, "buildReviewPipeline", vec![json!("code"), d])
        .expect("buildReviewPipeline");
    assert!(rdm_devtools::workflow::is_fn(&run), "a $fn handle: {run}");
    let result = host
        .call(&run, vec![json!({ "target": "task/demo" })])
        .expect("runReview resolves");
    assert_eq!(result["survivors"], json!([]));
    assert_eq!(result["coverage"]["complete"], json!(true));
    let labels: Vec<String> = calls.borrow().iter().map(|c| c.label.clone()).collect();
    assert!(
        labels.iter().all(|l| l.starts_with("find:code:")),
        "{labels:?}"
    );
    assert!(labels.len() >= 2, "every code reviewer ran: {labels:?}");
    host.shutdown().expect("clean shutdown");
}

#[test]
fn parallel_finders_all_arrive_before_any_refuter() {
    let mut host = Host::start_default().expect("start host");
    let module = host.import(&review_lib()).expect("import");
    let (agent, calls) = fake_agent(
        &mut host,
        Box::new(|label| {
            if label == "find:code:correctness" {
                Ok(json!({ "findings": [
                    { "id": "f1", "severity": "blocking", "confidence": 90, "what_fails": "x" },
                    { "id": "f2", "severity": "concern", "confidence": 90, "what_fails": "y" }
                ]}))
            } else if label.starts_with("refute:") {
                Ok(json!({ "refuted": false, "confidence": 90 }))
            } else {
                Ok(clean_finder(label))
            }
        }),
    );
    let d = deps(&mut host, agent);
    let run = host
        .call_export(&module, "buildReviewPipeline", vec![json!("code"), d])
        .unwrap();
    host.call(&run, vec![json!({ "target": "t" })]).unwrap();
    let calls = calls.borrow();
    let last_find = calls
        .iter()
        .filter(|c| c.label.starts_with("find:"))
        .map(|c| c.seq)
        .max()
        .unwrap();
    let refutes: Vec<usize> = calls
        .iter()
        .filter(|c| c.label.starts_with("refute:"))
        .map(|c| c.seq)
        .collect();
    assert_eq!(refutes.len(), 2, "{calls:?}");
    assert!(
        refutes.iter().all(|&s| s > last_find),
        "every finder must arrive before any refuter: {calls:?}"
    );
}

#[test]
fn rust_rejections_and_js_throws_propagate() {
    let mut host = Host::start_default().expect("start host");
    let module = host.import(&review_lib()).expect("import");

    // A synchronous JavaScript throw comes back as a JsError with its message.
    let err = host
        .call_export(
            &module,
            "buildReviewPipeline",
            vec![json!("bogus"), json!({})],
        )
        .unwrap_err();
    let js = err.as_js().expect("a JavaScript error");
    assert!(js.message.contains("unknown review mode: bogus"), "{js:?}");
    assert!(!js.stack.is_empty(), "the stack is carried");

    // A Rust-side rejection reaches JavaScript as a thrown agent() call: every
    // finder rejects, none is retried (a throw is not a null), and the review
    // core's wholesale-failure guard rejects the run.
    let (agent, calls) = fake_agent(&mut host, Box::new(|_| Err("rust-side boom".to_owned())));
    let d = deps(&mut host, agent);
    let run = host
        .call_export(&module, "buildReviewPipeline", vec![json!("plan"), d])
        .unwrap();
    let err = host.call(&run, vec![json!({ "target": "t" })]).unwrap_err();
    let js = err.as_js().expect("a JavaScript rejection");
    assert!(
        js.message.contains("every plan dimension finder failed"),
        "{js:?}"
    );
    assert!(
        calls.borrow().iter().all(|c| !c.label.ends_with(":retry")),
        "a rejected finder is not retried"
    );
    // The host stays usable after a JavaScript error.
    let floor = host.export(&module, "CONFIDENCE_FLOOR").unwrap();
    assert!(floor.is_number());
    host.shutdown().unwrap();
}

#[test]
fn missing_runtime_is_an_actionable_spawn_error() {
    let missing = "/nonexistent/rdm-test/node";
    let err = Host::start(&HostConfig::new().runtime(missing)).unwrap_err();
    assert!(matches!(err, WorkflowError::Runtime(_)), "{err:?}");
    let text = err.to_string();
    assert!(text.contains(missing), "names the path: {text}");
    assert!(text.contains("mise install"), "names the fix: {text}");
    assert!(text.contains("RDM_TEST_NODE"), "names the override: {text}");
}

#[test]
fn invalid_module_reports_node_syntax_error_then_cleans_up() {
    let tree = MutantTree::copy(&repo_root(), &[".claude/workflows/lib/review.mjs"]).unwrap();
    let path = tree.root().join(".claude/workflows/lib/review.mjs");
    let text = std::fs::read_to_string(&path).unwrap();
    // Truncate just inside a function body: the module can no longer parse.
    let open = "function buildReviewPipeline(mode, deps) {";
    let cut = text.find(open).expect("anchor present") + open.len();
    std::fs::write(&path, &text[..cut]).unwrap();

    let mut host = Host::start_default().expect("start host");
    let dir = host.temp_dir().to_owned();
    let pid = host.pid();
    let err = host.import(&path).unwrap_err();
    let js = err
        .as_js()
        .unwrap_or_else(|| panic!("a JavaScript error: {err}"));
    assert_eq!(js.name, "SyntaxError", "{js:?}");
    assert_eq!(
        js.file,
        path.display().to_string(),
        "the error names the file"
    );
    assert!(err.to_string().contains("review.mjs"), "{err}");
    host.shutdown().expect("the host still shuts down cleanly");
    assert!(!dir.exists(), "temp dir removed");
    assert!(gone_within(pid, Duration::from_secs(3)), "runtime reaped");
}

#[test]
fn malformed_protocol_quotes_the_line_and_tears_down() {
    let config = HostConfig::new()
        .runtime(fixture())
        .runtime_args(["print", "garbage"]);
    let mut host = Host::start(&config).expect("start");
    let dir = host.temp_dir().to_owned();
    let pid = host.pid();
    let err = host.import(&review_lib()).unwrap_err();
    assert!(matches!(err, WorkflowError::Protocol { .. }), "{err:?}");
    let text = err.to_string();
    assert!(text.contains("garbage"), "quotes the line: {text}");
    assert!(gone_within(pid, Duration::from_secs(3)), "child reaped");
    assert!(!dir.exists(), "temp dir removed on the failure path");
    assert!(matches!(
        host.import(&review_lib()),
        Err(WorkflowError::Dead)
    ));
}

#[test]
fn hung_child_times_out_naming_the_pending_request() {
    let config = HostConfig::new()
        .runtime(fixture())
        .runtime_args(["sleep"])
        .timeout(Duration::from_millis(800));
    let mut host = Host::start(&config).expect("start");
    let dir = host.temp_dir().to_owned();
    let pid = host.pid();
    let err = host.import(&review_lib()).unwrap_err();
    match &err {
        WorkflowError::Session {
            pending,
            source: SessionError::TimedOut { .. },
        } => assert!(pending.contains("import of"), "{pending}"),
        other => panic!("expected a session timeout, got {other:?}"),
    }
    let text = err.to_string();
    assert!(text.contains("request #1"), "names the request: {text}");
    assert!(gone_within(pid, Duration::from_secs(3)), "group killed");
    assert!(!dir.exists(), "temp dir removed on the failure path");
}

#[test]
fn over_long_line_kills_the_session() {
    let spec = ProcessSpec::new(fixture())
        .arg("flood")
        .timeout(Duration::from_secs(10));
    let mut session = Session::spawn_with_line_cap(&spec, 1024).expect("spawn");
    let pid = session.pid();
    let err = session.recv_line().unwrap_err();
    assert!(
        matches!(err, SessionError::LineTooLong { cap: 1024, .. }),
        "{err:?}"
    );
    assert!(session.is_closed());
    assert!(gone_within(pid, Duration::from_secs(3)));
}

#[test]
fn concurrent_hosts_are_isolated() {
    let handles: Vec<_> = (0..4)
        .map(|n| {
            std::thread::spawn(move || {
                let mut host = Host::start_default().expect("start host");
                let module = host.import(&review_lib()).expect("import");
                let own = format!("finding-{n}");
                let planted = own.clone();
                let (agent, calls) = fake_agent(
                    &mut host,
                    Box::new(move |label| {
                        if label == "find:code:correctness" {
                            Ok(json!({ "findings": [{
                                "id": planted, "severity": "blocking",
                                "confidence": 90, "what_fails": "x"
                            }]}))
                        } else if label.starts_with("refute:") {
                            Ok(json!({ "refuted": false, "confidence": 90 }))
                        } else {
                            Ok(clean_finder(label))
                        }
                    }),
                );
                let d = deps(&mut host, agent);
                let run = host
                    .call_export(&module, "buildReviewPipeline", vec![json!("code"), d])
                    .unwrap();
                let out = host.call(&run, vec![json!({ "target": own })]).unwrap();
                let ids: Vec<String> = out["survivors"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|f| f["id"].as_str().unwrap().to_owned())
                    .collect();
                assert_eq!(ids, vec![own.clone()], "host {n} sees only its own finding");
                let refutes: Vec<String> = calls
                    .borrow()
                    .iter()
                    .filter(|c| c.label.starts_with("refute:"))
                    .map(|c| c.label.clone())
                    .collect();
                assert_eq!(refutes, vec![format!("refute:code:{own}")]);
                assert!(member(&out, "survivors").is_some());
                host.shutdown().unwrap();
            })
        })
        .collect();
    for h in handles {
        h.join().expect("isolated host thread");
    }
}

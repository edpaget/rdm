//! `createRun` from `scripts/lib/codex-runtime-state.mjs`: explicit identity,
//! an owned session, durable private evidence, and direct bounded `rdm`
//! commands against [`FAKE_RDM`](crate::support::FAKE_RDM). Ported from
//! `scripts/lib/codex-runtime-state.test.mjs`.
//!
//! Each test builds two git repos (`source`, `plan`) under its own root. The
//! fake finds its recording directory through `CODEX_FIXTURE_DIR`, which the
//! runtime passes on because it is neither `RDM_*` nor `GIT_*`. Context
//! methods are called through their `$fn` handles: they are closures over the
//! run and never read `this`.

use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rdm_devtools::workflow::{Host, WorkflowError, resolve_node};
use serde_json::{Value, json};

use crate::support::{
    Call, FAKE_RDM, Reaper, Root, calls, gone_within, host, journal, js_err, js_ok, manifest,
    read_pid, records, until, write_exec,
};

const STATE: &str = "scripts/lib/codex-runtime-state.mjs";

/// How long a cancelled or reaped command's delayed write (1 s in the fake)
/// is given to appear before its absence counts: the delay plus a 0.5 s
/// margin.
const LATE_EFFECT_WAIT: Duration = Duration::from_millis(1500);

struct State {
    host: Host,
    module: Value,
    _reaper: Reaper,
    source: PathBuf,
    plan: PathBuf,
    fixture: PathBuf,
    run_dir: PathBuf,
    bin: PathBuf,
    root: Root,
}

impl State {
    fn new() -> Self {
        Self::with_env(|_| Vec::new())
    }

    /// A fixture whose Node host also carries `extra(plan_repo)` in its
    /// environment.
    fn with_env(extra: impl FnOnce(&Path) -> Vec<(&'static str, OsString)>) -> Self {
        let root = Root::new();
        let source = root.seeded_repo("source");
        let plan = root.seeded_repo("plan");
        let fixture = root.mkdir("fixture");
        fs::write(fixture.join("reply"), "{}\n").unwrap();
        let bin = root.join("fake-rdm");
        write_exec(&bin, FAKE_RDM);
        let reaper = Reaper::new(&[&fixture]);
        let mut env = vec![("CODEX_FIXTURE_DIR", OsString::from(&fixture))];
        env.extend(extra(&plan));
        let mut host = host(&root, &env);
        let module = js_ok(host.import(&crate::support::repo().join(STATE)), "import");
        Self {
            host,
            module,
            _reaper: reaper,
            run_dir: root.join("run"),
            source,
            plan,
            fixture,
            bin,
            root,
        }
    }

    fn spec(&self) -> Value {
        json!({
            "sourceDir": self.source,
            "planRoot": self.plan,
            "rdmBin": self.bin,
            "project": "fixture",
            "session": "caller-existing-session",
            "runDir": self.run_dir,
            "operation": "estimate",
        })
    }

    fn reply(&self, text: &str) {
        fs::write(self.fixture.join("reply"), text).unwrap();
    }

    fn create(&mut self, spec: Value) -> Result<Value, WorkflowError> {
        let f = js_ok(self.host.export(&self.module, "createRun"), "createRun");
        self.host.call(&f, vec![spec])
    }

    fn open(&mut self) -> Value {
        let spec = self.spec();
        js_ok(self.create(spec), "createRun")
    }

    /// Calls context method `name`.
    fn method(
        &mut self,
        ctx: &Value,
        name: &str,
        args: Vec<Value>,
    ) -> Result<Value, WorkflowError> {
        self.host.call(&ctx[name], args)
    }

    fn rdm(&mut self, ctx: &Value, args: &[&str], opts: Value) -> Result<Value, WorkflowError> {
        self.method(ctx, "rdm", vec![json!(args), opts])
    }

    fn fail(&mut self, ctx: &Value, error: Value) {
        js_ok(self.method(ctx, "fail", vec![error]), "fail");
    }

    fn calls(&self) -> Vec<Call> {
        calls(&self.fixture)
    }

    fn journal(&self) -> Vec<Value> {
        journal(&self.run_dir)
    }

    fn manifest(&self) -> Value {
        manifest(&self.run_dir)
    }

    /// The first recorded call whose first argument is `first`, once its
    /// recording is complete.
    fn wait_for_call(&self, first: &str) -> Call {
        let mut found = None;
        assert!(
            until(Duration::from_secs(10), || {
                found = self
                    .calls()
                    .into_iter()
                    .find(|c| c.argv.first().map(String::as_str) == Some(first));
                found.is_some()
            }),
            "the fake never recorded a `{first}` call"
        );
        found.unwrap()
    }
}

fn contains_any(message: &str, words: &[&str]) -> bool {
    let lower = message.to_lowercase();
    words.iter().any(|w| lower.contains(w))
}

#[test]
fn create_run_binds_explicit_identity_and_direct_argv() {
    let mut s = State::new();
    s.reply("{\"answer\":42}\n");
    let ctx = s.open();
    let session = ctx["session"].as_str().unwrap().to_owned();
    assert_ne!(
        session, "caller-existing-session",
        "the run owns its session"
    );
    let out = js_ok(
        s.rdm(
            &ctx,
            &["phase", "show", "literal;$(touch nope)"],
            json!({"json": true}),
        ),
        "rdm",
    );
    assert_eq!(out, json!({"answer": 42}), "stdout parsed as JSON");
    let call = s.wait_for_call("phase");
    assert_eq!(call.argv, ["phase", "show", "literal;$(touch nope)"]);
    assert_eq!(call.env.get("RDM_SESSION"), Some(&session));
    assert_eq!(call.cwd, s.source.to_str().unwrap());
    assert_eq!(
        call.env.get("RDM_ROOT").map(String::as_str),
        s.plan.to_str()
    );
    assert_eq!(
        call.env.get("RDM_PROJECT").map(String::as_str),
        Some("fixture")
    );
    for dir in [&s.source, &s.plan, &s.root.path] {
        assert!(!dir.join("nope").exists(), "argv went through a shell");
    }
    let mode = fs::metadata(&s.run_dir).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o700, "private evidence directory");
    js_ok(
        s.method(
            &ctx,
            "record",
            vec![json!("agent-call"), json!({"threadId":"thread-one"})],
        ),
        "record",
    );
    js_ok(
        s.method(&ctx, "finish", vec![json!({"approved": true})]),
        "finish",
    );
    let manifest = s.manifest();
    assert_eq!(manifest["status"], "completed");
    assert_eq!(
        manifest["identity"]["parentSession"],
        "caller-existing-session"
    );
    let spec = s.spec();
    let e = js_err(s.create(spec), "reusing the evidence directory");
    assert!(contains_any(&e.message, &["exist"]), "{e}");
    let e = js_err(s.rdm(&ctx, &["show"], json!({})), "rdm after finish");
    assert!(contains_any(&e.message, &["finished", "closed"]), "{e}");
}

#[test]
fn create_run_rejects_implicit_identity_nested_checkout_and_symlinked_evidence() {
    let mut s = State::new();
    for (key, value) in [
        ("session", json!("")),
        ("project", json!("")),
        ("sourceDir", json!("relative")),
        ("rdmBin", json!("rdm")),
    ] {
        let mut spec = s.spec();
        spec[key] = value.clone();
        js_err(s.create(spec), &format!("{key} = {value}"));
    }
    let nested = s.source.join("nested");
    fs::create_dir(&nested).unwrap();
    let mut spec = s.spec();
    spec["sourceDir"] = json!(nested);
    let e = js_err(s.create(spec), "a nested source directory");
    assert!(contains_any(&e.message, &["root"]), "{e}");
    let alias = s.root.join("alias");
    std::os::unix::fs::symlink(&s.source, &alias).unwrap();
    let mut spec = s.spec();
    spec["runDir"] = json!(alias.join("run"));
    let e = js_err(s.create(spec), "a symlinked evidence parent");
    assert!(contains_any(&e.message, &["symlink", "canonical"]), "{e}");
    assert!(!s.run_dir.exists() && !alias.join("run").exists());
    assert!(s.calls().is_empty(), "no command ran");
}

#[test]
fn json_decode_failure_poisons_later_mutations() {
    let mut s = State::new();
    s.reply("not JSON\n");
    let ctx = s.open();
    js_err(
        s.rdm(&ctx, &["bad-json"], json!({"json": true, "mutating": true})),
        "an unparseable acknowledgement",
    );
    let e = js_err(s.method(&ctx, "finish", vec![json!({})]), "finish");
    assert!(contains_any(&e.message, &["uncertain"]), "{e}");
    let e = js_err(
        s.rdm(&ctx, &["update"], json!({"mutating": true})),
        "a later mutation",
    );
    assert!(contains_any(&e.message, &["uncertain"]), "{e}");
    assert_eq!(s.calls().len(), 1, "the later mutation never ran");
    s.fail(&ctx, json!({"message": "bad JSON"}));
    assert_eq!(s.manifest()["uncertainWrites"], true);
}

#[test]
fn acknowledged_write_records_plan_head_and_finishes() {
    let mut s = State::new();
    let ctx = s.open();
    js_ok(
        s.rdm(&ctx, &["update"], json!({"json": true, "mutating": true})),
        "rdm update",
    );
    js_ok(
        s.method(&ctx, "finish", vec![json!({"applied": true})]),
        "finish",
    );
    let head = s.root.git(&s.plan, &["rev-parse", "HEAD"]);
    let journal = s.journal();
    let acks = records(&journal, "write-acknowledged");
    assert_eq!(acks.len(), 1, "{journal:?}");
    assert_eq!(acks[0]["data"]["planHead"], head.as_str());
    assert_eq!(s.manifest()["status"], "completed");
}

#[test]
fn changeset_override_and_unjournaled_commit_fail_before_execution() {
    let mut s = State::new();
    let ctx = s.open();
    for args in [&["commit", "--changeset=someone-else"][..], &["commit"][..]] {
        let e = js_err(s.rdm(&ctx, args, json!({})), &format!("rdm {args:?}"));
        assert!(contains_any(&e.message, &["override", "mutating"]), "{e}");
    }
    assert!(s.calls().is_empty(), "nothing was executed");
    s.fail(&ctx, json!({"message": "test complete"}));
}

#[test]
fn mutation_timeout_is_uncertain_and_forbids_retry() {
    let mut s = State::new();
    let mut spec = s.spec();
    spec["rdmTimeoutMs"] = json!(100);
    let ctx = js_ok(s.create(spec), "createRun");
    let e = js_err(
        s.rdm(&ctx, &["sleep"], json!({"mutating": true})),
        "a hung mutation",
    );
    assert!(contains_any(&e.message, &["timed out", "etimedout"]), "{e}");
    // The pid the runtime journaled at spawn (the fake may be killed before
    // it records anything itself).
    let journal = s.journal();
    let started = records(&journal, "rdm-started");
    assert_eq!(started.len(), 1, "{journal:?}");
    let pid = started[0]["data"]["pid"].as_u64().expect("a spawned pid") as u32;
    assert!(
        gone_within(pid, Duration::from_secs(3)),
        "the timed-out command {pid} survived"
    );
    let e = js_err(
        s.rdm(&ctx, &["update"], json!({"mutating": true})),
        "a retry",
    );
    assert!(contains_any(&e.message, &["uncertain"]), "{e}");
    s.fail(&ctx, json!({"message": "timed out"}));
}

#[test]
fn inherited_git_dir_cannot_redirect_source_identity() {
    // GIT_DIR is set on the Node child only, pointing at the plan repo.
    let mut s = State::with_env(|plan| vec![("GIT_DIR", plan.join(".git").into_os_string())]);
    let ctx = s.open();
    assert_eq!(ctx["identity"]["sourceRoot"], json!(s.source));
    js_ok(s.method(&ctx, "finish", vec![json!({})]), "finish");
}

#[test]
fn adapter_readback_uncertainty_persists_in_failed_manifest() {
    let mut s = State::new();
    let ctx = s.open();
    js_ok(
        s.rdm(&ctx, &["update"], json!({"mutating": true})),
        "rdm update",
    );
    s.fail(
        &ctx,
        json!({"message": "readback mismatch", "uncertainWrites": true}),
    );
    let manifest = s.manifest();
    assert_eq!(manifest["status"], "failed");
    assert_eq!(manifest["uncertainWrites"], true);
    let journal = s.journal();
    let uncertain = records(&journal, "write-uncertain");
    assert_eq!(uncertain.len(), 1, "{journal:?}");
    assert_eq!(uncertain[0]["data"]["origin"], "adapter-readback");
}

#[test]
fn caller_rdm_overrides_do_not_reach_direct_commands() {
    let mut s = State::with_env(|_| vec![("RDM_CHANGESET", OsString::from("unrelated"))]);
    let ctx = s.open();
    js_ok(s.rdm(&ctx, &["show"], json!({"json": true})), "rdm show");
    let call = s.wait_for_call("show");
    let keys: Vec<&str> = call
        .env
        .keys()
        .map(String::as_str)
        .filter(|k| k.starts_with("RDM_"))
        .collect();
    assert_eq!(keys, ["RDM_BIN", "RDM_PROJECT", "RDM_ROOT", "RDM_SESSION"]);
    js_ok(s.method(&ctx, "finish", vec![json!({})]), "finish");
}

#[test]
fn manifest_records_runner_identity() {
    let mut s = State::new();
    let ctx = s.open();
    let runner = s.manifest()["runner"].clone();
    assert_eq!(runner["pid"], s.host.pid(), "{runner}");
    assert_eq!(runner["ppid"], std::process::id(), "{runner}");
    let canonical = |v: &Value| {
        PathBuf::from(v.as_str().expect("a path string"))
            .canonicalize()
            .expect("an existing path")
    };
    let node = resolve_node().unwrap().canonicalize().unwrap();
    assert_eq!(canonical(&runner["executable"]), node);
    let argv = runner["argv"].as_array().expect("argv array");
    assert_eq!(argv.len(), 2, "{runner}");
    assert_eq!(canonical(&argv[0]), node);
    assert_eq!(
        canonical(&argv[1]),
        s.host
            .temp_dir()
            .join("workflow_host.mjs")
            .canonicalize()
            .unwrap()
    );
    assert!(runner["hostname"].is_string(), "{runner}");
    let started = runner["approximateStartedAt"].as_str().expect("a string");
    // `Date.prototype.toISOString`: YYYY-MM-DDTHH:MM:SS.sssZ.
    let shape: String = started
        .chars()
        .map(|c| if c.is_ascii_digit() { 'd' } else { c })
        .collect();
    assert_eq!(shape, "dddd-dd-ddTdd:dd:dd.dddZ", "{started}");
    js_ok(s.method(&ctx, "finish", vec![json!({})]), "finish");
}

#[test]
fn cancellation_kills_mutation_descendants_and_blocks_completion() {
    let mut s = State::new();
    let controller = js_ok(s.host.construct("AbortController", Vec::new()), "new");
    let signal = js_ok(s.host.get_ref(&controller, "signal"), "signal");
    let mut spec = s.spec();
    spec["signal"] = signal;
    let ctx = js_ok(s.create(spec), "createRun");
    let pending = s
        .host
        .start_call(
            &ctx["rdm"],
            vec![json!(["descendant"]), json!({"mutating": true})],
        )
        .expect("start rdm");
    let call = s.wait_for_call("descendant");
    assert!(
        until(Duration::from_secs(10), || read_pid(
            &call.dir.join("child")
        )
        .is_some()),
        "the fake never started its descendant"
    );
    let child = read_pid(&call.dir.join("child")).unwrap();
    js_ok(s.host.invoke(&controller, "abort", Vec::new()), "abort");
    let e = js_err(s.host.await_call(pending), "a cancelled mutation");
    assert!(contains_any(&e.message, &["cancel", "abort"]), "{e}");
    assert!(
        gone_within(child, Duration::from_secs(3)),
        "descendant survived"
    );
    std::thread::sleep(LATE_EFFECT_WAIT);
    assert!(!s.plan.join("late-effect").exists(), "a late write landed");
    let e = js_err(
        s.rdm(&ctx, &["update"], json!({"mutating": true})),
        "a mutation after cancellation",
    );
    assert!(contains_any(&e.message, &["cancel", "abort"]), "{e}");
    let e = js_err(s.method(&ctx, "finish", vec![json!({})]), "finish");
    assert!(
        contains_any(&e.message, &["uncertain", "cancel", "abort"]),
        "{e}"
    );
    s.fail(&ctx, json!({"message": "cancelled"}));
    let manifest = s.manifest();
    assert_eq!(manifest["status"], "failed");
    assert_eq!(manifest["uncertainWrites"], true);
    assert_eq!(records(&s.journal(), "rdm-started").len(), 1);
}

#[test]
fn preaborted_run_starts_no_mutation_and_cannot_finish() {
    let mut s = State::new();
    let controller = js_ok(s.host.construct("AbortController", Vec::new()), "new");
    js_ok(s.host.invoke(&controller, "abort", Vec::new()), "abort");
    let signal = js_ok(s.host.get_ref(&controller, "signal"), "signal");
    let mut spec = s.spec();
    spec["signal"] = signal;
    let ctx = js_ok(s.create(spec), "createRun");
    let e = js_err(
        s.rdm(&ctx, &["fail"], json!({"mutating": true})),
        "a mutation in a cancelled run",
    );
    assert!(contains_any(&e.message, &["cancel", "abort"]), "{e}");
    assert!(!s.plan.join("effect").exists(), "the mutation ran");
    assert!(s.calls().is_empty(), "the fake was invoked");
    let e = js_err(s.method(&ctx, "finish", vec![json!({})]), "finish");
    assert!(contains_any(&e.message, &["cancel", "abort"]), "{e}");
    s.fail(&ctx, json!({"message": "cancelled"}));
    assert!(records(&s.journal(), "write-intent").is_empty());
}

#[test]
fn exited_command_group_is_reaped_before_return() {
    let mut s = State::new();
    let ctx = s.open();
    js_ok(s.rdm(&ctx, &["orphan"], json!({})), "rdm orphan");
    let call = s.wait_for_call("orphan");
    let child = read_pid(&call.dir.join("child")).expect("the descendant was recorded");
    assert!(
        gone_within(child, Duration::from_secs(1)),
        "the leftover group outlived the command"
    );
    std::thread::sleep(LATE_EFFECT_WAIT);
    assert!(!s.plan.join("late-effect").exists(), "a late write landed");
    js_ok(s.method(&ctx, "finish", vec![json!({})]), "finish");
}

#[test]
fn output_limit_fails_mutation_conservatively() {
    let mut s = State::new();
    let ctx = s.open();
    let e = js_err(
        s.rdm(&ctx, &["overflow"], json!({"mutating": true})),
        "an overflowing mutation",
    );
    assert!(e.message.contains("output exceeded"), "{e}");
    let e = js_err(s.method(&ctx, "finish", vec![json!({})]), "finish");
    assert!(contains_any(&e.message, &["uncertain"]), "{e}");
    s.fail(&ctx, json!({"message": "output limit"}));
    assert_eq!(s.manifest()["uncertainWrites"], true);
}

#[test]
fn active_command_blocks_finish_and_fail() {
    let mut s = State::new();
    let controller = js_ok(s.host.construct("AbortController", Vec::new()), "new");
    let signal = js_ok(s.host.get_ref(&controller, "signal"), "signal");
    let mut spec = s.spec();
    spec["signal"] = signal;
    let ctx = js_ok(s.create(spec), "createRun");
    let pending = s
        .host
        .start_call(&ctx["rdm"], vec![json!(["sleep"])])
        .expect("start rdm");
    s.wait_for_call("sleep");
    let e = js_err(s.method(&ctx, "finish", vec![json!({})]), "finish");
    assert!(e.message.contains("active"), "{e}");
    let e = js_err(
        s.method(&ctx, "fail", vec![json!({"message": "premature failure"})]),
        "fail",
    );
    assert!(e.message.contains("active"), "{e}");
    assert_eq!(s.manifest()["status"], "running");
    assert!(!s.host.is_settled(&pending));
    js_ok(s.host.invoke(&controller, "abort", Vec::new()), "abort");
    let e = js_err(s.host.await_call(pending), "the aborted command");
    assert!(e.message.contains("cancel"), "{e}");
    s.fail(&ctx, json!({"message": "cancelled"}));
    assert_eq!(s.manifest()["status"], "failed");
}

//! The whole Codex runtime — `runRuntime` in `scripts/lib/codex-runtime.mjs`
//! and the documented CLI `scripts/rdm-codex.mjs` — over a plan repo seeded
//! with the binary under test, with
//! [`FAKE_RUNNER_CODEX`](crate::support::FAKE_RUNNER_CODEX) first on the
//! runtime's `PATH`. Ported from `scripts/lib/codex-runtime-review-process.test.mjs`
//! (code and plan review) and `scripts/lib/codex-runtime-queue.test.mjs` (the
//! estimate queue).
//!
//! Each scenario is split into an observation (`observe_*`: run the runtime
//! and collect its evidence; only an infrastructure problem fails it) and
//! checks over that evidence, so [`crate::mutants`] can run the same
//! observation against a planted edit and require the same check to fail.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rdm_devtools::process::{ProcessSpec, RunError, run_bounded};
use rdm_devtools::workflow::{Host, JsError, resolve_node};
use serde_json::{Value, json};

use crate::plan_fixture::{Sandbox, rdm_bin};
use crate::support::{
    Call, Event, FAKE_RUNNER_CODEX, HOST_TIMEOUT, Reaper, Root, alive, calls, events, host_config,
    journal, manifest, peak, records, signal, until, write_exec,
};
use crate::workflow_support::{Failure, Lib, Outcome, infra, load, split};

/// The runtime module, relative to a source root.
pub const RUNTIME: &str = "scripts/lib/codex-runtime.mjs";
/// The documented CLI, relative to a source root.
pub const CLI: &str = "scripts/rdm-codex.mjs";

/// The finding every review scenario plants on its first dimension.
const PLANTED: &str = "planted-subtraction";

/// A committed plan repo and source checkout under one root, with the fake
/// `codex` and its responses.
pub struct Fixture {
    pub root: Root,
    pub source: PathBuf,
    pub plans: PathBuf,
    pub fixture: PathBuf,
    pub bin: PathBuf,
    pub run_dir: PathBuf,
    pub spec: Value,
    pub source_head: String,
    pub plan_head: String,
    /// `rdm model resolve <step> --host codex --format json`, by step.
    pub expected: BTreeMap<String, Value>,
    reaper: Option<Reaper>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Kill every fake group before the root is removed.
        self.reaper.take();
    }
}

impl Fixture {
    fn rdm(root: &Root, plans: &Path, args: &[&str]) -> Result<String, Failure> {
        let out = root
            .sandbox
            .rdm()
            .env("RDM_ROOT", plans)
            .env("RDM_SESSION", "seed")
            .args(args)
            .current_dir(&root.path)
            .output()
            .map_err(infra)?;
        if !out.status.success() {
            return Err(Failure::Infra(format!(
                "rdm {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            )));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    fn git(root: &Root, dir: &Path, args: &[&str]) -> Result<String, Failure> {
        Ok(root.sandbox.git(dir, args)?.trim().to_owned())
    }

    /// The shared skeleton: a source repo (built by `source`), a plan repo
    /// seeded by `seed`, optional `rdm.toml` text committed after it, and the
    /// fake `codex`.
    fn build(
        source: impl FnOnce(&Root, &Path) -> Result<(), Failure>,
        seed: &[&[&str]],
        profile: Option<&str>,
    ) -> Result<Self, Failure> {
        let root = Root::new();
        let src = root.mkdir("source");
        source(&root, &src)?;
        let plans = root.mkdir("plans");
        Self::rdm(&root, &plans, &["init", "--default-project", "fixture"])?;
        for args in seed {
            Self::rdm(&root, &plans, args)?;
        }
        Self::rdm(&root, &plans, &["commit", "-m", "test: seed"])?;
        if let Some(text) = profile {
            let path = plans.join("rdm.toml");
            let mut toml = fs::read_to_string(&path).map_err(infra)?;
            toml.push_str(text);
            fs::write(&path, toml).map_err(infra)?;
            Self::git(&root, &plans, &["commit", "-qam", "test: codex profile"])?;
        }
        let mut expected = BTreeMap::new();
        for step in ["review-find", "review-verify", "review-consolidate", "plan"] {
            let text = Self::rdm(
                &root,
                &plans,
                &[
                    "model", "resolve", step, "--host", "codex", "--format", "json",
                ],
            )?;
            let v: Value = serde_json::from_str(&text).map_err(infra)?;
            expected.insert(step.to_owned(), v);
        }
        let bin = root.mkdir("bin");
        write_exec(&bin.join("codex"), FAKE_RUNNER_CODEX);
        let fixture = root.mkdir("fixture");
        fs::create_dir_all(fixture.join("responses")).map_err(infra)?;
        let stream = format!(
            "{}\n{}\n",
            json!({"type":"thread.started","thread_id":"fixture-thread"}),
            json!({"type":"turn.completed","usage":{"input_tokens":1,"output_tokens":1}})
        );
        fs::write(fixture.join("stream"), stream).map_err(infra)?;
        let reaper = Reaper::new(&[&fixture]);
        Ok(Self {
            source_head: Self::git(&root, &src, &["rev-parse", "HEAD"])?,
            plan_head: Self::git(&root, &plans, &["rev-parse", "HEAD"])?,
            run_dir: root.join("run"),
            source: src,
            plans,
            fixture,
            bin,
            spec: Value::Null,
            expected,
            reaper: Some(reaper),
            root,
        })
    }

    fn respond(&self, key: &str, value: &str) -> Result<(), Failure> {
        fs::write(self.fixture.join("responses").join(key), value).map_err(infra)
    }

    fn set(&self, name: &str, value: &str) -> Result<(), Failure> {
        fs::write(self.fixture.join(name), value).map_err(infra)
    }

    /// A code- or plan-review fixture: `sum.mjs` changed from addition to
    /// subtraction, a planted blocking finding on `correctness` (code) and
    /// `coherence` (plan), a lower-confidence DUPLICATE of it on `tests`
    /// (code) and `architectural-fit` (plan), a consolidator that merges the
    /// two (pipeline ids `c0`/`c1`: each operation's first selected finding
    /// dimension yields the planted finding, the next the duplicate), a
    /// refuter that upholds the merged unit (or answers malformed JSON), and
    /// an optional `rdm.toml` profile override.
    pub fn review(
        operation: &str,
        malformed_refuter: bool,
        profile: Option<&str>,
    ) -> Result<Self, Failure> {
        let mut base = String::new();
        let mut fx = Self::build(
            |root, src| {
                Self::git(root, src, &["init", "-q", "-b", "main"])?;
                fs::write(src.join("sum.mjs"), "export const add = (a, b) => a + b;\n")
                    .map_err(infra)?;
                Self::git(root, src, &["add", "."])?;
                Self::git(root, src, &["commit", "-qm", "base"])?;
                base = Self::git(root, src, &["rev-parse", "HEAD"])?;
                fs::write(src.join("sum.mjs"), "export const add = (a, b) => a - b;\n")
                    .map_err(infra)?;
                Self::git(root, src, &["add", "."])?;
                Self::git(root, src, &["commit", "-qm", "head"])?;
                Ok(())
            },
            &[&[
                "roadmap",
                "create",
                "example",
                "--title",
                "Example",
                "--body",
                "Review fixture.",
                "--no-edit",
                "--project",
                "fixture",
            ]],
            profile,
        )?;
        let plan_file = fx.root.join("implementation-plan.md");
        fs::write(
            &plan_file,
            "Implement add(a,b) using subtraction. Acceptance criterion: add(2,3) returns 5. Add a test and run node --test.",
        )
        .map_err(infra)?;
        for dimension in ["correctness", "coherence"] {
            fx.respond(
                &format!("finder-{dimension}"),
                &json!({"findings":[{"id":PLANTED,"concern":dimension,"severity":"blocking","confidence":95,
                    "what_fails":"Subtraction cannot satisfy addition acceptance criteria.","location":"sum.mjs:1"}]})
                .to_string(),
            )?;
        }
        for dimension in ["tests", "architectural-fit"] {
            fx.respond(
                &format!("finder-{dimension}"),
                &json!({"findings":[{"id":format!("{PLANTED}-dup"),"concern":dimension,"severity":"blocking","confidence":90,
                    "what_fails":"add() subtracts instead of adding.","location":"sum.mjs:1"}]})
                .to_string(),
            )?;
        }
        fx.respond(
            "consolidator",
            &json!({"clusters":[{"ids":["c0","c1"],"why":"both report the subtraction in add()"}]})
                .to_string(),
        )?;
        fx.respond("finder", &json!({"findings":[]}).to_string())?;
        fx.respond(
            "ac",
            &json!({"ac":[{"criterion":"add(2,3) returns 5","status":"FAIL","evidence":"sum.mjs:1 subtracts"}]})
                .to_string(),
        )?;
        fx.respond(
            "refuter",
            if malformed_refuter {
                "{malformed-refuter"
            } else {
                r#"{"refuted":false,"confidence":95}"#
            },
        )?;
        fx.set("delay-finder", "0.15")?;
        fx.set("delay-refuter", "0.01")?;
        let mut spec = json!({
            "operation": operation, "sourceDir": fx.source, "planRoot": fx.plans,
            "rdmBin": rdm_bin(), "project": "fixture", "session": "parent",
            "runDir": fx.run_dir, "concurrency": 2,
            "target": "Acceptance criteria: add(2,3) returns 5.", "planFile": plan_file,
        });
        if operation == "code-review" {
            spec["base"] = json!(base);
            spec["head"] = json!(fx.source_head);
        }
        fx.spec = spec;
        Ok(fx)
    }

    /// The estimate-queue fixture: roadmap `example` with five unestimated
    /// phases; each judgment spawns a descendant and answers after 0.75 s.
    pub fn queue() -> Result<Self, Failure> {
        let phases: Vec<Vec<String>> = (1..=5)
            .map(|n| {
                [
                    "phase",
                    "create",
                    &format!("target-{n}"),
                    "--number",
                    &n.to_string(),
                    "--title",
                    &format!("Target {n}"),
                    "--body",
                    "Unset estimate.",
                    "--no-edit",
                    "--roadmap",
                    "example",
                    "--project",
                    "fixture",
                ]
                .map(str::to_owned)
                .to_vec()
            })
            .collect();
        let mut seed: Vec<Vec<&str>> = vec![vec![
            "roadmap",
            "create",
            "example",
            "--title",
            "Example",
            "--body",
            "Queue fixture",
            "--no-edit",
            "--project",
            "fixture",
        ]];
        seed.extend(
            phases
                .iter()
                .map(|p| p.iter().map(String::as_str).collect()),
        );
        let seed: Vec<&[&str]> = seed.iter().map(Vec::as_slice).collect();
        let mut fx = Self::build(
            |root, src| {
                Self::git(root, src, &["init", "-q", "-b", "main"])?;
                fs::write(src.join("README"), "fixture").map_err(infra)?;
                Self::git(root, src, &["add", "."])?;
                Self::git(root, src, &["commit", "-qm", "seed"])?;
                Ok(())
            },
            &seed,
            None,
        )?;
        let list: Value = serde_json::from_str(&Self::rdm(
            &fx.root,
            &fx.plans,
            &[
                "phase",
                "list",
                "--roadmap",
                "example",
                "--project",
                "fixture",
                "--format",
                "json",
            ],
        )?)
        .map_err(infra)?;
        for phase in list.as_array().into_iter().flatten() {
            let stem = phase["stem"]
                .as_str()
                .ok_or_else(|| infra("phase without a stem"))?;
            fx.respond(
                &format!("estimate-{stem}"),
                &json!({"stem":stem,"difficulty":"moderate","justification":"Bounded fixture work."})
                    .to_string(),
            )?;
        }
        fx.set("delay-estimator", "0.75")?;
        fx.set("spawn-descendant", "")?;
        fx.spec = json!({
            "operation": "estimate", "sourceDir": fx.source, "planRoot": fx.plans,
            "rdmBin": rdm_bin(), "project": "fixture", "session": "parent",
            "roadmap": "example", "apply": false, "concurrency": 2,
            "runDir": fx.run_dir, "rdmTimeoutMs": 120000,
        });
        Ok(fx)
    }

    /// The runtime's `PATH`: the fake `codex` first, then the inherited one.
    fn path(&self) -> OsString {
        let mut path = OsString::from(&self.bin);
        if let Some(inherited) = std::env::var_os("PATH") {
            path.push(":");
            path.push(inherited);
        }
        path
    }

    /// The Node host the runtime runs in.
    pub fn host(&self, extra: &[(&str, OsString)]) -> Result<Host, Failure> {
        let mut env = vec![
            ("PATH", self.path()),
            ("CODEX_FIXTURE_DIR", OsString::from(&self.fixture)),
        ];
        env.extend(extra.iter().cloned());
        Host::start(&host_config(&self.root, &env)).map_err(infra)
    }

    /// Every recorded judgment call.
    pub fn calls(&self) -> Vec<Call> {
        calls(&self.fixture)
    }

    /// Every event line.
    pub fn events(&self) -> Vec<Event> {
        events(&self.fixture)
    }

    /// Neither repo moved or changed, and the run journaled no write.
    pub fn no_writes(&self) -> Outcome {
        let head = |dir: &Path| Self::git(&self.root, dir, &["rev-parse", "HEAD"]);
        let status = |dir: &Path| {
            Self::git(
                &self.root,
                dir,
                &["status", "--porcelain", "--untracked-files=all"],
            )
        };
        check_eq!(head(&self.source)?, self.source_head, "source HEAD moved");
        check_eq!(status(&self.source)?, "", "source checkout changed");
        check_eq!(head(&self.plans)?, self.plan_head, "plan HEAD moved");
        check_eq!(status(&self.plans)?, "", "plan checkout changed");
        let journal = journal(&self.run_dir);
        check!(
            records(&journal, "write-intent").is_empty(),
            "the run journaled a write: {journal:?}"
        );
        Ok(())
    }
}

/// Evidence of one `runRuntime` call made through a [`Host`].
pub struct RuntimeRun {
    /// Its result, or the JavaScript error it threw.
    pub result: Result<Value, JsError>,
    pub events: Vec<Event>,
    pub calls: Vec<Call>,
    pub journal: Vec<Value>,
    pub manifest: Value,
}

/// Runs `runRuntime(fx.spec)` from `lib`'s runtime module.
pub fn observe_runtime(lib: &Lib, fx: &Fixture) -> Result<RuntimeRun, Failure> {
    observe_runtime_with(lib, fx, &[])
}

/// [`observe_runtime`] with `extra` added to the Node host's environment.
pub fn observe_runtime_with(
    lib: &Lib,
    fx: &Fixture,
    extra: &[(&str, OsString)],
) -> Result<RuntimeRun, Failure> {
    let mut host = fx.host(extra)?;
    let module = load(host.import(&lib.path(RUNTIME)))?;
    let run = load(host.export(&module, "runRuntime"))?;
    let result = split(host.call(&run, vec![fx.spec.clone()]))?;
    drop(host);
    Ok(RuntimeRun {
        result,
        events: fx.events(),
        calls: fx.calls(),
        journal: journal(&fx.run_dir),
        manifest: manifest(&fx.run_dir),
    })
}

fn starts<'a>(events: &'a [Event], role: Option<&str>) -> Vec<&'a Event> {
    events
        .iter()
        .filter(|e| e.kind == "start" && role.is_none_or(|r| e.role == r))
        .collect()
}

fn call_of(run: &RuntimeRun, pid: u32) -> Result<&Call, Failure> {
    run.calls
        .iter()
        .find(|c| c.pid == pid)
        .ok_or_else(|| Failure::Infra(format!("no recorded call for pid {pid}")))
}

/// Each judgment ran at exactly the model and effort core resolves for its
/// role: finders at `review-find`, the refuter at `review-verify`, the
/// consolidator at `review-consolidate`.
pub fn check_models(fx: &Fixture, run: &RuntimeRun) -> Outcome {
    for start in starts(&run.events, None) {
        let call = call_of(run, start.pid)?;
        let step = match start.role.as_str() {
            "refuter" => "review-verify",
            "consolidator" => "review-consolidate",
            _ => "review-find",
        };
        let profile = &fx.expected[step];
        check_eq!(
            call.after("-m"),
            profile["model"].as_str(),
            "{} call {} ran on the wrong model for {step}",
            start.role,
            start.pid
        );
        let effort = format!(
            "model_reasoning_effort=\"{}\"",
            profile["effort"].as_str().unwrap_or_default()
        );
        check!(
            call.configs().contains(&effort.as_str()),
            "{} call {} lacks {effort} for {step}: {:?}",
            start.role,
            start.pid,
            call.argv
        );
    }
    Ok(())
}

/// Everything the full review runner guarantees for `operation`.
pub fn check_review(fx: &Fixture, run: &RuntimeRun, operation: &str) -> Outcome {
    let value = match &run.result {
        Ok(v) => v,
        Err(e) => return Err(Failure::Check(format!("runRuntime threw {e}\n{}", e.stack))),
    };
    let report = &value["result"];
    check_eq!(
        report["coverage"]["complete"],
        json!(true),
        "coverage: {report}"
    );
    check_eq!(
        report["budget"]["graded"],
        json!(1),
        "graded refutations: {report}"
    );
    let list = if operation == "code-review" {
        &report["survivors"]
    } else {
        &report["findings"]
    };
    check!(
        list.as_array()
            .is_some_and(|l| l.iter().any(|f| f["id"] == PLANTED)),
        "the planted finding did not survive: {report}"
    );
    if operation == "code-review" {
        check_eq!(report["outcome"], json!("rework"), "outcome: {report}");
    }
    let all = starts(&run.events, None);
    let finders = starts(&run.events, Some("finder"));
    let refuters = starts(&run.events, Some("refuter"));
    check!(finders.len() >= 3, "only {} finders ran", finders.len());
    // The planted finding and its duplicate were merged into one unit, so
    // the duplicate cost no refuter of its own.
    check_eq!(refuters.len(), 1, "exactly one independent refuter");
    check_eq!(
        starts(&run.events, Some("consolidator")).len(),
        1,
        "exactly one consolidator"
    );
    check_eq!(
        report["budget"]["collapsed"],
        json!(1),
        "the duplicate was merged: {report}"
    );
    let mut pids: Vec<u32> = all.iter().map(|e| e.pid).collect();
    pids.sort_unstable();
    pids.dedup();
    check_eq!(pids.len(), all.len(), "every judgment is a fresh process");
    let first_refuter = run
        .events
        .iter()
        .position(|e| e.kind == "start" && e.role == "refuter")
        .unwrap_or(run.events.len());
    let finder_ends = run.events[..first_refuter]
        .iter()
        .filter(|e| e.kind == "end" && e.role == "finder")
        .count();
    check_eq!(
        finder_ends,
        finders.len(),
        "every finder must finish before the first refuter starts: {:?}",
        run.events
    );
    check_models(fx, run)?;
    for start in &all {
        let call = call_of(run, start.pid)?;
        check_eq!(
            call.after("--sandbox"),
            Some("read-only"),
            "{:?}",
            call.argv
        );
        check!(call.has("--ephemeral"), "not ephemeral: {:?}", call.argv);
        check!(!call.has("resume"), "resumed a context: {:?}", call.argv);
        check!(
            call.schema
                .as_ref()
                .is_some_and(|s| s["properties"].is_object()),
            "no schema: {:?}",
            call.schema
        );
        check!(
            call.prompt.len() > 100,
            "a trivial prompt: {:?}",
            call.prompt
        );
    }
    let resolved = records(&run.journal, "model-resolved");
    let tier = |step: &str| {
        resolved
            .iter()
            .find(|e| e["data"]["step"] == step)
            .map(|e| e["data"]["core"]["tier"].clone())
    };
    check_eq!(tier("review-find"), Some(json!("medium")), "finder tier");
    check_eq!(tier("review-verify"), Some(json!("large")), "refuter tier");
    check_eq!(
        tier("review-consolidate"),
        Some(json!("large")),
        "consolidator tier"
    );
    check_eq!(
        records(&run.journal, "agent-completed").len(),
        all.len(),
        "one agent-completed record per judgment"
    );
    check_eq!(run.manifest["status"], json!("completed"), "manifest");
    fx.no_writes()
}

fn review_runner_maps_models_and_keeps_the_finder_barrier(operation: &str) {
    let fx = Fixture::review(operation, false, None).unwrap_or_else(|f| panic!("{f}"));
    let run = observe_runtime(&Lib::real(), &fx).unwrap_or_else(|f| panic!("{f}"));
    check_review(&fx, &run, operation).unwrap_or_else(|f| panic!("{f}"));
}

fn review_runner_takes_finder_effort_from_the_codex_profile(operation: &str) {
    let fx = Fixture::review(
        operation,
        false,
        Some("\n[models.profiles.codex.medium]\neffort = \"low\"\n"),
    )
    .unwrap_or_else(|f| panic!("{f}"));
    assert_eq!(
        fx.expected["review-find"]["effort"], "low",
        "the profile override reached core resolution"
    );
    let run = observe_runtime(&Lib::real(), &fx).unwrap_or_else(|f| panic!("{f}"));
    if let Err(e) = &run.result {
        panic!("runRuntime threw {e}");
    }
    let finders = starts(&run.events, Some("finder"));
    assert!(!finders.is_empty());
    for start in finders {
        let call = call_of(&run, start.pid).unwrap_or_else(|f| panic!("{f}"));
        assert!(
            call.configs().contains(&"model_reasoning_effort=\"low\""),
            "{:?}",
            call.argv
        );
    }
    fx.no_writes().unwrap_or_else(|f| panic!("{f}"));
}

fn review_runner_rejects_malformed_refuter_json(operation: &str) {
    let fx = Fixture::review(operation, true, None).unwrap_or_else(|f| panic!("{f}"));
    let run = observe_runtime(&Lib::real(), &fx).unwrap_or_else(|f| panic!("{f}"));
    let e = match &run.result {
        Ok(v) => panic!("a malformed refutation was accepted: {v}"),
        Err(e) => e,
    };
    let message = e.message.to_lowercase();
    assert!(
        message.contains("incomplete") || message.contains("malformed"),
        "{e}"
    );
    assert_eq!(starts(&run.events, Some("refuter")).len(), 1);
    assert!(
        !records(&run.journal, "agent-failed").is_empty(),
        "{:?}",
        run.journal
    );
    assert!(records(&run.journal, "run-completed").is_empty());
    assert_eq!(run.manifest["status"], "failed");
    fx.no_writes().unwrap_or_else(|f| panic!("{f}"));
}

#[test]
fn code_review_runner_maps_models_and_keeps_the_finder_barrier() {
    review_runner_maps_models_and_keeps_the_finder_barrier("code-review");
}

#[test]
fn plan_review_runner_maps_models_and_keeps_the_finder_barrier() {
    review_runner_maps_models_and_keeps_the_finder_barrier("plan-review");
}

#[test]
fn code_review_runner_takes_finder_effort_from_the_codex_profile() {
    review_runner_takes_finder_effort_from_the_codex_profile("code-review");
}

#[test]
fn plan_review_runner_takes_finder_effort_from_the_codex_profile() {
    review_runner_takes_finder_effort_from_the_codex_profile("plan-review");
}

#[test]
fn code_review_runner_rejects_malformed_refuter_json() {
    review_runner_rejects_malformed_refuter_json("code-review");
}

#[test]
fn plan_review_runner_rejects_malformed_refuter_json() {
    review_runner_rejects_malformed_refuter_json("plan-review");
}

// -- the refutation budget: resolved from config / RDM_MAX_REFUTATIONS --

/// The run's single `refutation-budget` journal record.
fn budget_record(run: &RuntimeRun) -> Value {
    let found = records(&run.journal, "refutation-budget");
    assert_eq!(found.len(), 1, "{:?}", run.journal);
    found[0]["data"].clone()
}

/// How many `rdm config get max_refutations --raw` reads the run started.
fn budget_config_reads(run: &RuntimeRun) -> usize {
    records(&run.journal, "read-started")
        .into_iter()
        .filter(|r| r["data"]["args"] == json!(["config", "get", "max_refutations", "--raw"]))
        .count()
}

/// A budget of `0` grades nothing: the finders and the consolidator run, no
/// refuter does, and the ungraded blocking unit leaves the evidence
/// incomplete — which the runtime refuses as "Review incomplete".
fn assert_zero_budget_refused(fx: &Fixture, run: &RuntimeRun) {
    match &run.result {
        Ok(v) => panic!("a zero budget produced a complete review: {v}"),
        Err(e) => assert!(e.message.contains("Review incomplete"), "{e}"),
    }
    assert!(!starts(&run.events, Some("finder")).is_empty());
    assert_eq!(starts(&run.events, Some("consolidator")).len(), 1);
    assert_eq!(
        starts(&run.events, Some("refuter")).len(),
        0,
        "{:?}",
        run.events
    );
    assert_eq!(run.manifest["status"], "failed");
    fx.no_writes().unwrap_or_else(|f| panic!("{f}"));
}

fn zero_budget_from_config_dispatches_no_refuter(operation: &str) {
    let fx = Fixture::review(operation, false, Some("\nmax_refutations = 0\n"))
        .unwrap_or_else(|f| panic!("{f}"));
    let run = observe_runtime(&Lib::real(), &fx).unwrap_or_else(|f| panic!("{f}"));
    assert_eq!(
        budget_record(&run),
        json!({"layer": "config-get", "envForwarded": false, "value": "0"})
    );
    assert_eq!(budget_config_reads(&run), 1, "{:?}", run.journal);
    assert_zero_budget_refused(&fx, &run);
}

fn zero_budget_from_env_dispatches_no_refuter(operation: &str) {
    let fx = Fixture::review(operation, false, None).unwrap_or_else(|f| panic!("{f}"));
    let run = observe_runtime_with(
        &Lib::real(),
        &fx,
        &[("RDM_MAX_REFUTATIONS", OsString::from("0"))],
    )
    .unwrap_or_else(|f| panic!("{f}"));
    assert_eq!(
        budget_record(&run),
        json!({"layer": "config-get", "envForwarded": true, "value": "0"})
    );
    assert_zero_budget_refused(&fx, &run);
}

#[test]
fn code_review_zero_budget_from_config_dispatches_no_refuter() {
    zero_budget_from_config_dispatches_no_refuter("code-review");
}

#[test]
fn plan_review_zero_budget_from_config_dispatches_no_refuter() {
    zero_budget_from_config_dispatches_no_refuter("plan-review");
}

#[test]
fn code_review_zero_budget_from_env_dispatches_no_refuter() {
    zero_budget_from_env_dispatches_no_refuter("code-review");
}

#[test]
fn plan_review_zero_budget_from_env_dispatches_no_refuter() {
    zero_budget_from_env_dispatches_no_refuter("plan-review");
}

#[test]
fn refutation_budget_env_beats_config() {
    let fx = Fixture::review("code-review", false, Some("\nmax_refutations = 0\n"))
        .unwrap_or_else(|f| panic!("{f}"));
    let run = observe_runtime_with(
        &Lib::real(),
        &fx,
        &[("RDM_MAX_REFUTATIONS", OsString::from("5"))],
    )
    .unwrap_or_else(|f| panic!("{f}"));
    assert_eq!(
        budget_record(&run),
        json!({"layer": "config-get", "envForwarded": true, "value": "5"})
    );
    check_review(&fx, &run, "code-review").unwrap_or_else(|f| panic!("{f}"));
}

#[test]
fn malformed_refutation_budget_env_is_refused_before_any_refuter() {
    let fx = Fixture::review("code-review", false, None).unwrap_or_else(|f| panic!("{f}"));
    let run = observe_runtime_with(
        &Lib::real(),
        &fx,
        &[("RDM_MAX_REFUTATIONS", OsString::from("5abc"))],
    )
    .unwrap_or_else(|f| panic!("{f}"));
    // `rdm config get max_refutations` applies the same grammar the engine
    // does, so the forwarded override is refused at that read, naming the
    // variable to fix.
    match &run.result {
        Ok(v) => panic!("a malformed budget was accepted: {v}"),
        Err(e) => assert!(
            e.message.contains("RDM_MAX_REFUTATIONS") && e.message.contains("non-negative integer"),
            "{e}"
        ),
    }
    assert!(starts(&run.events, None).is_empty(), "{:?}", run.events);
    assert_eq!(run.manifest["status"], "failed");
    fx.no_writes().unwrap_or_else(|f| panic!("{f}"));
}

#[test]
fn refutation_budget_payload_beats_config_and_skips_the_config_read() {
    let mut fx = Fixture::review("code-review", false, Some("\nmax_refutations = 0\n"))
        .unwrap_or_else(|f| panic!("{f}"));
    fx.spec["maxRefutations"] = json!(5);
    let run = observe_runtime(&Lib::real(), &fx).unwrap_or_else(|f| panic!("{f}"));
    assert_eq!(budget_record(&run), json!({"layer": "payload", "value": 5}));
    assert_eq!(
        starts(&run.events, Some("refuter")).len(),
        1,
        "{:?}",
        run.events
    );
    assert_eq!(budget_config_reads(&run), 0, "{:?}", run.journal);
    check_review(&fx, &run, "code-review").unwrap_or_else(|f| panic!("{f}"));
}

/// Evidence of one estimate preview run through the CLI.
pub struct PreviewRun {
    /// Whether the CLI exited 0.
    pub success: bool,
    /// Its stdout and stderr, interleaved (stdout alone on success).
    pub output: String,
    pub events: Vec<Event>,
    pub calls: Vec<Call>,
    pub manifest: Value,
}

/// Runs `node <lib>/scripts/rdm-codex.mjs spec.json` for the queue fixture
/// under the sandbox, bounded by `run_bounded`.
pub fn observe_preview(lib: &Lib, fx: &Fixture) -> Result<PreviewRun, Failure> {
    let spec_file = fx.root.join("spec.json");
    fs::write(&spec_file, fx.spec.to_string()).map_err(infra)?;
    let log = fx.root.join("cli.log");
    let mut spec = ProcessSpec::new(resolve_node().map_err(infra)?)
        .arg(lib.path(CLI))
        .arg(&spec_file)
        .cwd(&fx.source)
        .output_file(&log)
        .timeout(Duration::from_secs(60));
    for key in Sandbox::removals() {
        spec = spec.env_remove(key);
    }
    for (key, value) in fx.root.sandbox.vars() {
        spec = spec.env(key, value);
    }
    spec = spec
        .env("TMPDIR", fx.root.join("tmp"))
        .env("PATH", fx.path())
        .env("CODEX_FIXTURE_DIR", &fx.fixture);
    let success = match run_bounded(&spec, rdm_devtools::process::Hooks::new()) {
        Ok(_) => true,
        Err(RunError::NonZeroExit { .. }) => false,
        Err(e) => return Err(infra(format!("running the CLI: {e}"))),
    };
    Ok(PreviewRun {
        success,
        output: fs::read_to_string(&log).map_err(infra)?,
        events: fx.events(),
        calls: fx.calls(),
        manifest: manifest(&fx.run_dir),
    })
}

/// The runtime's agent semaphore held live judgment children to the
/// configured concurrency of 2.
pub fn check_bounded(run: &PreviewRun) -> Outcome {
    check_eq!(
        peak(&run.events),
        2,
        "the runtime's agent semaphore must bound live children: {:?}",
        run.events
    );
    Ok(())
}

/// The phase stem a recorded estimator call was asked about.
fn stem_of(call: &Call) -> Option<&str> {
    call.prompt
        .lines()
        .find_map(|l| l.strip_prefix("Phase stem: "))
        .map(str::trim)
}

fn check_preview(fx: &Fixture, run: &PreviewRun) -> Outcome {
    check!(run.success, "the CLI failed:\n{}", run.output);
    let out: Value = serde_json::from_str(&run.output)
        .map_err(|e| Failure::Check(format!("CLI stdout is not JSON ({e}):\n{}", run.output)))?;
    check_eq!(
        out["result"]["proposed"].as_array().map(Vec::len),
        Some(5),
        "proposals: {out}"
    );
    check_bounded(run)?;
    check_eq!(starts(&run.events, None).len(), 5, "judgments launched");
    let ends = run.events.iter().filter(|e| e.kind == "end").count();
    check_eq!(ends, 5, "every judgment completed");
    let mut stems: Vec<&str> = run.calls.iter().filter_map(stem_of).collect();
    stems.sort_unstable();
    stems.dedup();
    check_eq!(stems.len(), 5, "five distinct targets: {stems:?}");
    for call in &run.calls {
        let descendant = call
            .descendant
            .ok_or_else(|| infra(format!("call {} recorded no descendant", call.pid)))?;
        check!(
            until(Duration::from_secs(5), || !alive(call.pid)
                && !alive(descendant)),
            "the completed child group of {} survived",
            call.pid
        );
    }
    check_eq!(run.manifest["status"], json!("completed"), "manifest");
    fx.no_writes()
}

#[test]
fn estimate_preview_bounds_five_codex_children_to_two() {
    let fx = Fixture::queue().unwrap_or_else(|f| panic!("{f}"));
    let run = observe_preview(&Lib::real(), &fx).unwrap_or_else(|f| panic!("{f}"));
    check_preview(&fx, &run).unwrap_or_else(|f| panic!("{f}"));
}

#[test]
fn estimate_cancellation_launches_no_queued_judgment() {
    let fx = Fixture::queue().unwrap_or_else(|f| panic!("{f}"));
    let mut host = fx
        .host(&[("FIXTURE_HOLD", OsString::from("1"))])
        .unwrap_or_else(|f| panic!("{f}"));
    let module = host
        .import(&Lib::real().path(RUNTIME))
        .unwrap_or_else(|e| panic!("import: {e}"));
    let run = host.export(&module, "runRuntime").unwrap();
    let pending = host
        .start_call(&run, vec![fx.spec.clone()])
        .expect("start runRuntime");
    assert!(
        until(HOST_TIMEOUT, || starts(&fx.events(), None).len() >= 2),
        "the active judgments never started"
    );
    assert!(!host.is_settled(&pending), "the run ended early");
    // The runtime and the transport both handle SIGTERM in the runner process.
    signal(&host.pid().to_string(), "TERM");
    let e = match host.await_call(pending) {
        Err(rdm_devtools::workflow::WorkflowError::Js(e)) => e,
        other => panic!("expected a cancelled run, got {other:?}"),
    };
    assert!(e.message.to_lowercase().contains("cancel"), "{e}");
    let events = fx.events();
    assert_eq!(
        events.len(),
        2,
        "queued judgments must never launch after cancellation: {events:?}"
    );
    for call in fx.calls() {
        let descendant = call.descendant.expect("a held call records its descendant");
        assert!(
            until(Duration::from_secs(5), || !alive(call.pid)
                && !alive(descendant)),
            "the cancelled child group of {} survived",
            call.pid
        );
    }
    let journal = journal(&fx.run_dir);
    assert_eq!(records(&journal, "agent-started").len(), 2);
    assert!(records(&journal, "run-completed").is_empty());
    assert_eq!(manifest(&fx.run_dir)["status"], "failed");
    fx.no_writes().unwrap_or_else(|f| panic!("{f}"));
    host.shutdown()
        .expect("the runner survives its handled SIGTERM");
}

//! The emitted lane, exercised in a foreign repo. Byte identity says the
//! emitted engine is the one this checkout ships; these tests say it works
//! somewhere that is neither this repo nor Rust.
//!
//! [`Downstream::new`] builds, in one temp directory: a Python/TypeScript
//! source repo on `feature/checkout` (one commit ahead of `main`); its own
//! plan repo whose default project is a decoy and which carries project
//! `acme-web` (roadmap `checkout-revamp` with phases `checkout-form` and
//! `order-summary`, task `tidy-cli`); its own rdm path, `tools/acme-rdm` (a
//! symlink to the binary under test); and the lane that binary emits with
//! `agent-config claude --skills --project acme-web --out <repo>`. Every
//! script runs with `PATH=/usr/bin:/bin` from the source repo, so a command
//! that dropped the fixture's `rdmBin` cannot run and one that dropped
//! `--project` lands in the decoy.
//!
//! Three layers, kept apart:
//!
//! 1. **Helper extraction that bypasses the driver** — the emitted engine's
//!    helpers are compiled out of the emitted bytes with
//!    `rdm_devtools::workflow::helper_source` (proven byte-exact by
//!    `invert_helper_source`) and executed under Node; the resolver, the
//!    environment guards and the persist ladders they build run against the
//!    fixture.
//! 2. **Mocked-driver execution** — the emitted engine's whole driver runs
//!    under a Rust-scripted clean reviewer fleet (item identity `{ task }`
//!    plus the pinned `source`/`base`/`expectedHead`/`expectedBranch`, with
//!    `persist: true`), and the `persistScript` it returns is executed
//!    against the fixture plan. Two additions to the documented shape are
//!    required by the real binary, not by the engine: the pinned source is
//!    the task's `rdm worktree add` checkout of the fixture repo (a change
//!    review binds to a registered checkout), and `implements` names a plan
//!    seeded for the task (a change review records the plan it implements).
//! 3. **Planted corruptions in the emitted bytes** — three mutants of the
//!    emitted engine, each caught by a check or a JavaScript exception.
//!
//! These are component tests: real emitted JavaScript under Node with a fake
//! host, real rdm and real git. No Claude host is observed here; that is
//! `scripts/observe-workflow-listing.sh`.

use std::path::PathBuf;

use rdm_devtools::workflow::{Host, WorkflowError, helper_source, invert_helper_source};
use serde_json::{Value, json};

use crate::plan_fixture::{PlanRepo, Run, Sandbox, review_id};
use crate::workflow_support::{
    Agent, Failure, Lib, Outcome, Reply, infra, load, run_driver, run_mutant, run_real,
};

/// The emitted review engine, relative to the fixture repo.
pub const ENGINE: &str = ".claude/workflows/rdm-wf-review-refute-fix.js";
/// The fixture's project.
pub const PROJECT: &str = "acme-web";
/// The fixture's roadmap.
pub const ROADMAP: &str = "checkout-revamp";
/// The fixture's first phase stem.
pub const PHASE: &str = "phase-1-checkout-form";
/// The fixture's task.
pub const TASK: &str = "tidy-cli";
/// The fixture's feature branch.
pub const BRANCH: &str = "feature/checkout";
/// The fixture phase's body: an anchored comment must quote it verbatim.
pub const PHASE_QUOTE: &str = "Build the acme checkout form.";

/// The helpers layer 1 extracts.
const HELPERS: [&str; 5] = [
    "resolveReviewers",
    "resolveRdmBin",
    "parseProjectArg",
    "projectFlag",
    "persistReviewCommands",
];

/// A foreign consumer repo with its own plan repo, rdm path and emitted lane.
pub struct Downstream {
    /// The plan repo (its temp directory owns everything below).
    pub plan: PlanRepo,
    /// The source repo the lane was emitted into.
    pub repo: PathBuf,
    /// The fixture's own rdm path.
    pub bin: PathBuf,
    /// The user context scripts and emissions run under.
    pub sandbox: Sandbox,
}

impl Downstream {
    /// Builds the fixture and emits the lane into it.
    pub fn new() -> Result<Self, Failure> {
        let plan = PlanRepo::with_decoy(&[PROJECT])?;
        plan.seed(&[
            &[
                "roadmap",
                "create",
                ROADMAP,
                "--title",
                "Checkout revamp",
                "--body",
                "Revamp the acme checkout flow.",
                "--no-edit",
                "--project",
                PROJECT,
            ],
            &[
                "phase",
                "create",
                "checkout-form",
                "--title",
                "Checkout form",
                "--number",
                "1",
                "--body",
                PHASE_QUOTE,
                "--no-edit",
                "--roadmap",
                ROADMAP,
                "--project",
                PROJECT,
            ],
            &[
                "phase",
                "create",
                "order-summary",
                "--title",
                "Order summary",
                "--number",
                "2",
                "--body",
                "Summarize orders on the confirmation screen.",
                "--no-edit",
                "--roadmap",
                ROADMAP,
                "--project",
                PROJECT,
            ],
            &[
                "task",
                "create",
                TASK,
                "--title",
                "Tidy the acme CLI",
                "--body",
                "Tidy up the acme command line.",
                "--no-edit",
                "--project",
                PROJECT,
            ],
        ])?;
        let sandbox = Sandbox::new(plan.dir.path())?;

        let repo = plan.dir.path().join("repo");
        for (file, text) in [
            (
                "src/acme/api.py",
                "\"\"\"Order API.\"\"\"\n\n\ndef _normalize(order):\n    return {\"id\": order[\"id\"], \"total\": order[\"total\"]}\n",
            ),
            (
                "web/src/client.ts",
                "const BASE = \"/api\";\n\nfunction join(a: string, b: string): string {\n  return a + b;\n}\n",
            ),
        ] {
            let path = repo.join(file);
            std::fs::create_dir_all(path.parent().expect("a parent")).map_err(infra)?;
            std::fs::write(&path, text).map_err(infra)?;
        }
        sandbox.git(&repo, &["init", "--quiet", "-b", "main"])?;
        sandbox.git(&repo, &["add", "-A"])?;
        sandbox.git(&repo, &["commit", "--quiet", "-m", "seed: acme base"])?;
        sandbox.git(&repo, &["checkout", "--quiet", "-b", BRANCH])?;
        let client = repo.join("web/src/client.ts");
        let mut text = std::fs::read_to_string(&client).map_err(infra)?;
        text.push_str("\nexport function listOrders(limit: number): Promise<string[]> {\n  return fetch(BASE + \"/orders?limit=\" + limit).then((r) => r.json());\n}\n");
        std::fs::write(&client, text).map_err(infra)?;
        sandbox.git(
            &repo,
            &["commit", "--quiet", "-am", "feat: add order listing"],
        )?;

        let tools = plan.dir.path().join("tools");
        std::fs::create_dir_all(&tools).map_err(infra)?;
        let bin = tools.join("acme-rdm");
        std::os::unix::fs::symlink(crate::plan_fixture::rdm_bin(), &bin).map_err(infra)?;

        let out = sandbox
            .command(&bin)
            .args([
                "agent-config",
                "claude",
                "--skills",
                "--project",
                PROJECT,
                "--out",
            ])
            .arg(&repo)
            .current_dir(&repo)
            .output()
            .map_err(infra)?;
        check!(
            out.status.success(),
            "emitting the lane into the fixture failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        Ok(Self {
            plan,
            repo,
            bin,
            sandbox,
        })
    }

    /// The fixture's rdm path as a string.
    pub fn bin_str(&self) -> String {
        self.bin.to_string_lossy().into_owned()
    }

    /// A bare-`PATH` script run from the source repo, with the sandbox HOME.
    pub fn run(&self) -> Run {
        Run::bare()
            .cwd(&self.repo)
            .env("HOME", &self.sandbox.home.to_string_lossy())
    }

    /// `git rev-parse <what>` in the source repo.
    pub fn rev(&self, what: &str) -> Result<String, Failure> {
        Ok(self
            .sandbox
            .git(&self.repo, &["rev-parse", what])?
            .trim()
            .to_owned())
    }

    /// Reads a review back through the fixture's own binary, under the bare
    /// `PATH`.
    pub fn review(&self, id: &str) -> Result<Value, Failure> {
        let out = self.plan.run_ok(
            &self.run(),
            &format!(
                "'{}' review show '{id}' --project {PROJECT} --format json",
                self.bin_str()
            ),
        )?;
        serde_json::from_str(&out).map_err(|e| Failure::Infra(format!("review json: {e}: {out}")))
    }
}

fn fixture() -> Downstream {
    Downstream::new().unwrap_or_else(|f| panic!("{f}"))
}

fn names_with_meta() -> Vec<&'static str> {
    HELPERS.iter().copied().chain(["meta"]).collect()
}

/// A started host holding the helpers extracted from `lib`'s engine.
struct Helpers {
    host: Host,
    fns: Value,
}

impl Helpers {
    fn open(lib: &Lib) -> Result<Self, Failure> {
        let src = lib.read(ENGINE)?;
        let mut host = Host::start_default()?;
        let fns = load(host.extract_helpers(&src, &names_with_meta()))?;
        Ok(Self { host, fns })
    }

    fn call(&mut self, name: &str, args: Vec<Value>) -> Result<Value, Failure> {
        Ok(self.host.call(&self.fns[name], args)?)
    }

    fn throws(&mut self, name: &str, args: Vec<Value>) -> Result<bool, Failure> {
        match self.host.call(&self.fns[name], args) {
            Ok(_) => Ok(false),
            Err(WorkflowError::Js(_)) => Ok(true),
            Err(e) => Err(e.into()),
        }
    }
}

fn keys(v: &Value) -> Vec<String> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|d| d["key"].as_str().map(str::to_owned))
        .collect()
}

fn rework_result() -> Value {
    json!({
        "mode": "code",
        "outcome": "rework",
        "survivors": [{
            "id": "f1", "severity": "blocking", "confidence": 90,
            "what_fails": "x", "quote": PHASE_QUOTE
        }]
    })
}

fn clean_result() -> Value {
    json!({ "mode": "code", "outcome": "reviewed", "survivors": [] })
}

// --- Layer 1: helper extraction that bypasses the driver ---------------------

#[test]
fn emitted_engine_helpers_round_trip_to_the_emitted_bytes() {
    let ds = fixture();
    let src = std::fs::read_to_string(ds.repo.join(ENGINE)).expect("read the emitted engine");
    let names = names_with_meta();
    let transformed = helper_source(&src, &names).expect("transform the emitted engine");
    let back = invert_helper_source(&transformed, &names).expect("invert the transform");
    assert!(
        back == src,
        "the executed helpers are exactly the emitted bytes"
    );
    run_real(|_| {
        let helpers = Helpers::open(&Lib::at(&ds.repo))?;
        let stem = ENGINE
            .rsplit('/')
            .next()
            .and_then(|f| f.strip_suffix(".js"))
            .unwrap_or_default();
        check_eq!(
            helpers.fns["meta"]["name"],
            json!(stem),
            "meta.name is the emitted file's stem"
        );
        Ok(())
    });
}

#[test]
fn raw_emitted_engine_does_not_import() {
    let ds = fixture();
    let raw = ds.plan.dir.path().join("raw.mjs");
    std::fs::copy(ds.repo.join(ENGINE), &raw).expect("copy the emitted engine");
    let mut host = Host::start_default().expect("start host");
    match host.import(&raw) {
        Err(WorkflowError::Js(e)) => {
            assert_eq!(
                e.name, "SyntaxError",
                "the untransformed engine is not a module: {e}"
            )
        }
        other => panic!("importing the raw emitted engine must fail with a SyntaxError: {other:?}"),
    }
}

/// The emitted resolver and environment guards, executed.
fn resolvers(lib: &Lib) -> Outcome {
    let mut h = Helpers::open(lib)?;
    let all = keys(&h.call("resolveReviewers", vec![json!("code"), Value::Null])?);
    check!(all.len() >= 2, "more than one code reviewer ships: {all:?}");
    check_eq!(
        keys(&h.call("resolveReviewers", vec![json!("code"), Host::undefined()])?),
        all,
        "an omitted reviewer set runs every reviewer"
    );
    let picked: Vec<String> = all.iter().take(2).cloned().collect();
    check_eq!(
        keys(&h.call("resolveReviewers", vec![json!("code"), json!(picked)])?),
        picked,
        "a caller-selected set runs exactly those reviewers"
    );
    let mut with_unknown = picked.clone();
    with_unknown.push("not-a-reviewer".to_owned());
    check_eq!(
        keys(&h.call("resolveReviewers", vec![json!("code"), json!(with_unknown)])?),
        picked,
        "an unknown reviewer name is dropped"
    );
    check_eq!(
        h.call("resolveRdmBin", vec![Host::undefined()])?,
        json!("rdm"),
        "an absent rdmBin defaults to rdm on PATH"
    );
    check!(
        h.throws("resolveRdmBin", vec![json!(42)])?,
        "a non-string rdmBin is rejected"
    );
    for bad in ["a b", "a;rm -rf /", "$(x)"] {
        check!(
            h.throws("parseProjectArg", vec![json!(bad)])?,
            "parseProjectArg rejects {bad:?}"
        );
    }
    check_eq!(
        h.call("projectFlag", vec![json!({ "project": PROJECT })])?,
        json!(format!(" --project {PROJECT}")),
        "projectFlag threads the project"
    );
    check_eq!(
        h.call("projectFlag", vec![json!({})])?,
        json!(""),
        "no project, no flag"
    );
    Ok(())
}

#[test]
fn emitted_resolvers_and_env_guards_execute() {
    let ds = fixture();
    run_real(|_| resolvers(&Lib::at(&ds.repo)));
}

/// The persist ladders the emitted engine builds, run against the fixture
/// plan from the source repo under the bare `PATH`.
fn ladders(ds: &Downstream, lib: &Lib) -> Outcome {
    let mut h = Helpers::open(lib)?;
    let cfg = json!({ "rdmBin": ds.bin_str(), "project": PROJECT });
    let target = format!("phase/{ROADMAP}/{PHASE}");
    let rework = h.call(
        "persistReviewCommands",
        vec![rework_result(), json!(target), cfg.clone()],
    )?;
    let script = crate::plan_fixture::script(&rework);
    let out = ds.plan.run(&ds.run(), &format!("set -eu\n{script}"))?;
    check!(
        out.status.success(),
        "the emitted rework ladder failed ({:?}):\n{script}\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    check_eq!(
        stdout
            .lines()
            .filter(|l| l.starts_with("reviewId="))
            .count(),
        1,
        "the ladder reports one review id: {stdout}"
    );
    let id = review_id(&stdout).unwrap_or_default();
    let review = ds.review(&id)?;
    check_eq!(review["state"], json!("submitted"), "{review}");
    check_eq!(review["verdict"], json!("request-changes"), "{review}");
    let comments = review["comments"].as_array().cloned().unwrap_or_default();
    check_eq!(comments.len(), 1, "one persisted comment: {review}");
    check_eq!(
        comments[0]["anchor"]["quote"],
        json!(PHASE_QUOTE),
        "the comment keeps its anchor"
    );
    check_eq!(
        comments[0]["resolution"]["state"],
        json!("resolved"),
        "the anchor locates its quote in the fixture phase"
    );

    let clean = h.call(
        "persistReviewCommands",
        vec![clean_result(), json!(format!("task/{TASK}")), cfg],
    )?;
    let script = crate::plan_fixture::script(&clean);
    let out = ds.plan.run(&ds.run(), &format!("set -eu\n{script}"))?;
    check!(
        out.status.success(),
        "the emitted clean ladder failed:\n{script}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let id = review_id(&String::from_utf8_lossy(&out.stdout)).unwrap_or_default();
    let review = ds.review(&id)?;
    check_eq!(review["state"], json!("submitted"), "{review}");
    check!(
        review["verdict"].is_string(),
        "the clean review carries a verdict: {review}"
    );
    check!(
        review["comments"].as_array().is_none_or(Vec::is_empty),
        "a clean ladder persists no comment: {review}"
    );
    Ok(())
}

#[test]
fn emitted_persist_ladders_run_against_the_foreign_plan() {
    let ds = fixture();
    run_real(|_| ladders(&ds, &Lib::at(&ds.repo)));
}

// --- Layer 2: mocked-driver execution of the emitted engine ------------------

#[test]
fn emitted_engine_driver_persists_through_the_foreign_fixture() {
    let ds = fixture();
    run_real(|_| {
        let lib = Lib::at(&ds.repo);
        // A change review records the plan it implements.
        ds.plan.seed(&[&[
            "plan",
            "create",
            "tidy-plan",
            "--implements",
            &format!("task/{TASK}"),
            "--title",
            "Tidy plan",
            "--body",
            "Tidy the CLI.",
            "--no-edit",
            "--project",
            PROJECT,
        ]])?;
        // `rdm review source` binds to a registered checkout, so the pinned
        // source is the task's worktree of the fixture repo, one commit
        // ahead of `main`.
        let wt = ds.plan.run_ok(
            &ds.run(),
            &format!(
                "'{}' worktree add task/{TASK} --project {PROJECT}",
                ds.bin_str()
            ),
        )?;
        let wt = PathBuf::from(wt.trim());
        std::fs::write(wt.join("CHANGELOG.md"), "- Tidied the acme CLI.\n").map_err(infra)?;
        ds.sandbox.git(&wt, &["add", "CHANGELOG.md"])?;
        ds.sandbox.git(
            &wt,
            &["commit", "--quiet", "-m", "chore: tidy the acme CLI"],
        )?;
        let rev = |what: &[&str]| -> Result<String, Failure> {
            let mut args = vec!["rev-parse"];
            args.extend_from_slice(what);
            Ok(ds.sandbox.git(&wt, &args)?.trim().to_owned())
        };
        let args = json!({
            "mode": "code",
            "task": TASK,
            "rdmBin": ds.bin_str(),
            "project": PROJECT,
            "source": wt.to_string_lossy(),
            "base": rev(&["main"])?,
            "expectedHead": rev(&["HEAD"])?,
            "expectedBranch": rev(&["--abbrev-ref", "HEAD"])?,
            "implements": "plan/tidy-plan",
            "persist": true,
        });
        let agent = Agent::scripted(|call| {
            if call.label == "find:code:ac" {
                return Reply::Value(json!({
                    "ac": [{ "criterion": "Tidy the CLI", "status": "PASS", "evidence": "done" }],
                    "findings": []
                }));
            }
            if call.label.starts_with("refute:") {
                return Reply::Value(json!({ "refuted": false, "confidence": 95 }));
            }
            Reply::Value(json!({ "findings": [] }))
        });
        let result = run_driver(&lib, ENGINE, args, &agent)?.map_err(Failure::Js)?;
        check_eq!(
            result["outcome"],
            json!("reviewed"),
            "a clean fleet: {result}"
        );
        check!(
            !agent.calls_with("find:").is_empty(),
            "the driver dispatched its finders"
        );
        let script = result["persistScript"]
            .as_str()
            .ok_or_else(|| Failure::Check(format!("persist:true returns a ladder: {result}")))?;
        let out = ds.plan.run(&ds.run(), script)?;
        check!(
            out.status.success(),
            "the returned persist ladder failed ({:?}):\n{script}\n{}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr)
        );
        let id = review_id(&String::from_utf8_lossy(&out.stdout))
            .ok_or_else(|| Failure::Check("the ladder printed no review id".to_owned()))?;
        let review = ds.review(&id)?;
        check_eq!(review["state"], json!("submitted"), "{review}");
        check_eq!(
            review["target"]["kind"],
            json!("change"),
            "a source-bound review targets the change: {review}"
        );
        Ok(())
    });
}

// --- Layer 3: planted corruptions in the emitted bytes -----------------------

#[test]
fn mutant_a_hardcoded_binary_is_caught() {
    let ds = fixture();
    let lib = Lib::mutant_at(
        &ds.repo,
        "A (binary literal)",
        &[ENGINE],
        &[(
            ENGINE,
            "const bin = persistRdmBin(cfg && cfg.rdmBin)",
            "const bin = './target/debug/rdm'",
        )],
    );
    run_mutant(lib, |lib| ladders(&ds, lib));
}

#[test]
fn mutant_b_hardcoded_project_is_caught() {
    let ds = fixture();
    let lib = Lib::mutant_at(
        &ds.repo,
        "B (project literal)",
        &[ENGINE],
        &[(
            ENGINE,
            "const proj = persistProjectFlag(cfg)",
            "const proj = ' --project rdm'",
        )],
    );
    run_mutant(lib, |lib| ladders(&ds, lib));
}

#[test]
fn mutant_c_ignored_reviewer_selection_is_caught() {
    let ds = fixture();
    let lib = Lib::mutant_at(
        &ds.repo,
        "C (reviewer selection)",
        &[ENGINE],
        &[(
            ENGINE,
            "  if (reviewers === null || reviewers === undefined) return dims.slice();",
            "  return dims.slice();",
        )],
    );
    run_mutant(lib, resolvers);
}

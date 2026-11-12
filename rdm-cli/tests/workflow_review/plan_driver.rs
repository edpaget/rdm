//! The plan-review driver (`runPlanReviewDriver` in
//! `.claude/workflows/lib/plan-review.mjs`), executed three ways:
//!
//! - **LIB** — the real driver with a Rust `runPlanReview` callback that
//!   records every review context it is handed, so only agents the driver
//!   dispatches on its own account could appear (and none may);
//! - **REAL** — the same driver over the real `buildReviewPipeline('plan')`
//!   from `.claude/workflows/lib/review.mjs`, with a recording agent;
//! - **SHIP** — the shipped `.claude/workflows/rdm-wf-plan-review.js` driver,
//!   loaded with `Host::load_driver`.
//!
//! The commands the driver returns or names — read commands and persist
//! ladders — are executed with `PATH=/usr/bin:/bin` against
//! `CARGO_BIN_EXE_rdm` in a per-test plan repo whose default project is the
//! decoy (project `prv`: roadmap `r` with `phase-1-x`/`phase-2-y`, task `t`,
//! plan `p`), and the resulting state is read back. Ported from
//! `scripts/lib/plan-review-hoist.test.mjs`; see
//! `docs/test-migration-inventory.md` § "Phase 3".

use std::cell::RefCell;
use std::rc::Rc;

use rdm_devtools::workflow::{Host, JsError};
use serde_json::{Value, json};

use crate::plan_fixture::{PlanRepo, Run, rdm_bin, review_id, script};
use crate::support::{
    Agent, Failure, Js, Lib, Outcome, PLAN_ENGINE, Reply, infra, run_driver, run_real, strings,
};

const PROJECT: &str = "prv";
const HEAD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const BASE: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

// --- Fixtures --------------------------------------------------------------------

/// The seed every executing case shares, into a repo whose default project
/// is `default_project`.
fn seed_into(repo: &PlanRepo) -> Outcome {
    repo.seed(&[
        &[
            "roadmap",
            "create",
            "r",
            "--title",
            "R",
            "--body",
            "## Intent\n\nThe roadmap body.",
            "--tags",
            "needs-plan-review",
            "--no-edit",
            "--project",
            PROJECT,
        ],
        &[
            "phase",
            "create",
            "x",
            "--title",
            "X",
            "--number",
            "1",
            "--body",
            "Phase x body.",
            "--tags",
            "needs-plan-review,depends-unlanded",
            "--no-edit",
            "--roadmap",
            "r",
            "--project",
            PROJECT,
        ],
        &[
            "phase",
            "create",
            "y",
            "--title",
            "Y",
            "--number",
            "2",
            "--body",
            "Phase y body.",
            "--no-edit",
            "--roadmap",
            "r",
            "--project",
            PROJECT,
        ],
        &[
            "task",
            "create",
            "t",
            "--title",
            "T",
            "--body",
            "Task body.",
            "--tags",
            "needs-plan-review",
            "--no-edit",
            "--project",
            PROJECT,
        ],
        &[
            "plan",
            "create",
            "p",
            "--title",
            "P",
            "--implements",
            "task/t",
            "--body",
            "The plan body.",
            "--no-edit",
            "--project",
            PROJECT,
        ],
    ])
}

/// A decoy-default repo with the shared seed in project `prv`.
fn plan_repo() -> Result<PlanRepo, Failure> {
    let repo = PlanRepo::with_decoy(&[PROJECT])?;
    seed_into(&repo)?;
    Ok(repo)
}

/// The engine axes for executing emitted commands against [`plan_repo`].
fn axes() -> Value {
    json!({ "rdmBin": rdm_bin(), "project": PROJECT })
}

fn with(base: &Value, extra: Value) -> Value {
    let mut out = base.as_object().cloned().unwrap_or_default();
    if let Some(o) = extra.as_object() {
        for (k, v) in o {
            out.insert(k.clone(), v.clone());
        }
    }
    Value::Object(out)
}

/// An agent that records every call and returns an empty finding set.
fn inert() -> Agent {
    Agent::scripted(|_| Reply::Value(json!({ "findings": [], "ok": true })))
}

/// Finders return one blocking finding (so a refuter is dispatched too);
/// refuters refute nothing.
fn probe() -> Agent {
    Agent::scripted(|call| {
        if call.label.starts_with("find:") {
            Reply::Value(
                json!({ "findings": [{ "id": "f1", "concern": "specification", "severity": "blocking",
                "confidence": 90, "what_fails": "x", "why": "y", "recommendation": "z" }] }),
            )
        } else {
            Reply::Value(json!({ "refuted": false, "confidence": 90, "rationale": "r" }))
        }
    })
}

/// One LIB run: the result (or throw), the agent, the review contexts the
/// driver handed `runPlanReview`, in order.
struct Driven {
    result: Result<Value, JsError>,
    agent: Agent,
    contexts: Vec<Value>,
}

impl Driven {
    fn ok(&self) -> Result<&Value, Failure> {
        self.result.as_ref().map_err(|e| Failure::Js(e.clone()))
    }
    fn targets(&self) -> Vec<Value> {
        self.contexts.iter().map(|c| c["target"].clone()).collect()
    }
}

/// Runs the real driver with a Rust `runPlanReview` answering `review`.
fn drive_with(
    js: &mut Js,
    args: Value,
    agent: Agent,
    mut review: impl FnMut(&Value) -> Result<Value, String> + 'static,
) -> Result<Driven, Failure> {
    let mut deps = agent.install(&mut js.host);
    let contexts = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&contexts);
    deps["runPlanReview"] = js.host.register_fn(move |a| {
        let ctx = a.first().cloned().unwrap_or(Value::Null);
        seen.borrow_mut().push(ctx.clone());
        review(&ctx)
    });
    deps["findModel"] = json!("model-find");
    deps["verifyModel"] = json!("model-verify");
    let result = js.plan_try_call("runPlanReviewDriver", vec![args, deps])?;
    let contexts = contexts.borrow().clone();
    Ok(Driven {
        result,
        agent,
        contexts,
    })
}

/// LIB: every unit's review returns `survivors`.
fn drive_lib(js: &mut Js, args: Value, survivors: Value) -> Result<Driven, Failure> {
    drive_with(js, args, inert(), move |_| {
        Ok(json!({ "survivors": survivors, "acTable": null, "budget": null, "coverage": null }))
    })
}

/// REAL: the driver over the real plan pipeline, both wired to `agent`.
fn drive_real(js: &mut Js, args: Value, agent: Agent) -> Result<(Value, Agent), Failure> {
    let mut deps = agent.install(&mut js.host);
    let pipeline = js.call("buildReviewPipeline", vec![json!("plan"), deps.clone()])?;
    deps["runPlanReview"] = pipeline;
    let result = js
        .plan_try_call("runPlanReviewDriver", vec![args, deps])?
        .map_err(Failure::Js)?;
    Ok((result, agent))
}

/// SHIP: the shipped engine's driver.
fn drive_ship(lib: &Lib, args: Value) -> Result<(Value, Agent), Failure> {
    let agent = inert();
    let out = run_driver(lib, PLAN_ENGINE, args, &agent)?.map_err(Failure::Js)?;
    Ok((out, agent))
}

fn judgment_only(labels: &[String]) -> bool {
    labels.iter().all(|l| {
        l.starts_with("find:") || l.starts_with("refute:") || l.starts_with("consolidate:")
    })
}

fn blocking(what: &str) -> Value {
    json!({ "id": "c1", "concern": "specification", "severity": "blocking", "confidence": 95, "what_fails": what })
}

fn rejects_with(r: &Result<Value, JsError>, needle: &str) -> bool {
    r.as_ref().is_err_and(|e| e.message.contains(needle))
}

/// Runs a read command and parses its JSON.
fn read(repo: &PlanRepo, run: &Run, cmd: &Value) -> Result<Value, Failure> {
    let cmd = cmd
        .as_str()
        .ok_or_else(|| Failure::Check(format!("not a command: {cmd}")))?;
    let out = repo.run_ok(run, cmd)?;
    serde_json::from_str(&out)
        .map_err(|e| Failure::Check(format!("{cmd} printed non-JSON: {e}: {out}")))
}

/// Runs a read command and returns its stdout verbatim.
fn read_text(repo: &PlanRepo, cmd: &Value) -> Result<String, Failure> {
    let cmd = cmd
        .as_str()
        .ok_or_else(|| Failure::Check(format!("not a command: {cmd}")))?;
    repo.run_ok(&Run::bare(), cmd)
}

/// `<show_args> --project prv --numbered`, run directly: the numbered body
/// rdm itself renders for that document.
fn numbered(repo: &PlanRepo, show_args: &[&str]) -> Result<String, Failure> {
    let mut args = show_args.to_vec();
    args.extend(["--project", PROJECT, "--numbered"]);
    repo.ok("reader", &args)
}

/// The text of each line of a `--numbered` rendering, in order.
fn numbered_lines(rendering: &str) -> Vec<String> {
    rendering
        .lines()
        .map(|l| l.split_once('\t').map_or("", |(_, t)| t).to_owned())
        .collect()
}

fn phase_tags(repo: &PlanRepo, stem: &str) -> Result<Vec<String>, Failure> {
    Ok(strings(&repo.phase(PROJECT, "r", stem)?["tags"]))
}

// --- A: what the driver dispatches, reads and returns -----------------------------

#[test]
fn every_target_dispatches_only_finders_and_refuters() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let runs = [
            (
                "phase",
                json!({ "roadmap": "r", "phase": "phase-1-x", "tags": ["needs-plan-review"] }),
            ),
            ("task", json!({ "task": "t", "tags": [] })),
            (
                "roadmap",
                json!({ "roadmap": "r", "phases": [{ "stem": "phase-1-x" }, { "stem": "phase-2-y" }], "tags": [] }),
            ),
            (
                "implementation-plan",
                json!({ "implementationPlan": true, "planSlug": "p", "persist": true }),
            ),
        ];
        for (name, args) in runs {
            let (_, agent) = drive_real(&mut js, args, inert())?;
            let labels = agent.labels();
            check!(
                !labels.is_empty(),
                "{name}: the run dispatched no agent at all"
            );
            check!(
                judgment_only(&labels),
                "{name}: non-judgment agent(s) dispatched: {labels:?}"
            );
        }
        Ok(())
    });
}

#[test]
fn unit_commands_read_their_own_document() {
    run_real(|lib| {
        let repo = plan_repo()?;
        let mut js = Js::open(lib)?;
        let run = Run::bare();
        let d = drive_lib(
            &mut js,
            with(
                &axes(),
                json!({ "roadmap": "r", "phase": "phase-1-x", "tags": [] }),
            ),
            json!([]),
        )?;
        d.ok()?;
        check_eq!(d.contexts.len(), 1, "one unit");
        let ctx = &d.contexts[0];
        check_eq!(
            ctx["target"],
            json!("phase/r/phase-1-x"),
            "the graded target is an identifier, not a body"
        );
        check_eq!(
            read_text(&repo, &ctx["itemCommand"])?,
            numbered(&repo, &["phase", "show", "phase-1-x", "--roadmap", "r"])?,
            "the item command prints the unit's own phase, numbered"
        );
        let roadmap = read(&repo, &run, &ctx["roadmapCommand"])?;
        check_eq!(
            roadmap["slug"],
            json!("r"),
            "the roadmap command reads its parent"
        );

        let t = drive_lib(
            &mut js,
            with(&axes(), json!({ "task": "t", "tags": [] })),
            json!([]),
        )?;
        t.ok()?;
        check_eq!(t.contexts[0]["target"], json!("task/t"), "task target");
        check_eq!(
            read_text(&repo, &t.contexts[0]["itemCommand"])?,
            numbered(&repo, &["task", "show", "t"])?,
            "the item command prints the task, numbered"
        );
        check_eq!(
            t.contexts[0]["roadmapCommand"],
            Value::Null,
            "a task has no parent roadmap"
        );
        check!(
            d.agent.calls().is_empty() && t.agent.calls().is_empty(),
            "no agent of the driver's own"
        );
        Ok(())
    });
}

#[test]
fn roadmap_sweep_reviews_caller_named_stems() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let swept = drive_lib(
            &mut js,
            json!({ "roadmap": "r", "phases": [{ "stem": "phase-1-x" }, { "stem": "phase-2-y" }], "tags": [] }),
            json!([]),
        )?;
        check_eq!(
            swept.targets(),
            vec![
                json!("roadmap/r"),
                json!("phase/r/phase-1-x"),
                json!("phase/r/phase-2-y")
            ],
            "the roadmap document, then each named stem"
        );
        let alone = drive_lib(&mut js, json!({ "roadmap": "r", "tags": [] }), json!([]))?;
        check_eq!(
            alone.targets(),
            vec![json!("roadmap/r")],
            "with no stems the roadmap document is reviewed alone"
        );
        Ok(())
    });
}

#[test]
fn terminal_stems_skipped_and_reported() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let d = drive_lib(
            &mut js,
            json!({ "roadmap": "r", "tags": [], "phases": [
                { "stem": "phase-1-x", "status": "done" },
                { "stem": "phase-2-y", "status": "in-progress" },
                { "stem": "phase-3-z", "status": "wont-fix" }] }),
            json!([]),
        )?;
        let r = d.ok()?;
        check_eq!(
            d.targets(),
            vec![json!("roadmap/r"), json!("phase/r/phase-2-y")],
            "terminal stems are excluded"
        );
        let skipped: Vec<Value> = r["skippedPhases"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|p| p["stem"].clone())
            .collect();
        check_eq!(
            skipped,
            vec![json!("phase-1-x"), json!("phase-3-z")],
            "and reported"
        );
        check!(
            r["summary"]
                .as_str()
                .is_some_and(|s| s.contains("skipped 2 terminal phase")),
            "the summary counts them: {}",
            r["summary"]
        );
        // Fail-open: an unknown or missing status stays in the sweep.
        let odd = drive_lib(
            &mut js,
            json!({ "roadmap": "r", "tags": [], "phases": [
                { "stem": "phase-1-x", "status": "a-future-status" }, { "stem": "phase-2-y" }] }),
            json!([]),
        )?;
        check_eq!(odd.ok()?["skippedPhases"], json!([]), "nothing skipped");
        check_eq!(odd.contexts.len(), 3, "both stay in the sweep");
        Ok(())
    });
}

/// What the retired tag gate put on a result and on each of its units.
const GATE_KEYS: [&str; 3] = ["gateAction", "gatePendingCount", "clearsPlanReviewTag"];

/// `result` and every unit on it carry no gate key and no pending-gate
/// clause, and every unit passed.
fn passed_with_no_gate(what: &str, result: &Value) -> Outcome {
    let units = result["units"].as_array().cloned().unwrap_or_default();
    check!(!units.is_empty(), "{what}: units were reviewed: {result}");
    for carrier in std::iter::once(result).chain(units.iter()) {
        for key in GATE_KEYS {
            check!(carrier.get(key).is_none(), "{what}: no {key} on {carrier}");
        }
        check!(
            !carrier["summary"]
                .as_str()
                .unwrap_or_default()
                .contains("gate pending"),
            "{what}: no pending-gate clause: {}",
            carrier["summary"]
        );
    }
    for unit in &units {
        check_eq!(
            unit["outcome"],
            json!("reviewed"),
            "{what}: {} carries the verdict",
            unit["ident"]
        );
        check_eq!(
            unit.get("status"),
            Some(&json!(null)),
            "{what}: {} writes no status",
            unit["ident"]
        );
    }
    Ok(())
}

/// Runs a passing unit's persist ladder. The ladder prints the review id,
/// and exactly one submitted, approving review with that id is then recorded
/// on `target`.
fn lands_one_approving_review(repo: &PlanRepo, unit: &Value, target: &str) -> Outcome {
    let ladder = unit["persistScript"].as_str().unwrap_or_default();
    check!(
        !ladder.is_empty(),
        "{target}: a passing unit carries a persist ladder: {unit}"
    );
    check!(
        repo.reviews_on(PROJECT, target)?.is_empty(),
        "{target}: nothing is recorded until the caller runs the ladder"
    );
    let printed = repo.run_ok(&Run::bare(), ladder)?;
    let id = review_id(&printed)
        .ok_or_else(|| Failure::Check(format!("{target}: no reviewId= line: {printed}")))?;
    let reviews = repo.reviews_on(PROJECT, target)?;
    check_eq!(reviews.len(), 1, "{target}: the ladder landed one review");
    check_eq!(
        reviews[0]["id"],
        json!(id),
        "{target}: it is the review the ladder named"
    );
    check_eq!(
        reviews[0]["state"],
        json!("submitted"),
        "{target}: submitted"
    );
    check_eq!(
        reviews[0]["verdict"],
        json!("approve"),
        "{target}: approving"
    );
    Ok(())
}

fn roadmap_tags(repo: &PlanRepo) -> Result<Vec<String>, Failure> {
    Ok(strings(
        &repo.json(&[
            "roadmap",
            "show",
            "r",
            "--project",
            PROJECT,
            "--format",
            "json",
        ])?["tags"],
    ))
}

/// The tags `seed_into` gives the roadmap, `phase-1-x` and `phase-2-y`.
fn seeded_tags() -> [Vec<String>; 3] {
    [
        vec!["needs-plan-review".to_owned()],
        vec![
            "needs-plan-review".to_owned(),
            "depends-unlanded".to_owned(),
        ],
        vec![],
    ]
}

fn current_tags(repo: &PlanRepo) -> Result<[Vec<String>; 3], Failure> {
    Ok([
        roadmap_tags(repo)?,
        phase_tags(repo, "phase-1-x")?,
        phase_tags(repo, "phase-2-y")?,
    ])
}

#[test]
fn passing_unit_carries_the_verdict_and_a_persist_ladder_and_no_gate() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let phase = with(
            &axes(),
            json!({ "roadmap": "r", "phase": "phase-1-x", "priorReviews": [] }),
        );
        let phase_tagged = with(
            &phase,
            json!({ "tags": ["needs-plan-review", "depends-unlanded"] }),
        );
        let sweep = with(
            &axes(),
            json!({ "roadmap": "r", "priorReviews": [], "phases": [
                { "stem": "phase-1-x", "priorReviews": [] },
                { "stem": "phase-2-y", "priorReviews": [] }] }),
        );
        let sweep_tagged = with(
            &axes(),
            json!({ "roadmap": "r", "priorReviews": [], "tags": ["needs-plan-review"], "phases": [
                { "stem": "phase-1-x", "priorReviews": [],
                  "tags": ["needs-plan-review", "depends-unlanded"] },
                { "stem": "phase-2-y", "priorReviews": [], "tags": [] }] }),
        );

        // A single phase, driven through the lib.
        let repo = plan_repo()?;
        let tagged = drive_lib(&mut js, phase_tagged.clone(), json!([]))?;
        let plain = drive_lib(&mut js, phase.clone(), json!([]))?;
        check!(
            tagged.agent.calls().is_empty() && plain.agent.calls().is_empty(),
            "the driver dispatches no agent of its own"
        );
        let result = plain.ok()?.clone();
        check_eq!(
            tagged.ok()?,
            &result,
            "supplying the item's tags changes nothing on the result"
        );
        passed_with_no_gate("phase", &result)?;
        check_eq!(
            result["outcome"],
            json!("reviewed"),
            "the flattened verdict"
        );
        check_eq!(
            result["persistScript"],
            result["units"][0]["persistScript"],
            "the flattened ladder is the unit's"
        );
        let document = repo.phase(PROJECT, "r", "phase-1-x")?;
        lands_one_approving_review(&repo, &result, "phase/r/phase-1-x")?;
        let after = repo.phase(PROJECT, "r", "phase-1-x")?;
        for field in ["tags", "body", "status", "title"] {
            check_eq!(
                after[field],
                document[field],
                "the ladder left the phase's {field} alone"
            );
        }
        check_eq!(current_tags(&repo)?, seeded_tags(), "no tag was written");

        // A roadmap sweep: every unit, each with its own ladder.
        let repo = plan_repo()?;
        let tagged = drive_lib(&mut js, sweep_tagged.clone(), json!([]))?;
        let plain = drive_lib(&mut js, sweep.clone(), json!([]))?;
        let result = plain.ok()?.clone();
        check_eq!(
            tagged.ok()?,
            &result,
            "supplying every unit's tags changes nothing on the sweep"
        );
        passed_with_no_gate("sweep", &result)?;
        let units = result["units"].as_array().cloned().unwrap_or_default();
        check_eq!(units.len(), 3, "the roadmap body and both phases");
        for (unit, target) in
            units
                .iter()
                .zip(["roadmap/r", "phase/r/phase-1-x", "phase/r/phase-2-y"])
        {
            lands_one_approving_review(&repo, unit, target)?;
        }
        check_eq!(current_tags(&repo)?, seeded_tags(), "no tag was written");

        // The shipped engine, single phase and sweep.
        let repo = plan_repo()?;
        let (shipped_tagged, _) = drive_ship(lib, phase_tagged)?;
        let (shipped, _) = drive_ship(lib, phase)?;
        check_eq!(
            shipped_tagged,
            shipped,
            "the shipped engine ignores the tags too"
        );
        passed_with_no_gate("shipped phase", &shipped)?;
        lands_one_approving_review(&repo, &shipped, "phase/r/phase-1-x")?;
        check_eq!(current_tags(&repo)?, seeded_tags(), "no tag was written");
        let repo = plan_repo()?;
        let (swept_tagged, _) = drive_ship(lib, sweep_tagged)?;
        let (swept, _) = drive_ship(lib, sweep)?;
        check_eq!(swept_tagged, swept, "and on a sweep");
        passed_with_no_gate("shipped sweep", &swept)?;
        let units = swept["units"].as_array().cloned().unwrap_or_default();
        check_eq!(units.len(), 3, "the roadmap body and both phases");
        for (unit, target) in
            units
                .iter()
                .zip(["roadmap/r", "phase/r/phase-1-x", "phase/r/phase-2-y"])
        {
            lands_one_approving_review(&repo, unit, target)?;
        }
        check_eq!(current_tags(&repo)?, seeded_tags(), "no tag was written");
        Ok(())
    });
}

#[test]
fn rework_renders_round_note() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let d = drive_lib(
            &mut js,
            json!({ "roadmap": "r", "phase": "phase-1-x", "tags": ["needs-plan-review"] }),
            json!([blocking("ambiguous step")]),
        )?;
        let r = d.ok()?;
        check_eq!(r["outcome"], json!("rework"), "a live blocker");
        for key in GATE_KEYS {
            check!(r.get(key).is_none(), "no {key} on a rework either: {r}");
        }
        let note = r["roundNote"].as_str().unwrap_or_default();
        check!(
            note.starts_with("## Plan Review Round 1 — rework")
                && note.contains("[blocking] specification: ambiguous step"),
            "the round note is rendered from the finding and returned: {note}"
        );
        Ok(())
    });
}

fn prior(count: usize, comments: Value) -> Value {
    let reviews: Vec<Value> = (0..count)
        .map(|i| {
            json!({ "id": format!("2026-01-0{}-0000-aaaa", i + 1), "state": "submitted",
                    "created": format!("2026-01-0{}", i + 1), "comments": comments })
        })
        .collect();
    json!(reviews)
}

#[test]
fn round_from_caller_prior_reviews() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        for persist in [false, true] {
            let args = json!({ "roadmap": "r", "phase": "phase-1-x", "tags": ["needs-plan-review"],
                               "priorReviews": prior(2, json!([])), "persist": persist });
            let d = drive_lib(&mut js, args, json!([blocking("still ambiguous")]))?;
            let r = d.ok()?;
            check_eq!(
                r["units"][0]["round"],
                json!(3),
                "persist={persist}: two recorded reviews put this pass on round 3"
            );
            check_eq!(
                r["outcome"],
                json!("escalated"),
                "persist={persist}: round 3 with a live blocker escalates"
            );
        }
        Ok(())
    });
}

#[test]
fn repeat_detection_reads_prior_comments() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let finding = blocking("still ambiguous");
        // The body a prior pass's persist ladder would have written, built by
        // the real writer.
        let body = js.call("formatCommentBody", vec![finding.clone()])?;
        let d = drive_lib(
            &mut js,
            json!({ "roadmap": "r", "phase": "phase-1-x", "tags": ["needs-plan-review"],
                    "priorReviews": prior(1, json!([{ "id": 1, "body": body }])) }),
            json!([finding]),
        )?;
        let unit = &d.ok()?["units"][0];
        check_eq!(unit["round"], json!(2), "one prior review: round 2");
        let repeats: Vec<Value> = unit["repeats"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|f| f["id"].clone())
            .collect();
        check_eq!(
            repeats,
            vec![json!("c1")],
            "the live finding repeats the prior comment"
        );
        check_eq!(unit["newlyReported"], json!([]), "nothing is fresh");
        Ok(())
    });
}

#[test]
fn absent_vs_empty_prior_reviews() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let absent = drive_lib(
            &mut js,
            json!({ "roadmap": "r", "phase": "phase-1-x", "tags": ["needs-plan-review"] }),
            json!([]),
        )?;
        let u = &absent.ok()?["units"][0];
        check_eq!(
            u["roundUnknown"],
            json!(true),
            "no priorReviews: unverifiable"
        );
        check_eq!(u["round"], json!(1), "still fails toward round 1");
        check!(
            u["summary"]
                .as_str()
                .is_some_and(|s| s.contains("[round unknown: no priorReviews were supplied")),
            "a visible clause: {}",
            u["summary"]
        );
        let empty = drive_lib(
            &mut js,
            json!({ "roadmap": "r", "phase": "phase-1-x", "tags": ["needs-plan-review"], "priorReviews": [] }),
            json!([]),
        )?;
        let u = &empty.ok()?["units"][0];
        check_eq!(
            u["roundUnknown"],
            json!(false),
            "the caller looked and found none"
        );
        check_eq!(u["round"], json!(1), "round 1");
        check!(
            !u["summary"]
                .as_str()
                .unwrap_or_default()
                .contains("[round unknown:"),
            "no clause: {}",
            u["summary"]
        );
        Ok(())
    });
}

#[test]
fn persist_ladder_lands_unit_review() {
    run_real(|lib| {
        let repo = plan_repo()?;
        let mut js = Js::open(lib)?;
        let d = drive_lib(
            &mut js,
            with(
                &axes(),
                json!({ "roadmap": "r", "phase": "phase-1-x", "tags": [], "persist": true }),
            ),
            json!([]),
        )?;
        let r = d.ok()?;
        check!(d.agent.calls().is_empty(), "no persisting agent runs");
        let script = r["persistScript"].as_str().unwrap_or_default();
        check!(!script.is_empty(), "persist returns a ladder: {r}");
        check!(
            repo.reviews_on(PROJECT, "phase/r/phase-1-x")?.is_empty(),
            "the driver itself persisted nothing"
        );
        repo.run_ok(&Run::bare(), script)?;
        let reviews = repo.reviews_on(PROJECT, "phase/r/phase-1-x")?;
        check_eq!(reviews.len(), 1, "the ladder landed one review on the unit");
        check_eq!(reviews[0]["state"], json!("submitted"), "submitted");
        check_eq!(reviews[0]["target"]["kind"], json!("phase"), "on the phase");
        let off = drive_lib(
            &mut js,
            json!({ "roadmap": "r", "phase": "phase-1-x", "tags": [], "persist": false }),
            json!([]),
        )?;
        let off = off.ok()?;
        check!(
            off.get("persistCommands").is_none() && off.get("persistScript").is_none(),
            "persist off returns no ladder: {off}"
        );
        Ok(())
    });
}

#[test]
fn phase_target_persists_by_default() {
    run_real(|lib| {
        let repo = plan_repo()?;
        let mut js = Js::open(lib)?;
        let d = drive_lib(
            &mut js,
            with(
                &axes(),
                json!({ "roadmap": "r", "phase": "phase-1-x", "tags": [] }),
            ),
            json!([blocking("the phase is ambiguous")]),
        )?;
        let r = d.ok()?;
        let unit = &r["units"][0];
        check_eq!(
            unit["findings"].as_array().map(Vec::len),
            Some(1),
            "the unit produced a finding"
        );
        let script = unit["persistScript"].as_str().unwrap_or_default();
        check!(
            !script.is_empty(),
            "with no persist argument the unit carries a ladder: {unit}"
        );
        repo.run_ok(&Run::bare(), script)?;
        let reviews = repo.reviews_on(PROJECT, "phase/r/phase-1-x")?;
        check_eq!(
            reviews.len(),
            1,
            "the ladder landed one review on the phase"
        );
        check_eq!(reviews[0]["state"], json!("submitted"), "submitted");

        let swept = drive_lib(
            &mut js,
            with(
                &axes(),
                json!({ "roadmap": "r", "phases": [{ "stem": "phase-1-x", "tags": [] }], "tags": [] }),
            ),
            json!([blocking("the phase is ambiguous")]),
        )?;
        let units = swept.ok()?["units"].as_array().cloned().unwrap_or_default();
        check_eq!(units.len(), 2, "the roadmap and its phase");
        for u in units {
            check!(
                u["persistScript"].as_str().is_some_and(|s| !s.is_empty()),
                "every swept unit carries a ladder: {u}"
            );
        }
        Ok(())
    });
}

#[test]
fn wont_fix_texts_suppress_matching_finding() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let dismissed = "the plan omits regenerating the checked-in plugin tree entirely";
        let f = json!({ "id": "af-2", "concern": "architectural-fit", "severity": "blocking", "confidence": 90,
                        "what_fails": dismissed });
        let suppressed = drive_lib(
            &mut js,
            json!({ "roadmap": "r", "phase": "phase-1-x", "tags": [], "wontFixedTexts": [dismissed] }),
            json!([f.clone()]),
        )?;
        let s = suppressed.ok()?;
        check_eq!(s["findings"], json!([]), "suppressed");
        check_eq!(s["outcome"], json!("reviewed"), "so the unit is clean");
        let kept = drive_lib(
            &mut js,
            json!({ "roadmap": "r", "phase": "phase-1-x", "tags": [] }),
            json!([f]),
        )?;
        check_eq!(
            kept.ok()?["findings"].as_array().map(Vec::len),
            Some(1),
            "without the list it is reported"
        );
        Ok(())
    });
}

#[test]
fn build_review_units_is_pure() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let built = js.plan_call(
            "buildReviewUnits",
            vec![json!({ "kind": "roadmap", "roadmap": "r", "phases": [{ "stem": "phase-1-a", "tags": ["x"] }],
                         "tags": ["y"] })],
        )?;
        let units = built["units"].as_array().cloned().unwrap_or_default();
        check_eq!(units.len(), 2, "the roadmap and its phase");
        check_eq!(units[0]["kind"], json!("roadmap"), "roadmap first");
        check_eq!(units[1]["ident"], json!("phase-1-a"), "then the phase");
        check_eq!(built["skippedPhases"], json!([]), "nothing skipped");
        check!(
            units
                .iter()
                .all(|u| u.get("body").is_none() && u.get("tags").is_none()),
            "no unit carries a document body or a tag list: {units:?}"
        );
        Ok(())
    });
}

// --- A2: line spans come only from the unit's own document -------------------------

#[test]
fn every_item_command_prints_its_own_document_numbered() {
    run_real(|lib| {
        let repo = plan_repo()?;
        let mut js = Js::open(lib)?;
        let runs = [
            (
                json!({ "roadmap": "r", "phase": "phase-1-x", "tags": [] }),
                vec![vec!["phase", "show", "phase-1-x", "--roadmap", "r"]],
            ),
            (
                json!({ "task": "t", "tags": [] }),
                vec![vec!["task", "show", "t"]],
            ),
            (
                json!({ "roadmap": "r", "phases": [{ "stem": "phase-1-x" }, { "stem": "phase-2-y" }], "tags": [] }),
                vec![
                    vec!["roadmap", "show", "r"],
                    vec!["phase", "show", "phase-1-x", "--roadmap", "r"],
                    vec!["phase", "show", "phase-2-y", "--roadmap", "r"],
                ],
            ),
            (
                json!({ "implementationPlan": true, "planSlug": "p", "roadmap": "r" }),
                vec![vec!["plan", "show", "p"]],
            ),
        ];
        for (args, docs) in runs {
            let d = drive_lib(&mut js, with(&axes(), args.clone()), json!([]))?;
            d.ok()?;
            check_eq!(d.contexts.len(), docs.len(), "{args}: one context per unit");
            for (ctx, doc) in d.contexts.iter().zip(docs) {
                check_eq!(
                    read_text(&repo, &ctx["itemCommand"])?,
                    numbered(&repo, &doc)?,
                    "{args}: {} reads its own document, numbered",
                    ctx["target"]
                );
                if !ctx["roadmapCommand"].is_null() {
                    let roadmap = read(&repo, &Run::bare(), &ctx["roadmapCommand"])?;
                    check_eq!(
                        roadmap["slug"],
                        json!("r"),
                        "{args}: {} reads its parent roadmap as JSON, un-numbered",
                        ctx["target"]
                    );
                }
            }
            if args.get("phases").is_some() {
                check_eq!(
                    d.contexts[0]["roadmapCommand"],
                    Value::Null,
                    "the roadmap unit reads its own document once, not twice"
                );
            }
        }
        Ok(())
    });
}

#[test]
fn sweep_phase_unit_span_snippet_comes_from_the_phase() {
    run_real(|lib| {
        let repo = plan_repo()?;
        let mut js = Js::open(lib)?;
        let spanned = json!([{ "id": "s1", "concern": "specification", "severity": "blocking", "confidence": 90,
                               "what_fails": "line one is wrong", "start_line": 1, "end_line": 1 }]);
        let d = drive_lib(
            &mut js,
            with(
                &axes(),
                json!({ "roadmap": "r", "phases": [{ "stem": "phase-1-x", "tags": [] }], "tags": [], "persist": true }),
            ),
            spanned,
        )?;
        let r = d.ok()?;
        let units = r["units"].as_array().cloned().unwrap_or_default();
        let phase_unit = units
            .iter()
            .find(|u| u["ident"] == json!("phase-1-x"))
            .ok_or_else(|| Failure::Check(format!("no phase unit: {r}")))?;
        repo.run_ok(&Run::bare(), &script(&phase_unit["persistCommands"]))?;
        let reviews = repo.reviews_on(PROJECT, "phase/r/phase-1-x")?;
        check_eq!(reviews.len(), 1, "the phase unit's review landed");
        let c = &reviews[0]["comments"][0];
        let phase_line = numbered_lines(&numbered(
            &repo,
            &["phase", "show", "phase-1-x", "--roadmap", "r"],
        )?)[0]
            .clone();
        let roadmap_line = numbered_lines(&numbered(&repo, &["roadmap", "show", "r"])?)[0].clone();
        check_eq!(
            c["anchor"]["snippet"],
            json!(phase_line),
            "the snippet is line 1 of the phase body"
        );
        check!(
            phase_line != roadmap_line,
            "and not line 1 of the roadmap body ({roadmap_line:?})"
        );
        Ok(())
    });
}

#[test]
fn plan_review_persists_line_range_anchors_end_to_end() {
    run_real(|lib| {
        let repo = plan_repo()?;
        repo.seed(&[&[
            "phase",
            "create",
            "z",
            "--title",
            "Z",
            "--number",
            "3",
            "--body",
            "## Steps\n\n1. Add the retry loop.\n2. Back off by an unspecified amount,\n   wrapped onto a second line.\n\n## Acceptance Criteria\n\n- [ ] It retries.",
            "--no-edit",
            "--roadmap",
            "r",
            "--project",
            PROJECT,
        ]])?;
        let agent = Agent::scripted(|call| {
            if call.label.starts_with("find:") {
                let findings = if call.label.contains(":specification") {
                    json!([{ "id": "backoff", "concern": "specification", "severity": "blocking", "confidence": 90,
                             "what_fails": "the backoff is unspecified", "start_line": 4, "end_line": 5 }])
                } else if call.label.contains(":scope") {
                    json!([{ "id": "retry", "concern": "scope", "severity": "concern", "confidence": 85,
                             "what_fails": "the loop is a separable outcome", "start_line": 3 }])
                } else {
                    json!([])
                };
                Reply::Value(json!({ "findings": findings }))
            } else if call.label.starts_with("refute:") {
                Reply::Value(json!({ "refuted": false, "confidence": 90, "lines_ok": true }))
            } else {
                Reply::Null
            }
        });
        let mut js = Js::open(lib)?;
        let (r, _) = drive_real(
            &mut js,
            with(
                &axes(),
                json!({ "roadmap": "r", "phase": "phase-3-z", "tags": [], "persist": true }),
            ),
            agent,
        )?;
        let out = repo.run_ok(
            &Run::bare(),
            r["persistScript"].as_str().unwrap_or_default(),
        )?;
        check!(
            out.lines().any(|l| l == "anchorsDegraded=none"),
            "every requested anchor landed: {out}"
        );
        let lines = numbered_lines(&numbered(
            &repo,
            &["phase", "show", "phase-3-z", "--roadmap", "r"],
        )?);
        let reviews = repo.reviews_on(PROJECT, "phase/r/phase-3-z")?;
        check_eq!(reviews.len(), 1, "one review on the phase");
        let cs = reviews[0]["comments"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        check_eq!(
            cs.len(),
            2,
            "one comment per spanned finding: {}",
            reviews[0]
        );
        for (id, start, end) in [("backoff", 4, 5), ("retry", 3, 3)] {
            let c = cs
                .iter()
                .find(|c| {
                    c["body"]
                        .as_str()
                        .is_some_and(|b| b.contains(&format!("finding-id: {id}")))
                })
                .ok_or_else(|| Failure::Check(format!("no comment for {id}: {}", reviews[0])))?;
            check_eq!(
                c["anchor"]["anchor_type"],
                json!("line-range"),
                "{id}: a line-range anchor"
            );
            check_eq!(
                (
                    c["anchor"]["start_line"].clone(),
                    c["anchor"]["end_line"].clone()
                ),
                (json!(start), json!(end)),
                "{id}: the requested span"
            );
            check_eq!(
                c["anchor"]["snippet"],
                json!(lines[start - 1..end].join("\n")),
                "{id}: the snippet rdm captured is those numbered lines"
            );
            let h = js.call("parseCommentHeader", vec![c["body"].clone()])?;
            check_eq!(
                h["anchor"],
                json!("line"),
                "{id}: the header reads back as a line anchor"
            );
        }
        Ok(())
    });
}

// --- B: the shipped engine ----------------------------------------------------------

#[test]
fn shipped_engine_dispatches_only_finders_and_refuters() {
    run_real(|lib| {
        let (r, _) = drive_ship(
            lib,
            json!({ "roadmap": "r", "phase": "phase-1-x", "tags": [] }),
        )?;
        check_eq!(
            r["kind"],
            json!("phase"),
            "the engine compiles, runs and reports"
        );
        for args in [
            json!({ "roadmap": "r", "phase": "phase-1-x", "tags": [] }),
            json!({ "task": "t", "tags": [] }),
            json!({ "roadmap": "r", "phases": [{ "stem": "phase-1-x" }], "tags": [] }),
            json!({ "implementationPlan": true, "planSlug": "p", "persist": true }),
        ] {
            let (_, agent) = drive_ship(lib, args.clone())?;
            let labels = agent.labels();
            check!(!labels.is_empty(), "{args}: no agent dispatched");
            check!(
                judgment_only(&labels),
                "{args}: non-judgment agent(s): {labels:?}"
            );
        }
        Ok(())
    });
}

#[test]
fn shipped_engine_string_payload_equivalent() {
    run_real(|lib| {
        let payload =
            json!({ "roadmap": "r", "phase": "phase-1-x", "tags": ["needs-plan-review"] });
        let (obj, _) = drive_ship(lib, payload.clone())?;
        let (s, agent) = drive_ship(lib, json!(payload.to_string()))?;
        check_eq!(obj["outcome"], json!("reviewed"), "the object payload");
        check!(
            obj["persistCommands"]
                .as_array()
                .is_some_and(|c| !c.is_empty()),
            "it carries a persist ladder: {obj}"
        );
        check_eq!(
            (&s["outcome"], &s["persistCommands"]),
            (&obj["outcome"], &obj["persistCommands"]),
            "a JSON-string payload behaves identically"
        );
        check!(judgment_only(&agent.labels()), "judgment agents only");
        Ok(())
    });
}

// --- C: the implementation-plan path -------------------------------------------------

#[test]
fn implementation_plan_grades_plan_by_slug() {
    run_real(|lib| {
        let repo = plan_repo()?;
        let mut js = Js::open(lib)?;
        let d = drive_lib(
            &mut js,
            with(
                &axes(),
                json!({ "implementationPlan": true, "planSlug": "p", "persist": true }),
            ),
            json!([]),
        )?;
        d.ok()?;
        check_eq!(d.contexts.len(), 1, "one unit");
        check_eq!(
            d.contexts[0]["target"],
            json!("plan/p"),
            "the plan, by slug"
        );
        check_eq!(
            read_text(&repo, &d.contexts[0]["itemCommand"])?,
            numbered(&repo, &["plan", "show", "p"])?,
            "the item command prints the plan, numbered"
        );
        check!(d.agent.calls().is_empty(), "no agent of the driver's own");
        Ok(())
    });
}

#[test]
fn implementation_plan_names_the_item_it_implements() {
    run_real(|lib| {
        let repo = plan_repo()?;
        let mut js = Js::open(lib)?;
        let run = Run::bare();
        let plan = json!({ "implementationPlan": true, "planSlug": "p" });

        let phase = drive_lib(
            &mut js,
            with(
                &with(&axes(), plan.clone()),
                json!({ "roadmap": "r", "phase": "phase-1-x" }),
            ),
            json!([]),
        )?;
        phase.ok()?;
        check_eq!(
            phase.contexts[0]["targetKind"],
            json!("implementation-plan"),
            "the driver states the document kind under review"
        );
        check_eq!(
            read(&repo, &run, &phase.contexts[0]["implementsCommand"])?,
            repo.phase(PROJECT, "r", "phase-1-x")?,
            "the item command prints the phase the plan implements"
        );
        check_eq!(
            read(&repo, &run, &phase.contexts[0]["roadmapCommand"])?["slug"],
            json!("r"),
            "and the parent roadmap is still named beside it"
        );

        let task = drive_lib(
            &mut js,
            with(&with(&axes(), plan.clone()), json!({ "task": "t" })),
            json!([]),
        )?;
        task.ok()?;
        check_eq!(
            read(&repo, &run, &task.contexts[0]["implementsCommand"])?,
            repo.task(PROJECT, "t")?,
            "the task form prints the task"
        );

        let unnamed = drive_lib(&mut js, with(&axes(), plan), json!([]))?;
        unnamed.ok()?;
        check_eq!(
            unnamed.contexts[0]["implementsCommand"],
            Value::Null,
            "with no item named there is no item command"
        );
        Ok(())
    });
}

#[test]
fn item_command_reaches_finders_only_when_an_item_is_named() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let plan = json!({ "implementationPlan": true, "planSlug": "p" });
        let finder_prompts = |agent: &Agent| -> Vec<(String, String)> {
            agent
                .calls_with("find:")
                .into_iter()
                .map(|c| (c.label, c.prompt))
                .collect()
        };

        // With an item named, every finder is handed the command that reads it.
        let named = with(&plan, json!({ "task": "t" }));
        let lib_run = drive_lib(&mut js, named.clone(), json!([]))?;
        let command = lib_run.contexts[0]["implementsCommand"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        check!(!command.is_empty(), "the driver built an item command");
        let (_, agent) = drive_real(&mut js, named, inert())?;
        let with_item = finder_prompts(&agent);
        check!(!with_item.is_empty(), "finders ran");
        for (label, prompt) in &with_item {
            check!(
                prompt.contains(&command),
                "{label} is handed the item command:\n{prompt}"
            );
        }

        // With none, each finder's prompt is exactly the prompt of a context
        // that has no such key at all.
        let bare = drive_lib(&mut js, plan.clone(), json!([]))?;
        let ctx = json!({ "target": bare.contexts[0]["target"],
                          "itemCommand": bare.contexts[0]["itemCommand"] });
        let (_, agent) = drive_real(&mut js, plan, inert())?;
        let without_item = finder_prompts(&agent);
        check_eq!(
            without_item.len(),
            with_item.len(),
            "the same finders run either way"
        );
        let dims = js.get("DIMENSIONS")?;
        for (label, prompt) in &without_item {
            let key = label.rsplit(':').next().unwrap_or_default();
            let dim = dims["plan"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|d| d["key"] == json!(key))
                .cloned()
                .ok_or_else(|| Failure::Check(format!("{label}: not a plan reviewer")))?;
            check_eq!(
                json!(prompt),
                js.call("findPrompt", vec![json!("plan"), dim, ctx.clone()])?,
                "{label}: no item named leaves the prompt untouched"
            );
        }
        Ok(())
    });
}

#[test]
fn plan_slug_persists_to_plan_target() {
    run_real(|lib| {
        let repo = plan_repo()?;
        let mut js = Js::open(lib)?;
        let d = drive_lib(
            &mut js,
            with(
                &axes(),
                json!({ "implementationPlan": true, "planSlug": "p", "persist": { "on": "plan/p" } }),
            ),
            json!([]),
        )?;
        let r = d.ok()?;
        check_eq!(r["planSlug"], json!("p"), "the slug is reported");
        repo.run_ok(
            &Run::bare(),
            r["persistScript"].as_str().unwrap_or_default(),
        )?;
        let reviews = repo.reviews_on(PROJECT, "plan/p")?;
        check_eq!(reviews.len(), 1, "the ladder landed a review on plan/p");
        check_eq!(reviews[0]["state"], json!("submitted"), "submitted");

        let defaulted = drive_lib(
            &mut js,
            with(
                &axes(),
                json!({ "implementationPlan": true, "planSlug": "p" }),
            ),
            json!([]),
        )?;
        let dr = defaulted.ok()?;
        let script = dr["persistScript"].as_str().unwrap_or_default();
        check!(
            !script.is_empty(),
            "a slug-named plan with no persist argument still returns a ladder: {dr}"
        );
        repo.run_ok(&Run::bare(), script)?;
        let reviews = repo.reviews_on(PROJECT, "plan/p")?;
        check_eq!(
            reviews.len(),
            2,
            "the default ladder landed a second review on plan/p"
        );

        let free = drive_lib(
            &mut js,
            json!({ "implementationPlan": true, "planFile": "/tmp/loose-plan.md", "persist": { "on": "plan/p" } }),
            json!([]),
        )?;
        let f = free.ok()?;
        check!(
            f.get("planSlug").is_none() && f.get("persistCommands").is_none(),
            "a free-form file has no persisted target: {f}"
        );
        check!(
            free.agent
                .logs()
                .iter()
                .any(|m| m.contains("persist ignored")),
            "and says so: {:?}",
            free.agent.logs()
        );

        let unasked = drive_lib(
            &mut js,
            json!({ "implementationPlan": true, "planFile": "/tmp/loose-plan.md" }),
            json!([]),
        )?;
        let u = unasked.ok()?;
        check!(
            u.get("persistCommands").is_none() && u.get("persistScript").is_none(),
            "a free-form file with no persist argument returns no ladder: {u}"
        );
        check!(
            !unasked
                .agent
                .logs()
                .iter()
                .any(|m| m.contains("persist ignored")),
            "and logs nothing, because nothing was asked for: {:?}",
            unasked.agent.logs()
        );
        Ok(())
    });
}

#[test]
fn free_form_plan_read_by_quoted_path() {
    run_real(|lib| {
        let repo = plan_repo()?;
        let mut js = Js::open(lib)?;
        let dir = repo.tmpdir("plans")?;
        let path = dir.join("an odd'name.md");
        let content = "# A free-form plan\n\nWith $HOME and `ticks` kept literal.\n";
        std::fs::write(&path, content).map_err(infra)?;
        let path = path.to_string_lossy().into_owned();
        let d = drive_lib(
            &mut js,
            json!({ "implementationPlan": true, "planFile": path }),
            json!([]),
        )?;
        let r = d.ok()?;
        check_eq!(d.contexts.len(), 1, "one unit");
        check_eq!(
            d.contexts[0]["target"],
            json!(format!("the implementation plan at {path}")),
            "graded from its path"
        );
        let cmd = d.contexts[0]["itemCommand"].as_str().unwrap_or_default();
        let out = repo.run_ok(&Run::bare(), cmd)?;
        let want: String = content
            .lines()
            .enumerate()
            .map(|(i, l)| format!("{:>6}\t{l}\n", i + 1))
            .collect();
        check_eq!(
            out,
            want,
            "the returned read command prints the file line-numbered, quoting intact"
        );
        check_eq!(r["planFile"], json!(path), "the path is reported");
        check!(d.agent.calls().is_empty(), "no agent of the driver's own");
        Ok(())
    });
}

#[test]
fn free_form_path_reaches_every_reviewer() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let path = "/tmp/sentinel-free-form-plan-51c2.md";
        let (_, agent) = drive_real(
            &mut js,
            json!({ "implementationPlan": true, "planFile": path }),
            inert(),
        )?;
        let calls = agent.calls();
        check!(!calls.is_empty(), "reviewers ran");
        for c in &calls {
            check!(
                c.prompt.contains(path),
                "{} is told where the plan is",
                c.label
            );
        }
        Ok(())
    });
}

#[test]
fn implementation_plan_without_document_refused_before_agents() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        for args in [
            json!({ "implementationPlan": true }),
            json!("--implementation-plan"),
        ] {
            let r = js.plan_try_call("parsePlanArgs", vec![args.clone()])?;
            check!(rejects_with(&r, "names no document"), "{args}: {r:?}");
        }
        let d = drive_lib(&mut js, json!({ "implementationPlan": true }), json!([]))?;
        check!(
            rejects_with(&d.result, "names no document"),
            "the driver refuses: {:?}",
            d.result
        );
        check!(d.agent.calls().is_empty(), "before a single agent");
        check!(d.contexts.is_empty(), "and before any review");
        Ok(())
    });
}

#[test]
fn plan_naming_modes_mutually_exclusive() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let r = js.plan_try_call(
            "parsePlanArgs",
            vec![json!({ "implementationPlan": true, "planSlug": "p", "planFile": "/tmp/p.md" })],
        )?;
        check!(rejects_with(&r, "both name the plan under review"), "{r:?}");
        let r = js.plan_try_call(
            "parsePlanArgs",
            vec![json!({ "roadmap": "r", "planFile": "/tmp/p.md" })],
        )?;
        check!(rejects_with(&r, "requires --implementation-plan"), "{r:?}");
        let ok = js.plan_call(
            "parsePlanArgs",
            vec![json!({ "target": "--implementation-plan", "planFile": "/tmp/p.md" })],
        )?;
        check_eq!(
            ok["planFile"],
            json!("/tmp/p.md"),
            "the structured key names the file"
        );
        let r = js.plan_try_call(
            "parsePlanArgs",
            vec![json!("--implementation-plan --planFile /tmp/p.md")],
        )?;
        check!(
            rejects_with(&r, "names no document"),
            "a positional string can never name a file to read: {r:?}"
        );
        Ok(())
    });
}

#[test]
fn implementation_plan_is_report_only() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let d = drive_lib(
            &mut js,
            json!({ "implementationPlan": true, "planSlug": "p", "persist": true }),
            json!([]),
        )?;
        let r = d.ok()?;
        for key in ["gateAction", "gatePendingCount", "units", "roundNote"] {
            check!(r.get(key).is_none(), "no {key}: {r}");
        }
        check_eq!(r["kind"], json!("implementation-plan"), "kind");
        check!(r["summary"].is_string(), "a summary");
        Ok(())
    });
}

#[test]
fn parse_plan_args_refusals() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        for (args, needle) in [
            (json!({}), "no target"),
            (json!(""), "no target"),
            (
                json!({ "roadmap": "r", "phase": "phase-1-x", "planSlug": "p" }),
                "requires --implementation-plan",
            ),
            (
                json!({ "implementationPlan": true, "planSlug": "p", "persist": { "on": "plan/other" } }),
                "disagrees with planSlug",
            ),
            (
                json!({ "task": "t", "persist": "yes" }),
                "persist must be omitted",
            ),
        ] {
            let r = js.plan_try_call("parsePlanArgs", vec![args.clone()])?;
            check!(
                rejects_with(&r, needle),
                "{args}: expected /{needle}/, got {r:?}"
            );
        }
        let ok = js.plan_call(
            "parsePlanArgs",
            vec![json!({ "implementationPlan": true, "planSlug": "p", "persist": true })],
        )?;
        check_eq!(
            ok["kind"],
            json!("implementation-plan"),
            "a legal planSlug parses"
        );
        check_eq!(ok["planSlug"], json!("p"), "slug");
        Ok(())
    });
}

#[test]
fn retired_transport_keys_not_parsed() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let parsed = js.plan_call(
            "parsePlanArgs",
            vec![json!({ "implementationPlan": true, "planSlug": "p", "planText": "a body that must not be graded",
                         "fetched": { "body": "x", "tags": [] }, "mechanicalModel": "m", "gateMode": "apply",
                         "tags": ["needs-plan-review"] })],
        )?;
        for gone in ["planText", "fetched", "mechanicalModel", "gateMode", "tags"] {
            check!(
                parsed.get(gone).is_none(),
                "{gone} is not carried: {parsed}"
            );
        }
        Ok(())
    });
}

// --- D: the reviewer set — by target type, or the caller's list ----------------------

/// The item-document reviewers a phase or a task gets by default.
const ITEM_SET: [&str; 3] = ["scope", "specification", "acceptance-criteria"];
/// The implementation-plan reviewers.
const PLAN_SET: [&str; 3] = ["code-change", "confirmation", "architectural-fit"];

/// `resolveReviewers('plan', set, target)`, as reviewer keys.
fn resolved_for(
    js: &mut Js,
    set: Value,
    target: Value,
) -> Result<Result<Vec<String>, JsError>, Failure> {
    Ok(js
        .try_call("resolveReviewers", vec![json!("plan"), set, target])?
        .map(|v| {
            v.as_array()
                .into_iter()
                .flatten()
                .filter_map(|d| d["key"].as_str().map(str::to_owned))
                .collect()
        }))
}

/// The default set for `target`.
fn default_for(js: &mut Js, target: Value) -> Result<Vec<String>, Failure> {
    resolved_for(js, Value::Null, target)?.map_err(Failure::Js)
}

/// `coverage.selected` of each reported unit, by `kind/ident`.
fn selected_by_unit(result: &Value) -> Vec<(String, Vec<String>)> {
    result["units"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|u| {
            (
                format!(
                    "{}/{}",
                    u["kind"].as_str().unwrap_or_default(),
                    u["ident"].as_str().unwrap_or_default()
                ),
                strings(&u["coverage"]["selected"]),
            )
        })
        .collect()
}

fn owned(keys: &[&str]) -> Vec<String> {
    keys.iter().map(|k| (*k).to_owned()).collect()
}

#[test]
fn item_targets_select_scope_specification_and_criteria() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let with_intent: Vec<String> = ITEM_SET
            .iter()
            .chain(["intent-alignment"].iter())
            .map(|k| (*k).to_owned())
            .collect();
        for kind in ["phase", "task"] {
            check_eq!(
                default_for(&mut js, json!({ "kind": kind }))?,
                owned(&ITEM_SET),
                "{kind}: the item-document set"
            );
        }
        check_eq!(
            default_for(&mut js, json!({ "kind": "phase", "intentRecorded": true }))?,
            with_intent,
            "a phase whose roadmap records an Intent adds intent alignment"
        );
        check_eq!(
            default_for(&mut js, json!({ "kind": "task", "intentRecorded": true }))?,
            owned(&ITEM_SET),
            "a task has no parent roadmap, so the flag adds nothing"
        );
        for not_true in [json!(false), json!("true"), json!(1), Value::Null] {
            check_eq!(
                default_for(
                    &mut js,
                    json!({ "kind": "phase", "intentRecorded": not_true })
                )?,
                owned(&ITEM_SET),
                "only a literal true states a recorded Intent ({not_true})"
            );
        }

        // Through the driver: a single phase and a task each run that set.
        let (phase, agent) = drive_real(
            &mut js,
            json!({ "roadmap": "r", "phase": "phase-1-x", "tags": [], "intentRecorded": true }),
            inert(),
        )?;
        check_eq!(
            strings(&phase["coverage"]["selected"]),
            with_intent,
            "a phase target with a recorded Intent"
        );
        check_eq!(
            agent.calls_with("find:").len(),
            with_intent.len(),
            "one finder per selected reviewer"
        );
        let (task, _) = drive_real(
            &mut js,
            json!({ "task": "t", "tags": [], "intentRecorded": true }),
            inert(),
        )?;
        check_eq!(
            strings(&task["coverage"]["selected"]),
            owned(&ITEM_SET),
            "a task target"
        );
        Ok(())
    });
}

#[test]
fn roadmap_target_reviews_body_and_phases_with_their_own_sets() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        check_eq!(
            default_for(&mut js, json!({ "kind": "roadmap" }))?,
            owned(&["specification"]),
            "the roadmap body gets the specification reviewer alone"
        );
        check_eq!(
            default_for(
                &mut js,
                json!({ "kind": "roadmap", "intentRecorded": true })
            )?,
            owned(&["specification"]),
            "whatever the caller says about its Intent"
        );

        let sweep = json!({ "roadmap": "r", "phases": [{ "stem": "phase-1-x" }, { "stem": "phase-2-y" }], "tags": [] });
        let (r, _) = drive_real(&mut js, sweep.clone(), inert())?;
        check_eq!(
            selected_by_unit(&r),
            vec![
                ("roadmap/r".to_owned(), owned(&["specification"])),
                ("phase/phase-1-x".to_owned(), owned(&ITEM_SET)),
                ("phase/phase-2-y".to_owned(), owned(&ITEM_SET)),
            ],
            "the body gets specification; each phase gets the item-document set"
        );

        let (r, _) = drive_real(
            &mut js,
            with(&sweep, json!({ "intentRecorded": true })),
            inert(),
        )?;
        let with_intent: Vec<String> = ITEM_SET
            .iter()
            .chain(["intent-alignment"].iter())
            .map(|k| (*k).to_owned())
            .collect();
        check_eq!(
            selected_by_unit(&r),
            vec![
                ("roadmap/r".to_owned(), owned(&["specification"])),
                ("phase/phase-1-x".to_owned(), with_intent.clone()),
                ("phase/phase-2-y".to_owned(), with_intent),
            ],
            "a recorded Intent adds intent alignment to each phase, not to the body"
        );
        Ok(())
    });
}

#[test]
fn implementation_plan_target_selects_its_own_rubric() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let plan_set = default_for(&mut js, json!({ "kind": "implementation-plan" }))?;
        check_eq!(
            plan_set,
            owned(&PLAN_SET),
            "code change, confirmation, architectural fit"
        );
        let item_set = default_for(&mut js, json!({ "kind": "phase", "intentRecorded": true }))?;
        for key in &plan_set {
            check!(
                !item_set.contains(key),
                "{key} is not an item-document reviewer"
            );
        }
        check_eq!(
            default_for(
                &mut js,
                json!({ "kind": "implementation-plan", "intentRecorded": true })
            )?,
            owned(&PLAN_SET),
            "a recorded Intent adds nothing to an implementation plan"
        );

        let (r, agent) = drive_real(
            &mut js,
            json!({ "implementationPlan": true, "planSlug": "p", "roadmap": "r", "phase": "phase-1-x",
                    "intentRecorded": true }),
            inert(),
        )?;
        let finders: Vec<String> = agent
            .calls_with("find:")
            .into_iter()
            .map(|c| c.label)
            .collect();
        check_eq!(
            finders,
            PLAN_SET
                .iter()
                .map(|k| format!("find:plan:{k}"))
                .collect::<Vec<_>>(),
            "exactly the three implementation-plan finders"
        );
        check_eq!(
            strings(&r["coverage"]["selected"]),
            owned(&PLAN_SET),
            "and the result says so"
        );
        Ok(())
    });
}

#[test]
fn explicit_list_is_exact_whatever_the_target() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        check_eq!(
            resolved_for(
                &mut js,
                json!(["scope"]),
                json!({ "kind": "implementation-plan" })
            )?
            .map_err(Failure::Js)?,
            owned(&["scope"]),
            "a named reviewer runs even outside the target's rubric"
        );
        check_eq!(
            resolved_for(
                &mut js,
                json!(["architectural-fit", "scope"]),
                json!({ "kind": "roadmap" })
            )?
            .map_err(Failure::Js)?,
            owned(&["scope", "architectural-fit"]),
            "out of the whole catalogue, in declaration order"
        );
        for set in [Value::Null, json!(["scope"])] {
            let r = resolved_for(&mut js, set.clone(), json!({ "kind": "plan" }))?;
            check!(
                r.as_ref()
                    .is_err_and(|e| e.message.contains("unknown plan review target kind")),
                "{set}: an unrecognised kind is refused: {r:?}"
            );
        }
        let all = keys(&mut js, "plan")?;
        for target in [Host::undefined(), Value::Null, json!({})] {
            check_eq!(
                resolved_for(&mut js, Value::Null, target.clone())?.map_err(Failure::Js)?,
                all,
                "{target}: no kind stated runs the whole catalogue"
            );
        }
        let code = keys(&mut js, "code")?;
        let r = js.call(
            "resolveReviewers",
            vec![json!("code"), Value::Null, json!({ "kind": "nonsense" })],
        )?;
        check_eq!(
            r.as_array().map(Vec::len),
            Some(code.len()),
            "code mode ignores the target"
        );

        // Through the driver, one list still applies to every unit of a sweep.
        let (r, _) = drive_real(
            &mut js,
            json!({ "roadmap": "r", "phases": [{ "stem": "phase-1-x" }], "tags": [],
                    "reviewers": ["scope"], "intentRecorded": true }),
            inert(),
        )?;
        check_eq!(
            selected_by_unit(&r),
            vec![
                ("roadmap/r".to_owned(), owned(&["scope"])),
                ("phase/phase-1-x".to_owned(), owned(&["scope"])),
            ],
            "the caller's list overrides the selection on the body and the phase alike"
        );
        Ok(())
    });
}

#[test]
fn driver_states_each_units_kind_and_intent() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let stated = |d: &Driven| -> Vec<(Value, Value, Value)> {
            d.contexts
                .iter()
                .map(|c| {
                    (
                        c["target"].clone(),
                        c["targetKind"].clone(),
                        c["intentRecorded"].clone(),
                    )
                })
                .collect()
        };
        let sweep = drive_lib(
            &mut js,
            json!({ "roadmap": "r", "phases": [{ "stem": "phase-1-x" }], "tags": [], "intentRecorded": true }),
            json!([]),
        )?;
        check_eq!(
            stated(&sweep),
            vec![
                (json!("roadmap/r"), json!("roadmap"), json!(false)),
                (json!("phase/r/phase-1-x"), json!("phase"), json!(true)),
            ],
            "the body is a roadmap with no parent; the phase has one"
        );
        let task = drive_lib(
            &mut js,
            json!({ "task": "t", "tags": [], "intentRecorded": true }),
            json!([]),
        )?;
        check_eq!(
            stated(&task),
            vec![(json!("task/t"), json!("task"), json!(false))],
            "a task has no parent roadmap"
        );
        let unstated = drive_lib(
            &mut js,
            json!({ "roadmap": "r", "phase": "phase-1-x", "tags": [] }),
            json!([]),
        )?;
        check_eq!(
            stated(&unstated),
            vec![(json!("phase/r/phase-1-x"), json!("phase"), json!(false))],
            "an omitted flag states no Intent"
        );
        let positional = js.plan_call(
            "parsePlanArgs",
            vec![json!("r phase-1-x --intentRecorded true")],
        )?;
        check_eq!(
            positional["intentRecorded"],
            json!(false),
            "the flag is read from the structured key only"
        );
        Ok(())
    });
}

fn keys(js: &mut Js, mode: &str) -> Result<Vec<String>, Failure> {
    let dims = js.get("DIMENSIONS")?;
    Ok(dims[mode]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|d| d["key"].as_str().map(str::to_owned))
        .collect())
}

fn resolved(js: &mut Js, mode: &str, set: Value) -> Result<Result<Vec<String>, JsError>, Failure> {
    Ok(js
        .try_call("resolveReviewers", vec![json!(mode), set])?
        .map(|v| {
            v.as_array()
                .into_iter()
                .flatten()
                .filter_map(|d| d["key"].as_str().map(str::to_owned))
                .collect()
        }))
}

#[test]
fn reviewer_set_absent_runs_all() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        for mode in ["plan", "code"] {
            let all = keys(&mut js, mode)?;
            check!(!all.is_empty(), "{mode} has reviewers");
            for set in [Host::undefined(), Value::Null] {
                check_eq!(
                    resolved(&mut js, mode, set.clone())?.map_err(Failure::Js)?,
                    all,
                    "{mode}: {set} runs every reviewer"
                );
            }
        }
        Ok(())
    });
}

#[test]
fn reviewer_set_declaration_order() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let plan = keys(&mut js, "plan")?;
        let code = keys(&mut js, "code")?;
        let picked = json!([plan[2], plan[0]]);
        check_eq!(
            resolved(&mut js, "plan", picked)?.map_err(Failure::Js)?,
            vec![plan[0].clone(), plan[2].clone()],
            "declaration order, never the caller's"
        );
        check_eq!(
            resolved(&mut js, "code", json!([code[1]]))?.map_err(Failure::Js)?,
            vec![code[1].clone()],
            "exactly the one named"
        );
        check_eq!(
            resolved(&mut js, "code", json!(code[1]))?.map_err(Failure::Js)?,
            vec![code[1].clone()],
            "a bare string is a one-element set"
        );
        Ok(())
    });
}

#[test]
fn reviewer_set_unknown_dropped() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let plan = keys(&mut js, "plan")?;
        check_eq!(
            resolved(
                &mut js,
                "plan",
                json!([plan[0], "coherance", "not-a-reviewer"])
            )?
            .map_err(Failure::Js)?,
            vec![plan[0].clone()],
            "an unrecognised name selects nothing and raises nothing"
        );
        Ok(())
    });
}

#[test]
fn reviewer_set_only_empty_refused() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let plan = keys(&mut js, "plan")?;
        check_eq!(
            resolved(&mut js, "plan", json!([plan[0]]))?
                .map_err(Failure::Js)?
                .len(),
            1,
            "a one-reviewer set is legal"
        );
        for set in [json!([]), json!(["nope"])] {
            let r = js.try_call("resolveReviewers", vec![json!("plan"), set.clone()])?;
            check!(rejects_with(&r, "resolved to NO reviewer"), "{set}: {r:?}");
        }
        let r = js.try_call("resolveReviewers", vec![json!("nonsense"), Value::Null])?;
        check!(rejects_with(&r, "unknown review mode"), "{r:?}");
        Ok(())
    });
}

#[test]
fn reviewer_set_no_selection_predicate() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let dims = js.get("DIMENSIONS")?;
        let modes = dims.as_object().cloned().unwrap_or_default();
        check!(!modes.is_empty(), "there are modes");
        for (mode, list) in modes {
            for d in list.as_array().into_iter().flatten() {
                check!(
                    d.get("when").is_none(),
                    "{mode}/{} carries a `when` predicate — selection belongs to the caller",
                    d["key"]
                );
            }
        }
        Ok(())
    });
}

#[test]
fn caller_set_reaches_pipeline() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let plan = keys(&mut js, "plan")?;
        let picked = json!([plan[0], plan[1]]);
        let d = drive_lib(
            &mut js,
            json!({ "roadmap": "r", "phase": "phase-1-x", "tags": [], "reviewers": picked }),
            json!([]),
        )?;
        check_eq!(
            d.contexts[0]["reviewers"],
            picked,
            "the set is passed through"
        );
        let omitted = drive_lib(
            &mut js,
            json!({ "roadmap": "r", "phase": "phase-1-x", "tags": [] }),
            json!([]),
        )?;
        check_eq!(
            omitted.contexts[0]["reviewers"],
            Value::Null,
            "omitted stays omitted: the core decides"
        );
        Ok(())
    });
}

#[test]
fn caller_set_narrows_real_finders() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let plan = keys(&mut js, "plan")?;
        let (_, agent) = drive_real(
            &mut js,
            json!({ "roadmap": "r", "phase": "phase-1-x", "tags": [], "reviewers": [plan[0]] }),
            inert(),
        )?;
        let finders: Vec<String> = agent
            .labels()
            .into_iter()
            .filter(|l| l.starts_with("find:"))
            .collect();
        check_eq!(
            finders,
            vec![format!("find:plan:{}", plan[0])],
            "only the selected finder"
        );
        let (_, all) = drive_real(
            &mut js,
            json!({ "roadmap": "r", "phase": "phase-1-x", "tags": [] }),
            inert(),
        )?;
        check_eq!(
            all.labels()
                .into_iter()
                .filter(|l| l.starts_with("find:"))
                .collect::<Vec<_>>(),
            ITEM_SET
                .iter()
                .map(|k| format!("find:plan:{k}"))
                .collect::<Vec<_>>(),
            "the phase's own set when omitted"
        );
        Ok(())
    });
}

#[test]
fn empty_set_refused_every_target() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        for args in [
            json!({ "roadmap": "r", "phase": "phase-1-x", "tags": [] }),
            json!({ "task": "t", "tags": [] }),
            json!({ "roadmap": "r", "phases": [{ "stem": "phase-1-x" }, { "stem": "phase-2-y" }], "tags": [] }),
            json!({ "implementationPlan": true, "planSlug": "a-plan" }),
        ] {
            for set in [
                json!([]),
                json!(["coherance"]),
                json!(["nope", "also-nope"]),
            ] {
                let d = drive_lib(&mut js, with(&args, json!({ "reviewers": set })), json!([]))?;
                check!(
                    rejects_with(&d.result, "resolved to NO reviewer"),
                    "{args} with {set} refuses rather than reporting an empty sweep: {:?}",
                    d.result
                );
            }
        }
        Ok(())
    });
}

#[test]
fn refusal_before_any_agent() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let d = drive_lib(
            &mut js,
            json!({ "roadmap": "r", "phases": [{ "stem": "phase-1-x" }], "tags": [], "reviewers": ["coherance"] }),
            json!([]),
        )?;
        check!(
            rejects_with(&d.result, "resolved to NO reviewer"),
            "{:?}",
            d.result
        );
        check!(d.agent.calls().is_empty(), "no agent dispatched");
        check!(d.contexts.is_empty(), "no review started");
        let plan = keys(&mut js, "plan")?;
        let ok = drive_lib(
            &mut js,
            json!({ "roadmap": "r", "phase": "phase-1-x", "tags": [], "reviewers": [plan[0]] }),
            json!([]),
        )?;
        let r = ok.ok()?;
        check_eq!(
            r["units"].as_array().map(Vec::len),
            Some(1),
            "the healthy path reviews"
        );
        check_eq!(r["outcome"], json!("reviewed"), "clean");
        check_eq!(r["failedUnits"], json!([]), "no loss");
        check!(
            !r["summary"]
                .as_str()
                .unwrap_or_default()
                .contains("NOT reviewed"),
            "an unchanged summary: {}",
            r["summary"]
        );
        Ok(())
    });
}

#[test]
fn failed_unit_named_not_dropped() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let d = drive_with(
            &mut js,
            json!({ "roadmap": "r", "phases": [{ "stem": "phase-1-x" }, { "stem": "phase-2-y" }], "tags": [] }),
            inert(),
            |ctx| {
                if ctx["target"]
                    .as_str()
                    .is_some_and(|t| t.contains("phase-1-x"))
                {
                    Err("this unit could not be reviewed".to_owned())
                } else {
                    Ok(
                        json!({ "survivors": [], "acTable": null, "budget": null, "coverage": null }),
                    )
                }
            },
        )?;
        let r = d.ok()?;
        let units = r["units"].as_array().cloned().unwrap_or_default();
        check_eq!(units.len(), 2, "the roadmap body and the surviving phase");
        check!(
            units.iter().all(|u| !u["ident"]
                .as_str()
                .unwrap_or_default()
                .contains("phase-1-x")),
            "the lost unit is not among them: {units:?}"
        );
        let failed = strings(&r["failedUnits"]);
        check!(
            failed.len() == 1 && failed[0].contains("phase-1-x"),
            "the lost unit is counted and named: {failed:?}"
        );
        check!(
            r["summary"]
                .as_str()
                .unwrap_or_default()
                .contains("NOT reviewed"),
            "the summary never reads as a clean sweep: {}",
            r["summary"]
        );
        check!(
            d.agent.logs().iter().any(|l| l.contains("NOT reviewed")),
            "the loss is logged: {:?}",
            d.agent.logs()
        );
        Ok(())
    });
}

// --- E: the environment axes, executed ---------------------------------------------

/// Every command a LIB run built or named: the reviewers' read commands, the
/// persist ladders, and the implementation-plan read command.
fn commands_from(d: &Driven) -> Result<Vec<(String, String)>, Failure> {
    let r = d.ok()?;
    let mut out = Vec::new();
    for ctx in &d.contexts {
        for key in ["itemCommand", "roadmapCommand", "implementsCommand"] {
            if let Some(c) = ctx[key].as_str() {
                out.push((format!("{} {key}", ctx["target"]), c.to_owned()));
            }
        }
    }
    for u in r["units"].as_array().into_iter().flatten() {
        let persist = script(&u["persistCommands"]);
        if !persist.is_empty() {
            out.push((format!("{} persist", u["ident"]), persist));
        }
    }
    let persist = script(&r["persistCommands"]);
    if !persist.is_empty() {
        out.push(("persist".to_owned(), persist));
    }
    if let Some(c) = r["itemCommand"].as_str() {
        out.push(("itemCommand".to_owned(), c.to_owned()));
    }
    Ok(out)
}

fn e_targets() -> [Value; 4] {
    [
        json!({ "task": "t", "tags": ["needs-plan-review"], "persist": true }),
        json!({ "roadmap": "r", "phase": "phase-1-x", "tags": ["needs-plan-review"], "persist": true }),
        json!({ "roadmap": "r", "phases": [{ "stem": "phase-1-x", "tags": ["needs-plan-review"] }],
                "tags": ["needs-plan-review"], "persist": true }),
        json!({ "implementationPlan": true, "planSlug": "p", "roadmap": "r", "phase": "phase-1-x", "persist": true }),
    ]
}

#[test]
fn injected_axes_reach_executed_commands() {
    run_real(|lib| {
        let repo = plan_repo()?;
        let mut js = Js::open(lib)?;
        let mut executed = 0;
        for t in e_targets() {
            let d = drive_lib(&mut js, with(&t, axes()), json!([]))?;
            let cmds = commands_from(&d)?;
            check!(cmds.len() >= 2, "{t}: commands were built: {cmds:?}");
            // Bare PATH and a decoy default project: a dropped binary cannot
            // resolve and a dropped --project misses the seeded items.
            for (what, cmd) in cmds {
                let out = repo.run(&Run::bare(), &cmd)?;
                check!(
                    out.status.success(),
                    "{t}: {what} failed under a bare PATH on the decoy repo:\n{cmd}\n{}",
                    String::from_utf8_lossy(&out.stderr)
                );
                executed += 1;
            }
        }
        // A single-unit target carries its ladder twice, on the unit and
        // flattened on the result, and `commands_from` runs both copies.
        // task: 1 read + 2 persists; phase: 2 reads + 2 persists; sweep:
        // 3 reads + 2 persists; implementation plan: 3 reads + 1 persist.
        check_eq!(executed, 16, "every target's commands ran");
        // Every executed persist ladder landed a review in the target
        // project (the phase: two from its own run, one from the sweep).
        for (target, want) in [
            ("task/t", 2),
            ("phase/r/phase-1-x", 3),
            ("roadmap/r", 1),
            ("plan/p", 1),
        ] {
            check_eq!(
                repo.reviews_on(PROJECT, target)?.len(),
                want,
                "the executed persist ladders wrote {target} in the target project"
            );
        }
        Ok(())
    });
}

#[test]
fn omitted_axes_use_plain_rdm_and_default_project() {
    run_real(|lib| {
        // Here the default project IS the target project.
        let repo = PlanRepo::init(PROJECT)?;
        seed_into(&repo)?;
        let mut js = Js::open(lib)?;
        let d = drive_lib(&mut js, e_targets()[2].clone(), json!([]))?;
        let cmds = commands_from(&d)?;
        // Three reads (the roadmap, the phase, the phase's parent) and the
        // two units' persist ladders.
        check_eq!(cmds.len(), 5, "commands were built: {cmds:?}");
        // Without `rdm` on PATH the plain-`rdm` commands cannot resolve…
        let (what, first) = &cmds[0];
        check!(
            !repo.run(&Run::bare(), first)?.status.success(),
            "{what} names a plain `rdm`, so it cannot run without one on PATH"
        );
        // …and with the PATH shim every command runs on the default project.
        let run = Run::bare().path_prepend(&repo.rdm_on_path()?);
        for (what, cmd) in cmds {
            let out = repo.run(&run, &cmd)?;
            check!(
                out.status.success(),
                "{what} failed with `rdm` on PATH:\n{cmd}\n{}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        for target in ["roadmap/r", "phase/r/phase-1-x"] {
            check_eq!(
                repo.reviews_on(PROJECT, target)?.len(),
                1,
                "the persist ladder reached {target} in the default project"
            );
        }
        Ok(())
    });
}

#[test]
fn invalid_axes_refused_at_parse() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        for (args, needle) in [
            (
                json!({ "task": "t", "rdmBin": 42 }),
                "rdmBin must be a string path",
            ),
            (
                json!({ "task": "t", "rdmBin": {} }),
                "rdmBin must be a string path",
            ),
            (
                json!({ "task": "t", "project": "a b" }),
                "plain project name",
            ),
            (
                json!({ "task": "t", "project": "a;rm -rf /" }),
                "plain project name",
            ),
            (json!({ "task": "t", "project": 7 }), "plain project name"),
        ] {
            let r = js.plan_try_call("parsePlanArgs", vec![args.clone()])?;
            check!(rejects_with(&r, needle), "{args}: {r:?}");
        }
        for (arg, want) in [
            (Host::undefined(), "rdm"),
            (json!(""), "rdm"),
            (json!("/x/rdm"), "/x/rdm"),
        ] {
            check_eq!(
                js.plan_call("resolveRdmBin", vec![arg.clone()])?,
                json!(want),
                "resolveRdmBin({arg})"
            );
        }
        check_eq!(
            js.plan_call("parseProjectArg", vec![Host::undefined()])?,
            json!(""),
            "no project"
        );
        check_eq!(
            js.plan_call("parseProjectArg", vec![json!("demo")])?,
            json!("demo"),
            "a project"
        );
        check_eq!(
            js.plan_call("projectFlag", vec![json!({ "project": "demo" })])?,
            json!(" --project demo"),
            "the flag"
        );
        check_eq!(
            js.plan_call("projectFlag", vec![json!({})])?,
            json!(""),
            "no flag"
        );
        Ok(())
    });
}

#[test]
fn axes_not_read_from_flag_string() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let parsed = js.plan_call(
            "parsePlanArgs",
            vec![json!("--task t --rdmBin /evil/rdm --project pwned")],
        )?;
        check_eq!(
            parsed["rdmBin"],
            json!("rdm"),
            "the binary is not tokenized out of the string"
        );
        check_eq!(parsed["project"], json!(""), "nor the project");
        Ok(())
    });
}

// --- F: the source pin -------------------------------------------------------------

const PIN_PATH: &str = "/work/wt-sentinel-9d41";
const PIN_BRANCH: &str = "roadmap/sentinel-branch";

fn pinned(extra: Value) -> Value {
    with(
        &json!({ "reviewers": ["specification"], "source": PIN_PATH, "base": BASE,
                 "expectedHead": HEAD, "expectedBranch": PIN_BRANCH }),
        extra,
    )
}

fn carries_pin(prompt: &str) -> bool {
    [PIN_PATH, BASE, HEAD, PIN_BRANCH]
        .iter()
        .all(|v| prompt.contains(v))
}

#[test]
fn source_pin_task_form_reaches_every_prompt() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let (_, agent) = drive_real(
            &mut js,
            pinned(json!({ "implementationPlan": true, "planSlug": "p", "task": "t" })),
            probe(),
        )?;
        check!(!agent.calls_with("find:").is_empty(), "a finder ran");
        check!(!agent.calls_with("refute:").is_empty(), "a refuter ran");
        for c in agent.calls() {
            check!(
                carries_pin(&c.prompt) && c.prompt.contains("'task/t'"),
                "{} carries the injected pin bound to task/t:\n{}",
                c.label,
                c.prompt
            );
        }
        Ok(())
    });
}

#[test]
fn source_pin_phase_form_binds_phase() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let (_, agent) = drive_real(
            &mut js,
            pinned(
                json!({ "implementationPlan": true, "planSlug": "p", "roadmap": "r", "phase": "phase-4-d" }),
            ),
            probe(),
        )?;
        let calls = agent.calls();
        check!(!calls.is_empty(), "reviewers ran");
        for c in calls {
            check!(
                carries_pin(&c.prompt) && c.prompt.contains("'phase/r/phase-4-d'"),
                "{} binds the pin to the phase:\n{}",
                c.label,
                c.prompt
            );
        }
        Ok(())
    });
}

#[test]
fn source_pin_sweep_binds_each_unit() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let (_, agent) = drive_real(
            &mut js,
            pinned(
                json!({ "roadmap": "r", "phases": [{ "stem": "phase-1-a" }, { "stem": "phase-2-b" }], "tags": [] }),
            ),
            probe(),
        )?;
        let calls = agent.calls();
        let of = |needle: &str| -> Vec<_> {
            calls
                .iter()
                .filter(|c| c.prompt.contains(needle))
                .cloned()
                .collect()
        };
        let roadmap: Vec<_> = calls
            .iter()
            .filter(|c| c.prompt.contains("roadmap/r") && !c.prompt.contains("phase/r/"))
            .cloned()
            .collect();
        let p1 = of("phase/r/phase-1-a");
        let p2 = of("phase/r/phase-2-b");
        check!(
            !roadmap.is_empty() && !p1.is_empty() && !p2.is_empty(),
            "every unit dispatched reviewers"
        );
        for c in &roadmap {
            check!(
                !c.prompt.contains(PIN_PATH),
                "the roadmap-body unit carries no source pin:\n{}",
                c.prompt
            );
        }
        for (own, sibling, unit) in [
            ("phase-1-a", "phase-2-b", &p1),
            ("phase-2-b", "phase-1-a", &p2),
        ] {
            for c in unit.iter() {
                check!(
                    carries_pin(&c.prompt) && c.prompt.contains(&format!("'phase/r/{own}'")),
                    "{own}: {} binds its own ref:\n{}",
                    c.label,
                    c.prompt
                );
                check!(!c.prompt.contains(sibling), "{own} never carries {sibling}");
            }
        }
        check!(
            p1.iter().any(|c| c.label.starts_with("refute:")),
            "a refuter ran for phase-1-a"
        );
        Ok(())
    });
}

#[test]
fn source_pin_single_target_binds_itself() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        for (args, own) in [
            (
                json!({ "roadmap": "r", "phase": "phase-3-c", "tags": [] }),
                "'phase/r/phase-3-c'",
            ),
            (json!({ "task": "t2", "tags": [] }), "'task/t2'"),
        ] {
            let (_, agent) = drive_real(&mut js, pinned(args), probe())?;
            let calls = agent.calls();
            check!(!calls.is_empty(), "{own}: reviewers ran");
            for c in calls {
                check!(
                    carries_pin(&c.prompt) && c.prompt.contains(own),
                    "{own}: {} binds the pin to its own item",
                    c.label
                );
            }
        }
        Ok(())
    });
}

#[test]
fn unpinned_prompts_deterministic() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let args = json!({ "task": "t3", "tags": [], "reviewers": ["specification"] });
        let (_, a) = drive_real(&mut js, args.clone(), probe())?;
        let (_, b) = drive_real(&mut js, args, probe())?;
        let pa: Vec<String> = a.calls().into_iter().map(|c| c.prompt).collect();
        let pb: Vec<String> = b.calls().into_iter().map(|c| c.prompt).collect();
        check!(!pa.is_empty(), "reviewers ran");
        check_eq!(pa, pb, "unpinned prompts are byte-identical across runs");
        Ok(())
    });
}

#[test]
fn malformed_source_pin_refused_before_agents() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let short = "a".repeat(10);
        for (args, needle) in [
            (
                json!({ "task": "t", "source": "/wt", "base": BASE }),
                "needs all four",
            ),
            (
                json!({ "task": "t", "expectedHead": HEAD, "expectedBranch": "main" }),
                "needs all four",
            ),
            (
                json!({ "task": "t", "source": "/wt", "base": BASE, "expectedHead": "not-a-sha", "expectedBranch": "main" }),
                "full hex commit id",
            ),
            (
                json!({ "task": "t", "source": "/wt", "base": BASE, "expectedHead": short, "expectedBranch": "main" }),
                "full hex commit id",
            ),
            (
                json!({ "task": "t", "source": "/wt", "base": "not-a-sha", "expectedHead": HEAD, "expectedBranch": "main" }),
                "base must be a full hex commit id",
            ),
            (
                json!({ "task": "t", "source": "/wt", "base": short, "expectedHead": HEAD, "expectedBranch": "main" }),
                "base must be a full hex commit id",
            ),
            (
                json!({ "implementationPlan": true, "planSlug": "p", "source": "/wt", "base": BASE,
                        "expectedHead": HEAD, "expectedBranch": "main" }),
                "no item to bind it to",
            ),
        ] {
            let r = js.plan_try_call("parsePlanArgs", vec![args.clone()])?;
            check!(
                rejects_with(&r, needle),
                "{args}: expected /{needle}/, got {r:?}"
            );
        }
        let d = drive_lib(
            &mut js,
            json!({ "task": "t", "source": "/wt", "base": BASE }),
            json!([]),
        )?;
        check!(rejects_with(&d.result, "needs all four"), "{:?}", d.result);
        check!(
            d.agent.calls().is_empty() && d.contexts.is_empty(),
            "before any agent"
        );
        Ok(())
    });
}

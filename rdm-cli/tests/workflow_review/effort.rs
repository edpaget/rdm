//! Reasoning-effort threading through the review pipeline: the dispatch lane
//! resolves `rdm model resolve review-find|review-verify|review-consolidate`
//! into a `{model, effort}` profile and hands the pair to the review engines
//! as `findModel`/`findEffort`, `verifyModel`/`verifyEffort` and
//! `consolidateModel`/`consolidateEffort`. Supplied, the effort reaches every
//! finder (including its one retry), every refuter and every consolidator
//! `agent()` call; not supplied, no call carries an `effort` key at all;
//! invalid, it is refused before any agent is dispatched. Decided by executing
//! the real `buildReviewPipeline` (both modes) and the real plan-review driver
//! against a recording agent and reading the options each call received.
//!
//! Ported from `scripts/lib/review-effort.test.mjs` (see
//! `docs/test-migration-inventory.md` § 2(b)); the review-engine driver arm is
//! `driver::find_and_verify_effort_reach_every_finder_and_refuter`.

use serde_json::{Value, json};

use crate::support::{Agent, AgentCall, Failure, Js, Lib, Outcome, Reply, run_real};

/// Each mode's dimensions, in declaration order.
const CODE_DIMS: &[&str] = &[
    "ac",
    "correctness",
    "tests",
    "architecture",
    "api-docs",
    "changelog",
    "security",
];
const PLAN_DIMS: &[&str] = &[
    "coherence",
    "architectural-fit",
    "unit-of-work",
    "intent-alignment",
    "restraint",
];

fn dims(mode: &str) -> &'static [&'static str] {
    if mode == "code" { CODE_DIMS } else { PLAN_DIMS }
}

/// Every finder returns one gating finding (so a refuter is dispatched for
/// it), every refuter a non-refuting verdict, the consolidator `null` (so it
/// is retried once and then fails open to singletons, leaving one refuter per
/// finding), and the first dimension's first finder call returns `null` so
/// the pipeline's single finder retry runs too.
fn recording(mode: &'static str) -> Agent {
    let first = format!("find:{mode}:{}", dims(mode)[0]);
    let mut nulled = false;
    Agent::scripted(move |call: &AgentCall| {
        if call.label.starts_with("refute:") {
            return Reply::Value(json!({ "refuted": false, "confidence": 95 }));
        }
        if call.label.starts_with("consolidate:") {
            return Reply::Null;
        }
        if !nulled && call.label == first {
            nulled = true;
            return Reply::Null;
        }
        let dim = call.label.split(':').nth(2).unwrap_or_default().to_owned();
        let finding = json!({
            "id": format!("{dim}-1"), "concern": dim, "severity": "blocking",
            "confidence": 90, "what_fails": format!("something in {dim}"),
        });
        if mode == "code" && dim == "ac" {
            Reply::Value(json!({
                "ac": [{ "criterion": "AC1: it works", "status": "PASS", "evidence": "covered" }],
                "findings": [finding],
            }))
        } else {
            Reply::Value(json!({ "findings": [finding] }))
        }
    })
}

fn has_effort(c: &AgentCall) -> bool {
    c.opts.get("effort").is_some()
}

fn supplied_effort_reaches_every_call(lib: &Lib, mode: &'static str) -> Outcome {
    let mut js = Js::open(lib)?;
    let agent = recording(mode);
    js.review_ok(
        mode,
        &agent,
        json!({
            "target": "x", "findModel": "m-find", "findEffort": "low",
            "verifyModel": "m-verify", "verifyEffort": "xhigh",
            "consolidateModel": "m-consolidate", "consolidateEffort": "high",
            "maxRefutations": 50,
        }),
    )?;
    let mut expected: Vec<String> = dims(mode)
        .iter()
        .map(|d| format!("find:{mode}:{d}"))
        .collect();
    expected.push(format!("find:{mode}:{}:retry", dims(mode)[0]));
    expected.sort();
    let finders = agent.calls_with("find:");
    let mut labels: Vec<String> = finders.iter().map(|c| c.label.clone()).collect();
    labels.sort();
    check_eq!(
        labels,
        expected,
        "one finder per dimension plus the first dimension's retry"
    );
    let refuters = agent.calls_with("refute:");
    check_eq!(
        refuters.len(),
        dims(mode).len(),
        "one refuter per gating finding"
    );
    for c in &finders {
        check_eq!(
            (c.opts["model"].clone(), c.opts["effort"].clone()),
            (json!("m-find"), json!("low")),
            "{} carries findModel/findEffort",
            c.label
        );
    }
    for c in &refuters {
        check_eq!(
            (c.opts["model"].clone(), c.opts["effort"].clone()),
            (json!("m-verify"), json!("xhigh")),
            "{} carries verifyModel/verifyEffort",
            c.label
        );
    }
    let consolidators = agent.calls_with("consolidate:");
    check_eq!(consolidators.len(), 2, "the consolidator and its one retry");
    for c in &consolidators {
        check_eq!(
            (c.opts["model"].clone(), c.opts["effort"].clone()),
            (json!("m-consolidate"), json!("high")),
            "{} carries consolidateModel/consolidateEffort",
            c.label
        );
    }
    Ok(())
}

fn absent_effort_adds_no_key(lib: &Lib, mode: &'static str) -> Outcome {
    let mut js = Js::open(lib)?;
    for ctx in [
        json!({ "target": "x", "maxRefutations": 50 }),
        json!({ "target": "x", "findEffort": "", "verifyEffort": null, "consolidateEffort": "", "maxRefutations": 50 }),
    ] {
        let agent = recording(mode);
        js.review_ok(mode, &agent, ctx.clone())?;
        check!(
            !agent.calls_with("find:").is_empty() && !agent.calls_with("refute:").is_empty(),
            "the run dispatched finders and refuters ({ctx})"
        );
        for c in agent.calls() {
            check!(
                !has_effort(&c),
                "{} has an effort key ({ctx}): {}",
                c.label,
                c.opts
            );
        }
    }
    Ok(())
}

fn invalid_effort_refused_before_any_agent(lib: &Lib, mode: &'static str) -> Outcome {
    let mut js = Js::open(lib)?;
    for extra in [
        json!({ "findEffort": "turbo" }),
        json!({ "verifyEffort": "turbo" }),
        json!({ "consolidateEffort": "turbo" }),
    ] {
        let mut ctx = json!({ "target": "x" });
        for (k, v) in extra.as_object().into_iter().flatten() {
            ctx[k] = v.clone();
        }
        let agent = recording(mode);
        let result = js.review(mode, &agent, ctx)?;
        check!(
            result.as_ref().is_err_and(|e| e.message.contains("turbo")),
            "{extra} was not refused naming the value: {result:?}"
        );
        check!(
            agent.calls().is_empty(),
            "{extra}: agents were dispatched: {:?}",
            agent.labels()
        );
    }
    Ok(())
}

#[test]
fn supplied_effort_reaches_every_finder_retry_and_refuter_code() {
    run_real(|lib| supplied_effort_reaches_every_call(lib, "code"));
}

#[test]
fn supplied_effort_reaches_every_finder_retry_and_refuter_plan() {
    run_real(|lib| supplied_effort_reaches_every_call(lib, "plan"));
}

#[test]
fn absent_effort_adds_no_key_code() {
    run_real(|lib| absent_effort_adds_no_key(lib, "code"));
}

#[test]
fn absent_effort_adds_no_key_plan() {
    run_real(|lib| absent_effort_adds_no_key(lib, "plan"));
}

#[test]
fn invalid_effort_refused_before_any_agent_code() {
    run_real(|lib| invalid_effort_refused_before_any_agent(lib, "code"));
}

#[test]
fn invalid_effort_refused_before_any_agent_plan() {
    run_real(|lib| invalid_effort_refused_before_any_agent(lib, "plan"));
}

#[test]
fn every_claude_effort_level_accepted() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        for effort in ["low", "medium", "high", "xhigh", "max"] {
            let agent = recording("plan");
            js.review_ok(
                "plan",
                &agent,
                json!({
                    "target": "x", "findEffort": effort, "verifyEffort": effort,
                    "consolidateEffort": effort,
                }),
            )?;
            check!(!agent.calls().is_empty(), "{effort}: no agent dispatched");
            for c in agent.calls() {
                check_eq!(
                    c.opts.get("effort"),
                    Some(&json!(effort)),
                    "{effort} reached {}",
                    c.label
                );
            }
        }
        Ok(())
    });
}

/// The real plan-review driver over the real plan pipeline, both wired to
/// `agent`.
fn drive_plan(js: &mut Js, args: Value, agent: &Agent) -> Result<Value, Failure> {
    let mut deps = agent.install(&mut js.host);
    deps["runPlanReview"] = js.call("buildReviewPipeline", vec![json!("plan"), deps.clone()])?;
    js.plan_try_call("runPlanReviewDriver", vec![args, deps])?
        .map_err(Failure::Js)
}

#[test]
fn plan_driver_effort_args_reach_real_finders_and_refuters() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let agent = recording("plan");
        drive_plan(
            &mut js,
            json!({
                "task": "t", "tags": [], "findModel": "m-find", "findEffort": "medium",
                "verifyModel": "m-verify", "verifyEffort": "high",
                "consolidateModel": "m-consolidate", "consolidateEffort": "low",
                "maxRefutations": 50,
            }),
            &agent,
        )?;
        let (finders, refuters) = (agent.calls_with("find:"), agent.calls_with("refute:"));
        check!(
            !finders.is_empty() && !refuters.is_empty(),
            "the run dispatched finders and refuters: {:?}",
            agent.labels()
        );
        for c in &finders {
            check_eq!(c.opts.get("effort"), Some(&json!("medium")), "{}", c.label);
        }
        for c in &refuters {
            check_eq!(c.opts.get("effort"), Some(&json!("high")), "{}", c.label);
        }
        let consolidators = agent.calls_with("consolidate:");
        check!(
            !consolidators.is_empty(),
            "the run dispatched the consolidator: {:?}",
            agent.labels()
        );
        for c in &consolidators {
            check_eq!(
                (c.opts.get("model"), c.opts.get("effort")),
                (Some(&json!("m-consolidate")), Some(&json!("low"))),
                "{}",
                c.label
            );
        }
        Ok(())
    });
}

#[test]
fn plan_driver_without_effort_args_adds_no_key() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let agent = recording("plan");
        drive_plan(&mut js, json!({ "task": "t", "tags": [] }), &agent)?;
        check!(!agent.calls().is_empty(), "the run dispatched agents");
        for c in agent.calls() {
            check!(!has_effort(&c), "{} has an effort key: {}", c.label, c.opts);
        }
        Ok(())
    });
}

#[test]
fn parse_plan_args_normalises_effort_like_model() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let p = js.plan_call(
            "parsePlanArgs",
            vec![json!({
                "task": "t", "findEffort": "  low ", "verifyEffort": "max",
                "consolidateModel": " m-c ", "consolidateEffort": " high",
            })],
        )?;
        check_eq!(
            (p["findEffort"].clone(), p["verifyEffort"].clone()),
            (json!("low"), json!("max")),
            "surrounding whitespace is trimmed"
        );
        check_eq!(
            (
                p["consolidateModel"].clone(),
                p["consolidateEffort"].clone()
            ),
            (json!("m-c"), json!("high")),
            "the consolidator pair is trimmed like the others"
        );
        let q = js.plan_call(
            "parsePlanArgs",
            vec![json!({
                "task": "t", "findEffort": "   ", "verifyEffort": 7,
                "consolidateModel": "  ", "consolidateEffort": 3,
            })],
        )?;
        check_eq!(
            (q["findEffort"].clone(), q["verifyEffort"].clone()),
            (Value::Null, Value::Null),
            "blank and non-string efforts normalise to null"
        );
        check_eq!(
            (
                q["consolidateModel"].clone(),
                q["consolidateEffort"].clone()
            ),
            (Value::Null, Value::Null),
            "a blank consolidator model and a non-string effort normalise to null"
        );
        Ok(())
    });
}

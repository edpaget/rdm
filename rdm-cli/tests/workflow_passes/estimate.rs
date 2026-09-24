//! The phase-estimation pass.
//!
//! Imports the canonical `.claude/workflows/lib/estimate.mjs` for the
//! helpers and `buildEstimatePipeline`, and loads the real
//! `.claude/workflows/rdm-wf-estimate.js` driver for the engine cases. The
//! binary's own `phase list --format json` drives selection, and every
//! returned list/writeback command is executed against the real binary; the
//! claims are about the plan state read back afterwards (the difficulty, the
//! tier rdm-core derives from it, an untouched body, and no commit). Ported
//! from `scripts/verify-workflow-estimate.sh` and
//! `scripts/lib/estimate-writeback.test.mjs`.

use std::cell::RefCell;
use std::rc::Rc;

use rdm_devtools::workflow::{Host, JsError};
use serde_json::{Value, json};

use crate::generators::Generator;
use crate::plan_fixture::{PlanRepo, Run, rdm_bin};
use crate::workflow_support::{Agent, Failure, Lib, Module, Reply, run_driver, run_real, strings};

const LIB: &str = ".claude/workflows/lib/estimate.mjs";
const ENGINE: &str = ".claude/workflows/rdm-wf-estimate.js";
const PROJECT: &str = "est-verify";
const ROADMAP: &str = "rm-est";

const GEN: Generator = Generator {
    script: "scripts/gen-workflow-estimate.sh",
    lib: LIB,
    engine: ENGINE,
    template: "rdm-core/src/templates/workflows/rdm-wf-estimate.js",
    plugin: "plugins/rdm/workflows/rdm-wf-estimate.js",
    begin: ">>> estimate-core:begin",
};

fn rejects(r: &Result<Value, JsError>, needle: &str) -> bool {
    r.as_ref().is_err_and(|e| e.message.contains(needle))
}

/// Roadmap `rm-est` with phases a/b/c (bodies `ORIGINAL BODY <X>`); the phase
/// numbered `estimated` is pre-estimated `hard`.
fn seeded(default_project: Option<&str>, estimated: &str) -> Result<PlanRepo, Failure> {
    let repo = match default_project {
        Some(p) => PlanRepo::init(p)?,
        None => PlanRepo::with_decoy(&[PROJECT])?,
    };
    let phase = |slug: &'static str, n: &'static str, body: &'static str| {
        vec![
            "phase",
            "create",
            slug,
            "--title",
            slug,
            "--number",
            n,
            "--body",
            body,
            "--no-edit",
            "--roadmap",
            ROADMAP,
            "--project",
            PROJECT,
        ]
    };
    repo.seed(&[
        &[
            "roadmap",
            "create",
            ROADMAP,
            "--title",
            "Estimate RM",
            "--body",
            "seed",
            "--no-edit",
            "--project",
            PROJECT,
        ],
        &phase("a", "1", "ORIGINAL BODY A"),
        &phase("b", "2", "ORIGINAL BODY B"),
        &phase("c", "3", "ORIGINAL BODY C"),
        &[
            "phase",
            "update",
            estimated,
            "--difficulty",
            "hard",
            "--no-edit",
            "--roadmap",
            ROADMAP,
            "--project",
            PROJECT,
        ],
    ])?;
    Ok(repo)
}

fn phase_list(repo: &PlanRepo) -> Result<Value, Failure> {
    repo.json(&[
        "phase",
        "list",
        "--roadmap",
        ROADMAP,
        "--project",
        PROJECT,
        "--format",
        "json",
    ])
}

fn show(repo: &PlanRepo, stem: &str) -> Result<Value, Failure> {
    repo.phase(PROJECT, ROADMAP, stem)
}

// --- Pure helpers ------------------------------------------------------------------

#[test]
fn parse_estimate_args() {
    run_real(|lib| {
        let mut m = Module::open(lib, LIB)?;
        for args in [
            json!({}),
            json!({ "roadmap": "" }),
            json!("not json"),
            json!("null"),
        ] {
            let r = m.try_call("parseEstimateArgs", vec![args.clone()])?;
            check!(rejects(&r, "roadmap slug is required"), "{args}: {r:?}");
        }
        check_eq!(
            m.call(
                "parseEstimateArgs",
                vec![json!({ "roadmap": "rm", "rdmBin": "rdm" })]
            )?,
            json!({ "roadmap": "rm", "phase": null, "rdmBin": "rdm", "project": "" }),
            "defaults: no narrowing, no project flag"
        );
        for (phase, want) in [(json!(3), 3), (json!("2"), 2)] {
            check_eq!(
                m.call(
                    "parseEstimateArgs",
                    vec![json!({ "roadmap": "rm", "phase": phase, "rdmBin": "rdm" })]
                )?["phase"],
                json!(want),
                "phase {phase}"
            );
        }
        for bad in [0, -1] {
            let r = m.try_call(
                "parseEstimateArgs",
                vec![json!({ "roadmap": "rm", "phase": bad, "rdmBin": "rdm" })],
            )?;
            check!(rejects(&r, "positive integer"), "phase {bad}: {r:?}");
        }
        check_eq!(
            m.call(
                "parseEstimateArgs",
                vec![json!(r#"{"roadmap":"rm","rdmBin":"rdm"}"#)]
            )?["roadmap"],
            json!("rm"),
            "a stringified payload is coerced"
        );
        Ok(())
    });
}

#[test]
fn select_unestimated() {
    run_real(|lib| {
        let mut m = Module::open(lib, LIB)?;
        check_eq!(
            m.call(
                "selectUnestimated",
                vec![json!([{ "stem": "a" }, { "stem": "b", "difficulty": "hard" }, { "stem": "c", "model": "small" },
                            { "stem": "d", "difficulty": "easy", "model": "small" }])]
            )?,
            json!(["a"]),
            "only a phase with neither difficulty nor model"
        );
        check_eq!(
            m.call("selectUnestimated", vec![json!([])])?,
            json!([]),
            "empty"
        );
        check_eq!(
            m.call("selectUnestimated", vec![Value::Null])?,
            json!([]),
            "non-array"
        );
        Ok(())
    });
}

#[test]
fn summary_text() {
    run_real(|lib| {
        let mut m = Module::open(lib, LIB)?;
        let summary = json!({ "roadmap": "rm", "skipped": ["phase-3-c"], "estimated": [
            { "stem": "phase-1-a", "difficulty": "easy", "justification": "small" },
            { "stem": "phase-2-b", "difficulty": "hard", "justification": "big" }] });
        let text = m.call("buildEstimateSummaryText", vec![summary])?;
        let text = text.as_str().unwrap_or_default();
        for part in [
            "roadmap/rm",
            "phase-1-a: easy — small",
            "phase-2-b: hard — big",
            "(1): phase-3-c",
        ] {
            check!(text.contains(part), "the summary renders {part:?}: {text}");
        }
        let empty = json!({ "roadmap": "rm", "estimated": [], "skipped": [] });
        check_eq!(
            m.call("buildEstimateSummaryText", vec![empty.clone()])?,
            m.call("buildEstimateSummaryText", vec![empty])?,
            "deterministic"
        );
        Ok(())
    });
}

#[test]
fn estimator_prompt_embeds_phase_body() {
    run_real(|lib| {
        let mut m = Module::open(lib, LIB)?;
        let body = "SENTINEL phase body 3e9a: rewrite the parser";
        let p = m.call("buildEstimatorPrompt", vec![json!(body)])?;
        check!(
            p.as_str().is_some_and(|p| p.contains(body)),
            "the injected body reaches the rater"
        );
        Ok(())
    });
}

#[test]
fn rdm_bin_resolution_and_refusals() {
    run_real(|lib| {
        let mut m = Module::open(lib, LIB)?;
        for absent in [
            Host::undefined(),
            Value::Null,
            json!(""),
            json!("   "),
            json!("\t"),
        ] {
            check_eq!(
                m.call("resolveRdmBin", vec![absent.clone()])?,
                json!("rdm"),
                "{absent} defaults"
            );
            check_eq!(
                m.call(
                    "parseEstimateArgs",
                    vec![json!({ "roadmap": "rm", "rdmBin": absent })]
                )?["rdmBin"],
                json!("rdm"),
                "{absent} defaults through parse"
            );
        }
        check_eq!(
            m.call("parseEstimateArgs", vec![json!({ "roadmap": "rm" })])?["rdmBin"],
            json!("rdm"),
            "a missing key defaults"
        );
        check_eq!(
            m.call("parseEstimateArgs", vec![json!(r#"{"roadmap":"rm"}"#)])?["rdmBin"],
            json!("rdm"),
            "and through a stringified payload"
        );
        for bad in [json!(42), json!({}), json!([]), json!(true)] {
            let r = m.try_call(
                "parseEstimateArgs",
                vec![json!({ "roadmap": "rm", "rdmBin": bad })],
            )?;
            check!(
                rejects(&r, "rdmBin"),
                "a non-string rdmBin {bad} is refused: {r:?}"
            );
        }
        for args in [json!({}), json!({ "rdmBin": "rdm" })] {
            let r = m.try_call("parseEstimateArgs", vec![args.clone()])?;
            check!(
                rejects(&r, "roadmap slug is required"),
                "the roadmap is checked first: {args}"
            );
        }
        let r = m.try_call(
            "parseEstimateArgs",
            vec![json!({ "roadmap": "rm", "rdmBin": 42 })],
        )?;
        let msg = r.err().map(|e| e.message).unwrap_or_default();
        check!(
            msg.contains("rdmBin must be a string")
                && msg.contains("\"rdm\"")
                && msg.contains("PATH"),
            "the refusal names the requirement, the sentinel and the default: {msg}"
        );
        for (v, want) in [("rdm", "rdm"), ("/opt/x/rdm", "/opt/x/rdm")] {
            check_eq!(
                m.call(
                    "parseEstimateArgs",
                    vec![json!({ "roadmap": "rm", "rdmBin": v })]
                )?["rdmBin"],
                json!(want),
                "{v} is kept verbatim"
            );
        }
        check_eq!(
            m.call("resolveRdmBin", vec![json!("rdm ")])?,
            json!("rdm "),
            "a present value is verbatim, not the default branch"
        );
        Ok(())
    });
}

#[test]
fn project_arg_validation() {
    run_real(|lib| {
        let mut m = Module::open(lib, LIB)?;
        check_eq!(
            m.call(
                "parseEstimateArgs",
                vec![json!({ "roadmap": "rm", "rdmBin": "rdm" })]
            )?["project"],
            json!(""),
            "absent means no flag"
        );
        for falsy in [
            Host::undefined(),
            Value::Null,
            json!(""),
            json!(0),
            json!(false),
        ] {
            let p = m.call("parseProjectArg", vec![falsy.clone()])?;
            check_eq!(p, json!(""), "{falsy} means no project");
            check_eq!(
                m.call("projectFlag", vec![json!({ "project": p })])?,
                json!(""),
                "and no flag"
            );
        }
        for cfg in [json!({}), Value::Null] {
            check_eq!(
                m.call("projectFlag", vec![cfg.clone()])?,
                json!(""),
                "{cfg}: no flag"
            );
        }
        check_eq!(
            m.call("projectFlag", vec![json!({ "project": "demo" })])?,
            json!(" --project demo"),
            "a configured project"
        );
        for hostile in [
            json!("a b"),
            json!("a;rm -rf /"),
            json!("$(x)"),
            json!("`x`"),
            json!("a\nb"),
            json!("a|b"),
            json!(7),
            json!({}),
        ] {
            let r = m.try_call("parseProjectArg", vec![hostile.clone()])?;
            check!(
                rejects(&r, "project must be a plain project name"),
                "{hostile}: {r:?}"
            );
        }
        check_eq!(
            m.call(
                "parseEstimateArgs",
                vec![json!({ "roadmap": "rm", "rdmBin": "rdm", "project": "rdm-atlas.v2_x" })]
            )?["project"],
            json!("rdm-atlas.v2_x"),
            "a plain name survives"
        );
        Ok(())
    });
}

// --- Executed commands -------------------------------------------------------------

#[test]
fn list_command_executes() {
    run_real(|lib| {
        let mut m = Module::open(lib, LIB)?;
        let repo = seeded(None, "phase-3-c")?;
        let cmd = m.call(
            "estimateListCommand",
            vec![
                json!(ROADMAP),
                json!({ "rdmBin": rdm_bin(), "project": PROJECT }),
            ],
        )?;
        let out = repo.run_ok(&Run::bare(), cmd.as_str().unwrap_or_default())?;
        let listed: Value =
            serde_json::from_str(&out).map_err(|e| Failure::Check(format!("{e}: {out}")))?;
        check_eq!(
            listed,
            phase_list(&repo)?,
            "the returned command lists the roadmap's phases"
        );
        check_eq!(listed.as_array().map(Vec::len), Some(3), "all three");

        let home = seeded(Some(PROJECT), "phase-3-c")?;
        let plain = m.call("estimateListCommand", vec![json!(ROADMAP), json!({})])?;
        let plain = plain.as_str().unwrap_or_default();
        check!(
            !home.run(&Run::bare(), plain)?.status.success(),
            "a plain `rdm` needs one on PATH"
        );
        let out = home.run_ok(&Run::bare().path_prepend(&home.rdm_on_path()?), plain)?;
        let listed: Value =
            serde_json::from_str(&out).map_err(|e| Failure::Check(format!("{e}: {out}")))?;
        check_eq!(
            listed,
            phase_list(&home)?,
            "and lists the default project's roadmap"
        );
        Ok(())
    });
}

#[test]
fn real_phase_list_selects_unestimated() {
    run_real(|lib| {
        let mut m = Module::open(lib, LIB)?;
        let repo = seeded(None, "phase-2-b")?;
        let mut picked = strings(&m.call("selectUnestimated", vec![phase_list(&repo)?])?);
        picked.sort();
        check_eq!(
            picked,
            vec!["phase-1-a".to_owned(), "phase-3-c".to_owned()],
            "the real JSON omits difficulty/model on an unestimated phase"
        );
        Ok(())
    });
}

/// One pass of `buildEstimatePipeline` over the binary's real phase list;
/// the rater returns `ratings[stem]`. Returns the summary and every stem list
/// the rater was handed.
fn pass(m: &mut Module, repo: &PlanRepo, ratings: Value) -> Result<(Value, Vec<Value>), Failure> {
    let rated = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&rated);
    let agent = Agent::scripted(|_| Reply::Throw("no agent may run".into()));
    let mut deps = agent.install(&mut m.host);
    deps["parallelRate"] = m.host.register_fn(move |a| {
        let stems = a.first().cloned().unwrap_or(Value::Null);
        seen.borrow_mut().push(stems.clone());
        Ok(json!(strings(&stems)
            .into_iter()
            .map(|s| json!({ "stem": s, "difficulty": ratings[&s],
                             "justification": format!("touches `{s}` — costs $HOME and a \"quoted\" clause") }))
            .collect::<Vec<_>>()))
    });
    let run = m.call("buildEstimatePipeline", vec![deps])?;
    let summary = m
        .try_invoke(
            &run,
            vec![json!({ "roadmap": ROADMAP, "project": PROJECT, "rdmBin": rdm_bin(), "phaseList": phase_list(repo)? })],
        )?
        .map_err(Failure::Js)?;
    let rated = rated.borrow().clone();
    Ok((summary, rated))
}

fn run_writebacks(repo: &PlanRepo, summary: &Value) -> Result<(), Failure> {
    for e in summary["estimated"].as_array().into_iter().flatten() {
        check_eq!(
            e["writebackCommands"].as_array().map(Vec::len),
            Some(1),
            "{}: one command per phase",
            e["stem"]
        );
        repo.run_ok(
            &Run::bare(),
            e["writebackScript"].as_str().unwrap_or_default(),
        )?;
    }
    Ok(())
}

fn ratings() -> Value {
    json!({ "phase-1-a": "easy", "phase-2-b": "moderate" })
}

#[test]
fn writeback_sets_difficulty_and_core_tier() {
    run_real(|lib| {
        let mut m = Module::open(lib, LIB)?;
        let repo = seeded(None, "phase-3-c")?;
        let head = repo.head();
        let (summary, rated) = pass(&mut m, &repo, ratings())?;
        check_eq!(
            rated,
            vec![json!(["phase-1-a", "phase-2-b"])],
            "exactly the unestimated phases are rated, once"
        );
        check_eq!(
            summary["estimated"]
                .as_array()
                .map(|a| a.iter().map(|e| e["stem"].clone()).collect::<Vec<_>>()),
            Some(vec![json!("phase-1-a"), json!("phase-2-b")]),
            "both come back with a writeback"
        );
        check_eq!(
            summary["skipped"],
            json!(["phase-3-c"]),
            "the estimated phase is never rated"
        );
        run_writebacks(&repo, &summary)?;
        for (stem, difficulty, model, body) in [
            ("phase-1-a", "easy", "small", "ORIGINAL BODY A"),
            ("phase-2-b", "moderate", "medium", "ORIGINAL BODY B"),
        ] {
            let p = show(&repo, stem)?;
            check_eq!(
                p["difficulty"],
                json!(difficulty),
                "{stem}: the rating was persisted"
            );
            check_eq!(
                p["model"],
                json!(model),
                "{stem}: rdm-core derived the tier from the difficulty"
            );
            check!(
                p["body"].as_str().is_some_and(|b| b.trim() == body),
                "{stem}: the body is untouched: {}",
                p["body"]
            );
        }
        check_eq!(repo.head(), head, "the writeback commits nothing");
        Ok(())
    });
}

#[test]
fn estimated_phase_left_untouched() {
    run_real(|lib| {
        let mut m = Module::open(lib, LIB)?;
        let repo = seeded(None, "phase-3-c")?;
        let before = show(&repo, "phase-3-c")?;
        check_eq!(before["difficulty"], json!("hard"), "seeded hard");
        check_eq!(before["model"], json!("large"), "with its core tier");
        let (summary, _) = pass(&mut m, &repo, ratings())?;
        run_writebacks(&repo, &summary)?;
        check_eq!(
            show(&repo, "phase-3-c")?,
            before,
            "the pre-estimated phase is exactly as it was"
        );
        Ok(())
    });
}

#[test]
fn second_pass_idempotent() {
    run_real(|lib| {
        let mut m = Module::open(lib, LIB)?;
        let repo = seeded(None, "phase-3-c")?;
        let (first, _) = pass(&mut m, &repo, ratings())?;
        run_writebacks(&repo, &first)?;
        let (second, rated) = pass(&mut m, &repo, ratings())?;
        check!(
            rated.is_empty(),
            "nothing to rate means no fan-out: {rated:?}"
        );
        check_eq!(second["estimated"], json!([]), "no writeback on a re-run");
        check_eq!(
            second["skipped"],
            json!(["phase-1-a", "phase-2-b", "phase-3-c"]),
            "every phase now reads back as estimated"
        );
        Ok(())
    });
}

// --- The engine --------------------------------------------------------------------

fn rater() -> Agent {
    Agent::scripted(|call| match call.label.strip_prefix("estimate:rate:") {
        Some(stem) => {
            Reply::Value(json!({ "stem": stem, "difficulty": "moderate", "justification": "j" }))
        }
        None => Reply::Throw(format!("unexpected agent {}", call.label)),
    })
}

/// The caller's side: run the engine without `phaseList`, execute the list
/// command it names, and return the parsed list.
fn listed_by_engine(lib: &Lib, repo: &PlanRepo, args: &Value, run: &Run) -> Result<Value, Failure> {
    let agent = rater();
    let r = run_driver(lib, ENGINE, args.clone(), &agent)?.map_err(Failure::Js)?;
    check_eq!(r["fetchError"], json!(true), "no phase list, no run: {r}");
    check!(agent.calls().is_empty(), "no agent without the list");
    let out = repo.run_ok(run, r["listCommand"].as_str().unwrap_or_default())?;
    serde_json::from_str(&out).map_err(|e| Failure::Check(format!("{e}: {out}")))
}

fn with_list(args: &Value, list: Value) -> Value {
    let mut a = args.clone();
    a["phaseList"] = list;
    a
}

#[test]
fn engine_commands_use_injected_axes() {
    run_real(|lib| {
        let repo = seeded(None, "phase-3-c")?;
        let args = json!({ "roadmap": ROADMAP, "rdmBin": rdm_bin(), "project": PROJECT });
        let list = listed_by_engine(lib, &repo, &args, &Run::bare())?;
        let agent = rater();
        let r = run_driver(lib, ENGINE, with_list(&args, list), &agent)?.map_err(Failure::Js)?;
        let rated: Vec<String> = agent.labels();
        check_eq!(
            rated,
            vec![
                "estimate:rate:phase-1-a".to_owned(),
                "estimate:rate:phase-2-b".to_owned()
            ],
            "one rater per unestimated phase"
        );
        for c in agent.calls() {
            let stem = c.label.trim_start_matches("estimate:rate:");
            check!(
                c.prompt.contains(rdm_bin()) && c.prompt.contains(stem),
                "{}: the rater is told how to read its phase with the injected binary",
                c.label
            );
        }
        run_writebacks(&repo, &r)?;
        for stem in ["phase-1-a", "phase-2-b"] {
            check_eq!(
                show(&repo, stem)?["difficulty"],
                json!("moderate"),
                "{stem}: the executed writeback reached the target project"
            );
        }
        Ok(())
    });
}

#[test]
fn engine_output_deterministic() {
    run_real(|lib| {
        let repo = seeded(None, "phase-3-c")?;
        let args = with_list(
            &json!({ "roadmap": ROADMAP, "rdmBin": rdm_bin(), "project": PROJECT }),
            phase_list(&repo)?,
        );
        let a_agent = rater();
        let a = run_driver(lib, ENGINE, args.clone(), &a_agent)?.map_err(Failure::Js)?;
        let b_agent = rater();
        let b = run_driver(lib, ENGINE, args, &b_agent)?.map_err(Failure::Js)?;
        check_eq!(a, b, "identical inputs, identical result");
        let prompts = |ag: &Agent| ag.calls().into_iter().map(|c| c.prompt).collect::<Vec<_>>();
        check_eq!(
            prompts(&a_agent),
            prompts(&b_agent),
            "and identical prompts"
        );
        Ok(())
    });
}

#[test]
fn engine_without_rdm_bin_uses_plain_rdm() {
    run_real(|lib| {
        // The default project is the target project; no axes are passed.
        let repo = seeded(Some(PROJECT), "phase-3-c")?;
        let shim = Run::bare().path_prepend(&repo.rdm_on_path()?);
        let args = json!({ "roadmap": ROADMAP });
        let list = listed_by_engine(lib, &repo, &args, &shim)?;
        let agent = rater();
        let r = run_driver(lib, ENGINE, with_list(&args, list), &agent)?.map_err(Failure::Js)?;
        check!(!agent.calls().is_empty(), "the run dispatched raters");
        let first = r["estimated"][0]["writebackScript"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        check!(
            !repo.run(&Run::bare(), &first)?.status.success(),
            "without `rdm` on PATH the returned writeback cannot run"
        );
        run_writebacks_with(&repo, &r, &shim)?;
        check_eq!(
            show(&repo, "phase-1-a")?["difficulty"],
            json!("moderate"),
            "it ran through PATH"
        );
        Ok(())
    });
}

fn run_writebacks_with(repo: &PlanRepo, summary: &Value, run: &Run) -> Result<(), Failure> {
    for e in summary["estimated"].as_array().into_iter().flatten() {
        repo.run_ok(run, e["writebackScript"].as_str().unwrap_or_default())?;
    }
    Ok(())
}

#[test]
fn generator_in_sync() {
    GEN.in_sync();
}

#[test]
fn generator_drift_detected_then_healed() {
    GEN.drift_detected_then_healed();
}

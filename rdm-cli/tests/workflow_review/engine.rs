//! The standalone `rdm-wf-review-refute-fix` engine: its driver executed
//! with fake host primitives, and its stamped helpers extracted from the
//! shipped template copy without running the driver.

use rdm_devtools::workflow::{Host, is_undefined};
use serde_json::{Value, json};

use crate::support::{
    Agent, Failure, Lib, REVIEW_ENGINE, REVIEW_ENGINE_TEMPLATE, Reply, run_real, split,
};

const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SHA_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

/// Clean finders; the `ac` finder reports one PASS row.
fn clean_agent() -> Agent {
    Agent::scripted(|call| {
        if call.label == "find:code:ac" {
            Reply::Value(
                json!({ "ac": [{ "criterion": "AC1: works", "status": "PASS", "evidence": "test" }], "findings": [] }),
            )
        } else if call.label.starts_with("find:") {
            Reply::Value(json!({ "findings": [] }))
        } else {
            Reply::Value(json!({ "refuted": false, "confidence": 90 }))
        }
    })
}

/// Loads the engine's driver and calls it with `args`.
fn run_driver(
    lib: &Lib,
    args: Value,
    agent: &Agent,
) -> Result<Result<Value, rdm_devtools::workflow::JsError>, Failure> {
    crate::support::run_driver(lib, REVIEW_ENGINE, args, agent)
}

fn legacy_shape(lib: &Lib, mode: &str) -> crate::support::Outcome {
    let agent = clean_agent();
    let out = run_driver(
        lib,
        json!({ "mode": mode, "context": { "target": "legacy" } }),
        &agent,
    )?
    .map_err(Failure::Js)?;
    check_eq!(
        out["mode"],
        json!(mode),
        "the legacy shape reports its mode"
    );
    check_eq!(
        out["survivors"],
        json!([]),
        "clean finders leave no survivors"
    );
    check!(
        out.get("outcome").is_none_or(is_undefined),
        "a legacy report carries no outcome (it never approves)"
    );
    check!(
        out["budget"].is_object() && out["coverage"].is_object(),
        "budget and coverage are reported: {out}"
    );
    check!(
        !agent.calls_with(&format!("find:{mode}:")).is_empty(),
        "the driver dispatched {mode} finders"
    );
    check!(!agent.logs().is_empty(), "the driver logs its summary");
    Ok(())
}

#[test]
fn legacy_survivors_only_shape_code() {
    run_real(|lib| legacy_shape(lib, "code"));
}

#[test]
fn legacy_survivors_only_shape_plan() {
    run_real(|lib| legacy_shape(lib, "plan"));
}

#[test]
fn mixed_task_phase_identity_rejected() {
    run_real(|lib| {
        let agent = clean_agent();
        let r = run_driver(
            lib,
            json!({ "mode": "code", "task": "repair", "roadmap": "alpha", "phase": "phase-1-work" }),
            &agent,
        )?;
        check!(
            r.as_ref()
                .is_err_and(|e| e.message.contains("ambiguous review target")),
            "both identities are refused: {r:?}"
        );
        check!(
            agent.calls().is_empty(),
            "no agent is dispatched for an ambiguous target"
        );
        Ok(())
    });
}

#[test]
fn emitted_template_helpers_run_without_driver() {
    run_real(|lib| {
        let script = lib.read(REVIEW_ENGINE_TEMPLATE)?;
        let mut host = Host::start_default()?;
        let helpers = crate::support::load(host.extract_helpers(
            &script,
            &[
                "resolveRdmBin",
                "parseProjectArg",
                "projectFlag",
                "buildReviewPipeline",
                "classifyOutcome",
            ],
        ))?;
        let call =
            |host: &mut Host, name: &str, args: Vec<Value>| split(host.call(&helpers[name], args));
        check_eq!(
            call(&mut host, "resolveRdmBin", vec![Host::undefined()])?.map_err(Failure::Js)?,
            json!("rdm"),
            "an absent rdmBin defaults to PATH"
        );
        check_eq!(
            call(&mut host, "resolveRdmBin", vec![json!("/x/rdm")])?.map_err(Failure::Js)?,
            json!("/x/rdm"),
            "an explicit rdmBin is kept"
        );
        check!(
            call(&mut host, "resolveRdmBin", vec![json!(42)])?.is_err(),
            "a non-string rdmBin is refused"
        );
        check_eq!(
            call(&mut host, "parseProjectArg", vec![json!("demo")])?.map_err(Failure::Js)?,
            json!("demo"),
            "a plain project name"
        );
        check!(
            call(&mut host, "parseProjectArg", vec![json!("bad name;rm")])?.is_err(),
            "shell metacharacters are refused"
        );
        check_eq!(
            call(&mut host, "projectFlag", vec![json!({ "project": "demo" })])?
                .map_err(Failure::Js)?,
            json!(" --project demo"),
            "the project flag for a configured project"
        );
        check_eq!(
            call(&mut host, "projectFlag", vec![json!({})])?.map_err(Failure::Js)?,
            json!(""),
            "no flag without a project"
        );

        // The stamped review core runs from the emitted copy too.
        let agent = clean_agent();
        let deps = agent.install(&mut host);
        let run = call(&mut host, "buildReviewPipeline", vec![json!("code"), deps])?
            .map_err(Failure::Js)?;
        let out =
            split(host.call(&run, vec![json!({ "target": "task/t" })]))?.map_err(Failure::Js)?;
        check_eq!(
            out["acTable"][0]["status"],
            json!("PASS"),
            "the emitted pipeline resolves the AC table"
        );
        let outcome = call(
            &mut host,
            "classifyOutcome",
            vec![json!({ "planFindings": [], "codeReviews": [out["survivors"]], "acTable": out["acTable"] })],
        )?
        .map_err(Failure::Js)?;
        check_eq!(
            outcome,
            json!("reviewed"),
            "a clean emitted review classifies reviewed"
        );
        check!(
            agent
                .logs()
                .iter()
                .all(|l| !l.contains("review-refute-fix (")),
            "the driver never ran during extraction"
        );
        Ok(())
    });
}

fn standalone_args() -> Value {
    json!({
        "mode": "code", "task": "demo-task", "source": "/checkouts/demo", "base": SHA_B,
        "expectedHead": SHA_A, "expectedBranch": "topic", "persist": true, "gate": true,
        "rdmBin": "/opt/rdm/bin/rdm", "project": "demo"
    })
}

#[test]
fn driver_deterministic_and_emits_no_done_trailer() {
    run_real(|lib| {
        let first = run_driver(lib, standalone_args(), &clean_agent())?.map_err(Failure::Js)?;
        let second = run_driver(lib, standalone_args(), &clean_agent())?.map_err(Failure::Js)?;
        check_eq!(
            first.to_string(),
            second.to_string(),
            "the driver's result is byte-identical across runs"
        );
        check_eq!(
            first["outcome"],
            json!("reviewed"),
            "a clean, complete review is reviewed"
        );
        check_eq!(
            first["status"],
            json!("reviewed"),
            "and maps to the reviewed status"
        );
        check_eq!(
            first["writesCompletion"],
            json!(true),
            "a reviewed outcome permits the land-time trailer"
        );
        let mut lines = Vec::new();
        for key in ["persistCommands", "gateCommands"] {
            let cmds = first[key].as_array().cloned().unwrap_or_default();
            check!(
                !cmds.is_empty(),
                "{key} are returned for the orchestrator to run"
            );
            lines.extend(
                cmds.iter()
                    .filter_map(Value::as_str)
                    .flat_map(str::lines)
                    .map(str::to_owned),
            );
        }
        check!(
            lines
                .iter()
                .all(|l| !l.trim_start().starts_with("Done:") && !l.contains("\nDone:")),
            "no returned command writes a Done: completion directive"
        );
        check!(
            lines
                .iter()
                .any(|l| l.contains("/opt/rdm/bin/rdm") && l.contains("--project demo")),
            "the caller's rdmBin and project reach the returned commands"
        );
        Ok(())
    });
}

//! The find → refute → filter pipeline, in both review modes.

use serde_json::json;

use crate::support::{
    Agent, Failure, Js, Lib, MutantVerdict, Outcome, Reply, find, ids, mutant_verdict, plant,
    run_mutant, run_real,
};

const CTX_TARGET: &str = "phase widget/phase-1-foo";

fn ctx() -> serde_json::Value {
    json!({ "target": CTX_TARGET })
}

fn code_findings() -> serde_json::Value {
    plant(&[(
        "correctness",
        json!([
            { "id": "real-bug", "concern": "correctness", "severity": "blocking", "confidence": 90, "what_fails": "off-by-one" },
            { "id": "false-alarm", "concern": "correctness", "severity": "concern", "confidence": 85, "what_fails": "looks wrong but is fine" },
            { "id": "low-conf", "concern": "correctness", "severity": "suggestion", "confidence": 50, "what_fails": "possible nit" }
        ]),
    )])
}

fn code_verdicts() -> serde_json::Value {
    plant(&[
        ("real-bug", json!({ "refuted": false, "confidence": 95 })),
        ("false-alarm", json!({ "refuted": true, "confidence": 88 })),
        ("low-conf", json!({ "refuted": false, "confidence": 40 })),
    ])
}

/// Only the real, un-refuted, high-confidence finding survives; one finder
/// per code dimension; a distinct fresh refuter per gating finding whose
/// prompt is not a finder's; the target reaches every prompt; the `ac`
/// finder returning no `ac` array leaves no AC table.
fn code_mode_drops_refuted_and_low_confidence(lib: &Lib) -> Outcome {
    let mut js = Js::open(lib)?;
    let dims = js.call("resolveReviewers", vec![json!("code")])?;
    let dim_count = dims.as_array().map_or(0, Vec::len);
    let agent = Agent::planted(code_findings(), code_verdicts());
    let out = js.review_ok("code", &agent, ctx())?;

    check_eq!(
        ids(&out, "survivors"),
        vec!["real-bug"],
        "only the real finding survives"
    );
    check_eq!(
        out["acTable"],
        json!(null),
        "no AC table without an `ac` array"
    );
    let finders = agent.calls_with("find:");
    let refuters = agent.calls_with("refute:");
    check!(dim_count > 1, "code mode has several dimensions: {dims}");
    check_eq!(finders.len(), dim_count, "one finder per code dimension");
    let distinct: std::collections::BTreeSet<_> = finders.iter().map(|c| c.label.clone()).collect();
    check_eq!(
        distinct.len(),
        dim_count,
        "finder labels are distinct per dimension"
    );
    check_eq!(
        refuters.len(),
        2,
        "a fresh refuter per GATING finding (the suggestion is passed through)"
    );
    let refute_labels: std::collections::BTreeSet<_> =
        refuters.iter().map(|c| c.label.clone()).collect();
    check_eq!(
        refute_labels.len(),
        refuters.len(),
        "refuter labels are unique per finding"
    );
    check!(
        refuters
            .iter()
            .all(|r| finders.iter().all(|f| f.prompt != r.prompt)),
        "refuter prompts differ from finder prompts"
    );
    check!(
        agent.calls().iter().all(|c| c.prompt.contains(CTX_TARGET)),
        "context.target is threaded into every finder and refuter prompt"
    );
    let survivor = find(&out, "survivors", "real-bug")
        .cloned()
        .unwrap_or_default();
    check_eq!(
        survivor["severity"],
        json!("blocking"),
        "survivor keeps its severity"
    );
    check_eq!(
        survivor["confidence"],
        json!(90),
        "survivor keeps the finder's confidence"
    );
    Ok(())
}

#[test]
fn code_mode_drops_refuted_and_low_confidence_findings() {
    run_real(code_mode_drops_refuted_and_low_confidence);
}

#[test]
fn code_mode_drops_refuted_and_low_confidence_findings_mutant_floor_comparison() {
    run_mutant(
        Lib::mutant(
            "floor-comparison",
            &[(
                crate::support::REVIEW_LIB,
                "  if (confidence < CONFIDENCE_FLOOR) return false;\n  return true;\n}",
                "  if (confidence > CONFIDENCE_FLOOR) return false;\n  return true;\n}",
            )],
        ),
        code_mode_drops_refuted_and_low_confidence,
    );
}

/// A mutant whose replacement text is not valid JavaScript never reaches the
/// scenario, so it must be classified as not run — never as caught.
#[test]
fn syntax_breaking_mutant_is_a_load_failure_not_a_catch() {
    let lib = Lib::mutant(
        "syntax-breaking",
        &[(
            crate::support::REVIEW_LIB,
            "  if (confidence < CONFIDENCE_FLOOR) return false;\n  return true;\n}",
            "  if (confidence < CONFIDENCE_FLOOR) return false;\n  return true;\n}}}",
        )],
    );
    match mutant_verdict(&lib, code_mode_drops_refuted_and_low_confidence) {
        MutantVerdict::NotRun(Failure::Load(e)) => {
            assert_eq!(e.name, "SyntaxError", "the import failed to parse: {e}");
        }
        other => panic!("a syntax-breaking mutant must be a load failure, got {other:?}"),
    }
}

fn survival_rule(lib: &Lib) -> Outcome {
    let mut js = Js::open(lib)?;
    check_eq!(
        js.get("CONFIDENCE_FLOOR")?,
        json!(70),
        "the confidence floor is 70"
    );
    let cases = [
        (
            json!({ "confidence": 70 }),
            json!({ "refuted": false }),
            true,
            "exactly at the floor survives",
        ),
        (
            json!({ "confidence": 69 }),
            json!({ "refuted": false }),
            false,
            "below the floor is dropped",
        ),
        (
            json!({ "confidence": 100 }),
            json!({ "refuted": true }),
            false,
            "refuted is dropped regardless of confidence",
        ),
        (
            json!({ "confidence": 69 }),
            json!(null),
            false,
            "the floor applies to an ungraded finding",
        ),
        (
            json!({ "confidence": 70 }),
            json!(null),
            true,
            "an ungraded finding at the floor survives",
        ),
    ];
    for (finding, verdict, want, why) in cases {
        let got = js.call("survives", vec![finding, verdict])?;
        check_eq!(got, json!(want), "{why}");
    }
    Ok(())
}

#[test]
fn survival_rule_boundaries() {
    run_real(survival_rule);
}

#[test]
fn rank_total_order() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let ranked = js.call(
            "rankFindings",
            vec![json!([
                { "id": "b", "severity": "concern", "confidence": 80 },
                { "id": "a", "severity": "concern", "confidence": 80 },
                { "id": "c", "severity": "blocking", "confidence": 10 },
                { "id": "d", "severity": "suggestion", "confidence": 99 },
                { "id": "e", "severity": "concern", "confidence": 90 }
            ])],
        )?;
        check_eq!(
            ids(&json!({ "r": ranked }), "r"),
            vec!["c", "e", "a", "b", "d"],
            "blocking first; concerns by confidence desc then id; suggestion last"
        );
        Ok(())
    });
}

fn plan_findings() -> serde_json::Value {
    plant(&[
        (
            "coherence",
            json!([
                { "id": "vague-step", "concern": "coherence", "severity": "blocking", "confidence": 88, "what_fails": "step 3 is ambiguous" },
                { "id": "nonissue", "concern": "coherence", "severity": "concern", "confidence": 80, "what_fails": "reads odd but is fine" },
                { "id": "weak-plan", "concern": "coherence", "severity": "suggestion", "confidence": 50, "what_fails": "minor plan nit" }
            ]),
        ),
        ("architectural-fit", json!([])),
        ("unit-of-work", json!([])),
        ("intent-alignment", json!([])),
        ("restraint", json!([])),
    ])
}

fn plan_verdicts() -> serde_json::Value {
    plant(&[
        ("vague-step", json!({ "refuted": false, "confidence": 90 })),
        ("nonissue", json!({ "refuted": true, "confidence": 85 })),
        ("weak-plan", json!({ "refuted": false, "confidence": 40 })),
    ])
}

#[test]
fn plan_mode_battery() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let agent = Agent::planted(plan_findings(), plan_verdicts());
        let out = js.review_ok("plan", &agent, ctx())?;
        check_eq!(
            ids(&out, "survivors"),
            vec!["vague-step"],
            "plan: only the real finding survives"
        );
        check_eq!(
            out["acTable"],
            json!(null),
            "plan mode never resolves an AC table"
        );
        let finders: Vec<String> = agent
            .calls_with("find:")
            .into_iter()
            .map(|c| c.label)
            .collect();
        check_eq!(
            finders,
            vec![
                "find:plan:coherence",
                "find:plan:architectural-fit",
                "find:plan:unit-of-work",
                "find:plan:intent-alignment",
                "find:plan:restraint"
            ],
            "one finder per plan dimension, in dimension order"
        );
        check_eq!(
            agent.calls_with("refute:").len(),
            2,
            "a fresh refuter per GATING plan finding"
        );
        check!(
            agent.calls().iter().all(|c| c.prompt.contains(CTX_TARGET)),
            "context.target is threaded into every plan prompt"
        );
        Ok(())
    });
}

fn deterministic(lib: &Lib, mode: &str) -> Outcome {
    let (findings, verdicts) = if mode == "code" {
        (code_findings(), code_verdicts())
    } else {
        (plan_findings(), plan_verdicts())
    };
    let mut a = Js::open(lib)?;
    let first = a.review_ok(
        mode,
        &Agent::planted(findings.clone(), verdicts.clone()),
        ctx(),
    )?;
    let mut b = Js::open(lib)?;
    let second = b.review_ok(mode, &Agent::planted(findings, verdicts), ctx())?;
    check_eq!(
        first.to_string(),
        second.to_string(),
        "{mode} review output is byte-identical across runs"
    );
    Ok(())
}

#[test]
fn output_deterministic_code() {
    run_real(|lib| deterministic(lib, "code"));
}

#[test]
fn output_deterministic_plan() {
    run_real(|lib| deterministic(lib, "plan"));
}

#[test]
fn thrown_finder_drops_only_its_dimension() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let base = Agent::planted(code_findings(), code_verdicts());
        let agent = Agent::scripted(move |call| {
            if call.label == "find:code:correctness" {
                return Reply::Throw("boom finder".to_owned());
            }
            base.reply_for(call)
        });
        let out = js.review_ok("code", &agent, ctx())?;
        check_eq!(
            ids(&out, "survivors"),
            Vec::<String>::new(),
            "the thrown dimension's findings are gone, the run still resolves"
        );
        check!(
            out["coverage"]["failed"]
                .as_array()
                .is_some_and(|f| f.contains(&json!("correctness"))),
            "the thrown dimension is recorded as not having run: {}",
            out["coverage"]
        );
        Ok(())
    });
}

#[test]
fn crashed_refuter_keeps_finding() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let base = Agent::planted(
            plant(&[(
                "correctness",
                json!([{ "id": "infra", "concern": "correctness", "severity": "blocking", "confidence": 90, "what_fails": "real bug" }]),
            )]),
            json!({}),
        );
        let agent = Agent::scripted(move |call| {
            if call.label.starts_with("refute:") {
                return Reply::Throw("boom refuter".to_owned());
            }
            base.reply_for(call)
        });
        let out = js.review_ok("code", &agent, ctx())?;
        check_eq!(
            ids(&out, "survivors"),
            vec!["infra"],
            "a refuter crash keeps the finding"
        );
        check_eq!(
            find(&out, "survivors", "infra").map(|f| f["refuterError"].clone()),
            Some(json!(true)),
            "and marks it refuterError"
        );
        Ok(())
    });
}

#[test]
fn unknown_mode_throws() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let agent = Agent::planted(json!({}), json!({}));
        let err = match js.review("bogus", &agent, ctx())? {
            Ok(v) => {
                return Err(crate::support::Failure::Check(format!(
                    "bogus mode resolved: {v}"
                )));
            }
            Err(e) => e,
        };
        check!(err.message.contains("unknown review mode: bogus"), "{err}");
        check!(
            agent.calls().is_empty(),
            "no agent is dispatched for an unknown mode"
        );
        Ok(())
    });
}

#[test]
fn review_modes_are_code_and_plan() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        for mode in ["code", "plan"] {
            let agent = Agent::planted(json!({}), json!({}));
            let out = js.review(mode, &agent, ctx())?;
            check!(out.is_ok(), "{mode} is a review mode: {out:?}");
        }
        for mode in ["Code", "review", "", "ac"] {
            let agent = Agent::planted(json!({}), json!({}));
            let out = js.review(mode, &agent, ctx())?;
            check!(
                out.as_ref()
                    .is_err_and(|e| e.message.contains("unknown review mode")),
                "{mode:?} is not a review mode: {out:?}"
            );
        }
        Ok(())
    });
}

#[test]
fn finder_schema_accepts_category() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let agent = Agent::planted(json!({}), json!({}));
        js.review_ok("code", &agent, ctx())?;
        let calls = agent.calls();
        let schema_of = |label: &str| {
            calls
                .iter()
                .find(|c| c.label == label)
                .map(|c| c.opts["schema"].clone())
                .unwrap_or_default()
        };
        let finding = schema_of("find:code:correctness")["properties"]["findings"]["items"].clone();
        check_eq!(
            finding["properties"]["category"]["type"],
            json!("string"),
            "a FINDING may carry a `category` string"
        );
        check_eq!(
            finding["required"],
            json!(["id", "concern", "severity", "confidence", "what_fails"]),
            "`category` is optional: the required set is unchanged"
        );
        check_eq!(
            finding["properties"]["severity"]["enum"],
            json!(["blocking", "concern", "suggestion"]),
            "the severity vocabulary is the three-value contract"
        );
        check_eq!(
            schema_of("find:code:ac")["properties"]["findings"],
            schema_of("find:code:correctness")["properties"]["findings"],
            "the ac reviewer's narrative findings share the FINDING shape"
        );
        let outcome = js.call(
            "classifyPlanOutcome",
            vec![json!([{ "id": "b", "concern": "security", "category": "path-traversal", "severity": "blocking", "confidence": 90 }])],
        )?;
        check_eq!(
            outcome,
            json!("rework"),
            "a `category` slug does not perturb `concern`/`severity` classification"
        );
        Ok(())
    });
}

#[test]
fn ac_finder_dispatched_with_ac_review_schema() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let row = json!({ "criterion": "the CLI must reject empty input", "status": "FAIL", "evidence": "no such check exists" });
        let table = json!([row]);
        let reply = table.clone();
        let agent = Agent::scripted(move |call| {
            if call.label == "find:code:ac" {
                Reply::Value(json!({ "ac": reply }))
            } else if call.label.starts_with("find:") {
                Reply::Value(json!({ "findings": [] }))
            } else {
                Reply::Throw(format!("unexpected agent label: {}", call.label))
            }
        });
        let out = js.review_ok("code", &agent, ctx())?;
        check_eq!(
            out["survivors"],
            json!([]),
            "the AC table is not folded into findings"
        );
        check_eq!(
            out["acTable"],
            table,
            "the FAIL row is resolved intact as the AC table"
        );
        let ac_schema = js.get("AC_REVIEW_SCHEMA")?;
        let findings_schema = js.get("FINDINGS_SCHEMA")?;
        let ac_call = agent.calls_with("find:code:ac");
        check_eq!(ac_call.len(), 1, "the ac finder was dispatched once");
        check_eq!(
            ac_call[0].opts["schema"],
            ac_schema,
            "the ac finder is dispatched with AC_REVIEW_SCHEMA"
        );
        let other = agent.calls_with("find:code:correctness");
        check_eq!(
            other[0].opts["schema"],
            findings_schema,
            "other finders get FINDINGS_SCHEMA"
        );

        let plan = js.review_ok(
            "plan",
            &Agent::planted(plan_findings(), plan_verdicts()),
            ctx(),
        )?;
        check_eq!(
            plan["acTable"],
            json!(null),
            "plan mode never populates an AC table"
        );
        Ok(())
    });
}

#[test]
fn architectural_blocker_ranked_ahead_of_nit() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let agent = Agent::planted(
            plant(&[
                ("coherence", json!([])),
                ("intent-alignment", json!([])),
                (
                    "architectural-fit",
                    json!([{ "id": "tier-downgrade", "concern": "architectural-fit", "severity": "blocking", "confidence": 92,
                              "what_fails": "The plan silently downgrades the review tier on failure." }]),
                ),
                (
                    "unit-of-work",
                    json!([{ "id": "impl-nit", "concern": "unit-of-work", "severity": "concern", "confidence": 85,
                              "what_fails": "Off-by-one in the proposed pseudo-code." }]),
                ),
            ]),
            plant(&[
                (
                    "tier-downgrade",
                    json!({ "refuted": false, "confidence": 95 }),
                ),
                ("impl-nit", json!({ "refuted": false, "confidence": 88 })),
            ]),
        );
        let out = js.review_ok("plan", &agent, ctx())?;
        check_eq!(
            ids(&out, "survivors"),
            vec!["tier-downgrade", "impl-nit"],
            "the blocker ranks first"
        );
        check_eq!(
            find(&out, "survivors", "tier-downgrade").map(|f| f["severity"].clone()),
            Some(json!("blocking")),
            "the architectural finding stays blocking"
        );
        check_eq!(
            find(&out, "survivors", "impl-nit").map(|f| f["severity"].clone()),
            Some(json!("concern")),
            "the nit is not promoted"
        );
        Ok(())
    });
}

#[test]
fn restraint_finding_survives() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let agent = Agent::planted(
            plant(&[(
                "restraint",
                json!([{ "id": "over-specified", "concern": "restraint", "severity": "blocking", "confidence": 90,
                          "what_fails": "the plan prescribes a threshold the implementer should choose" }]),
            )]),
            plant(&[(
                "over-specified",
                json!({ "refuted": false, "confidence": 92 }),
            )]),
        );
        let out = js.review_ok("plan", &agent, ctx())?;
        check!(
            agent.calls_with("find:plan:restraint").len() == 1,
            "restraint runs by default in plan mode"
        );
        check_eq!(
            ids(&out, "survivors"),
            vec!["over-specified"],
            "a seeded restraint finding survives"
        );
        Ok(())
    });
}

fn null_agent() -> Agent {
    Agent::scripted(|_| Reply::Null)
}

#[test]
fn all_null_finders_reject_with_model() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let agent = null_agent();
        let mut c = ctx();
        c["findModel"] = json!("bogus-model");
        c["verifyModel"] = json!("bogus-model");
        let err = match js.review("code", &agent, c)? {
            Ok(v) => {
                return Err(crate::support::Failure::Check(format!(
                    "all-null finders resolved: {v}"
                )));
            }
            Err(e) => e,
        };
        check!(
            err.message.contains("every code dimension finder failed"),
            "{err}"
        );
        check!(
            err.message.contains("[models]"),
            "the model path names the [models] bindings: {err}"
        );
        let calls = agent.calls();
        check!(!calls.is_empty(), "finders were dispatched");
        check!(
            calls
                .iter()
                .all(|c| c.model() == Some(&json!("bogus-model"))),
            "findModel is threaded onto every finder call"
        );

        let clean = Agent::planted(json!({}), json!({}));
        let mut c = ctx();
        c["findModel"] = json!("haiku");
        c["verifyModel"] = json!("opus");
        let out = js.review_ok("code", &clean, c)?;
        check_eq!(
            out["survivors"],
            json!([]),
            "a genuinely clean review with models set still resolves"
        );
        Ok(())
    });
}

#[test]
fn all_null_finders_reject_without_model() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let agent = null_agent();
        let err = match js.review("code", &agent, ctx())? {
            Ok(v) => {
                return Err(crate::support::Failure::Check(format!(
                    "all-null finders resolved: {v}"
                )));
            }
            Err(e) => e,
        };
        check!(
            err.message
                .contains("every code dimension finder failed after one retry each"),
            "{err}"
        );
        check!(
            !err.message.contains("[models]"),
            "no [models] text without a model: {err}"
        );
        check!(
            agent.calls().iter().all(|c| c.model().is_none()),
            "no model key is invented"
        );
        Ok(())
    });
}

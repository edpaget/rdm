//! Finder retry, per-dimension participation, and the absent-vs-clean AC
//! table.

use std::collections::HashMap;

use serde_json::{Value, json};

use crate::support::{
    Agent, Js, Label, Lib, Outcome, REVIEW_LIB, Reply, ids, parse_label, run_mutant, run_real,
};

const TARGET: &str = "phase widget/phase-1-foo";

fn ctx() -> Value {
    json!({ "target": TARGET })
}

/// A finder script: `attempts[dim]` is consumed in order (the last entry
/// repeats); unscripted dimensions return a clean, participating payload.
fn scripted(attempts: Vec<(&str, Vec<Option<Value>>)>) -> Agent {
    let mut plan: HashMap<String, Vec<Option<Value>>> = attempts
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect();
    let mut cursor: HashMap<String, usize> = HashMap::new();
    Agent::scripted(move |call| match parse_label(&call.label) {
        Label::Find { dim, .. } => match plan.get_mut(&dim) {
            None if dim == "ac" => Reply::Value(json!({ "ac": [], "findings": [] })),
            None => Reply::Value(json!({ "findings": [] })),
            Some(script) => {
                let i = cursor.entry(dim.clone()).or_insert(0);
                let reply = script.get(*i).or_else(|| script.last()).cloned().flatten();
                *i += 1;
                reply.map_or(Reply::Null, Reply::Value)
            }
        },
        Label::Refute { .. } => Reply::Value(json!({ "refuted": false, "confidence": 95 })),
        Label::Other => Reply::Throw(format!("unexpected label: {}", call.label)),
    })
}

fn dims(js: &mut Js, mode: &str) -> Result<Vec<String>, crate::support::Failure> {
    let d = js.call("resolveReviewers", vec![json!(mode)])?;
    Ok(d.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x["key"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default())
}

fn count(agent: &Agent, label: &str) -> usize {
    agent.calls().iter().filter(|c| c.label == label).count()
}

fn blocking(id: &str) -> Value {
    json!({ "id": id, "concern": "correctness", "severity": "blocking", "confidence": 90, "what_fails": "x" })
}

fn finder_retried_once(lib: &Lib) -> Outcome {
    let mut js = Js::open(lib)?;
    let all = dims(&mut js, "code")?;
    let agent = scripted(vec![(
        "correctness",
        vec![None, Some(json!({ "findings": [blocking("flaky-1")] }))],
    )]);
    let out = js.review_ok("code", &agent, ctx())?;
    check!(
        ids(&out, "survivors").contains(&"flaky-1".to_owned()),
        "the retry's result reaches the survivors"
    );
    check_eq!(
        out["coverage"]["retried"],
        json!(["correctness"]),
        "the retried dimension is recorded"
    );
    check_eq!(
        out["coverage"]["failed"],
        json!([]),
        "a dimension that succeeded on retry is not a failure"
    );
    check_eq!(
        out["coverage"]["complete"],
        json!(true),
        "retried-then-succeeded is complete coverage"
    );
    let clause = js.call("coverageSummaryClause", vec![out["coverage"].clone()])?;
    check_eq!(
        clause,
        json!(""),
        "a complete run appends no summary clause"
    );
    check_eq!(
        count(&agent, "find:code:correctness"),
        1,
        "one first attempt"
    );
    check_eq!(
        count(&agent, "find:code:correctness:retry"),
        1,
        "exactly one retry"
    );
    for k in all.iter().filter(|k| *k != "correctness") {
        check_eq!(
            count(&agent, &format!("find:code:{k}")),
            1,
            "healthy dimension {k} ran once"
        );
        check_eq!(
            count(&agent, &format!("find:code:{k}:retry")),
            0,
            "{k} was not retried"
        );
    }
    Ok(())
}

#[test]
fn finder_retried_exactly_once() {
    run_real(finder_retried_once);
}

#[test]
fn mutant_retry_deleted() {
    run_mutant(
        Lib::mutant(
            "retry-deleted",
            &[(
                REVIEW_LIB,
                "          rec.retried = true;\n",
                "          rec.error = 'null'; throw new Error('MUTANT: retry dispatch deleted');\n",
            )],
        ),
        finder_retried_once,
    );
}

#[test]
fn empty_payload_not_retried() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let agent = scripted(vec![("correctness", vec![Some(json!({ "findings": [] }))])]);
        let out = js.review_ok("code", &agent, ctx())?;
        check!(
            agent.calls().iter().all(|c| !c.label.ends_with(":retry")),
            "an empty payload is not retried"
        );
        check_eq!(
            out["coverage"]["complete"],
            json!(true),
            "an empty payload counts as participation"
        );
        check_eq!(
            out["coverage"]["failed"],
            json!([]),
            "and is never a failure"
        );
        Ok(())
    });
}

fn dead_dimensions_recorded(lib: &Lib) -> Outcome {
    let mut js = Js::open(lib)?;
    let all = dims(&mut js, "code")?;
    check!(all.len() > 3, "enough code dimensions: {all:?}");
    let dead = vec![all[0].clone(), all[3].clone()];
    let mut script: Vec<(&str, Vec<Option<Value>>)> = vec![(
        "correctness",
        vec![Some(json!({ "findings": [blocking("live-1")] }))],
    )];
    for k in &dead {
        script.push((k.as_str(), vec![None, None]));
    }
    let agent = scripted(script);
    let out = js.review_ok("code", &agent, ctx())?;
    let cov = &out["coverage"];
    check_eq!(
        cov["failed"],
        json!(dead),
        "both dead dimensions are recorded, in selection order"
    );
    check_eq!(
        cov["total"],
        json!(all.len()),
        "total is the selected dimension count"
    );
    let ran: Vec<&String> = all.iter().filter(|k| !dead.contains(k)).collect();
    check_eq!(cov["ran"], json!(ran), "the rest ran, in selection order");
    check_eq!(
        cov["complete"],
        json!(false),
        "a run missing a dimension is not complete"
    );
    check!(
        ids(&out, "survivors").contains(&"live-1".to_owned()),
        "live dimensions still contribute"
    );
    for k in &dead {
        check_eq!(
            count(&agent, &format!("find:code:{k}")),
            1,
            "{k} first attempt"
        );
        check_eq!(
            count(&agent, &format!("find:code:{k}:retry")),
            1,
            "{k} retried exactly once"
        );
    }
    let projected = js.call("buildReviewCoverage", vec![json!([cov]), json!(null)])?;
    let clause = js.call("coverageSummaryClause", vec![projected])?;
    let clause = clause.as_str().unwrap_or_default().to_owned();
    check!(
        clause.contains(&format!(
            "[review coverage: {}/{} dimensions ran; failed: ",
            ran.len(),
            all.len()
        )),
        "the clause states the coverage: {clause}"
    );
    for k in &dead {
        check!(
            clause.contains(k.as_str()),
            "the clause names {k}: {clause}"
        );
    }
    check!(
        !clause.chars().any(|c| matches!(c, '"' | '\'' | '$' | '`')),
        "the clause is free of shell-quoting hazards: {clause}"
    );
    Ok(())
}

#[test]
fn dead_dimensions_recorded_in_order() {
    run_real(dead_dimensions_recorded);
}

#[test]
fn mutant_complete_hardcoded() {
    run_mutant(
        Lib::mutant(
            "complete-hardcoded",
            &[(
                REVIEW_LIB,
                "      complete: attempts.every((a) => a.ran),\n",
                "      complete: true, // MUTANT\n",
            )],
        ),
        dead_dimensions_recorded,
    );
}

#[test]
fn absent_vs_clean_ac_table() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let dead = js.review_ok("code", &scripted(vec![("ac", vec![None, None])]), ctx())?;
        check_eq!(
            dead["acTable"],
            json!(null),
            "a dead ac finder leaves acTable null"
        );
        check_eq!(
            dead["coverage"]["acDimensionRan"],
            json!(false),
            "recorded as not having run"
        );
        check_eq!(
            dead["coverage"]["acTableAbsent"],
            json!(true),
            "ABSENT is recorded explicitly"
        );
        let clause = js.call("coverageSummaryClause", vec![dead["coverage"].clone()])?;
        check!(
            clause.as_str().is_some_and(|c| c.contains("NO AC TABLE")),
            "the absent table is named: {clause}"
        );

        let clean = js.review_ok(
            "code",
            &scripted(vec![(
                "ac",
                vec![Some(json!({ "ac": [], "findings": [] }))],
            )]),
            ctx(),
        )?;
        check_eq!(
            clean["acTable"],
            json!([]),
            "a clean run reports an empty table"
        );
        check_eq!(
            clean["coverage"]["acDimensionRan"],
            json!(true),
            "the ac dimension ran"
        );
        check_eq!(
            clean["coverage"]["acTableAbsent"],
            json!(false),
            "a clean table is not an absent one"
        );
        let clause = js.call("coverageSummaryClause", vec![clean["coverage"].clone()])?;
        check_eq!(clause, json!(""), "a clean complete run appends nothing");

        let plan = js.review_ok("plan", &scripted(vec![]), ctx())?;
        check_eq!(
            plan["coverage"]["acDimensionRan"],
            json!(null),
            "plan mode reports no ac participation"
        );
        check_eq!(
            plan["coverage"]["acTableAbsent"],
            json!(false),
            "plan mode is never AC-table-absent"
        );
        let clause = js.call("coverageSummaryClause", vec![plan["coverage"].clone()])?;
        check_eq!(clause, json!(""), "a complete plan run appends nothing");

        check_eq!(
            js.call("acTableHasGap", vec![json!(null)])?,
            json!(false),
            "an absent table is not a gap"
        );
        check_eq!(
            js.call("acTableHasGap", vec![json!([])])?,
            json!(false),
            "an empty table is not a gap"
        );
        Ok(())
    });
}

fn no_models_guard(lib: &Lib) -> Outcome {
    let mut js = Js::open(lib)?;
    for mode in ["code", "plan"] {
        let all = dims(&mut js, mode)?;
        let one = scripted(vec![(all[0].as_str(), vec![None, None])]);
        let out = js.review_ok(mode, &one, json!({ "target": TARGET }))?;
        check_eq!(
            out["coverage"]["failed"],
            json!([all[0]]),
            "{mode}: one dead finder is recorded, not fatal"
        );
        check_eq!(
            out["coverage"]["complete"],
            json!(false),
            "{mode}: the run is marked incomplete"
        );

        let every = scripted(all.iter().map(|k| (k.as_str(), vec![None, None])).collect());
        let r = js.review(mode, &every, json!({ "target": TARGET }))?;
        let want = format!("every {mode} dimension finder failed");
        check!(
            r.as_ref().is_err_and(|e| e.message.contains(&want)),
            "{mode}: a wholesale failure with no model still rejects: {r:?}"
        );
    }
    let all = dims(&mut js, "code")?;
    let every = scripted(all.iter().map(|k| (k.as_str(), vec![None, None])).collect());
    let r = js.review(
        "code",
        &every,
        json!({ "target": TARGET, "findModel": "bogus" }),
    )?;
    let msg = r.err().map(|e| e.message).unwrap_or_default();
    check!(
        msg.contains("every code dimension finder failed"),
        "shared prefix: {msg}"
    );
    check!(
        msg.contains("[models] tier bindings"),
        "the model path names the [models] bindings: {msg}"
    );
    Ok(())
}

#[test]
fn no_models_dead_finder_recorded_not_fatal() {
    run_real(no_models_guard);
}

#[test]
fn mutant_model_conditional_guard() {
    run_mutant(
        Lib::mutant(
            "model-conditional-guard",
            &[(
                REVIEW_LIB,
                "    if (dims.length > 0 && perDimension.every((d) => d === null || d === undefined)) {\n",
                "    if (findModel && dims.length > 0 && perDimension.every((d) => d === null || d === undefined)) { // MUTANT\n",
            )],
        ),
        no_models_guard,
    );
}

#[test]
fn dead_dimension_does_not_gate() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let all = dims(&mut js, "code")?;
        for seed in [None, Some(json!({ "findings": [blocking("blk")] }))] {
            let base = |extra: Option<(&str, Vec<Option<Value>>)>| {
                let mut s: Vec<(&str, Vec<Option<Value>>)> = Vec::new();
                if let Some(v) = &seed {
                    s.push(("correctness", vec![Some(v.clone())]));
                }
                if let Some(e) = extra {
                    s.push(e);
                }
                scripted(s)
            };
            let healthy = js.review_ok("code", &base(None), ctx())?;
            let with_dead = js.review_ok(
                "code",
                &base(Some((all[2].as_str(), vec![None, None]))),
                ctx(),
            )?;
            let classify = |js: &mut Js, survivors: &Value| {
                js.call(
                    "classifyOutcome",
                    vec![
                        json!({ "planFindings": [], "acTable": null, "codeReviews": [survivors] }),
                    ],
                )
            };
            let a = classify(&mut js, &with_dead["survivors"])?;
            let b = classify(&mut js, &healthy["survivors"])?;
            check_eq!(a, b, "a dead dimension yields the same outcome");
            check!(
                with_dead["coverage"]["failed"] != healthy["coverage"]["failed"],
                "they differ only in recorded coverage"
            );
        }
        let frozen = [
            (
                json!({ "planFindings": [{ "severity": "blocking" }] }),
                "escalated",
            ),
            (
                json!({ "planFindings": [], "acTable": [{ "status": "FAIL" }] }),
                "rework",
            ),
            (
                json!({ "planFindings": [], "acTable": [{ "status": "PASS" }], "codeReviews": [[]] }),
                "reviewed",
            ),
            (
                json!({ "planFindings": [], "acTable": null, "codeReviews": [[{ "severity": "blocking" }]] }),
                "rework",
            ),
            (
                json!({ "planFindings": [], "acTable": null, "codeReviews": [[{ "severity": "concern" }]] }),
                "reviewed",
            ),
            (
                json!({ "planFindings": [], "acTable": null, "codeReviews": [[{ "severity": "concern" }]], "tier": "large" }),
                "rework",
            ),
            (json!({}), "reviewed"),
        ];
        for (input, want) in frozen {
            let got = js.call("classifyOutcome", vec![input.clone()])?;
            check_eq!(got, json!(want), "legacy classifyOutcome input {input}");
        }
        Ok(())
    });
}

#[test]
fn projection_helpers() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let build = |js: &mut Js, rounds: Value, plan: Value| {
            js.call("buildReviewCoverage", vec![rounds, plan])
        };
        let clause = |js: &mut Js, c: Value| js.call("coverageSummaryClause", vec![c]);
        check_eq!(
            build(&mut js, json!([]), json!(null))?,
            json!(null),
            "nothing reported is null"
        );
        check_eq!(
            build(&mut js, json!(null), json!(null))?,
            json!(null),
            "an older caller reporting nothing is null"
        );
        check_eq!(
            clause(&mut js, json!(null))?,
            json!(""),
            "a null projection appends nothing"
        );
        let bad = json!({ "total": 3, "selected": ["a", "b", "c"], "ran": ["a", "b"], "failed": ["c"], "retried": ["c"], "complete": false, "acTableAbsent": false });
        let good = json!({ "total": 3, "selected": ["a", "b", "c"], "ran": ["a", "b", "c"], "failed": [], "retried": [], "complete": true, "acTableAbsent": false });
        let p = build(&mut js, json!([bad, good]), json!(null))?;
        check_eq!(
            p["complete"],
            json!(false),
            "any incomplete round makes the run incomplete"
        );
        check_eq!(
            p["everIncomplete"],
            json!(true),
            "everIncomplete mirrors it"
        );
        check_eq!(
            p["failed"],
            json!(["c"]),
            "the reported failure is the incomplete round's"
        );
        check_eq!(p["rounds"], json!(2), "both rounds are counted");
        let c = clause(&mut js, p)?;
        check!(
            c.as_str()
                .is_some_and(|s| s.contains("[review coverage: 2/3 dimensions ran; failed: c]")),
            "exact clause: {c}"
        );
        let healthy = build(&mut js, json!([good, good]), json!(null))?;
        check_eq!(
            clause(&mut js, healthy)?,
            json!(""),
            "an all-complete projection is silent"
        );
        let merged = build(&mut js, json!([good]), json!([bad]))?;
        check_eq!(
            merged["planRounds"],
            json!(1),
            "plan rounds are counted separately"
        );
        check_eq!(
            merged["failed"],
            json!(["c"]),
            "the plan round gap is reported"
        );
        let single = build(&mut js, json!([]), bad)?;
        check_eq!(
            single["everIncomplete"],
            json!(true),
            "a single plan coverage object is accepted"
        );
        Ok(())
    });
}

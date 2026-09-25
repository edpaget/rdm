//! The non-gating refutation skip and the refutation budget.

use std::collections::{BTreeSet, HashMap};

use serde_json::{Value, json};

use crate::support::{
    Agent, AgentCall, Failure, Js, Lib, Outcome, REVIEW_LIB, Reply, find, ids, plant, run_mutant,
    run_real,
};

const TARGET: &str = "phase widget/phase-1-foo";

fn ctx() -> Value {
    json!({ "target": TARGET })
}

fn ctx_with(key: &str, value: Value) -> Value {
    let mut c = ctx();
    c[key] = value;
    c
}

/// `b01..bNN`, all blocking, confidence descending from `base`.
fn gating(n: usize, base: i64) -> Vec<Value> {
    (0..n)
        .map(|i| {
            let id = format!("b{:02}", i + 1);
            json!({ "id": id, "severity": "blocking", "confidence": base - i as i64, "what_fails": format!("defect {id}") })
        })
        .collect()
}

fn correctness(findings: Vec<Value>) -> Agent {
    Agent::planted(plant(&[("correctness", json!(findings))]), json!({}))
}

fn get<'a>(v: &'a Value, id: &str) -> Result<&'a Value, Failure> {
    find(v, "survivors", id)
        .ok_or_else(|| Failure::Check(format!("no survivor {id}: {}", v["survivors"])))
}

// --- §8: the non-gating skip -------------------------------------------------

#[test]
fn needs_refutation_fail_safe() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let cases = [
            (
                json!({ "severity": "blocking" }),
                true,
                "blocking is refuted",
            ),
            (
                json!({ "severity": "concern" }),
                true,
                "concern is refuted (it gates at the large tier)",
            ),
            (
                json!({ "severity": "suggestion" }),
                false,
                "suggestion is the one non-gating severity",
            ),
            (json!({}), true, "a missing severity is still refuted"),
            (
                json!({ "severity": "Suggestion" }),
                true,
                "an off-case severity is still refuted",
            ),
            (
                json!({ "severity": "nit" }),
                true,
                "an unknown severity is still refuted",
            ),
            (json!(null), true, "a null finding is still refuted"),
        ];
        for (finding, want, why) in cases {
            check_eq!(
                js.call("needsRefutation", vec![finding])?,
                json!(want),
                "{why}"
            );
        }
        Ok(())
    });
}

fn mixed_severities() -> Value {
    json!([
        { "id": "b1", "severity": "blocking", "confidence": 90, "what_fails": "real bug" },
        { "id": "c1", "severity": "concern", "confidence": 90, "what_fails": "real concern" },
        { "id": "s1", "severity": "suggestion", "confidence": 90, "what_fails": "readability nit" },
        { "id": "s2", "severity": "suggestion", "confidence": 60, "what_fails": "below-floor nit" }
    ])
}

fn non_gating_pass_through(lib: &Lib, mode: &str, dim: &str) -> Outcome {
    let mut js = Js::open(lib)?;
    let agent = Agent::planted(plant(&[(dim, mixed_severities())]), json!({}));
    let out = js.review_ok(mode, &agent, ctx())?;
    let refuters: BTreeSet<String> = agent
        .calls_with("refute:")
        .into_iter()
        .map(|c| c.label)
        .collect();
    let want: BTreeSet<String> = [format!("refute:{mode}:b1"), format!("refute:{mode}:c1")].into();
    check_eq!(
        refuters,
        want,
        "{mode}: exactly one refuter per gating finding, none for a suggestion"
    );
    let s1 = get(&out, "s1")?;
    check_eq!(
        s1["unrefuted"],
        json!(true),
        "{mode}: the pass-through is marked unrefuted"
    );
    check_eq!(
        s1["unrefutedReason"],
        json!("non-gating"),
        "{mode}: the reason is non-gating"
    );
    check!(
        find(&out, "survivors", "s2").is_none(),
        "{mode}: the floor still drops a below-floor suggestion"
    );
    check!(
        get(&out, "b1")?.get("unrefuted").is_none(),
        "{mode}: a graded blocking finding is not marked"
    );
    check!(
        get(&out, "c1")?.get("unrefuted").is_none(),
        "{mode}: a graded concern is not marked"
    );
    let b = &out["budget"];
    check_eq!(
        b["gating"],
        json!(2),
        "{mode}: only the gating findings are budget candidates"
    );
    check_eq!(b["graded"], json!(2), "{mode}: both were graded");
    check_eq!(
        b["passedThroughNonGating"],
        json!(2),
        "{mode}: both suggestions are non-gating pass-throughs"
    );
    check_eq!(
        b["passedThroughBudget"],
        json!(0),
        "{mode}: a non-gating pass-through consumes no budget"
    );
    check_eq!(b["hit"], json!(false), "{mode}: no budget hit");
    Ok(())
}

#[test]
fn non_gating_suggestion_passes_through_code() {
    run_real(|lib| non_gating_pass_through(lib, "code", "correctness"));
}

#[test]
fn non_gating_suggestion_passes_through_plan() {
    run_real(|lib| non_gating_pass_through(lib, "plan", "coherence"));
}

#[test]
fn mutant_non_gating_widened_to_concern() {
    run_mutant(
        Lib::mutant(
            "non-gating-widened",
            &[(
                REVIEW_LIB,
                "const NON_GATING_SEVERITIES = ['suggestion'];",
                "const NON_GATING_SEVERITIES = ['suggestion', 'concern']; // MUTANT",
            )],
        ),
        |lib| non_gating_pass_through(lib, "code", "correctness"),
    );
}

#[test]
fn mutant_unrefuted_marker_renamed() {
    run_mutant(
        Lib::mutant(
            "unrefuted-marker-renamed",
            &[(
                REVIEW_LIB,
                "unrefuted: true, unrefutedReason: 'non-gating'",
                "unrefutedMUTANT: true, unrefutedReason: 'non-gating'",
            )],
        ),
        |lib| non_gating_pass_through(lib, "code", "correctness"),
    );
}

#[test]
fn crashed_refuter_keeps_gating_finding() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let base = correctness(vec![mixed_severities()[0].clone()]);
        let agent = Agent::scripted(move |call| {
            if call.label.starts_with("refute:") {
                return Reply::Throw("boom refuter".to_owned());
            }
            base.reply_for(call)
        });
        let out = js.review_ok("code", &agent, ctx())?;
        check_eq!(
            ids(&out, "survivors"),
            vec!["b1"],
            "a crashed refuter keeps its gating finding"
        );
        check!(
            get(&out, "b1")?.get("unrefuted").is_none(),
            "a crash is not marked as a deliberate skip"
        );
        Ok(())
    });
}

// --- §9: the refutation budget -----------------------------------------------

fn default_budget(lib: &Lib) -> Outcome {
    let mut js = Js::open(lib)?;
    check_eq!(
        js.get("DEFAULT_MAX_REFUTATIONS")?,
        json!(5),
        "the default budget is 5"
    );
    let ok = [
        (json!({ "$undefined": true }), 5, "unset"),
        (json!(null), 5, "null"),
        (json!(""), 5, "empty string"),
        (json!(5), 5, "a number"),
        (json!("5"), 5, "an integer string"),
        (json!(0), 0, "0 is legal and distinct from unset"),
    ];
    for (value, want, why) in ok {
        check_eq!(
            js.call("resolveRefutationBudget", vec![value])?,
            json!(want),
            "{why}"
        );
    }
    for bad in [
        json!("5abc"),
        json!(-1),
        json!(-0.0),
        json!(1.5),
        json!({}),
        json!([]),
        json!(true),
        json!("five"),
    ] {
        let r = js.try_call("resolveRefutationBudget", vec![bad.clone()])?;
        let msg = r.err().map(|e| e.message).unwrap_or_default();
        check!(
            msg.contains("maxRefutations must be a non-negative integer"),
            "rejects {bad}: {msg:?}"
        );
        check!(
            msg.contains("0 means grade nothing"),
            "the error states what 0 means: {msg}"
        );
        check!(
            msg.contains("no \"uncapped\" sentinel"),
            "the error states there is no uncapped sentinel: {msg}"
        );
    }
    Ok(())
}

#[test]
fn default_budget_and_resolution() {
    run_real(default_budget);
}

#[test]
fn mutant_vii_default_budget() {
    run_mutant(
        Lib::mutant(
            "default-budget-3",
            &[(
                REVIEW_LIB,
                "const DEFAULT_MAX_REFUTATIONS = 5;",
                "const DEFAULT_MAX_REFUTATIONS = 3; // MUTANT",
            )],
        ),
        default_budget,
    );
}

fn rank_set() -> Value {
    json!([
        { "order": 0, "finding": { "id": "z", "severity": "suggestion", "confidence": 100 } },
        { "order": 1, "finding": { "id": "y", "severity": "concern", "confidence": 80 } },
        { "order": 2, "finding": { "id": "x", "severity": "blocking", "confidence": 70 } },
        { "order": 3, "finding": { "id": "w", "severity": "blocking", "confidence": 90 } },
        { "order": 4, "finding": { "id": "nit", "severity": "unknown-severity", "confidence": 100 } },
        { "order": 5, "finding": { "id": "a", "severity": "concern", "confidence": 80 } },
        { "order": 6, "finding": { "id": "dup", "severity": "blocking", "confidence": 70 } },
        { "order": 7, "finding": { "id": "dup", "severity": "blocking", "confidence": 70 } },
        { "order": 8, "finding": { "id": "noconf", "severity": "concern" } }
    ])
}

fn orders(v: &Value) -> Vec<i64> {
    v.as_array()
        .map(|a| a.iter().filter_map(|c| c["order"].as_i64()).collect())
        .unwrap_or_default()
}

fn rank_order(lib: &Lib) -> Outcome {
    let mut js = Js::open(lib)?;
    let set = rank_set();
    let ranked = js.call("rankBudgetCandidates", vec![set.clone()])?;
    check_eq!(
        orders(&ranked),
        vec![3, 6, 7, 2, 5, 1, 8, 0, 4],
        "severity, confidence desc, id, then source order"
    );
    let dup = js.call("rankBudgetCandidates", vec![json!([set[7], set[6]])])?;
    check_eq!(
        orders(&dup),
        vec![6, 7],
        "a shared id is ordered by source order whatever the input order"
    );
    check_eq!(
        js.call("rankBudgetCandidates", vec![json!(null)])?,
        json!([]),
        "a non-array yields an empty ranking"
    );
    Ok(())
}

#[test]
fn rank_budget_candidates_order() {
    run_real(rank_order);
}

#[test]
fn mutant_i_source_order_tiebreak() {
    run_mutant(
        Lib::mutant(
            "order-tiebreak-dropped",
            &[(
                REVIEW_LIB,
                "    return oa - ob;\n",
                "    return 0; // MUTANT\n",
            )],
        ),
        rank_order,
    );
}

#[test]
fn mutant_ii_confidence_ascending() {
    run_mutant(
        Lib::mutant(
            "confidence-ascending",
            &[(
                REVIEW_LIB,
                "    const cb = fb.confidence != null ? fb.confidence : 0;\n    if (ca !== cb) return cb - ca;\n",
                "    const cb = fb.confidence != null ? fb.confidence : 0;\n    if (ca !== cb) return ca - cb; // MUTANT\n",
            )],
        ),
        rank_order,
    );
}

fn under_at_over(lib: &Lib) -> Outcome {
    let mut js = Js::open(lib)?;
    // Under budget: 3 gating candidates.
    let agent = correctness(gating(3, 99));
    let out = js.review_ok("code", &agent, ctx())?;
    check_eq!(
        agent.refuted_ids().len(),
        3,
        "under budget: every gating finding is graded"
    );
    check_eq!(
        out["budget"],
        json!({ "max": 5, "produced": 3, "consolidated": 3, "collapsed": 0, "gating": 3, "graded": 3, "passedThroughNonGating": 0, "passedThroughBudget": 0, "refuterErrors": 0, "hit": false, "clustering": { "ran": true, "retried": true, "failedOpen": true, "reason": "null" } }),
        "under budget: exact accounting"
    );
    check_eq!(
        out["survivors"],
        json!([
            { "id": "b01", "severity": "blocking", "confidence": 99, "what_fails": "defect b01", "concern": "correctness" },
            { "id": "b02", "severity": "blocking", "confidence": 98, "what_fails": "defect b02", "concern": "correctness" },
            { "id": "b03", "severity": "blocking", "confidence": 97, "what_fails": "defect b03", "concern": "correctness" }
        ]),
        "under budget: survivors carry no marker and no extra field"
    );
    check!(
        !agent.logs().join("\n").contains("BUDGET HIT"),
        "under budget: no budget clause in the log"
    );

    // Exactly at budget: 5 candidates is not a hit.
    let agent = correctness(gating(5, 99));
    let out = js.review_ok("code", &agent, ctx())?;
    check_eq!(
        agent.refuted_ids().len(),
        5,
        "at budget: exactly 5 refuters"
    );
    check_eq!(
        out["budget"]["hit"],
        json!(false),
        "at budget: the boundary is not a hit"
    );
    check_eq!(
        out["budget"]["passedThroughBudget"],
        json!(0),
        "at budget: nothing overflowed"
    );

    // Over budget: 13 candidates.
    let planted = gating(13, 99);
    let agent = correctness(planted.clone());
    let out = js.review_ok("code", &agent, ctx())?;
    let graded: BTreeSet<String> = agent.refuted_ids().into_iter().collect();
    check_eq!(graded.len(), 5, "over budget: exactly 5 refuters");
    let records: Vec<Value> = planted
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let mut f = f.clone();
            f["concern"] = json!("correctness");
            json!({ "order": i, "finding": f })
        })
        .collect();
    let ranked = js.call("rankBudgetCandidates", vec![json!(records)])?;
    let top: BTreeSet<String> = ranked
        .as_array()
        .into_iter()
        .flatten()
        .take(5)
        .filter_map(|c| c["finding"]["id"].as_str().map(str::to_owned))
        .collect();
    check_eq!(
        graded,
        top,
        "over budget: the graded five are the module's own top five"
    );
    check_eq!(
        out["budget"],
        json!({ "max": 5, "produced": 13, "consolidated": 13, "collapsed": 0, "gating": 13, "graded": 5, "passedThroughNonGating": 0, "passedThroughBudget": 8, "refuterErrors": 0, "hit": true, "clustering": { "ran": true, "retried": true, "failedOpen": true, "reason": "null" } }),
        "over budget: exact accounting"
    );
    let overflow: Vec<&Value> = out["survivors"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|f| !graded.contains(f["id"].as_str().unwrap_or_default()))
        .collect();
    check_eq!(
        overflow.len(),
        8,
        "over budget: all 8 overflow findings survive the floor"
    );
    check!(
        overflow
            .iter()
            .all(|f| f["unrefuted"] == json!(true) && f["unrefutedReason"] == json!("budget")),
        "overflow findings are marked as cut for budget"
    );
    let log = agent.logs().join("\n");
    for needle in [
        "BUDGET HIT",
        "13 finding(s) produced",
        "5 graded",
        "8 passed through for budget",
        "cap 5",
    ] {
        check!(log.contains(needle), "the log states {needle:?}: {log}");
    }
    Ok(())
}

#[test]
fn refutation_budget_under_at_over_bound() {
    run_real(under_at_over);
}

#[test]
fn mutant_vi_budget_cut_off_by_one() {
    run_mutant(
        Lib::mutant(
            "budget-cut-off-by-one",
            &[(
                REVIEW_LIB,
                "    const toGrade = ranked.slice(0, maxRefutations);\n",
                "    const toGrade = ranked.slice(0, maxRefutations + 1); // MUTANT\n",
            )],
        ),
        under_at_over,
    );
}

fn four_states(lib: &Lib) -> Outcome {
    let mut js = Js::open(lib)?;
    let mut planted = gating(8, 99);
    planted.push(json!({ "id": "s1", "severity": "suggestion", "confidence": 90, "what_fails": "readability nit" }));
    let base = correctness(planted);
    let agent = Agent::scripted(move |call| {
        if call.label == "refute:code:b01" {
            return Reply::Throw("boom refuter b01".to_owned());
        }
        base.reply_for(call)
    });
    let out = js.review_ok("code", &agent, ctx())?;
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for f in out["survivors"].as_array().into_iter().flatten() {
        let unrefuted = f["unrefuted"] == json!(true);
        let crashed = f["refuterError"] == json!(true);
        let mut labels = Vec::new();
        if !unrefuted && !crashed {
            labels.push("graded-and-survived");
        }
        if unrefuted && f["unrefutedReason"] == json!("non-gating") {
            labels.push("skipped-as-non-gating");
        }
        if unrefuted && f["unrefutedReason"] == json!("budget") {
            labels.push("passed-over-for-budget");
        }
        if crashed && !unrefuted {
            labels.push("grading-crashed");
        }
        check_eq!(
            labels.len(),
            1,
            "finding {} maps to exactly one state: {labels:?}",
            f["id"]
        );
        seen.insert(labels[0]);
    }
    check_eq!(
        seen,
        BTreeSet::from([
            "graded-and-survived",
            "grading-crashed",
            "passed-over-for-budget",
            "skipped-as-non-gating"
        ]),
        "all four states occur and are distinguishable by markers alone"
    );
    let b01 = get(&out, "b01")?;
    check_eq!(
        b01["refuterError"],
        json!(true),
        "a crashed refuter marks refuterError"
    );
    check!(
        b01.get("unrefuted").is_none() && b01.get("unrefutedReason").is_none(),
        "a crash is never marked unrefuted"
    );
    check_eq!(
        get(&out, "s1")?["unrefutedReason"],
        json!("non-gating"),
        "the suggestion is non-gating"
    );
    check!(
        get(&out, "s1")?.get("refuterError").is_none(),
        "a deliberate skip is not a crash"
    );
    check_eq!(
        get(&out, "b08")?["unrefutedReason"],
        json!("budget"),
        "the lowest-ranked gating finding was cut"
    );
    check!(
        get(&out, "b08")?.get("refuterError").is_none(),
        "a budget skip is not a crash"
    );
    check!(
        get(&out, "b02")?.get("unrefuted").is_none()
            && get(&out, "b02")?.get("refuterError").is_none(),
        "a graded survivor carries no marker"
    );
    Ok(())
}

#[test]
fn budget_states_distinct() {
    run_real(four_states);
}

#[test]
fn mutant_iii_budget_reason_dropped() {
    run_mutant(
        Lib::mutant(
            "budget-reason-dropped",
            &[(
                REVIEW_LIB,
                "{ ...c.finding, unrefuted: true, unrefutedReason: 'budget' }",
                "{ ...c.finding, unrefuted: true }",
            )],
        ),
        four_states,
    );
}

#[test]
fn mutant_v_refuter_error_marker_dropped() {
    run_mutant(
        Lib::mutant(
            "refuter-error-dropped",
            &[(
                REVIEW_LIB,
                ".catch(() => ({ finding: { ...c.finding, refuterError: true }, verdict: null }))",
                ".catch(() => ({ finding: c.finding, verdict: null }))",
            )],
        ),
        four_states,
    );
}

#[test]
fn budget_ranking_deterministic_and_monotone() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        // Determinism: repeated runs, and one run whose refuters resolve in a
        // fixed permuted order; a duplicate id across two dimensions.
        let mut planted = gating(13, 99);
        planted.push(json!({ "id": "dup", "severity": "blocking", "confidence": 70, "what_fails": "shared id A" }));
        let tests = json!([{ "id": "dup", "severity": "blocking", "confidence": 70, "what_fails": "shared id B" }]);
        let mut snapshots = Vec::new();
        for run in 0..5 {
            let mut agent = Agent::planted(
                plant(&[("correctness", json!(planted)), ("tests", tests.clone())]),
                json!({}),
            );
            if run == 4 {
                agent = agent.hold_then_release("refute:", 5, vec![3, 1, 4, 0, 2]);
            }
            let out = js.review_ok("code", &agent, ctx())?;
            let refuters: BTreeSet<String> = agent.refuted_ids().into_iter().collect();
            snapshots.push((
                json!({ "survivors": out["survivors"], "budget": out["budget"] }).to_string(),
                refuters,
            ));
        }
        check!(!snapshots[0].1.is_empty(), "refuters were dispatched");
        for (i, s) in snapshots.iter().enumerate().skip(1) {
            check_eq!(
                s.0,
                snapshots[0].0,
                "run {i}: survivors and budget are byte-identical"
            );
            check_eq!(s.1, snapshots[0].1, "run {i}: the refuter set is identical");
        }

        // Monotonicity over every refutation subset, N and tier: budgeted
        // survivors are a superset, and rework never becomes reviewed.
        let proof = json!([
            { "id": "p1", "severity": "blocking", "confidence": 95 },
            { "id": "p2", "severity": "blocking", "confidence": 90 },
            { "id": "p3", "severity": "concern", "confidence": 88 },
            { "id": "p4", "severity": "concern", "confidence": 80 },
            { "id": "p5", "severity": "blocking", "confidence": 75 },
            { "id": "p6", "severity": "concern", "confidence": 72 },
            { "id": "p7", "severity": "blocking", "confidence": 90 },
            { "id": "p8", "severity": "concern", "confidence": 95 }
        ]);
        let records: Vec<Value> = proof
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
            .map(|(i, f)| json!({ "order": i, "finding": f }))
            .collect();
        let ranked = js.call("rankBudgetCandidates", vec![json!(records)])?;
        let ranked: Vec<(usize, Value)> = ranked
            .as_array()
            .into_iter()
            .flatten()
            .map(|c| {
                (
                    c["order"].as_u64().unwrap_or(0) as usize,
                    c["finding"].clone(),
                )
            })
            .collect();
        // survives() for each finding under each verdict shape the proof uses.
        let mut verdict_survives = HashMap::new();
        for (order, f) in &ranked {
            let refuted = js.call(
                "survives",
                vec![f.clone(), json!({ "refuted": true, "confidence": 10 })],
            )?;
            let kept = js.call(
                "survives",
                vec![f.clone(), json!({ "refuted": false, "confidence": 90 })],
            )?;
            let ungraded = js.call("survives", vec![f.clone(), json!(null)])?;
            verdict_survives.insert(
                *order,
                (
                    refuted == json!(true),
                    kept == json!(true),
                    ungraded == json!(true),
                ),
            );
        }
        let mut outcome_cache: HashMap<String, Value> = HashMap::new();
        let (mut checked, mut saw_rework) = (0, 0);
        for mask in 0u32..(1 << 8) {
            let survives_graded = |order: usize| {
                let (r, k, _) = verdict_survives[&order];
                if mask & (1 << order) != 0 { r } else { k }
            };
            let unbudgeted: Vec<&(usize, Value)> =
                ranked.iter().filter(|(o, _)| survives_graded(*o)).collect();
            for n in [0usize, 1, 3, 5, 99] {
                let budgeted: Vec<&(usize, Value)> = ranked
                    .iter()
                    .enumerate()
                    .filter(|(i, (o, _))| {
                        if *i < n {
                            survives_graded(*o)
                        } else {
                            verdict_survives[o].2
                        }
                    })
                    .map(|(_, c)| c)
                    .collect();
                for f in &unbudgeted {
                    check!(
                        budgeted.iter().any(|b| b.0 == f.0),
                        "budgeted survivors are a superset (mask {mask}, N {n})"
                    );
                }
                for tier in [json!({ "$undefined": true }), json!("large")] {
                    let mut classify = |set: &[&(usize, Value)]| -> Result<Value, Failure> {
                        let list: Vec<Value> = set.iter().map(|(_, f)| f.clone()).collect();
                        let input =
                            json!({ "planFindings": [], "codeReviews": [list], "tier": tier });
                        let key = input.to_string();
                        if let Some(v) = outcome_cache.get(&key) {
                            return Ok(v.clone());
                        }
                        let v = js.call("classifyOutcome", vec![input])?;
                        outcome_cache.insert(key, v.clone());
                        Ok(v)
                    };
                    let u = classify(&unbudgeted)?;
                    let b = classify(&budgeted)?;
                    if u == json!("rework") {
                        saw_rework += 1;
                        check_eq!(
                            b,
                            json!("rework"),
                            "a budget can never turn rework into reviewed (mask {mask}, N {n})"
                        );
                    }
                    checked += 1;
                }
            }
        }
        check_eq!(checked, 256 * 5 * 2, "every subset x N x tier was covered");
        check!(saw_rework > 0, "rework outcomes actually occurred");
        Ok(())
    });
}

#[test]
fn budget_zero_grades_nothing() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let agent = correctness(gating(4, 99));
        let out = js.review_ok("code", &agent, ctx_with("maxRefutations", json!(0)))?;
        check_eq!(agent.refuted_ids().len(), 0, "N=0 dispatches no refuter");
        check_eq!(
            out["budget"]["max"],
            json!(0),
            "N=0 is honoured, not treated as unset"
        );
        check_eq!(out["budget"]["graded"], json!(0), "N=0 grades nothing");
        check_eq!(
            out["budget"]["passedThroughBudget"],
            json!(4),
            "every gating candidate passes through"
        );
        check!(
            out["survivors"]
                .as_array()
                .into_iter()
                .flatten()
                .all(|f| f["unrefutedReason"] == json!("budget")),
            "every survivor is marked as cut for budget"
        );
        Ok(())
    });
}

#[test]
fn suggestions_never_consume_budget() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let mut planted = gating(5, 99);
        for (id, c) in [("s1", 95), ("s2", 94), ("s3", 93)] {
            planted.push(json!({ "id": id, "severity": "suggestion", "confidence": c }));
        }
        let agent = correctness(planted);
        let out = js.review_ok("code", &agent, ctx())?;
        check_eq!(
            agent.refuted_ids().len(),
            5,
            "suggestions do not displace a gating finding"
        );
        let b = &out["budget"];
        check_eq!(
            b["hit"],
            json!(false),
            "suggestions do not push the unit over budget"
        );
        check_eq!(b["produced"], json!(8), "produced counts every candidate");
        check_eq!(
            b["gating"],
            json!(5),
            "gating counts only the budget-consuming half"
        );
        check_eq!(
            b["passedThroughNonGating"],
            json!(3),
            "the suggestions are non-gating pass-throughs"
        );
        check_eq!(
            b["passedThroughBudget"],
            json!(0),
            "no suggestion is reported as cut for budget"
        );
        for id in ["s1", "s2", "s3"] {
            check_eq!(
                get(&out, id)?["unrefutedReason"],
                json!("non-gating"),
                "{id} carries the non-gating reason"
            );
        }
        Ok(())
    });
}

#[test]
fn crashed_finder_contributes_no_candidates() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let base = correctness(gating(2, 99));
        let agent = Agent::scripted(move |call: &AgentCall| {
            if call.label == "find:code:tests" {
                return Reply::Throw("boom finder".to_owned());
            }
            base.reply_for(call)
        });
        let out = js.review_ok("code", &agent, ctx())?;
        check_eq!(
            out["budget"]["produced"],
            json!(2),
            "a crashed finder contributes no candidates"
        );
        check_eq!(
            out["budget"]["hit"],
            json!(false),
            "and manufactures no budget hit"
        );
        Ok(())
    });
}

#[test]
fn per_run_override_reaches_pipeline() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let agent = correctness(gating(6, 99));
        let out = js.review_ok("code", &agent, ctx_with("maxRefutations", json!(2)))?;
        check_eq!(
            agent.refuted_ids().len(),
            2,
            "context.maxRefutations overrides the default"
        );
        check_eq!(out["budget"]["max"], json!(2), "the override is reported");
        check_eq!(
            out["budget"]["passedThroughBudget"],
            json!(4),
            "the rest overflow"
        );
        Ok(())
    });
}

#[test]
fn invalid_override_throws_before_any_agent() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let agent = correctness(gating(2, 99));
        let r = js.review("code", &agent, ctx_with("maxRefutations", json!("5abc")))?;
        check!(
            r.as_ref().is_err_and(|e| e
                .message
                .contains("maxRefutations must be a non-negative integer")),
            "an invalid budget rejects: {r:?}"
        );
        check!(
            agent.calls().is_empty(),
            "no agent() call burns tokens first"
        );
        Ok(())
    });
}

fn floor_not_bypassed(lib: &Lib) -> Outcome {
    let mut js = Js::open(lib)?;
    let mut planted = gating(5, 99);
    planted.push(json!({ "id": "z-high", "severity": "blocking", "confidence": 90, "what_fails": "over budget, above floor" }));
    planted.push(json!({ "id": "z-low", "severity": "blocking", "confidence": 69, "what_fails": "over budget, below floor" }));
    let out = js.review_ok("code", &correctness(planted), ctx())?;
    check_eq!(
        out["budget"]["passedThroughBudget"],
        json!(2),
        "both extra findings are over budget"
    );
    let s = ids(&out, "survivors");
    check!(
        s.contains(&"z-high".to_owned()),
        "an over-budget finding at 90 survives"
    );
    check!(
        !s.contains(&"z-low".to_owned()),
        "an over-budget finding at 69 is dropped by the floor"
    );
    Ok(())
}

#[test]
fn floor_not_bypassed_over_budget() {
    run_real(floor_not_bypassed);
}

#[test]
fn mutant_iv_floor_bypassed() {
    run_mutant(
        Lib::mutant(
            "floor-bypassed",
            &[(
                REVIEW_LIB,
                "    const survivors = graded.filter((g) => survives(g.finding, g.verdict)).map((g) => g.finding);\n",
                "    const survivors = graded.filter((g) => g.skipped || survives(g.finding, g.verdict)).map((g) => g.finding); // MUTANT\n",
            )],
        ),
        floor_not_bypassed,
    );
}

#[test]
fn over_budget_blockers_still_gate() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        // Severity-first ranking keeps a late sole blocker inside the budget.
        let mut planted: Vec<Value> = (1..=6)
            .map(|i| json!({ "id": format!("c{i}"), "severity": "concern", "confidence": 100 - i }))
            .collect();
        planted.push(json!({ "id": "zz-blocker", "severity": "blocking", "confidence": 90, "what_fails": "the real defect" }));
        planted.push(json!({ "id": "c7", "severity": "concern", "confidence": 93 }));
        let agent = correctness(planted);
        let out = js.review_ok("code", &agent, ctx_with("maxRefutations", json!(5)))?;
        check!(
            agent.refuted_ids().contains(&"zz-blocker".to_owned()),
            "the sole blocker is graded"
        );
        check!(
            get(&out, "zz-blocker")?.get("unrefutedReason").is_none(),
            "it was graded, not cut"
        );
        let o = js.call(
            "classifyOutcome",
            vec![json!({ "planFindings": [], "codeReviews": [out["survivors"]] })],
        )?;
        check_eq!(o, json!("rework"), "the unit classifies rework");

        // A rank-7 blocker really over budget still forces rework.
        let mut planted = gating(6, 99);
        planted.push(json!({ "id": "zz-late", "severity": "blocking", "confidence": 90, "what_fails": "the rank-7 defect" }));
        let out = js.review_ok(
            "code",
            &correctness(planted),
            ctx_with("maxRefutations", json!(5)),
        )?;
        check_eq!(out["budget"]["hit"], json!(true), "the bound was hit");
        let late = get(&out, "zz-late")?;
        check_eq!(late["unrefuted"], json!(true), "it was never graded");
        check_eq!(
            late["unrefutedReason"],
            json!("budget"),
            "it was cut for budget"
        );
        let o = js.call(
            "classifyOutcome",
            vec![json!({ "planFindings": [], "codeReviews": [out["survivors"]] })],
        )?;
        check_eq!(
            o,
            json!("rework"),
            "an ungraded over-budget blocker still forces rework"
        );

        // The same blocker below the floor is dropped.
        let mut planted = gating(6, 99);
        planted.push(json!({ "id": "zz-late", "severity": "blocking", "confidence": 69, "what_fails": "below the floor" }));
        let agent = Agent::planted(
            plant(&[("correctness", json!(planted))]),
            plant(&[("b01", json!({ "refuted": true, "confidence": 10 }))]),
        );
        let out = js.review_ok("code", &agent, ctx_with("maxRefutations", json!(5)))?;
        check!(
            find(&out, "survivors", "zz-late").is_none(),
            "a below-floor overflow finding is dropped"
        );
        Ok(())
    });
}

#[test]
fn ac_table_never_budgeted() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let gap = json!([{ "criterion": "AC1", "status": "FAIL", "evidence": "none" }]);
        let o = js.call(
            "classifyOutcome",
            vec![json!({ "planFindings": [], "codeReviews": [[]], "acTable": gap })],
        )?;
        check_eq!(
            o,
            json!("rework"),
            "the AC-table gate does not read the budget"
        );
        let agent = Agent::scripted(move |call| {
            if call.label == "find:code:ac" {
                Reply::Value(
                    json!({ "ac": [{ "criterion": "AC1", "status": "FAIL", "evidence": "none" }], "findings": [] }),
                )
            } else {
                Reply::Value(json!({ "findings": [] }))
            }
        });
        let out = js.review_ok("code", &agent, ctx_with("maxRefutations", json!(0)))?;
        let o = js.call("classifyOutcome", vec![json!({ "planFindings": [], "codeReviews": [out["survivors"]], "acTable": out["acTable"] })])?;
        check_eq!(
            o,
            json!("rework"),
            "an AC-table gap still forces rework at N=0"
        );
        Ok(())
    });
}

#[test]
fn budget_skipped_blocker_still_gates() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let skipped = json!([{ "id": "zz", "severity": "blocking", "confidence": 90, "unrefuted": true, "unrefutedReason": "budget" }]);
        check_eq!(
            js.call("hasBlocking", vec![skipped, json!("medium")])?,
            json!(true),
            "an ungraded blocking finding still gates"
        );
        let non_gating = json!([{ "id": "zz", "severity": "suggestion", "confidence": 90, "unrefuted": true, "unrefutedReason": "non-gating" }]);
        check_eq!(
            js.call("hasBlocking", vec![non_gating, json!("medium")])?,
            json!(false),
            "a non-gating pass-through does not gate"
        );
        Ok(())
    });
}

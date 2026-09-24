//! Outcome classification, the outcome→status mapping, the AC-table channel
//! and the mode-dispatched gate policy.

use serde_json::{Value, json};

use crate::support::{Agent, Failure, Js, Outcome, Reply, run_real};

fn classify(js: &mut Js, input: Value) -> Result<Value, Failure> {
    js.call("classifyOutcome", vec![input])
}

fn blocker() -> Value {
    json!([{ "id": "x", "severity": "blocking", "confidence": 90, "what_fails": "boom" }])
}

#[test]
fn classify_outcome_and_status_mapping() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        check_eq!(
            js.get("OUTCOMES")?,
            json!(["reviewed", "rework", "escalated"]),
            "the outcome vocabulary"
        );
        let b = blocker();
        let cases = [
            (
                json!({ "planFindings": b }),
                "escalated",
                "a blocking plan finding escalates",
            ),
            (
                json!({ "planFindings": [], "codeReviews": [b, b] }),
                "rework",
                "a blocking finding on the last round is rework",
            ),
            (
                json!({ "planFindings": [], "codeReviews": [[]] }),
                "reviewed",
                "a clean review is reviewed",
            ),
            (
                json!({ "planFindings": [], "codeFindings": b, "maxRework": 0 }),
                "rework",
                "budget 0 with a blocking first pass is rework",
            ),
        ];
        for (input, want, why) in cases {
            check_eq!(classify(&mut js, input)?, json!(want), "{why}");
        }
        let statuses = [
            ("reviewed", "phase", "reviewed"),
            ("reviewed", "task", "reviewed"),
            ("rework", "phase", "in-progress"),
            ("rework", "task", "in-progress"),
            ("escalated", "phase", "blocked"),
            ("escalated", "task", "blocked"),
        ];
        for (outcome, kind, want) in statuses {
            let got = js.call("statusFor", vec![json!(outcome), json!(kind)])?;
            check_eq!(got, json!(want), "statusFor({outcome}, {kind})");
        }
        let err = js.try_call("statusFor", vec![json!("PASS"), json!("phase")])?;
        check!(
            err.as_ref()
                .is_err_and(|e| e.message.contains("unknown outcome")),
            "a retired verdict word throws: {err:?}"
        );
        let err = js.try_call("statusFor", vec![json!("reviewed"), json!("roadmap")])?;
        check!(
            err.as_ref()
                .is_err_and(|e| e.message.contains("unknown item kind")),
            "an unknown item kind throws: {err:?}"
        );
        for (outcome, want) in [("reviewed", true), ("rework", false), ("escalated", false)] {
            check_eq!(
                js.call("writesCompletion", vec![json!(outcome)])?,
                json!(want),
                "writesCompletion({outcome})"
            );
        }
        let err = js.try_call("writesCompletion", vec![json!("BLOCKED")])?;
        check!(
            err.as_ref()
                .is_err_and(|e| e.message.contains("unknown outcome")),
            "{err:?}"
        );
        Ok(())
    });
}

#[test]
fn ac_table_gap_rules() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let row = |status: &str| json!([{ "criterion": "x", "status": status, "evidence": "y" }]);
        let cases = [
            (json!(null), false, "a null table is not a gap"),
            (
                json!({ "$undefined": true }),
                false,
                "an undefined table is not a gap",
            ),
            (json!([]), false, "an empty table is not a gap"),
            (row("PASS"), false, "an all-PASS table is not a gap"),
            (row("FAIL"), true, "a FAIL row is a gap"),
            (row("PARTIAL"), true, "a PARTIAL row is a gap"),
        ];
        for (table, want, why) in cases {
            check_eq!(js.call("acTableHasGap", vec![table])?, json!(want), "{why}");
        }
        Ok(())
    });
}

#[test]
fn ac_table_channel_forces_rework() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let table = |status: &str| json!([{ "criterion": "x", "status": status, "evidence": "e" }]);
        let cases = [
            (
                json!({ "tier": "medium", "codeReviews": [[]], "acTable": table("FAIL") }),
                "rework",
                "FAIL forces rework with zero findings",
            ),
            (
                json!({ "tier": "medium", "codeReviews": [[]], "acTable": table("PARTIAL") }),
                "rework",
                "PARTIAL forces rework",
            ),
            (
                json!({ "tier": "medium", "codeReviews": [[]], "acTable": table("PASS") }),
                "reviewed",
                "all-PASS does not force rework",
            ),
            (
                json!({ "tier": "medium", "codeReviews": [[]], "acTable": null }),
                "reviewed",
                "a null table does not force rework",
            ),
            (
                json!({ "tier": "medium", "planFindings": blocker(), "acTable": table("FAIL") }),
                "escalated",
                "a blocking plan finding wins over the AC gate",
            ),
        ];
        for (input, want, why) in cases {
            check_eq!(classify(&mut js, input)?, json!(want), "{why}");
        }
        Ok(())
    });
}

#[test]
fn gate_policy_table() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let policy = js.get("GATE_POLICY")?;
        let keys: Vec<&String> = policy
            .as_object()
            .map(|o| o.keys().collect())
            .unwrap_or_default();
        check_eq!(keys, vec!["code", "plan"], "exactly two gate modes");
        check_eq!(
            policy["code"],
            js.get("STATUS_MAPPING")?,
            "the code gate is the status mapping, not a fork"
        );
        let gate = |js: &mut Js, mode: &str, outcome: &str| {
            js.call("gateFor", vec![json!(mode), json!(outcome)])
        };
        let reviewed = gate(&mut js, "code", "reviewed")?;
        check_eq!(
            reviewed["status"],
            json!("reviewed"),
            "code reviewed status"
        );
        check_eq!(
            reviewed["writesCompletion"],
            json!(true),
            "code reviewed writes completion"
        );
        for outcome in ["reviewed", "rework", "escalated"] {
            let row = gate(&mut js, "code", outcome)?;
            check_eq!(
                row["clearsPlanReviewTag"],
                json!(false),
                "the code gate never clears the plan tag ({outcome})"
            );
        }
        check_eq!(
            gate(&mut js, "code", "escalated")?["reasonPrefix"],
            json!("[code]"),
            "code escalation prefix"
        );
        check_eq!(
            gate(&mut js, "plan", "reviewed")?["clearsPlanReviewTag"],
            json!(true),
            "plan reviewed clears the tag"
        );
        check_eq!(
            gate(&mut js, "plan", "rework")?["clearsPlanReviewTag"],
            json!(false),
            "plan rework leaves the tag"
        );
        check_eq!(
            gate(&mut js, "plan", "escalated")?["clearsPlanReviewTag"],
            json!(false),
            "plan escalated leaves the tag"
        );
        check_eq!(
            gate(&mut js, "plan", "escalated")?["reasonPrefix"],
            json!("[plan]"),
            "plan escalation prefix"
        );
        for outcome in ["reviewed", "rework", "escalated"] {
            let row = gate(&mut js, "plan", outcome)?;
            check_eq!(
                row.get("status"),
                Some(&json!(null)),
                "a plan row declares a literal null status ({outcome})"
            );
            check_eq!(
                row["writesCompletion"],
                json!(false),
                "a plan review never writes completion ({outcome})"
            );
        }
        for (mode, outcome, needle) in [
            ("bogus", "reviewed", "unknown gate mode"),
            ("plan", "bogus", "unknown outcome"),
            ("code", "PASS", "unknown outcome"),
        ] {
            let r = js.try_call("gateFor", vec![json!(mode), json!(outcome)])?;
            check!(
                r.as_ref().is_err_and(|e| e.message.contains(needle)),
                "gateFor({mode}, {outcome}): {r:?}"
            );
        }
        Ok(())
    });
}

fn deferred_ac_agent(findings: Value) -> Agent {
    let table = json!([{ "criterion": "Feature X ships fully configurable", "status": "PASS", "evidence": "src/x.rs:12, test_x" }]);
    Agent::scripted(move |call| {
        if call.label == "find:code:ac" {
            Reply::Value(json!({ "ac": table, "findings": findings }))
        } else if call.label.starts_with("find:") {
            Reply::Value(json!({ "findings": [] }))
        } else if call.label.starts_with("refute:") {
            Reply::Value(json!({ "refuted": false, "confidence": 92 }))
        } else {
            Reply::Throw(format!("unexpected agent label: {}", call.label))
        }
    })
}

fn deferred_ac_outcome(js: &mut Js, findings: Value) -> Result<(Value, Value), Failure> {
    let out = js.review_ok(
        "code",
        &deferred_ac_agent(findings),
        json!({ "target": "task widget/deferred-ac" }),
    )?;
    let gap = js.call("acTableHasGap", vec![out["acTable"].clone()])?;
    check_eq!(gap, json!(false), "the planted AC table is all-PASS");
    let outcome = classify(
        js,
        json!({ "acTable": out["acTable"], "codeReviews": [out["survivors"]] }),
    )?;
    Ok((out, outcome))
}

#[test]
fn blocking_ac_finding_forces_rework_with_all_pass_table() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let (out, outcome) = deferred_ac_outcome(
            &mut js,
            json!([{ "id": "deferred-ac", "concern": "ac", "severity": "blocking", "confidence": 90,
                     "what_fails": "Acceptance criterion \"Feature X ships fully configurable\" is deferred as a follow-up." }]),
        )?;
        check!(
            crate::support::ids(&out, "survivors").contains(&"deferred-ac".to_owned()),
            "the blocking ac finding survives refutation"
        );
        check_eq!(
            outcome,
            json!("rework"),
            "a surviving blocking ac finding forces rework through an all-PASS table"
        );

        // Negative control: the same table and no findings-array entry.
        let (out, outcome) = deferred_ac_outcome(&mut js, json!([]))?;
        check_eq!(out["survivors"], json!([]), "the control has no survivors");
        check_eq!(
            outcome,
            json!("reviewed"),
            "the control classifies reviewed"
        );
        Ok(())
    });
}

#[test]
fn classify_outcome_completeness_cases() {
    run_real(|lib| -> Outcome {
        let mut js = Js::open(lib)?;
        let complete = json!({ "complete": true, "selected": ["ac", "correctness"], "ran": ["ac", "correctness"], "failed": [], "acDimensionRan": true });
        let with = |base: &Value, patch: Value| {
            let mut v = base.clone();
            if let (Some(o), Some(p)) = (v.as_object_mut(), patch.as_object()) {
                for (k, x) in p {
                    o.insert(k.clone(), x.clone());
                }
            }
            v
        };
        let ac = json!([{ "criterion": "AC1: works", "status": "PASS", "evidence": "test" }]);
        let criteria = json!(["AC1: works"]);
        let escalated = [
            json!({ "planFindings": [], "codeReviews": [[]], "evidence": {
                "coverage": with(&complete, json!({ "complete": false, "failed": ["ac"], "acDimensionRan": false })), "acTable": null } }),
            json!({ "codeReviews": [[]], "evidence": { "criteria": criteria, "coverage": complete, "acTable": null } }),
            json!({ "codeReviews": [[]], "evidence": { "criteria": criteria, "coverage": complete, "acTable": [] } }),
            json!({ "codeReviews": [[]], "evidence": { "criteria": criteria, "coverage": complete, "acTable": [{ "status": "INVALID" }] } }),
            json!({ "codeReviews": [[]], "evidence": { "coverage": with(&complete, json!({ "complete": false, "failed": ["correctness"] })), "acTable": ac } }),
            json!({ "codeReviews": [[]], "evidence": { "criteria": criteria, "coverage": complete, "acTable": ac, "budget": { "passedThroughBudget": 1 } } }),
            json!({ "codeReviews": [[]], "evidence": { "criteria": criteria, "coverage": complete, "acTable": ac,
                    "survivors": [{ "severity": "concern", "refuterError": true }] } }),
        ];
        for (i, input) in escalated.into_iter().enumerate() {
            check_eq!(
                classify(&mut js, input)?,
                json!("escalated"),
                "incomplete evidence case {i} escalates"
            );
        }
        let reviewed = [
            json!({ "codeReviews": [[]], "evidence": { "criteria": criteria, "coverage": complete, "acTable": ac,
                    "survivors": [{ "severity": "suggestion", "unrefuted": true, "unrefutedReason": "non-gating" }] } }),
            json!({ "codeReviews": [[]], "evidence": { "criteria": criteria,
                    "coverage": { "complete": false, "last": complete }, "acTable": ac } }),
        ];
        for (i, input) in reviewed.into_iter().enumerate() {
            check_eq!(
                classify(&mut js, input)?,
                json!("reviewed"),
                "complete evidence case {i} is reviewed"
            );
        }
        Ok(())
    });
}

//! Plan-review consolidation helpers, per-unit gating and round notes.

use serde_json::{Value, json};

use crate::support::{Failure, Js, Lib, Outcome, PLAN_LIB, run_mutant, run_real};

fn coherence_blocker() -> Value {
    json!({ "id": "c", "concern": "coherence", "severity": "blocking", "confidence": 90, "what_fails": "ambiguous step" })
}

fn nit() -> Value {
    json!({ "id": "n", "concern": "coherence", "severity": "concern", "confidence": 80, "what_fails": "minor" })
}

#[test]
fn filter_plan_review_tag_preserves_siblings() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let cases = [
            (
                json!(["needs-plan-review", "depends-unlanded"]),
                json!(["depends-unlanded"]),
                "a sibling is preserved",
            ),
            (
                json!(["depends-unlanded", "needs-plan-review"]),
                json!(["depends-unlanded"]),
                "order is preserved",
            ),
            (
                json!(["needs-plan-review"]),
                json!([]),
                "the only tag leaves an empty list",
            ),
            (
                json!(["a", "b"]),
                json!(["a", "b"]),
                "a no-op when the tag is absent",
            ),
            (json!([]), json!([]), "an empty list stays empty"),
        ];
        for (tags, want, why) in cases {
            check_eq!(js.call("filterPlanReviewTag", vec![tags])?, want, "{why}");
        }
        let once = js.call(
            "filterPlanReviewTag",
            vec![json!(["needs-plan-review", "x"])],
        )?;
        check_eq!(
            js.call("filterPlanReviewTag", vec![once])?,
            json!(["x"]),
            "idempotent under re-application"
        );
        Ok(())
    });
}

#[test]
fn classify_plan_outcome() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let arch = json!({ "id": "a", "concern": "architectural-fit", "severity": "blocking", "confidence": 92, "what_fails": "violates constraint" });
        let empty = json!({ "id": "empty", "concern": "coherence", "severity": "blocking", "confidence": 95, "what_fails": "plan is empty" });
        let cases = [
            (json!([]), "reviewed", "no findings"),
            (json!([nit()]), "reviewed", "concern-only"),
            (
                json!([coherence_blocker()]),
                "rework",
                "a blocking coherence finding is a fixable rewrite",
            ),
            (
                json!([arch]),
                "escalated",
                "a blocking architectural-fit finding needs a human decision",
            ),
            (
                json!([empty]),
                "rework",
                "an empty/ambiguous plan is rework",
            ),
        ];
        for (survivors, want, why) in cases {
            check_eq!(
                js.call("classifyPlanOutcome", vec![survivors])?,
                json!(want),
                "{why}"
            );
        }
        Ok(())
    });
}

#[test]
fn per_unit_independent_gates() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let units = [
            ("roadmap-body", json!(["needs-plan-review"]), json!([])),
            (
                "phase-A",
                json!(["needs-plan-review", "depends-unlanded"]),
                json!([coherence_blocker()]),
            ),
            ("phase-B", json!(["needs-plan-review"]), json!([])),
            ("phase-C", json!(["needs-plan-review"]), json!([nit()])),
        ];
        for (id, tags, survivors) in units {
            let outcome = js.call("classifyPlanOutcome", vec![survivors])?;
            let gate = js.call("gateFor", vec![json!("plan"), outcome.clone()])?;
            check_eq!(
                gate.get("status"),
                Some(&json!(null)),
                "{id}: the plan gate persists no status"
            );
            let clears = gate["clearsPlanReviewTag"] == json!(true);
            let remaining = if clears {
                js.call("filterPlanReviewTag", vec![tags.clone()])?
            } else {
                tags.clone()
            };
            if id == "phase-A" {
                check_eq!(outcome, json!("rework"), "phase-A reworks");
                check!(!clears, "phase-A keeps needs-plan-review");
                check_eq!(remaining, tags, "phase-A's tags are untouched");
            } else {
                check_eq!(outcome, json!("reviewed"), "{id} is reviewed");
                check!(clears, "{id} clears needs-plan-review");
                check_eq!(remaining, json!([]), "{id}'s tag list is emptied");
            }
        }
        Ok(())
    });
}

#[test]
fn implementation_plan_skips_gate() {
    run_real(|lib| {
        let mut js = Js::open(lib)?;
        let outcome = js.call("classifyPlanOutcome", vec![json!([coherence_blocker()])])?;
        let gate = js.call("gateFor", vec![json!("plan"), outcome])?;
        check_eq!(
            gate.get("status"),
            Some(&json!(null)),
            "no status is persisted"
        );
        check_eq!(
            gate["writesCompletion"],
            json!(false),
            "no completion directive is written"
        );
        Ok(())
    });
}

fn round_note_round_trip(lib: &Lib) -> Outcome {
    let mut js = Js::open(lib)?;
    let findings = json!([
        { "severity": "blocking", "concern": "blocker_concern", "what_fails": "blocker_failure" },
        { "severity": "concern", "concern": "concern_concern", "what_fails": "concern_failure" },
        { "severity": "suggestion", "concern": "suggestion_concern", "what_fails": "suggestion_failure" }
    ]);
    let note = js.plan_call("formatRoundNote", vec![json!(1), json!("rework"), findings])?;
    let parsed = js.plan_call("parseRoundNotes", vec![note])?;
    check_eq!(parsed["round"], json!(1), "the round number round-trips");
    check_eq!(
        parsed["outcome"],
        json!("rework"),
        "the outcome round-trips"
    );
    let got: Vec<(Value, Value)> = parsed["findings"]
        .as_array()
        .ok_or_else(|| Failure::Check(format!("no findings parsed: {parsed}")))?
        .iter()
        .map(|f| (f["severity"].clone(), f["concern"].clone()))
        .collect();
    check_eq!(
        got,
        vec![
            (json!("blocking"), json!("blocker_concern")),
            (json!("concern"), json!("concern_concern")),
            (json!("suggestion"), json!("suggestion_concern")),
        ],
        "every severity and concern round-trips"
    );
    Ok(())
}

#[test]
fn round_note_severity_round_trip() {
    run_real(round_note_round_trip);
}

#[test]
fn mutant_round_note_regex_narrowed() {
    run_mutant(
        Lib::mutant(
            "round-note-regex-narrowed",
            &[(
                PLAN_LIB,
                "(blocking|concern|suggestion)",
                "(blocking|concern)",
            )],
        ),
        round_note_round_trip,
    );
}

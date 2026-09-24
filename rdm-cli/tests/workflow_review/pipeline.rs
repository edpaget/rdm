//! The find → refute → filter pipeline, in both review modes.

use serde_json::json;

use crate::support::{Agent, Js, Lib, Outcome, find, ids, plant, run_mutant, run_real};

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

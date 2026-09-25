//! The semantic consolidator: one agent between the finder barrier and the
//! budget cut that clusters duplicate findings, so a duplicate costs one
//! refuter and one budget slot instead of N.
//!
//! Every scenario runs the real `buildReviewPipeline` in BOTH modes through
//! [`Js::review`], with a scripted agent whose consolidator reply is supplied
//! here. Pipeline ids are `c<order>`, `order` being the flattened candidate
//! index (dimension selection order, then within-dimension order).

use std::collections::BTreeSet;

use serde_json::{Value, json};

use crate::support::{
    Agent, AgentCall, Failure, Js, Label, Lib, Outcome, Reply, find, ids, parse_label, run_real,
};

const MODES: [&str; 2] = ["code", "plan"];

/// Three reviewers per mode, in each mode's declaration order, so the
/// flattened `order` is fixed by this list.
fn reviewers(mode: &str) -> [&'static str; 3] {
    if mode == "code" {
        ["correctness", "tests", "architecture"]
    } else {
        ["coherence", "architectural-fit", "restraint"]
    }
}

fn ctx(mode: &str) -> Value {
    json!({ "target": "phase widget/phase-1-foo", "reviewers": reviewers(mode) })
}

fn ctx_with(mode: &str, extra: Value) -> Value {
    let mut c = ctx(mode);
    for (k, v) in extra.as_object().into_iter().flatten() {
        c[k] = v.clone();
    }
    c
}

/// What the consolidator answers. `NullThen`/`ThrowThen` fail the first
/// `consolidate:<mode>` call and answer the `:retry` call with the value.
#[derive(Clone)]
enum Consolidator {
    Reply(Value),
    Null,
    Throw,
    NullThen(Value),
    ThrowThen(Value),
}

/// A scripted fleet: finder `d` (the `d`-th reviewer) returns `findings[d]`;
/// a refuter returns `{ refuted: <id in refuted>, confidence: 90 }`, or
/// throws for an id in `crash`; the consolidator answers `consolidator`.
fn fleet(
    mode: &'static str,
    findings: [Vec<Value>; 3],
    consolidator: Consolidator,
    refuted: &[&str],
    crash: &[&str],
) -> Agent {
    let refuted: BTreeSet<String> = refuted.iter().map(|s| (*s).to_owned()).collect();
    let crash: BTreeSet<String> = crash.iter().map(|s| (*s).to_owned()).collect();
    let dims = reviewers(mode);
    Agent::scripted(move |call: &AgentCall| match parse_label(&call.label) {
        Label::Find { dim, .. } => {
            let i = dims.iter().position(|d| *d == dim);
            Reply::Value(json!({ "findings": i.map_or_else(Vec::new, |i| findings[i].clone()) }))
        }
        Label::Refute { id } if crash.contains(&id) => Reply::Throw("refuter crashed".into()),
        Label::Refute { id } => {
            Reply::Value(json!({ "refuted": refuted.contains(&id), "confidence": 90 }))
        }
        Label::Consolidate { retry } => match &consolidator {
            Consolidator::Reply(v) => Reply::Value(v.clone()),
            Consolidator::NullThen(v) | Consolidator::ThrowThen(v) if retry => {
                Reply::Value(v.clone())
            }
            Consolidator::Null | Consolidator::NullThen(_) => Reply::Null,
            Consolidator::Throw | Consolidator::ThrowThen(_) => {
                Reply::Throw("consolidator crashed".into())
            }
        },
        Label::Other => Reply::Throw(format!("unexpected label: {}", call.label)),
    })
}

fn f(id: &str, severity: &str, confidence: i64, extra: Value) -> Value {
    let mut v = json!({ "id": id, "severity": severity, "confidence": confidence,
                        "what_fails": format!("{id} fails") });
    for (k, x) in extra.as_object().into_iter().flatten() {
        v[k] = x.clone();
    }
    v
}

/// The duplicate corpus. Orders: c0 `x-a`, c1 `solo-1` (dim 0); c2 `x-b`
/// (dim 1); c3 `x-c`, c4 `solo-2` (dim 2). `x-a`/`x-b`/`x-c` are one defect
/// seen by three dimensions, with differing severity and confidence; `x-c`
/// is the highest-confidence member but the least severe.
fn corpus() -> [Vec<Value>; 3] {
    [
        vec![
            f(
                "x-a",
                "concern",
                80,
                json!({ "quote": "qa", "path": "a.rs", "why": "why-a", "recommendation": "rec-a" }),
            ),
            f("solo-1", "blocking", 90, json!({})),
        ],
        vec![f(
            "x-b",
            "blocking",
            85,
            json!({ "quote": "qb", "path": "b.rs", "why": "why-b", "recommendation": "rec-b" }),
        )],
        vec![
            f(
                "x-c",
                "suggestion",
                92,
                json!({ "quote": "qc", "path": "c.rs", "why": "why-c", "recommendation": "rec-c" }),
            ),
            f("solo-2", "concern", 88, json!({})),
        ],
    ]
}

const IDS: [&str; 5] = ["c0", "c1", "c2", "c3", "c4"];
const WHY: &str = "same missing null check";

fn merge_x() -> Value {
    json!({ "clusters": [
        { "ids": ["c0", "c2", "c3"], "why": WHY },
        { "ids": ["c1"], "why": "distinct" },
        { "ids": ["c4"], "why": "distinct" }
    ]})
}

fn singletons() -> Value {
    json!({ "clusters": IDS.iter().map(|id| json!({ "ids": [id], "why": "distinct" })).collect::<Vec<_>>() })
}

fn run(js: &mut Js, mode: &'static str, agent: &Agent, extra: Value) -> Result<Value, Failure> {
    js.review_ok(mode, agent, ctx_with(mode, extra))
}

/// Survivors deep-equal to the baseline, by id.
fn survivor(out: &Value, id: &str) -> Result<Value, Failure> {
    find(out, "survivors", id)
        .cloned()
        .ok_or_else(|| Failure::Check(format!("no survivor {id}: {}", out["survivors"])))
}

// --- AC: fail open ---------------------------------------------------------------

fn fail_open(lib: &Lib, mode: &'static str) -> Outcome {
    let mut js = Js::open(lib)?;
    let base_agent = fleet(mode, corpus(), Consolidator::Reply(singletons()), &[], &[]);
    let baseline = run(&mut js, mode, &base_agent, json!({}))?;
    check_eq!(
        baseline["budget"]["clustering"],
        json!({ "ran": true, "retried": false, "failedOpen": false, "reason": null }),
        "{mode}: an all-singletons partition is a valid, non-fail-open run"
    );
    let cases: Vec<(&str, Consolidator, &str, usize)> = vec![
        (
            "omitted id",
            Consolidator::Reply(json!({ "clusters": [
                { "ids": ["c0", "c2", "c3"], "why": WHY }, { "ids": ["c1"], "why": "d" }
            ]})),
            "missing-id",
            1,
        ),
        (
            "invented id",
            Consolidator::Reply(json!({ "clusters": [
                { "ids": ["c0", "c2", "c3", "c99"], "why": WHY },
                { "ids": ["c1"], "why": "d" }, { "ids": ["c4"], "why": "d" }
            ]})),
            "unknown-id",
            1,
        ),
        (
            "id in two clusters",
            Consolidator::Reply(json!({ "clusters": [
                { "ids": ["c0", "c2", "c3"], "why": WHY },
                { "ids": ["c1", "c0"], "why": "d" }, { "ids": ["c4"], "why": "d" }
            ]})),
            "duplicate-id",
            1,
        ),
        (
            "malformed clusters",
            Consolidator::Reply(json!({ "clusters": "c0 c2 c3" })),
            "invalid-shape",
            1,
        ),
        ("null consolidator", Consolidator::Null, "null", 2),
        ("throwing consolidator", Consolidator::Throw, "threw", 2),
    ];
    for (why, reply, reason, calls) in cases {
        let agent = fleet(mode, corpus(), reply, &[], &[]);
        let out = run(&mut js, mode, &agent, json!({}))?;
        check_eq!(
            out["survivors"],
            baseline["survivors"],
            "{mode} {why}: falls back to singletons"
        );
        let b = &out["budget"];
        check_eq!(
            b["clustering"]["failedOpen"],
            json!(true),
            "{mode} {why}: recorded as failed open"
        );
        check_eq!(
            b["clustering"]["reason"],
            json!(reason),
            "{mode} {why}: reason"
        );
        check_eq!(
            (b["consolidated"].clone(), b["collapsed"].clone()),
            (b["produced"].clone(), json!(0)),
            "{mode} {why}: nothing collapsed"
        );
        let labels: Vec<String> = agent
            .calls_with("consolidate:")
            .into_iter()
            .map(|c| c.label)
            .collect();
        let mut want = vec![format!("consolidate:{mode}")];
        if calls == 2 {
            want.push(format!("consolidate:{mode}:retry"));
        }
        check_eq!(
            labels,
            want,
            "{mode} {why}: null/throw retried exactly once, a bad partition never"
        );
        check_eq!(
            b["clustering"]["retried"],
            json!(calls == 2),
            "{mode} {why}: retried flag"
        );
    }
    Ok(())
}

#[test]
fn invalid_or_missing_partition_fails_open_to_singletons() {
    for mode in MODES {
        run_real(|lib| fail_open(lib, mode));
    }
}

// --- AC: a retry that answers ------------------------------------------------------

/// The consolidate labels an agent recorded, in call order.
fn consolidate_labels(agent: &Agent) -> Vec<String> {
    agent
        .calls_with("consolidate:")
        .into_iter()
        .map(|c| c.label)
        .collect()
}

fn retry_recovers(lib: &Lib, mode: &'static str) -> Outcome {
    let mut js = Js::open(lib)?;
    let want_labels = vec![
        format!("consolidate:{mode}"),
        format!("consolidate:{mode}:retry"),
    ];
    let first_try = fleet(mode, corpus(), Consolidator::Reply(merge_x()), &[], &[]);
    let merged = run(&mut js, mode, &first_try, json!({}))?;
    let singles = fleet(mode, corpus(), Consolidator::Reply(singletons()), &[], &[]);
    let unmerged = run(&mut js, mode, &singles, json!({}))?;

    for (why, first) in [
        ("null then valid", Consolidator::NullThen(merge_x())),
        ("throw then valid", Consolidator::ThrowThen(merge_x())),
    ] {
        let agent = fleet(mode, corpus(), first, &[], &[]);
        let out = run(&mut js, mode, &agent, json!({}))?;
        check_eq!(
            consolidate_labels(&agent),
            want_labels,
            "{mode} {why}: the first call and exactly one retry"
        );
        let b = &out["budget"];
        check_eq!(
            b["clustering"],
            json!({ "ran": true, "retried": true, "failedOpen": false, "reason": null }),
            "{mode} {why}: the retry's partition is used"
        );
        check_eq!(
            (b["consolidated"].clone(), b["collapsed"].clone()),
            (json!(3), json!(2)),
            "{mode} {why}: the three duplicates collapse"
        );
        check_eq!(
            survivor(&out, "x-c")?,
            survivor(&merged, "x-c")?,
            "{mode} {why}: the merged unit equals a first-try merge's"
        );
        check_eq!(
            survivor(&out, "x-c")?["mergedFrom"],
            json!(["c0", "c2", "c3"]),
            "{mode} {why}: the merged unit's members"
        );
    }

    let missing = json!({ "clusters": [
        { "ids": ["c0", "c2", "c3"], "why": WHY }, { "ids": ["c1"], "why": "d" }
    ]});
    for (why, first) in [
        ("null then invalid", Consolidator::NullThen(missing.clone())),
        (
            "throw then invalid",
            Consolidator::ThrowThen(missing.clone()),
        ),
    ] {
        let agent = fleet(mode, corpus(), first, &[], &[]);
        let out = run(&mut js, mode, &agent, json!({}))?;
        check_eq!(
            consolidate_labels(&agent),
            want_labels,
            "{mode} {why}: the first call and exactly one retry"
        );
        check_eq!(
            out["budget"]["clustering"],
            json!({ "ran": true, "retried": true, "failedOpen": true, "reason": "missing-id" }),
            "{mode} {why}: the retry's bad partition fails open with its reason"
        );
        check_eq!(
            out["survivors"],
            unmerged["survivors"],
            "{mode} {why}: falls back to singletons"
        );
    }
    Ok(())
}

#[test]
fn consolidator_retry_answer_is_used_or_fails_open() {
    for mode in MODES {
        run_real(|lib| retry_recovers(lib, mode));
    }
}

// --- AC: a valid assignment collapses ---------------------------------------------

fn collapses(lib: &Lib, mode: &'static str) -> Outcome {
    let mut js = Js::open(lib)?;
    let dims = reviewers(mode);
    let single = fleet(mode, corpus(), Consolidator::Reply(singletons()), &[], &[]);
    let baseline = run(&mut js, mode, &single, json!({}))?;
    let agent = fleet(mode, corpus(), Consolidator::Reply(merge_x()), &[], &[]);
    let out = run(&mut js, mode, &agent, json!({}))?;

    check_eq!(
        ids(&out, "survivors").into_iter().collect::<BTreeSet<_>>(),
        ["solo-1", "solo-2", "x-c"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect::<BTreeSet<_>>(),
        "{mode}: the three duplicates collapse into one unit"
    );
    let unit = survivor(&out, "x-c")?;
    check_eq!(
        unit["severity"],
        json!("blocking"),
        "{mode}: the most severe member's severity"
    );
    check_eq!(
        unit["confidence"],
        json!(92),
        "{mode}: exactly the max confidence — three agreeing dimensions add nothing"
    );
    check_eq!(
        unit["concerns"],
        json!([dims[0], dims[1], dims[2]]),
        "{mode}: the ordered union of member dimensions"
    );
    check_eq!(
        unit["concern"],
        json!(dims[2]),
        "{mode}: the representative's dimension"
    );
    check_eq!(
        unit["mergedFrom"],
        json!(["c0", "c2", "c3"]),
        "{mode}: the member pipeline ids"
    );
    check_eq!(unit["clusterWhy"], json!(WHY), "{mode}: the reply's why");
    for (k, v) in [
        ("what_fails", "x-c fails"),
        ("why", "why-c"),
        ("recommendation", "rec-c"),
        ("quote", "qc"),
        ("path", "c.rs"),
    ] {
        check_eq!(
            unit[k],
            json!(v),
            "{mode}: {k} is the highest-confidence member's"
        );
    }
    for id in ["solo-1", "solo-2"] {
        let s = survivor(&out, id)?;
        check_eq!(
            s,
            survivor(&baseline, id)?,
            "{mode}: singleton {id} equals the unconsolidated run's"
        );
        for k in ["concerns", "mergedFrom", "clusterWhy"] {
            check!(s.get(k).is_none(), "{mode}: singleton {id} carries {k}");
        }
    }
    let b = &out["budget"];
    check_eq!(
        (
            b["produced"].clone(),
            b["consolidated"].clone(),
            b["collapsed"].clone()
        ),
        (json!(5), json!(3), json!(2)),
        "{mode}: produced stays raw; consolidated/collapsed count units"
    );
    check_eq!(
        b["clustering"],
        json!({ "ran": true, "retried": false, "failedOpen": false, "reason": null }),
        "{mode}: a valid partition"
    );
    check_eq!(
        agent.refuted_ids().into_iter().collect::<BTreeSet<_>>(),
        ["solo-1", "solo-2", "x-c"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect::<BTreeSet<_>>(),
        "{mode}: the cluster costs one refuter"
    );
    Ok(())
}

#[test]
fn valid_assignment_collapses_duplicates() {
    for mode in MODES {
        run_real(|lib| collapses(lib, mode));
    }
}

fn tie_goes_to_earlier_order(lib: &Lib, mode: &'static str) -> Outcome {
    let mut js = Js::open(lib)?;
    let findings = [
        vec![f("t-a", "blocking", 90, json!({ "quote": "first" }))],
        vec![f("t-b", "blocking", 90, json!({ "quote": "second" }))],
        vec![],
    ];
    let agent = fleet(
        mode,
        findings,
        Consolidator::Reply(json!({ "clusters": [{ "ids": ["c1", "c0"], "why": "same" }] })),
        &[],
        &[],
    );
    let out = run(&mut js, mode, &agent, json!({}))?;
    check_eq!(
        ids(&out, "survivors"),
        vec!["t-a"],
        "{mode}: a confidence tie picks the earlier order"
    );
    check_eq!(
        survivor(&out, "t-a")?["quote"],
        json!("first"),
        "{mode}: the earlier member's narrative"
    );
    check_eq!(
        survivor(&out, "t-a")?["mergedFrom"],
        json!(["c0", "c1"]),
        "{mode}: mergedFrom is in flattened order"
    );
    Ok(())
}

#[test]
fn confidence_tie_picks_the_earlier_member() {
    for mode in MODES {
        run_real(|lib| tie_goes_to_earlier_order(lib, mode));
    }
}

// --- AC: a refuted cluster / a crashed cluster refuter -----------------------------

fn refuted_and_crashed(lib: &Lib, mode: &'static str) -> Outcome {
    let mut js = Js::open(lib)?;
    let agent = fleet(
        mode,
        corpus(),
        Consolidator::Reply(merge_x()),
        &["x-c"],
        &[],
    );
    let out = run(&mut js, mode, &agent, json!({}))?;
    check_eq!(
        ids(&out, "survivors")
            .into_iter()
            .filter(|id| id.starts_with("x-"))
            .count(),
        0,
        "{mode}: one refuted verdict drops every member of the cluster"
    );

    let agent = fleet(
        mode,
        corpus(),
        Consolidator::Reply(merge_x()),
        &[],
        &["x-c"],
    );
    let out = run(&mut js, mode, &agent, json!({}))?;
    let unit = survivor(&out, "x-c")?;
    check_eq!(
        (unit["refuterError"].clone(), unit["mergedFrom"].clone()),
        (json!(true), json!(["c0", "c2", "c3"])),
        "{mode}: a crashed cluster refuter keeps the whole merged unit"
    );
    check_eq!(
        out["budget"]["refuterErrors"],
        json!(1),
        "{mode}: one refuter error"
    );
    Ok(())
}

#[test]
fn refuted_cluster_drops_all_and_crashed_cluster_is_kept() {
    for mode in MODES {
        run_real(|lib| refuted_and_crashed(lib, mode));
    }
}

// --- AC: coverage ---------------------------------------------------------------

/// Defect A on three dimensions and B on two, all highest-ranked, plus four
/// distinct lower-ranked defects: nine gating candidates against a budget of
/// five. Orders: c0 a-1, c1 b-1, c2 d-1, c3 d-2 | c4 a-2, c5 b-2, c6 d-3 |
/// c7 a-3, c8 d-4.
fn coverage_corpus() -> [Vec<Value>; 3] {
    let x = |id: &str, c| f(id, "blocking", c, json!({}));
    [
        vec![x("a-1", 99), x("b-1", 98), x("d-1", 80), x("d-2", 80)],
        vec![x("a-2", 99), x("b-2", 98), x("d-3", 80)],
        vec![x("a-3", 99), x("d-4", 80)],
    ]
}

fn defect(id: &str) -> String {
    match id.chars().next() {
        Some('a') => "A".into(),
        Some('b') => "B".into(),
        _ => id.to_owned(),
    }
}

fn graded_defects(agent: &Agent) -> BTreeSet<String> {
    agent.refuted_ids().iter().map(|id| defect(id)).collect()
}

fn coverage(lib: &Lib, mode: &'static str) -> Outcome {
    let mut js = Js::open(lib)?;
    let partition = json!({ "clusters": [
        { "ids": ["c0", "c4", "c7"], "why": "A" }, { "ids": ["c1", "c5"], "why": "B" },
        { "ids": ["c2"], "why": "d" }, { "ids": ["c3"], "why": "d" },
        { "ids": ["c6"], "why": "d" }, { "ids": ["c8"], "why": "d" }
    ]});
    let open = fleet(mode, coverage_corpus(), Consolidator::Null, &[], &[]);
    let open_out = run(&mut js, mode, &open, json!({}))?;
    let merged = fleet(
        mode,
        coverage_corpus(),
        Consolidator::Reply(partition),
        &[],
        &[],
    );
    let merged_out = run(&mut js, mode, &merged, json!({}))?;
    check_eq!(
        (
            open_out["budget"]["hit"].clone(),
            merged_out["budget"]["hit"].clone()
        ),
        (json!(true), json!(true)),
        "{mode}: both runs exceed the budget"
    );
    let (before, after) = (graded_defects(&open), graded_defects(&merged));
    check!(
        after.len() > before.len(),
        "{mode}: consolidation grades strictly more distinct defects ({before:?} → {after:?})"
    );
    Ok(())
}

#[test]
fn consolidation_grades_more_distinct_defects_under_the_budget() {
    for mode in MODES {
        run_real(|lib| coverage(lib, mode));
    }
}

// --- AC: skip below two candidates ------------------------------------------------

fn skip(lib: &Lib, mode: &'static str) -> Outcome {
    let mut js = Js::open(lib)?;
    let one = f("only", "blocking", 90, json!({}));
    for (why, findings) in [
        ("0 candidates", [vec![], vec![], vec![]]),
        ("1 candidate", [vec![one], vec![], vec![]]),
    ] {
        let agent = fleet(mode, findings, Consolidator::Throw, &[], &[]);
        let out = run(&mut js, mode, &agent, json!({}))?;
        check!(
            agent.calls_with("consolidate:").is_empty(),
            "{mode} {why}: no consolidator call: {:?}",
            agent.labels()
        );
        check_eq!(
            out["budget"]["clustering"],
            json!({ "ran": false, "retried": false, "failedOpen": false, "reason": "skipped" }),
            "{mode} {why}: clustering did not run"
        );
    }
    Ok(())
}

#[test]
fn fewer_than_two_candidates_skip_the_consolidator() {
    for mode in MODES {
        run_real(|lib| skip(lib, mode));
    }
}

// --- AC: the refuter prompt stays single-finding ------------------------------------

fn refuter_prompt(lib: &Lib, mode: &'static str) -> Outcome {
    let mut js = Js::open(lib)?;
    let agent = fleet(mode, corpus(), Consolidator::Reply(merge_x()), &[], &[]);
    let context = ctx(mode);
    js.review_ok(mode, &agent, context.clone())?;
    let all_dims = js.get("DIMENSIONS")?[mode].clone();
    let dims = reviewers(mode);
    // Each finding's own dimension and raw finder object.
    let raws: Vec<(&str, Value)> = corpus()
        .iter()
        .zip(dims)
        .flat_map(|(list, d)| list.iter().map(move |f| (d, f.clone())))
        .collect();
    let refuters = agent.calls_with("refute:");
    check_eq!(refuters.len(), 3, "{mode}: three units graded");
    for call in refuters {
        let Label::Refute { id } = parse_label(&call.label) else {
            return Err(Failure::Check(format!("not a refuter: {}", call.label)));
        };
        let (dim_key, raw) = raws
            .iter()
            .find(|(_, f)| f["id"] == json!(id))
            .ok_or_else(|| Failure::Check(format!("{mode}: unknown refuted id {id}")))?;
        let dim = all_dims
            .as_array()
            .and_then(|a| a.iter().find(|d| d["key"] == json!(dim_key)))
            .cloned()
            .ok_or_else(|| Failure::Check(format!("{mode}: no dimension {dim_key}")))?;
        let want = js.call(
            "refutePrompt",
            vec![json!(mode), dim, raw.clone(), context.clone()],
        )?;
        check_eq!(
            json!(call.prompt),
            want,
            "{mode}: {id}'s refuter prompt is the representative's single-finding prompt"
        );
    }
    Ok(())
}

#[test]
fn refuter_prompt_is_the_representatives_single_finding_prompt() {
    for mode in MODES {
        run_real(|lib| refuter_prompt(lib, mode));
    }
}

// --- AC: determinism ----------------------------------------------------------------

fn determinism(lib: &Lib, mode: &'static str) -> Outcome {
    let mut js = Js::open(lib)?;
    let mut outs = Vec::new();
    for _ in 0..2 {
        let agent = fleet(mode, corpus(), Consolidator::Reply(merge_x()), &[], &[]);
        outs.push(run(&mut js, mode, &agent, json!({}))?);
    }
    check_eq!(
        outs[0],
        outs[1],
        "{mode}: the same assignment gives the same result"
    );
    Ok(())
}

#[test]
fn same_assignment_gives_equal_results() {
    for mode in MODES {
        run_real(|lib| determinism(lib, mode));
    }
}

// --- AC: model and effort -----------------------------------------------------------

fn model_and_effort(lib: &Lib, mode: &'static str) -> Outcome {
    let mut js = Js::open(lib)?;
    let agent = fleet(mode, corpus(), Consolidator::Reply(merge_x()), &[], &[]);
    run(
        &mut js,
        mode,
        &agent,
        json!({ "consolidateModel": "m-consolidate", "consolidateEffort": "xhigh" }),
    )?;
    let calls = agent.calls_with("consolidate:");
    check_eq!(calls.len(), 1, "{mode}: one consolidator call");
    check_eq!(
        (
            calls[0].opts["model"].clone(),
            calls[0].opts["effort"].clone()
        ),
        (json!("m-consolidate"), json!("xhigh")),
        "{mode}: the consolidator carries consolidateModel/consolidateEffort"
    );
    check!(
        calls[0].opts.get("agentType").is_none(),
        "{mode}: the consolidator is a judgment site, not a mechanical agent"
    );

    for extra in [
        json!({ "consolidateModel": "m-consolidate" }),
        json!({ "consolidateModel": "m-consolidate", "consolidateEffort": "" }),
        json!({ "consolidateModel": "m-consolidate", "consolidateEffort": null }),
    ] {
        let agent = fleet(mode, corpus(), Consolidator::Reply(merge_x()), &[], &[]);
        run(&mut js, mode, &agent, extra.clone())?;
        let calls = agent.calls_with("consolidate:");
        check_eq!(calls.len(), 1, "{mode} {extra}: one consolidator call");
        check_eq!(
            calls[0].opts["model"],
            json!("m-consolidate"),
            "{mode} {extra}: model"
        );
        check!(
            calls[0].opts.get("effort").is_none(),
            "{mode} {extra}: no effort key: {}",
            calls[0].opts
        );
    }

    let agent = fleet(mode, corpus(), Consolidator::Reply(merge_x()), &[], &[]);
    let result = js.review(
        mode,
        &agent,
        ctx_with(mode, json!({ "consolidateEffort": "turbo" })),
    )?;
    check!(
        result.as_ref().is_err_and(|e| e.message.contains("turbo")),
        "{mode}: an invalid consolidateEffort is refused naming the value: {result:?}"
    );
    check!(
        agent.calls().is_empty(),
        "{mode}: refused before any agent is dispatched: {:?}",
        agent.labels()
    );
    Ok(())
}

#[test]
fn consolidator_carries_its_model_and_effort() {
    for mode in MODES {
        run_real(|lib| model_and_effort(lib, mode));
    }
}

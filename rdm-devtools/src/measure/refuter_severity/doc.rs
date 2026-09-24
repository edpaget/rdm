//! `--check` (recompute and compare against a doc's recorded figures) and
//! `--audit` (corpus-free arithmetic consistency of the doc's own figures) for
//! the three sections, plus the doc reader.
//!
//! Comparisons follow the JavaScript originals: `!==` between a recorded value
//! and a computed number, `(x || 0)` where the original defaulted, and
//! messages that print a missing value as `undefined`.

use std::path::Path;

use serde_json::Value;

use super::rank::{
    CAP_VERDICT_RULE, CapVerdictInputs, CapVerdictRule, RANK_UNRECOVERABLE_REASONS, WITHIN_TOP_N,
    derive_cap_verdict,
};
use crate::measure::jsnum::{fmt_number, pct, to_number};
use crate::measure::sidecar::{io_message, js_display, truthy};

/// The three sections a doc must carry.
#[derive(Debug, Clone)]
pub struct DocSections {
    /// `nonGatingRefutationSkip`.
    pub section: Value,
    /// `refuterFanout`.
    pub fanout: Value,
    /// `determiningFindingRank`.
    pub rank: Value,
}

/// Reads `doc_arg` (resolved against `repo_root` when relative).
///
/// # Errors
///
/// When the file cannot be read, is not JSON, or lacks a section.
pub fn read_doc(doc_arg: &str, repo_root: &Path) -> Result<DocSections, String> {
    let path = repo_root.join(doc_arg);
    let raw = std::fs::read(&path).map_err(|e| {
        format!(
            "cannot read {doc_arg} ({}): {}",
            path.display(),
            io_message(&e)
        )
    })?;
    let parsed: Value = serde_json::from_slice(&raw).map_err(|e| {
        format!(
            "{doc_arg} is not parseable JSON (--check/--audit expect docs/token-baseline.json): {e}"
        )
    })?;
    let take = |name: &str| -> Result<Value, String> {
        match parsed.get(name) {
            Some(v) if truthy(Some(v)) => Ok(v.clone()),
            _ => Err(format!("{doc_arg} has no \"{name}\" section")),
        }
    };
    Ok(DocSections {
        section: take("nonGatingRefutationSkip")?,
        fanout: take("refuterFanout")?,
        rank: take("determiningFindingRank")?,
    })
}

/// JavaScript `ToNumber` of an optional JSON value (absent: `NaN`).
pub fn num(v: Option<&Value>) -> f64 {
    match v {
        None => f64::NAN,
        Some(Value::Null) => 0.0,
        Some(Value::Bool(b)) => f64::from(u8::from(*b)),
        Some(Value::Number(n)) => n.as_f64().unwrap_or(f64::NAN),
        Some(Value::String(s)) => to_number(s),
        Some(_) => f64::NAN,
    }
}

/// `(v || 0)` as a number.
pub fn num_or0(v: Option<&Value>) -> f64 {
    if truthy(v) { num(v) } else { 0.0 }
}

/// `a !== b` where `b` is a computed number.
pub fn ne_num(a: Option<&Value>, b: f64) -> bool {
    !matches!(a, Some(Value::Number(n)) if n.as_f64() == Some(b))
}

/// `a !== b` for two optional JSON values.
pub fn ne(a: Option<&Value>, b: Option<&Value>) -> bool {
    match (a, b) {
        (None, None) => false,
        (Some(Value::Number(x)), Some(Value::Number(y))) => x.as_f64() != y.as_f64(),
        (Some(Value::String(x)), Some(Value::String(y))) => x != y,
        (Some(Value::Bool(x)), Some(Value::Bool(y))) => x != y,
        (Some(Value::Null), Some(Value::Null)) => false,
        _ => true,
    }
}

fn get<'a>(v: Option<&'a Value>, key: &str) -> Option<&'a Value> {
    v.and_then(|x| x.get(key))
}

fn arr(v: Option<&Value>) -> &[Value] {
    v.and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

fn show(v: Option<&Value>) -> String {
    js_display(v)
}

const TOKEN_CLASSES: [&str; 4] = ["output", "uncachedInput", "cacheWrite", "cacheRead"];

/// Compares the report's `nonGatingRefutationSkip` figures with the doc's.
pub fn check_doc(report: &Value, section: &Value) -> Vec<String> {
    let mut missing = Vec::new();
    let expect_rows = arr(section.get("refuteBySeverity"));
    let got_rows = arr(report.get("refuteBySeverity"));
    if expect_rows.len() != got_rows.len() {
        missing.push(format!(
            "row count: doc has {}, corpus yields {}",
            expect_rows.len(),
            got_rows.len()
        ));
    }
    for row in expect_rows {
        let key = row.get("key");
        let Some(got) = got_rows.iter().find(|g| !ne(g.get("key"), key)) else {
            missing.push(format!(
                "row \"{}\" is in the doc but not in the corpus",
                show(key)
            ));
            continue;
        };
        for field in ["agentCount", "graded", "refuted", "refutedRate"]
            .into_iter()
            .chain(TOKEN_CLASSES)
        {
            if ne(row.get(field), got.get(field)) {
                missing.push(format!(
                    "{}.{field}: doc {} vs corpus {}",
                    show(key),
                    show(row.get(field)),
                    show(got.get(field))
                ));
            }
        }
    }
    let p = section.get("projected");
    for field in [
        "agentsNotSpawned",
        "allTokens",
        "freshTokens",
        "percentOfRefuteAgents",
        "percentOfRefuteTokens",
        "percentOfLaneTokens",
    ] {
        let got = get(report.get("projected"), field);
        if ne(get(p, field), got) {
            missing.push(format!(
                "projected.{field}: doc {} vs corpus {}",
                show(get(p, field)),
                show(got)
            ));
        }
    }
    missing
}

/// Corpus-free audit of a doc's `nonGatingRefutationSkip`. The projection is
/// re-derived with the severity set the doc records under
/// `projected.severities` (the JS original re-read the live set from
/// `review.mjs`; an audit that needs no JavaScript runtime reads the doc's own
/// declaration instead).
pub fn audit_doc(section: &Value) -> Vec<String> {
    let mut problems = Vec::new();
    let rows = arr(section.get("refuteBySeverity"));
    if rows.is_empty() {
        problems.push("refuteBySeverity is empty".to_owned());
    }
    let totals = section.get("refuteTotals");
    let mut sum_agents = 0.0;
    let mut sums = [0.0; 4];
    for r in rows {
        let key = show(r.get("key"));
        sum_agents += num_or0(r.get("agentCount"));
        for (i, c) in TOKEN_CLASSES.iter().enumerate() {
            sums[i] += num_or0(r.get(c));
        }
        if num_or0(r.get("graded")) > num_or0(r.get("agentCount")) {
            problems.push(format!(
                "{key}: graded {} exceeds agentCount {}",
                show(r.get("graded")),
                show(r.get("agentCount"))
            ));
        }
        if num_or0(r.get("refuted")) > num_or0(r.get("graded")) {
            problems.push(format!(
                "{key}: refuted {} exceeds graded {}",
                show(r.get("refuted")),
                show(r.get("graded"))
            ));
        }
        let rate = pct(num_or0(r.get("refuted")), num_or0(r.get("graded")));
        if ne_num(r.get("refutedRate"), rate) {
            problems.push(format!(
                "{key}.refutedRate: doc {}, derived {}",
                show(r.get("refutedRate")),
                fmt_number(rate)
            ));
        }
    }
    if ne_num(get(totals, "agentCount"), sum_agents) {
        problems.push(format!(
            "refuteBySeverity agent counts sum to {}, refuteTotals says {}",
            fmt_number(sum_agents),
            show(get(totals, "agentCount"))
        ));
    }
    for (i, c) in TOKEN_CLASSES.iter().enumerate() {
        if ne_num(get(totals, c), sums[i]) {
            problems.push(format!(
                "refuteBySeverity {c} sums to {}, refuteTotals says {}",
                fmt_number(sums[i]),
                show(get(totals, c))
            ));
        }
    }
    let p = section.get("projected");
    let Some(severities) = get(p, "severities").and_then(Value::as_array) else {
        problems.push(
            "projected.severities is missing, so the projection cannot be re-derived".to_owned(),
        );
        return problems;
    };
    let non_gating: Vec<&str> = severities.iter().filter_map(Value::as_str).collect();
    // projectDrop over the doc's own rows, with JavaScript arithmetic: a
    // missing figure is NaN, and `whole ?` treats NaN/0 as "no share".
    let dropped: Vec<&Value> = rows
        .iter()
        .filter(|r| {
            r.get("key")
                .and_then(Value::as_str)
                .is_some_and(|k| non_gating.contains(&k))
        })
        .collect();
    let mut s_agents = 0.0;
    let mut s = [0.0; 4];
    for r in &dropped {
        s_agents += num(r.get("agentCount"));
        for (i, c) in TOKEN_CLASSES.iter().enumerate() {
            s[i] += num(r.get(c));
        }
    }
    let zero_nan = |x: f64| if x.is_nan() { 0.0 } else { x };
    let all = |v: &[f64; 4]| v.iter().map(|x| zero_nan(*x)).sum::<f64>();
    let totals_all: f64 = TOKEN_CLASSES.iter().map(|c| num_or0(get(totals, c))).sum();
    let lane = section.get("laneTotals");
    let lane_all: f64 = TOKEN_CLASSES.iter().map(|c| num_or0(get(lane, c))).sum();
    let expected: [(&str, f64); 10] = [
        ("agentsNotSpawned", s_agents),
        ("output", s[0]),
        ("uncachedInput", s[1]),
        ("cacheWrite", s[2]),
        ("cacheRead", s[3]),
        ("allTokens", all(&s)),
        (
            "freshTokens",
            zero_nan(s[0]) + zero_nan(s[1]) + zero_nan(s[2]),
        ),
        (
            "percentOfRefuteAgents",
            pct(s_agents, num(get(totals, "agentCount"))),
        ),
        ("percentOfRefuteTokens", pct(all(&s), totals_all)),
        ("percentOfLaneTokens", pct(all(&s), lane_all)),
    ];
    for (field, want) in expected {
        if ne_num(get(p, field), want) {
            problems.push(format!(
                "projected.{field}: doc {}, derived from the doc's own rows {}",
                show(get(p, field)),
                fmt_number(want)
            ));
        }
    }
    problems
}

/// Compares the report's `refuterFanout` with the doc's.
pub fn check_fanout_doc(report: &Value, fanout: &Value) -> Vec<String> {
    let mut missing = Vec::new();
    let got = report.get("refuterFanout");
    let got_fp = get(got, "findingsPerFinder");
    let exp_fp = fanout.get("findingsPerFinder");
    let expect_rows = arr(get(exp_fp, "rows"));
    let got_rows = arr(get(got_fp, "rows"));
    if expect_rows.len() != got_rows.len() {
        missing.push(format!(
            "findingsPerFinder row count: doc has {}, corpus yields {}",
            expect_rows.len(),
            got_rows.len()
        ));
    }
    for row in expect_rows {
        let key = row.get("key");
        let Some(g) = got_rows.iter().find(|g| !ne(g.get("key"), key)) else {
            missing.push(format!(
                "findingsPerFinder row \"{}\" is in the doc but not in the corpus",
                show(key)
            ));
            continue;
        };
        for field in ["n", "min", "p50", "p90", "max", "refutersDispatched"] {
            if ne(row.get(field), g.get(field)) {
                missing.push(format!(
                    "findingsPerFinder.{}.{field}: doc {} vs corpus {}",
                    show(key),
                    show(row.get(field)),
                    show(g.get(field))
                ));
            }
        }
    }
    for field in ["unreadableFinderCount", "unresolvedLabelCount"] {
        if ne(get(exp_fp, field), get(got_fp, field)) {
            missing.push(format!(
                "findingsPerFinder.{field}: doc {} vs corpus {}",
                show(get(exp_fp, field)),
                show(get(got_fp, field))
            ));
        }
    }
    let exp_u = fanout.get("refuterCountsByUnit");
    let got_u = get(got, "refuterCountsByUnit");
    for field in [
        "n",
        "min",
        "p50",
        "p90",
        "max",
        "totalRefuters",
        "recoveredRefuters",
        "unrecoverableRefuterCount",
        "recoveryRatePercent",
    ] {
        if ne(get(exp_u, field), get(got_u, field)) {
            missing.push(format!(
                "refuterCountsByUnit.{field}: doc {} vs corpus {}",
                show(get(exp_u, field)),
                show(get(got_u, field))
            ));
        }
    }
    missing
}

fn monotonic(v: Option<&Value>) -> bool {
    let (min, p50, p90, max) = (
        num(get(v, "min")),
        num(get(v, "p50")),
        num(get(v, "p90")),
        num(get(v, "max")),
    );
    min <= p50 && p50 <= p90 && p90 <= max
}

fn quad(v: Option<&Value>) -> String {
    format!(
        "{}/{}/{}/{}",
        show(get(v, "min")),
        show(get(v, "p50")),
        show(get(v, "p90")),
        show(get(v, "max"))
    )
}

/// Corpus-free audit of a doc's `refuterFanout`.
pub fn audit_fanout_doc(fanout: &Value) -> Vec<String> {
    let mut problems = Vec::new();
    for r in arr(get(fanout.get("findingsPerFinder"), "rows")) {
        if !monotonic(Some(r)) {
            problems.push(format!(
                "findingsPerFinder.{}: min/p50/p90/max not monotonic ({})",
                show(r.get("key")),
                quad(Some(r))
            ));
        }
    }
    let u = fanout.get("refuterCountsByUnit");
    if num_or0(get(u, "n")) > 0.0 && !monotonic(u) {
        problems.push(format!(
            "refuterCountsByUnit: min/p50/p90/max not monotonic ({})",
            quad(u)
        ));
    }
    let rec_sum =
        num_or0(get(u, "recoveredRefuters")) + num_or0(get(u, "unrecoverableRefuterCount"));
    if ne_num(get(u, "totalRefuters"), rec_sum) {
        problems.push(format!(
            "refuterCountsByUnit: recoveredRefuters + unrecoverableRefuterCount ({}) !== totalRefuters ({})",
            fmt_number(rec_sum),
            show(get(u, "totalRefuters"))
        ));
    }
    let derived = pct(
        num_or0(get(u, "recoveredRefuters")),
        num_or0(get(u, "totalRefuters")),
    );
    if ne_num(get(u, "recoveryRatePercent"), derived) {
        problems.push(format!(
            "refuterCountsByUnit.recoveryRatePercent: doc {}, derived {}",
            show(get(u, "recoveryRatePercent")),
            fmt_number(derived)
        ));
    }
    problems
}

/// Compares the report's `determiningFindingRank` with the doc's.
pub fn check_rank_doc(report: &Value, rank: &Value) -> Vec<String> {
    let mut missing = Vec::new();
    let got = report.get("determiningFindingRank");
    let exp = Some(rank);

    let cmp_units =
        |missing: &mut Vec<String>, g: Option<&Value>, e: Option<&Value>, prefix: &str| {
            for f in [
                "total",
                "determining",
                "nonDetermining",
                "unrecoverable",
                "recoverable",
                "recoverableSharePercent",
            ] {
                if ne(get(e, f), get(g, f)) {
                    missing.push(format!(
                        "{prefix}.{f}: doc {} vs corpus {}",
                        show(get(e, f)),
                        show(get(g, f))
                    ));
                }
            }
        };
    let cmp_hist =
        |missing: &mut Vec<String>, g: Option<&Value>, e: Option<&Value>, prefix: &str| {
            let e = arr(e);
            let g = arr(g);
            if e.len() != g.len() {
                missing.push(format!(
                    "{prefix} row count: doc has {}, corpus yields {}",
                    e.len(),
                    g.len()
                ));
            }
            for row in e {
                let got_count = g
                    .iter()
                    .find(|x| !ne(x.get("rank"), row.get("rank")))
                    .and_then(|x| x.get("count"));
                if ne(got_count, row.get("count")) {
                    missing.push(format!(
                        "{prefix}[rank {}]: doc {} vs corpus {}",
                        show(row.get("rank")),
                        show(row.get("count")),
                        show(got_count)
                    ));
                }
            }
        };
    let cmp_within =
        |missing: &mut Vec<String>, g: Option<&Value>, e: Option<&Value>, prefix: &str| {
            for w in arr(e) {
                let Some(gw) = arr(g).iter().find(|x| !ne(x.get("n"), w.get("n"))) else {
                    missing.push(format!(
                        "{prefix}[n={}] is in the doc but not in the corpus",
                        show(w.get("n"))
                    ));
                    continue;
                };
                for f in ["count", "percentOfDetermining", "percentOfRecoverable"] {
                    if ne(w.get(f), gw.get(f)) {
                        missing.push(format!(
                            "{prefix}[n={}].{f}: doc {} vs corpus {}",
                            show(w.get("n")),
                            show(w.get(f)),
                            show(gw.get(f))
                        ));
                    }
                }
            }
        };
    let cmp_summary =
        |missing: &mut Vec<String>, g: Option<&Value>, e: Option<&Value>, prefix: &str| {
            let (g, e) = (g.filter(|v| truthy(Some(v))), e.filter(|v| truthy(Some(v))));
            match (g, e) {
                (None, None) => {}
                (Some(g), Some(e)) => {
                    for f in ["n", "min", "p50", "p90", "max"] {
                        if ne(e.get(f), g.get(f)) {
                            missing.push(format!(
                                "{prefix}.{f}: doc {} vs corpus {}",
                                show(e.get(f)),
                                show(g.get(f))
                            ));
                        }
                    }
                }
                (g, e) => missing.push(format!(
                    "{prefix}: doc {} it, corpus {} it",
                    if e.is_some() { "has" } else { "omits" },
                    if g.is_some() { "has" } else { "omits" }
                )),
            }
        };

    cmp_units(&mut missing, get(got, "units"), get(exp, "units"), "units");
    let exp_reasons = arr(get(exp, "unrecoverableByReason"));
    let got_reasons = arr(get(got, "unrecoverableByReason"));
    if exp_reasons.len() != got_reasons.len() {
        missing.push(format!(
            "unrecoverableByReason row count: doc has {}, corpus yields {}",
            exp_reasons.len(),
            got_reasons.len()
        ));
    }
    for row in exp_reasons {
        let got_count = got_reasons
            .iter()
            .find(|x| !ne(x.get("reason"), row.get("reason")))
            .and_then(|x| x.get("count"));
        if ne(got_count, row.get("count")) {
            missing.push(format!(
                "unrecoverableByReason[{}]: doc {} vs corpus {}",
                show(row.get("reason")),
                show(row.get("count")),
                show(got_count)
            ));
        }
    }
    for f in ["finders", "refuters", "runsAffected"] {
        let e = get(get(exp, "orphanAgents"), f);
        let g = get(get(got, "orphanAgents"), f);
        if ne(e, g) {
            missing.push(format!(
                "orphanAgents.{f}: doc {} vs corpus {}",
                show(e),
                show(g)
            ));
        }
    }
    cmp_hist(
        &mut missing,
        get(got, "rankHistogram"),
        get(exp, "rankHistogram"),
        "rankHistogram",
    );
    cmp_summary(
        &mut missing,
        get(got, "rankSummary"),
        get(exp, "rankSummary"),
        "rankSummary",
    );
    cmp_within(
        &mut missing,
        get(got, "withinTop"),
        get(exp, "withinTop"),
        "withinTop",
    );
    cmp_summary(
        &mut missing,
        get(got, "candidateSetSize"),
        get(exp, "candidateSetSize"),
        "candidateSetSize",
    );
    if ne(get(exp, "acTableGapUnits"), get(got, "acTableGapUnits")) {
        missing.push(format!(
            "acTableGapUnits: doc {} vs corpus {}",
            show(get(exp, "acTableGapUnits")),
            show(get(got, "acTableGapUnits"))
        ));
    }
    let el = get(exp, "largeTier");
    let gl = get(got, "largeTier");
    cmp_units(
        &mut missing,
        get(gl, "units"),
        get(el, "units"),
        "largeTier.units",
    );
    cmp_hist(
        &mut missing,
        get(gl, "rankHistogram"),
        get(el, "rankHistogram"),
        "largeTier.rankHistogram",
    );
    cmp_summary(
        &mut missing,
        get(gl, "rankSummary"),
        get(el, "rankSummary"),
        "largeTier.rankSummary",
    );
    cmp_within(
        &mut missing,
        get(gl, "withinTop"),
        get(el, "withinTop"),
        "largeTier.withinTop",
    );
    let ev = get(get(exp, "capVerdict"), "verdict");
    let gv = get(get(got, "capVerdict"), "verdict");
    if ne(ev, gv) {
        missing.push(format!(
            "capVerdict.verdict: doc {} vs corpus {}",
            show(ev),
            show(gv)
        ));
    }
    missing
}

/// Corpus-free audit of a doc's `determiningFindingRank`, including the
/// re-derivation of its cap verdict under `rule` (production:
/// [`CAP_VERDICT_RULE`]).
pub fn audit_rank_doc(rank: &Value, rule: &CapVerdictRule) -> Vec<String> {
    let mut problems = Vec::new();
    audit_block(&mut problems, Some(rank), "determiningFindingRank", true);
    let large = rank.get("largeTier");
    audit_block(
        &mut problems,
        large,
        "determiningFindingRank.largeTier",
        false,
    );

    // Widening the blocker set can only move a determining finding earlier.
    let base_within = arr(rank.get("withinTop"));
    for w in arr(get(large, "withinTop")) {
        let base = base_within
            .iter()
            .find(|b| !ne(b.get("n"), w.get("n")))
            .map_or(0.0, |b| num_or0(b.get("count")));
        if num_or0(w.get("count")) < base {
            let shown_base = base_within
                .iter()
                .find(|b| !ne(b.get("n"), w.get("n")))
                .map_or_else(
                    || "undefined".to_owned(),
                    |b| fmt_number(num_or0(b.get("count"))),
                );
            problems.push(format!(
                "largeTier.withinTop[n={}].count ({}) is below the default tier's ({shown_base}) — widening the blocker set can only move a determining finding earlier, never later",
                show(w.get("n")),
                show(w.get("count"))
            ));
        }
    }
    let large_det = num(get(get(large, "units"), "determining"));
    let base_det = num(get(rank.get("units"), "determining"));
    if large_det < base_det {
        problems.push("largeTier.units.determining is below the default tier's — widening the blocker set cannot un-determine a unit".to_owned());
    }

    let cap = rank.get("capVerdict");
    let inputs = get(cap, "inputs");
    let units = rank.get("units");
    for field in [
        "determining",
        "recoverable",
        "total",
        "recoverableSharePercent",
    ] {
        if ne(get(inputs, field), get(units, field)) {
            problems.push(format!(
                "capVerdict.inputs.{field}: doc {}, units block says {}",
                show(get(inputs, field)),
                show(get(units, field))
            ));
        }
    }
    let top5 = arr(rank.get("withinTop"))
        .iter()
        .find(|w| w.get("n").and_then(Value::as_f64) == Some(5.0))
        .map(|w| w.get("percentOfDetermining"));
    let top5_shown = match top5 {
        Some(v) => show(v),
        None => "0".to_owned(),
    };
    let top5_mismatch = match top5 {
        Some(v) => ne(get(inputs, "withinTop5PercentOfDetermining"), v),
        None => ne_num(get(inputs, "withinTop5PercentOfDetermining"), 0.0),
    };
    if top5_mismatch {
        problems.push(format!(
            "capVerdict.inputs.withinTop5PercentOfDetermining: doc {}, withinTop says {top5_shown}",
            show(get(inputs, "withinTop5PercentOfDetermining"))
        ));
    }
    let derived = derive_cap_verdict(
        &CapVerdictInputs {
            determining: num_or0(get(inputs, "determining")),
            recoverable: num_or0(get(inputs, "recoverable")),
            total: num_or0(get(inputs, "total")),
            recoverable_share_percent: num_or0(get(inputs, "recoverableSharePercent")),
            within_top5_percent_of_determining: num_or0(get(
                inputs,
                "withinTop5PercentOfDetermining",
            )),
        },
        rule,
    );
    let recorded = get(cap, "verdict");
    if recorded.and_then(Value::as_str) != Some(derived) {
        problems.push(format!(
            "capVerdict.verdict: doc \"{}\", re-derived from the doc's own figures \"{derived}\" — the supports/kills conclusion must follow from the numbers, not be asserted beside them",
            show(recorded)
        ));
    }
    let rule_fields: [(&str, Value); 5] = [
        (
            "supportsCapAtOrAbovePercent",
            crate::measure::jsnum::json_number(rule.supports_cap_at_or_above_percent),
        ),
        (
            "killsCapBelowPercent",
            crate::measure::jsnum::json_number(rule.kills_cap_below_percent),
        ),
        (
            "minRecoverableSharePercent",
            crate::measure::jsnum::json_number(rule.min_recoverable_share_percent),
        ),
        (
            "minDeterminingUnits",
            crate::measure::jsnum::json_number(rule.min_determining_units),
        ),
        ("basis", Value::String(rule.basis.to_owned())),
    ];
    for (key, want) in rule_fields {
        let doc = get(get(cap, "rule"), key);
        if ne(doc, Some(&want)) {
            problems.push(format!(
                "capVerdict.rule.{key}: doc {}, instrument {}",
                show(doc),
                show(Some(&want))
            ));
        }
    }

    let orphans = rank.get("orphanAgents");
    for f in ["finders", "refuters", "runsAffected"] {
        let v = get(orphans, f);
        if !matches!(v, Some(Value::Number(n)) if n.as_f64().is_some_and(|x| x >= 0.0)) {
            problems.push(format!(
                "orphanAgents.{f} must be a non-negative number, got {}",
                show(v)
            ));
        }
    }
    let gap = rank.get("acTableGapUnits");
    if !matches!(gap, Some(Value::Number(n)) if n.as_f64().is_some_and(|x| x >= 0.0)) {
        problems.push(format!(
            "acTableGapUnits must be a non-negative number, got {}",
            show(gap)
        ));
    }
    problems
}

/// [`audit_rank_doc`] under the production rule.
pub fn audit_rank_doc_default(rank: &Value) -> Vec<String> {
    audit_rank_doc(rank, &CAP_VERDICT_RULE)
}

fn audit_block(
    problems: &mut Vec<String>,
    block: Option<&Value>,
    prefix: &str,
    with_reasons: bool,
) {
    let u = get(block, "units");
    let total = num_or0(get(u, "total"));
    let determining = num_or0(get(u, "determining"));
    let non_determining = num_or0(get(u, "nonDetermining"));
    let unrecoverable = num_or0(get(u, "unrecoverable"));
    let recoverable = num_or0(get(u, "recoverable"));
    let partition = determining + non_determining + unrecoverable;
    if partition != total {
        problems.push(format!(
            "{prefix}.units: determining + nonDetermining + unrecoverable ({}) !== total ({})",
            fmt_number(partition),
            fmt_number(total)
        ));
    }
    if determining + non_determining != recoverable {
        problems.push(format!(
            "{prefix}.units.recoverable: doc {}, derived {}",
            fmt_number(recoverable),
            fmt_number(determining + non_determining)
        ));
    }
    let share = pct(recoverable, total);
    if ne_num(get(u, "recoverableSharePercent"), share) {
        problems.push(format!(
            "{prefix}.units.recoverableSharePercent: doc {}, derived {}",
            show(get(u, "recoverableSharePercent")),
            fmt_number(share)
        ));
    }

    let hist = arr(get(block, "rankHistogram"));
    let hist_sum: f64 = hist.iter().map(|r| num_or0(r.get("count"))).sum();
    if hist_sum != determining {
        problems.push(format!(
            "{prefix}.rankHistogram counts sum to {}, units.determining says {}",
            fmt_number(hist_sum),
            fmt_number(determining)
        ));
    }
    if hist.windows(2).any(|w| {
        num(w[0].get("rank")).partial_cmp(&num(w[1].get("rank"))) != Some(std::cmp::Ordering::Less)
    }) {
        problems.push(format!(
            "{prefix}.rankHistogram is not in strictly ascending rank order"
        ));
    }
    let summary = get(block, "rankSummary").filter(|v| truthy(Some(v)));
    if determining == 0.0 && summary.is_some() {
        problems.push(format!(
            "{prefix}.rankSummary is present with zero determining units"
        ));
    }
    if determining > 0.0 {
        match summary {
            None => problems.push(format!(
                "{prefix}.rankSummary is missing with {} determining units",
                fmt_number(determining)
            )),
            Some(s) => {
                if ne_num(s.get("n"), determining) {
                    problems.push(format!(
                        "{prefix}.rankSummary.n: doc {}, determining {}",
                        show(s.get("n")),
                        fmt_number(determining)
                    ));
                }
                if !monotonic(Some(s)) {
                    problems.push(format!(
                        "{prefix}.rankSummary: min/p50/p90/max not monotonic ({})",
                        quad(Some(s))
                    ));
                }
            }
        }
    }

    let within = arr(get(block, "withinTop"));
    let ns: Vec<String> = within.iter().map(|w| show(w.get("n"))).collect();
    let expected_ns: Vec<String> = WITHIN_TOP_N.iter().map(ToString::to_string).collect();
    if ns != expected_ns {
        problems.push(format!(
            "{prefix}.withinTop must carry exactly n={}, in that order",
            expected_ns.join(" and n=")
        ));
    }
    let mut previous = 0.0;
    for w in within {
        let count = num_or0(w.get("count"));
        let n = show(w.get("n"));
        if count < previous {
            problems.push(format!(
                "{prefix}.withinTop[n={n}].count ({}) is below a smaller n's count",
                fmt_number(count)
            ));
        }
        previous = count;
        if count > determining {
            problems.push(format!(
                "{prefix}.withinTop[n={n}].count ({}) exceeds units.determining ({})",
                fmt_number(count),
                fmt_number(determining)
            ));
        }
        let d_pct = pct(count, determining);
        let r_pct = pct(count, recoverable);
        if ne_num(w.get("percentOfDetermining"), d_pct) {
            problems.push(format!(
                "{prefix}.withinTop[n={n}].percentOfDetermining: doc {}, derived {}",
                show(w.get("percentOfDetermining")),
                fmt_number(d_pct)
            ));
        }
        if ne_num(w.get("percentOfRecoverable"), r_pct) {
            problems.push(format!(
                "{prefix}.withinTop[n={n}].percentOfRecoverable: doc {}, derived {}",
                show(w.get("percentOfRecoverable")),
                fmt_number(r_pct)
            ));
        }
    }

    if with_reasons {
        let reasons = arr(get(block, "unrecoverableByReason"));
        let reason_sum: f64 = reasons.iter().map(|r| num_or0(r.get("count"))).sum();
        if reason_sum != unrecoverable {
            problems.push(format!(
                "{prefix}.unrecoverableByReason counts sum to {}, units.unrecoverable says {}",
                fmt_number(reason_sum),
                fmt_number(unrecoverable)
            ));
        }
        let mut last: Option<usize> = None;
        for r in reasons {
            let name = show(r.get("reason"));
            match RANK_UNRECOVERABLE_REASONS.iter().position(|x| *x == name) {
                None => problems.push(format!(
                    "{prefix}.unrecoverableByReason: \"{name}\" is not in the closed vocabulary [{}] — unit statuses are decided PER UNIT, so there is no run-wide reason",
                    RANK_UNRECOVERABLE_REASONS.join(", ")
                )),
                Some(idx) if last.is_some_and(|l| idx <= l) => problems.push(format!(
                    "{prefix}.unrecoverableByReason is not in the fixed report order"
                )),
                Some(idx) => last = Some(idx),
            }
        }
    }
}

/// Every audit over the three sections.
pub fn audit_all(doc: &DocSections, rule: &CapVerdictRule) -> Vec<String> {
    let mut problems = audit_doc(&doc.section);
    problems.extend(audit_fanout_doc(&doc.fanout));
    problems.extend(audit_rank_doc(&doc.rank, rule));
    problems
}

/// Every check of `report` (as JSON) against the three sections.
pub fn check_all(report: &Value, doc: &DocSections) -> Vec<String> {
    let mut missing = check_doc(report, &doc.section);
    missing.extend(check_fanout_doc(report, &doc.fanout));
    missing.extend(check_rank_doc(report, &doc.rank));
    missing
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::measure::review_rules::checkout_root;

    fn load(rel: &str) -> DocSections {
        read_doc(rel, &checkout_root()).unwrap_or_else(|e| panic!("{e}"))
    }

    #[test]
    fn run_wide_orphan_reason_refused_by_audit() {
        let doc =
            load("tests/fixtures/token-determining-rank/expected-determiningFindingRank.json");
        assert!(audit_rank_doc_default(&doc.rank).is_empty());
        let mut rank = doc.rank.clone();
        if let Some(reasons) = rank["unrecoverableByReason"].as_array_mut() {
            reasons.push(serde_json::json!({"reason": "orphan-agent-in-run", "count": 1}));
        }
        // Keep every other figure consistent, so only the vocabulary can fail.
        let units = &mut rank["units"];
        let total = units["total"].as_u64().unwrap_or(0) + 1;
        let unrecoverable = units["unrecoverable"].as_u64().unwrap_or(0) + 1;
        let recoverable = units["recoverable"].as_f64().unwrap_or(0.0);
        units["total"] = total.into();
        units["unrecoverable"] = unrecoverable.into();
        #[allow(clippy::cast_precision_loss)]
        let share = crate::measure::jsnum::pct(recoverable, total as f64);
        units["recoverableSharePercent"] = crate::measure::jsnum::json_number(share);
        rank["capVerdict"]["inputs"]["total"] = total.into();
        rank["capVerdict"]["inputs"]["recoverableSharePercent"] =
            crate::measure::jsnum::json_number(share);
        let problems = audit_rank_doc_default(&rank);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("\"orphan-agent-in-run\" is not in the closed vocabulary"));
    }

    #[test]
    fn committed_verdict_rederives_only_under_the_real_rule() {
        let doc = load("docs/token-baseline.json");
        assert!(audit_all(&doc, &CAP_VERDICT_RULE).is_empty());
        let mutated = CapVerdictRule {
            min_determining_units: 2000.0,
            ..CAP_VERDICT_RULE
        };
        let problems = audit_rank_doc(&doc.rank, &mutated);
        assert!(
            problems.iter().any(|p| p.starts_with("capVerdict.verdict")),
            "a mutated threshold flips the committed verdict: {problems:?}"
        );
    }
}

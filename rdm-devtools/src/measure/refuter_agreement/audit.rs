//! Corpus-free arithmetic audits of `docs/token-baseline.json`'s
//! `refuterModelTiering` and `refuterBatching` sections, and the recursive
//! "no blended accuracy" check over every emitted report.

use serde_json::Value;

use super::corpus::{
    DIVERGENCE_CLASS, MIN_AUTHORITATIVE_SHARE, MIN_CORPUS_SIZE, MIN_DIVERGENCE_CLASS_SHARE,
    MIN_MINED_SHARE, TOKEN_CLASSES,
};
use super::score::{mean, rate};
use super::trials::MIN_BATCH_GROUP_SIZE;
use crate::measure::jsnum::{fmt_number, is_integer, js_round, json_number};
use crate::measure::refuter_severity::doc::{ne, num, num_or0};
use crate::measure::sidecar::js_display;

/// The closed decision vocabulary for the batching question.
pub const BATCHING_DECISIONS: [&str; 4] = [
    "ship-batched",
    "no-ship-worse-fn",
    "no-ship-anchoring",
    "no-measurement",
];

fn get<'a>(v: Option<&'a Value>, k: &str) -> Option<&'a Value> {
    v.and_then(|x| x.get(k))
}

fn show(v: Option<&Value>) -> String {
    js_display(v)
}

fn json_str(v: Option<&Value>) -> String {
    v.map_or_else(|| "undefined".to_owned(), Value::to_string)
}

fn opt_num(v: Option<f64>) -> Value {
    v.map_or(Value::Null, json_number)
}

fn non_empty_string(v: Option<&Value>) -> bool {
    v.and_then(Value::as_str)
        .is_some_and(|s| !crate::measure::jsnum::js_trim(s).is_empty())
}

fn is_plain_object(v: Option<&Value>) -> bool {
    matches!(v, Some(Value::Object(_)))
}

fn share(part: f64, size: f64) -> f64 {
    if size == 0.0 || size.is_nan() {
        0.0
    } else {
        js_round((part / size) * 1000.0) / 10.0
    }
}

fn rate_value(part: f64, whole: f64) -> Value {
    opt_num(rate(part, whole))
}

fn audit_set_rates(problems: &mut Vec<String>, s: Option<&Value>, label: &str, set_name: &str) {
    let fn_rate = rate_value(
        num_or0(get(s, "falseNegatives")),
        num_or0(get(s, "defectTrials")),
    );
    if ne(get(s, "falseNegativeRate"), Some(&fn_rate)) {
        problems.push(format!(
            "{label}.{set_name}.falseNegativeRate: doc {}, derived {}",
            show(get(s, "falseNegativeRate")),
            show(Some(&fn_rate))
        ));
    }
    let fp_rate = rate_value(
        num_or0(get(s, "falsePositives")),
        num_or0(get(s, "nonDefectTrials")),
    );
    if ne(get(s, "falsePositiveRate"), Some(&fp_rate)) {
        problems.push(format!(
            "{label}.{set_name}.falsePositiveRate: doc {}, derived {}",
            show(get(s, "falsePositiveRate")),
            show(Some(&fp_rate))
        ));
    }
}

fn audit_partition(problems: &mut Vec<String>, row: &Value, label: &str) {
    let a = row.get("authoritativeOnly");
    let j = row.get("judgementCallOnly");
    let all = row.get("all");
    for f in [
        "trials",
        "ungraded",
        "defectTrials",
        "nonDefectTrials",
        "falseNegatives",
        "falsePositives",
    ] {
        if num_or0(get(a, f)) + num_or0(get(j, f)) != num_or0(get(all, f)) {
            problems.push(format!(
                "{label}.all.{f} {} != authoritativeOnly {} + judgementCallOnly {}",
                show(get(all, f)),
                show(get(a, f)),
                show(get(j, f))
            ));
        }
    }
}

/// Audits a `refuterModelTiering` section; empty when consistent.
pub fn audit_tiering_section(section: &Value) -> Vec<String> {
    let mut problems = Vec::new();
    if !section.is_object() {
        return vec!["refuterModelTiering section is not an object".to_owned()];
    }
    let corpus = section.get("corpus");
    let size = get(corpus, "size");
    #[allow(clippy::cast_precision_loss)]
    let size_ok = size
        .and_then(Value::as_f64)
        .is_some_and(|n| is_integer(n) && n >= MIN_CORPUS_SIZE as f64);
    if !size_ok {
        problems.push(format!(
            "corpus.size must be an integer >= {MIN_CORPUS_SIZE}, got {}",
            json_str(size)
        ));
    }
    let size_n = num(size);
    for field in ["byClass", "byAuthority", "byProvenance"] {
        let Some(Value::Object(map)) = get(corpus, field) else {
            problems.push(format!("corpus.{field} is missing"));
            continue;
        };
        let sum: f64 = map
            .values()
            .map(|v| {
                let n = num(Some(v));
                if n.is_nan() { 0.0 } else { n }
            })
            .sum();
        if !matches!(size, Some(Value::Number(n)) if n.as_f64() == Some(sum)) {
            problems.push(format!(
                "corpus.{field} counts sum to {}, corpus.size says {}",
                fmt_number(sum),
                show(size)
            ));
        }
    }
    let share_check = |problems: &mut Vec<String>,
                       map_key: &str,
                       part_key: &str,
                       field: &str,
                       floor: f64,
                       label: &str| {
        if let Some(map) = get(corpus, map_key).filter(|m| m.is_object()) {
            let derived = share(num_or0(map.get(part_key)), num_or0(size));
            let _ = size_n;
            if ne(get(corpus, field), Some(&json_number(derived))) {
                problems.push(format!(
                    "corpus.{field}: doc {}, derived {}",
                    show(get(corpus, field)),
                    fmt_number(derived)
                ));
            }
            if derived < floor * 100.0 {
                problems.push(format!(
                    "{label} share {}% is below the {}% floor",
                    fmt_number(derived),
                    fmt_number(floor * 100.0)
                ));
            }
        }
    };
    share_check(
        &mut problems,
        "byClass",
        DIVERGENCE_CLASS,
        "divergenceClassShare",
        MIN_DIVERGENCE_CLASS_SHARE,
        "divergence-class",
    );
    share_check(
        &mut problems,
        "byAuthority",
        "authoritative",
        "authoritativeShare",
        MIN_AUTHORITATIVE_SHARE,
        "authoritative",
    );
    share_check(
        &mut problems,
        "byProvenance",
        "mined",
        "minedShare",
        MIN_MINED_SHARE,
        "mined",
    );

    let tiers: &[Value] = section
        .get("tiers")
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice);
    if tiers.len() < 2 {
        problems.push("tiers must record at least two model tiers".to_owned());
    }
    for t in tiers {
        let label = show(t.get("tier"));
        for set_name in ["authoritativeOnly", "judgementCallOnly", "all"] {
            let s = t.get(set_name);
            if !is_plain_object(s) {
                problems.push(format!("{label}.{set_name} is missing"));
                continue;
            }
            if num_or0(get(s, "falseNegatives")) > num_or0(get(s, "defectTrials")) {
                problems.push(format!(
                    "{label}.{set_name}: falseNegatives {} exceeds defectTrials {}",
                    show(get(s, "falseNegatives")),
                    show(get(s, "defectTrials"))
                ));
            }
            if num_or0(get(s, "falsePositives")) > num_or0(get(s, "nonDefectTrials")) {
                problems.push(format!(
                    "{label}.{set_name}: falsePositives {} exceeds nonDefectTrials {}",
                    show(get(s, "falsePositives")),
                    show(get(s, "nonDefectTrials"))
                ));
            }
            audit_set_rates(&mut problems, s, &label, set_name);
            let parts = num_or0(get(s, "defectTrials"))
                + num_or0(get(s, "nonDefectTrials"))
                + num_or0(get(s, "ungraded"));
            if parts != num_or0(get(s, "trials")) {
                problems.push(format!(
                    "{label}.{set_name}: defectTrials + nonDefectTrials + ungraded ({}) != trials {}",
                    fmt_number(parts),
                    show(get(s, "trials"))
                ));
            }
        }
        audit_partition(&mut problems, t, &label);
        let cost = t.get("cost");
        let summed: f64 = TOKEN_CLASSES
            .iter()
            .map(|c| {
                let n = num(get(cost, c));
                if n.is_nan() || !crate::measure::sidecar::truthy(get(cost, c)) {
                    0.0
                } else {
                    n
                }
            })
            .sum();
        if !matches!(get(cost, "totalTokens"), Some(Value::Number(n)) if n.as_f64() == Some(summed))
        {
            problems.push(format!(
                "{label}.cost.totalTokens: doc {}, four classes sum to {}",
                show(get(cost, "totalTokens")),
                fmt_number(summed)
            ));
        }
        let mean_tokens = opt_num(mean(summed, num_or0(get(cost, "dispatchedTrials"))));
        if ne(get(cost, "meanTokensPerTrial"), Some(&mean_tokens)) {
            problems.push(format!(
                "{label}.cost.meanTokensPerTrial: doc {}, derived {}",
                show(get(cost, "meanTokensPerTrial")),
                show(Some(&mean_tokens))
            ));
        }
        let mean_calls = opt_num(mean(
            num_or0(get(cost, "toolCalls")),
            num_or0(get(cost, "dispatchedTrials")),
        ));
        if ne(get(cost, "meanToolCallsPerTrial"), Some(&mean_calls)) {
            problems.push(format!(
                "{label}.cost.meanToolCallsPerTrial: doc {}, derived {}",
                show(get(cost, "meanToolCallsPerTrial")),
                show(Some(&mean_calls))
            ));
        }
        let sc = t.get("selfConsistency");
        let fr = rate_value(
            num_or0(get(sc, "replicateFlips")),
            num_or0(get(sc, "replicatePairs")),
        );
        if ne(get(sc, "flipRate"), Some(&fr)) {
            problems.push(format!(
                "{label}.selfConsistency.flipRate: doc {}, derived {}",
                show(get(sc, "flipRate")),
                show(Some(&fr))
            ));
        }
    }

    let baseline_tier = section.get("baselineTier");
    let baseline = tiers.iter().find(|t| !ne(t.get("tier"), baseline_tier));
    if baseline.is_none() {
        problems.push(format!(
            "baselineTier \"{}\" is not among the recorded tiers",
            show(baseline_tier)
        ));
    }
    for t in tiers {
        let label = show(t.get("tier"));
        if !ne(t.get("tier"), baseline_tier) {
            if !matches!(t.get("tokenDelta"), None | Some(Value::Null)) {
                problems.push(format!(
                    "{label} is the baseline and must carry tokenDelta: null"
                ));
            }
            continue;
        }
        let Some(delta) = t.get("tokenDelta").filter(|d| d.is_object()) else {
            problems.push(format!("{label}.tokenDelta is missing"));
            continue;
        };
        let Some(b) = baseline else { continue };
        let expected = js_round(
            (num(get(t.get("cost"), "meanTokensPerTrial"))
                - num(get(b.get("cost"), "meanTokensPerTrial")))
                * 10.0,
        ) / 10.0;
        if !matches!(delta.get("delta"), Some(Value::Number(n)) if n.as_f64() == Some(expected)) {
            problems.push(format!(
                "{label}.tokenDelta.delta: doc {}, derived {}",
                show(delta.get("delta")),
                fmt_number(expected)
            ));
        }
    }

    if !non_empty_string(section.get("decision")) {
        problems.push("decision must be a non-empty string".to_owned());
    }
    if !non_empty_string(section.get("doc")) {
        problems.push("doc pointer must be a non-empty string".to_owned());
    }
    if !is_plain_object(section.get("measurementWindow"))
        || !non_empty_string(get(section.get("measurementWindow"), "until"))
    {
        problems.push("measurementWindow.until must be recorded".to_owned());
    }
    problems
}

/// Audits a `refuterBatching` section; empty when consistent.
pub fn audit_batching_section(section: &Value) -> Vec<String> {
    let mut problems = Vec::new();
    if !section.is_object() {
        return vec!["refuterBatching section is not an object".to_owned()];
    }
    let decision = section.get("decision");
    if !decision
        .and_then(Value::as_str)
        .is_some_and(|d| BATCHING_DECISIONS.contains(&d))
    {
        problems.push(format!(
            "decision must be one of {}, got {}",
            BATCHING_DECISIONS.join("|"),
            json_str(decision)
        ));
    }
    if !non_empty_string(section.get("doc")) {
        problems.push("doc pointer must be a non-empty string".to_owned());
    }
    if !is_plain_object(section.get("measurementWindow"))
        || !non_empty_string(get(section.get("measurementWindow"), "until"))
    {
        problems.push("measurementWindow.until must be recorded".to_owned());
    }
    let no_measurement = decision.and_then(Value::as_str) == Some("no-measurement");

    match section.get("corpusPower").filter(|p| p.is_object()) {
        None => problems.push("corpusPower is missing".to_owned()),
        Some(power) => {
            let n0 = |k: &str| {
                let n = num(power.get(k));
                if n.is_nan() || !crate::measure::sidecar::truthy(power.get(k)) {
                    0.0
                } else {
                    n
                }
            };
            let accounted = n0("constructedExcluded")
                + n0("nonGatingExcluded")
                + n0("unrecoverableUnitExcluded")
                + n0("groupableItems");
            if accounted != num(power.get("totalItems")) {
                problems.push(format!(
                    "corpusPower: constructed + nonGating + unrecoverableUnit + groupable ({}) != totalItems {}",
                    fmt_number(accounted),
                    show(power.get("totalItems"))
                ));
            }
            let empty = serde_json::Map::new();
            let hist = power
                .get("sizeHistogram")
                .and_then(Value::as_object)
                .unwrap_or(&empty);
            let min_group = num(power.get("minGroupSize"));
            #[allow(clippy::cast_precision_loss)]
            if !is_integer(min_group) || min_group < MIN_BATCH_GROUP_SIZE as f64 {
                problems.push(format!(
                    "corpusPower.minGroupSize must be an integer >= {MIN_BATCH_GROUP_SIZE}, got {}",
                    json_str(power.get("minGroupSize"))
                ));
            }
            let (mut groups, mut items, mut q_groups, mut q_items) = (0.0, 0.0, 0.0, 0.0);
            for (size_key, count) in hist {
                let size = crate::measure::jsnum::to_number(size_key);
                let n = {
                    let x = num(Some(count));
                    if x.is_nan() || !crate::measure::sidecar::truthy(Some(count)) {
                        0.0
                    } else {
                        x
                    }
                };
                groups += n;
                items += size * n;
                if size >= min_group {
                    q_groups += n;
                    q_items += size * n;
                }
                if size == 1.0 && size >= min_group {
                    problems.push("corpusPower: a size-1 group may never qualify".to_owned());
                }
            }
            let pairs = [
                ("groupCount", groups, "histogram sums to"),
                ("groupableItems", items, "histogram sums to"),
                ("qualifyingGroups", q_groups, "derived"),
                ("qualifyingItems", q_items, "derived"),
            ];
            for (field, derived, verb) in pairs {
                if derived != num(power.get(field)) {
                    problems.push(format!(
                        "corpusPower.{field}: doc {}, {verb} {}",
                        show(power.get(field)),
                        fmt_number(derived)
                    ));
                }
            }
            let meets = q_groups >= num(power.get("minQualifyingGroups"))
                && q_items >= num(power.get("minQualifyingItems"));
            if power.get("meetsMinimum").and_then(Value::as_bool) != Some(meets) {
                problems.push(format!(
                    "corpusPower.meetsMinimum: doc {}, derived {meets}",
                    show(power.get("meetsMinimum"))
                ));
            }
            if !meets && !no_measurement {
                problems.push(format!(
                    "corpusPower says the batched arm is underpowered, but decision is \"{}\" — an underpowered arm can only carry decision \"no-measurement\"",
                    show(decision)
                ));
            }
            if let Some(by_mode) = power.get("sizeHistogramByMode").and_then(Value::as_object) {
                let mut merged: Vec<(String, f64)> = Vec::new();
                for mode_hist in by_mode.values() {
                    if let Some(h) = mode_hist.as_object() {
                        for (size, n) in h {
                            let add = {
                                let x = num(Some(n));
                                if x.is_nan() || !crate::measure::sidecar::truthy(Some(n)) {
                                    0.0
                                } else {
                                    x
                                }
                            };
                            match merged.iter_mut().find(|(k, _)| k == size) {
                                Some(slot) => slot.1 += add,
                                None => merged.push((size.clone(), add)),
                            }
                        }
                    }
                }
                let mut sizes: Vec<String> = hist.keys().cloned().collect();
                for (k, _) in &merged {
                    if !sizes.contains(k) {
                        sizes.push(k.clone());
                    }
                }
                for size in sizes {
                    let h = hist.get(&size).map_or(0.0, |v| {
                        let x = num(Some(v));
                        if x.is_nan() || !crate::measure::sidecar::truthy(Some(v)) {
                            0.0
                        } else {
                            x
                        }
                    });
                    let m = merged
                        .iter()
                        .find(|(k, _)| *k == size)
                        .map_or(0.0, |(_, v)| *v);
                    if h != m {
                        problems.push(format!(
                            "corpusPower.sizeHistogramByMode does not partition sizeHistogram at size {size}"
                        ));
                    }
                }
            }
        }
    }

    let arms: &[Value] = section
        .get("arms")
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice);
    if !no_measurement && arms.len() < 2 {
        problems.push(
            "a decision other than \"no-measurement\" requires both arms to be recorded".to_owned(),
        );
    }
    for a in arms {
        if !non_empty_string(a.get("arm")) {
            problems.push("an arm row carries no arm label".to_owned());
        }
        let label = show(a.get("arm"));
        for set_name in ["authoritativeOnly", "judgementCallOnly", "all"] {
            let s = a.get(set_name);
            if !is_plain_object(s) {
                problems.push(format!("{label}.{set_name} is missing"));
                continue;
            }
            audit_set_rates(&mut problems, s, &label, set_name);
            let parts = num_or0(get(s, "defectTrials"))
                + num_or0(get(s, "nonDefectTrials"))
                + num_or0(get(s, "ungraded"));
            if parts != num_or0(get(s, "trials")) {
                problems.push(format!(
                    "{label}.{set_name}: defectTrials + nonDefectTrials + ungraded != trials {}",
                    show(get(s, "trials"))
                ));
            }
        }
        audit_partition(&mut problems, a, &label);
        let cost = a.get("cost");
        let summed: f64 = TOKEN_CLASSES
            .iter()
            .map(|c| {
                let n = num(get(cost, c));
                if n.is_nan() || !crate::measure::sidecar::truthy(get(cost, c)) {
                    0.0
                } else {
                    n
                }
            })
            .sum();
        if !matches!(get(cost, "totalTokens"), Some(Value::Number(n)) if n.as_f64() == Some(summed))
        {
            problems.push(format!(
                "{label}.cost.totalTokens: doc {}, four classes sum to {}",
                show(get(cost, "totalTokens")),
                fmt_number(summed)
            ));
        }
        let per_finding = opt_num(mean(summed, num_or0(get(cost, "gradedFindings"))));
        if ne(get(cost, "meanTokensPerGradedFinding"), Some(&per_finding)) {
            problems.push(format!(
                "{label}.cost.meanTokensPerGradedFinding: doc {}, derived {}",
                show(get(cost, "meanTokensPerGradedFinding")),
                show(Some(&per_finding))
            ));
        }
        let per_dispatch = opt_num(mean(summed, num_or0(get(cost, "dispatches"))));
        if ne(get(cost, "meanTokensPerDispatch"), Some(&per_dispatch)) {
            problems.push(format!(
                "{label}.cost.meanTokensPerDispatch: doc {}, derived {}",
                show(get(cost, "meanTokensPerDispatch")),
                show(Some(&per_dispatch))
            ));
        }
        let sc = a.get("selfConsistency");
        let fr = rate_value(
            num_or0(get(sc, "replicateFlips")),
            num_or0(get(sc, "replicatePairs")),
        );
        if ne(get(sc, "flipRate"), Some(&fr)) {
            problems.push(format!(
                "{label}.selfConsistency.flipRate: doc {}, derived {}",
                show(get(sc, "flipRate")),
                show(Some(&fr))
            ));
        }
    }

    if let Some(anchoring) = section.get("anchoring").filter(|a| a.is_object()) {
        #[allow(clippy::cast_precision_loss)]
        if num(anchoring.get("minGroupSize")) < MIN_BATCH_GROUP_SIZE as f64 {
            problems.push(format!(
                "anchoring.minGroupSize {} is below the {MIN_BATCH_GROUP_SIZE} floor",
                show(anchoring.get("minGroupSize"))
            ));
        }
        for a in anchoring
            .get("arms")
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice)
        {
            let derived = rate_value(
                num_or0(a.get("allSameVerdict")),
                num_or0(a.get("dispatchesConsidered")),
            );
            if ne(a.get("allSameVerdictShare"), Some(&derived)) {
                problems.push(format!(
                    "anchoring.{}.allSameVerdictShare: doc {}, derived {}",
                    show(a.get("arm")),
                    show(a.get("allSameVerdictShare")),
                    show(Some(&derived))
                ));
            }
        }
    }
    problems
}

/// Every key path in `value` that names a blended accuracy
/// (`/accuracy|overallCorrect|combinedRate/i`). FN and FP are never averaged.
pub fn find_blended_accuracy_keys(value: &Value, prefix: &str) -> Vec<String> {
    let mut bad = Vec::new();
    match value {
        Value::Array(a) => {
            for (i, v) in a.iter().enumerate() {
                bad.extend(find_blended_accuracy_keys(v, &format!("{prefix}[{i}]")));
            }
        }
        Value::Object(o) => {
            for (k, v) in o {
                let lower = k.to_lowercase();
                if lower.contains("accuracy")
                    || lower.contains("overallcorrect")
                    || lower.contains("combinedrate")
                {
                    bad.push(format!("{prefix}.{k}"));
                }
                bad.extend(find_blended_accuracy_keys(v, &format!("{prefix}.{k}")));
            }
        }
        _ => {}
    }
    bad
}

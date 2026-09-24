//! Trial plans: the per-finding plan, the batch-power analysis under the
//! UNIT-SCOPED key, the batched plan, and expansion of batched results into
//! per-finding scoring rows.
//!
//! A batched refuter dispatch is exactly "one review unit's gating findings
//! for one dimension", because `buildReviewPipeline` runs once per review
//! unit. The grouping key is therefore `runId|unitIdent|mode|dim`: the coarser
//! `(runId, mode, dim)` merges findings from different units in one run, a
//! shape production can never produce, and inflates the apparent batch size.

use std::collections::BTreeMap;

use serde::Serialize;

use super::corpus::HISTORICAL_ONLY_SEVERITIES;
use crate::measure::jsjson::{JsMap, JsValue, js_str_cmp, obj};
use crate::measure::jsnum::{is_integer, js_trim};
use crate::measure::refuter_severity::extract::unit_ident;

/// Minimum group size at which a batched dispatch can exhibit anchoring.
pub const MIN_BATCH_GROUP_SIZE: usize = 3;
/// Pre-registered floor: qualifying groups.
pub const MIN_QUALIFYING_BATCH_GROUPS: usize = 6;
/// Pre-registered floor: qualifying items.
pub const MIN_QUALIFYING_BATCH_ITEMS: usize = 18;
/// Agent-index gap above which one key's members are separate dispatches (a
/// rework re-review).
pub const ROUND_SPLIT_GAP: f64 = 24.0;

/// One per-finding trial.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trial {
    /// `<corpusId>|<tier>|<replicate>`.
    pub trial_id: String,
    /// The corpus item.
    pub corpus_id: String,
    /// The tier.
    pub tier: String,
    /// 1-based replicate.
    pub replicate: usize,
}

/// The flat, deterministic per-finding plan: corpus order, then tier order,
/// then replicate order.
///
/// # Errors
///
/// With no tier or a zero replicate count.
pub fn build_trials(
    items: &[&JsValue],
    tiers: &[String],
    replicates: usize,
) -> Result<Vec<Trial>, String> {
    if tiers.is_empty() {
        return Err("buildTrials requires at least one tier".to_owned());
    }
    if replicates < 1 {
        return Err(format!(
            "replicates must be a positive integer, got {replicates}"
        ));
    }
    let mut out = Vec::new();
    for item in items {
        let id = item.get("id").map(JsValue::js_string).unwrap_or_default();
        for tier in tiers {
            for r in 1..=replicates {
                out.push(Trial {
                    trial_id: format!("{id}|{tier}|{r}"),
                    corpus_id: id.clone(),
                    tier: tier.clone(),
                    replicate: r,
                });
            }
        }
    }
    Ok(out)
}

fn non_empty_str(v: Option<&JsValue>) -> Option<&str> {
    v.and_then(JsValue::as_str)
        .filter(|s| !js_trim(s).is_empty())
}

/// An item's review-unit identity: the target's first line under the shared
/// unit-identity rule (a JSON-shaped or implausibly long first line is none).
pub fn unit_ident_of(item: &JsValue) -> Option<String> {
    if !item.is_object() {
        return None;
    }
    let target = item.get("target").and_then(JsValue::as_str).unwrap_or("");
    let first = target.split('\n').next().unwrap_or("");
    unit_ident(first)
}

/// The four-part key `runId|unitIdent|mode|dim.key`, or `None` when the item
/// cannot belong to a real dispatch.
pub fn batch_group_key_for(item: &JsValue) -> Option<String> {
    if !item.is_object() {
        return None;
    }
    let run_id = non_empty_str(item.get("provenance").and_then(|p| p.get("runId")))?;
    let unit = unit_ident_of(item)?;
    let mode = non_empty_str(item.get("mode"))?;
    let dim = non_empty_str(item.get("dim").and_then(|d| d.get("key")))?;
    Some(format!("{run_id}|{unit}|{mode}|{dim}"))
}

/// One batch group.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    /// The key (with `#r<n>` when a rework round was split off).
    pub key: String,
    /// The run id.
    pub run_id: String,
    /// The unit identity.
    pub unit_ident: String,
    /// The mode.
    pub mode: String,
    /// The dimension.
    pub dim: String,
    /// Member ids, in stable order.
    pub ids: Vec<String>,
    /// Members.
    pub size: usize,
    /// Every member carried `provenance.agentIndex`.
    pub agent_indexed: bool,
}

/// The batch-power analysis.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BatchPower {
    /// The qualifying floor.
    pub min_group_size: usize,
    /// Pre-registered group floor.
    pub min_qualifying_groups: usize,
    /// Pre-registered item floor.
    pub min_qualifying_items: usize,
    /// Items considered.
    pub total_items: usize,
    /// `constructed` items (no run, no unit).
    pub constructed_excluded: usize,
    /// Non-gating (historical-only) items.
    pub non_gating_excluded: usize,
    /// Items with no recoverable unit identity.
    pub unrecoverable_unit_excluded: usize,
    /// Items placed in a group.
    pub groupable_items: usize,
    /// The groups.
    pub groups: Vec<Group>,
    /// How many.
    pub group_count: usize,
    /// size → groups.
    pub size_histogram: BTreeMap<usize, usize>,
    /// mode → size → groups.
    pub size_histogram_by_mode: JsMap<BTreeMap<usize, usize>>,
    /// Items in size-1 groups.
    pub singleton_items: usize,
    /// Groups at or above the floor.
    pub qualifying_groups: usize,
    /// Items in those groups.
    pub qualifying_items: usize,
    /// Rework rounds split off.
    pub round_splits: usize,
    /// Groups whose members all carried an agent index.
    pub agent_indexed_groups: usize,
    /// Both pre-registered floors cleared.
    pub meets_minimum: bool,
}

fn agent_index(item: &JsValue) -> Option<f64> {
    item.get("provenance")
        .and_then(|p| p.get("agentIndex"))
        .and_then(JsValue::as_f64)
        .filter(|x| is_integer(*x))
}

fn split_by_round<'a>(members: &[&'a JsValue], gap: f64) -> Option<Vec<Vec<&'a JsValue>>> {
    if !members.iter().all(|m| agent_index(m).is_some()) {
        return None;
    }
    let mut sorted = members.to_vec();
    sorted.sort_by(|a, b| {
        agent_index(a)
            .partial_cmp(&agent_index(b))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut rounds: Vec<Vec<&JsValue>> = vec![vec![sorted[0]]];
    for w in sorted.windows(2) {
        if agent_index(w[1]).unwrap_or(0.0) - agent_index(w[0]).unwrap_or(0.0) > gap {
            rounds.push(Vec::new());
        }
        if let Some(last) = rounds.last_mut() {
            last.push(w[1]);
        }
    }
    Some(rounds)
}

/// Groups items (a corpus, or mined candidates — `groundTruth` is never read)
/// into the batches a real dispatch could form, after three counted
/// exclusions: constructed items, historical-only severities, and items with
/// no recoverable unit identity.
///
/// # Errors
///
/// When `min_group_size` is below 2.
pub fn group_corpus_for_batching(
    items: &[&JsValue],
    min_group_size: usize,
) -> Result<BatchPower, String> {
    if min_group_size < 2 {
        return Err(format!(
            "minGroupSize must be an integer >= 2, got {min_group_size}"
        ));
    }
    let (mut constructed, mut non_gating, mut unrecoverable) = (0, 0, 0);
    let mut buckets: Vec<(String, Vec<&JsValue>)> = Vec::new();
    for item in items {
        let kind = item
            .get("provenance")
            .and_then(|p| p.get("kind"))
            .and_then(JsValue::as_str);
        if kind == Some("constructed") {
            constructed += 1;
            continue;
        }
        let severity = item
            .get("finding")
            .and_then(|f| f.get("severity"))
            .and_then(JsValue::as_str);
        if severity.is_some_and(|s| HISTORICAL_ONLY_SEVERITIES.contains(&s)) {
            non_gating += 1;
            continue;
        }
        let Some(key) = batch_group_key_for(item) else {
            unrecoverable += 1;
            continue;
        };
        match buckets.iter_mut().find(|(k, _)| *k == key) {
            Some(slot) => slot.1.push(item),
            None => buckets.push((key, vec![item])),
        }
    }
    buckets.sort_by(|a, b| js_str_cmp(&a.0, &b.0));
    let mut groups = Vec::new();
    let (mut round_splits, mut agent_indexed_groups) = (0, 0);
    for (key, members) in &buckets {
        let parts: Vec<&str> = key.split('|').collect();
        let run_id = parts[0].to_owned();
        let dim = parts[parts.len() - 1].to_owned();
        let mode = parts[parts.len() - 2].to_owned();
        let unit = parts[1..parts.len() - 2].join("|");
        let rounds = split_by_round(members, ROUND_SPLIT_GAP);
        if let Some(r) = &rounds {
            agent_indexed_groups += 1;
            if r.len() > 1 {
                round_splits += r.len() - 1;
            }
        }
        let emit = rounds.clone().unwrap_or_else(|| vec![members.clone()]);
        let many = emit.len() > 1;
        for (i, round) in emit.iter().enumerate() {
            groups.push(Group {
                key: if many {
                    format!("{key}#r{}", i + 1)
                } else {
                    key.clone()
                },
                run_id: run_id.clone(),
                unit_ident: unit.clone(),
                mode: mode.clone(),
                dim: dim.clone(),
                ids: round
                    .iter()
                    .map(|m| m.get("id").map(JsValue::js_string).unwrap_or_default())
                    .collect(),
                size: round.len(),
                agent_indexed: rounds.is_some(),
            });
        }
    }
    let mut size_histogram = BTreeMap::new();
    let mut by_mode: JsMap<BTreeMap<usize, usize>> = JsMap::new();
    let (mut singleton, mut qualifying_groups, mut qualifying_items) = (0, 0, 0);
    for g in &groups {
        *size_histogram.entry(g.size).or_insert(0) += 1;
        *by_mode
            .entry(&g.mode, BTreeMap::new)
            .entry(g.size)
            .or_insert(0) += 1;
        if g.size == 1 {
            singleton += 1;
        }
        if g.size >= min_group_size {
            qualifying_groups += 1;
            qualifying_items += g.size;
        }
    }
    Ok(BatchPower {
        min_group_size,
        min_qualifying_groups: MIN_QUALIFYING_BATCH_GROUPS,
        min_qualifying_items: MIN_QUALIFYING_BATCH_ITEMS,
        total_items: items.len(),
        constructed_excluded: constructed,
        non_gating_excluded: non_gating,
        unrecoverable_unit_excluded: unrecoverable,
        groupable_items: groups.iter().map(|g| g.size).sum(),
        group_count: groups.len(),
        groups,
        size_histogram,
        size_histogram_by_mode: by_mode,
        singleton_items: singleton,
        qualifying_groups,
        qualifying_items,
        round_splits,
        agent_indexed_groups,
        meets_minimum: qualifying_groups >= MIN_QUALIFYING_BATCH_GROUPS
            && qualifying_items >= MIN_QUALIFYING_BATCH_ITEMS,
    })
}

fn histogram_line(h: &BTreeMap<usize, usize>) -> String {
    if h.is_empty() {
        return "(none)".to_owned();
    }
    h.iter()
        .map(|(s, n)| format!("{s}:{n}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Renders the batch-power analysis, ending in `POWER: SUFFICIENT|INSUFFICIENT`.
pub fn format_batch_power(s: &BatchPower) -> String {
    let mut out = vec![
        "Batch-size distribution the corpus can actually form (UNIT-SCOPED key: runId|unitIdent|mode|dim)".to_owned(),
        String::new(),
        format!("  corpus items                 {}", s.total_items),
        format!("- constructed (no run/unit)    {}", s.constructed_excluded),
        format!(
            "- non-gating ({})       {}",
            HISTORICAL_ONLY_SEVERITIES.join(", "),
            s.non_gating_excluded
        ),
        format!("- unrecoverable unit identity  {}", s.unrecoverable_unit_excluded),
        format!(
            "= groupable items              {}  in {} group(s)",
            s.groupable_items, s.group_count
        ),
        String::new(),
        format!("Size histogram (size:groups)   {}", histogram_line(&s.size_histogram)),
    ];
    for mode in super::corpus::MODES {
        out.push(format!(
            "  {mode:<4}                        {}",
            s.size_histogram_by_mode
                .get(mode)
                .map_or_else(|| "(none)".to_owned(), histogram_line)
        ));
    }
    out.push(String::new());
    out.push(format!(
        "Minimum group size for the anchoring measurement: {}",
        s.min_group_size
    ));
    out.push(format!(
        "Size-1 groups are EXCLUDED from it ({} item(s) in singleton groups).",
        s.singleton_items
    ));
    out.push(format!(
        "Qualifying population: {} group(s) / {} item(s) against floors of {} group(s) / {} item(s).",
        s.qualifying_groups, s.qualifying_items, s.min_qualifying_groups, s.min_qualifying_items
    ));
    if s.agent_indexed_groups == 0 {
        out.push("No group carries provenance.agentIndex, so every size below is an UPPER BOUND: a REWORK re-review is a second dispatch this key cannot yet split apart.".to_owned());
    } else {
        out.push(format!(
            "Round splitting applied to {} group(s); {} split(s) made.",
            s.agent_indexed_groups, s.round_splits
        ));
    }
    out.push(String::new());
    out.push(format!(
        "POWER: {}",
        if s.meets_minimum {
            "SUFFICIENT"
        } else {
            "INSUFFICIENT"
        }
    ));
    if !s.meets_minimum {
        out.push("A batched arm built from this population is byte-for-byte a per-finding arm across most of its items, so the anchoring effect would be unobservable. This is a NO-MEASUREMENT outcome, not a passing gate.".to_owned());
    }
    out.join("\n")
}

/// One batched trial (one dispatch).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchTrial {
    /// `<groupKey>|<tier>|<replicate>`.
    pub trial_id: String,
    /// The dispatch id (the trial id).
    pub dispatch_id: String,
    /// The group.
    pub group_key: String,
    /// The group's mode.
    pub mode: String,
    /// The group's dimension.
    pub dim: String,
    /// The tier.
    pub tier: String,
    /// 1-based replicate.
    pub replicate: usize,
    /// The members graded by this dispatch.
    pub corpus_ids: Vec<String>,
}

/// The batched plan.
#[derive(Debug, Clone, PartialEq)]
pub struct BatchPlan {
    /// One trial per qualifying group × tier × replicate.
    pub trials: Vec<BatchTrial>,
    /// The qualifying groups.
    pub groups: Vec<Group>,
    /// Every covered id, deduped in order.
    pub corpus_ids: Vec<String>,
    /// The population was below the pre-registered floor.
    pub underpowered: bool,
    /// Stamped when underpowered: forces the NO MEASUREMENT banner and
    /// suppresses any decision.
    pub no_measurement: bool,
}

/// Builds the batched plan from qualifying groups only. Refuses an
/// underpowered population unless `allow_underpowered`, which stamps it
/// `no_measurement`.
///
/// # Errors
///
/// On an underpowered population without `allow_underpowered`, no tier, or a
/// zero replicate count.
pub fn build_batch_trials(
    power: &BatchPower,
    tiers: &[String],
    replicates: usize,
    allow_underpowered: bool,
) -> Result<BatchPlan, String> {
    if tiers.is_empty() {
        return Err("buildBatchTrials requires at least one tier".to_owned());
    }
    if replicates < 1 {
        return Err(format!(
            "replicates must be a positive integer, got {replicates}"
        ));
    }
    if !power.meets_minimum && !allow_underpowered {
        return Err(format!(
            "buildBatchTrials refuses to build an UNDERPOWERED batched arm: {} qualifying group(s) / {} item(s) at minGroupSize {}, against floors of {} / {}. A batched arm dominated by size-1 batches is not evidence and may not be reported as a passing gate — mine more adjudicated findings, or report a no-measurement outcome. Pass --allow-underpowered to build it anyway; the result is stamped noMeasurement and can never carry a decision.",
            power.qualifying_groups,
            power.qualifying_items,
            power.min_group_size,
            power.min_qualifying_groups,
            power.min_qualifying_items
        ));
    }
    let groups: Vec<Group> = power
        .groups
        .iter()
        .filter(|g| g.size >= power.min_group_size)
        .cloned()
        .collect();
    let mut trials = Vec::new();
    for g in &groups {
        for tier in tiers {
            for r in 1..=replicates {
                let id = format!("{}|{tier}|{r}", g.key);
                trials.push(BatchTrial {
                    trial_id: id.clone(),
                    dispatch_id: id,
                    group_key: g.key.clone(),
                    mode: g.mode.clone(),
                    dim: g.dim.clone(),
                    tier: tier.clone(),
                    replicate: r,
                    corpus_ids: g.ids.clone(),
                });
            }
        }
    }
    let mut corpus_ids: Vec<String> = Vec::new();
    for id in groups.iter().flat_map(|g| g.ids.iter()) {
        if !corpus_ids.contains(id) {
            corpus_ids.push(id.clone());
        }
    }
    Ok(BatchPlan {
        trials,
        groups,
        corpus_ids,
        underpowered: !power.meets_minimum,
        no_measurement: !power.meets_minimum,
    })
}

/// Expanded batched rows plus the ids the resilience rules set aside.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Expanded {
    /// One scoring row per (dispatch, member).
    pub rows: Vec<JsValue>,
    /// `<dispatchId>:<id>` of verdicts for ids the dispatch did not contain.
    pub unknown_verdict_ids: Vec<String>,
    /// `<dispatchId>:<id>` of members the response did not grade.
    pub omitted_ids: Vec<String>,
}

/// A verdict object (`refuted`/`confidence`/`rationale`) normalized as the
/// JS tools recorded it.
pub fn verdict_value(v: &JsValue) -> JsValue {
    obj([
        (
            "refuted",
            v.get("refuted").and_then(JsValue::as_bool).into(),
        ),
        (
            "confidence",
            v.get("confidence").and_then(JsValue::as_f64).into(),
        ),
        (
            "rationale",
            v.get("rationale")
                .and_then(JsValue::as_str)
                .map(str::to_owned)
                .into(),
        ),
    ])
}

/// Flattens completed batched dispatches (`batchDispatches` rows) into
/// per-finding scoring rows: unknown ids are dropped (recorded), omitted ids
/// stay ungraded (never coerced to `refuted: false`), a crashed dispatch
/// leaves every member ungraded, and the dispatch's whole cost lands on its
/// FIRST row.
pub fn expand_batch_results(results: &[JsValue]) -> Expanded {
    let mut out = Expanded::default();
    for res in results {
        let corpus_ids: Vec<JsValue> = res
            .get("corpusIds")
            .and_then(JsValue::as_array)
            .cloned()
            .unwrap_or_default();
        let expected: Vec<String> = corpus_ids.iter().map(JsValue::js_string).collect();
        let dispatch_id = res
            .get("dispatchId")
            .map(JsValue::js_string)
            .unwrap_or_else(|| "undefined".to_owned());
        let mut by_id: Vec<(String, JsValue)> = Vec::new();
        if let Some(verdicts) = res.get("verdicts").and_then(JsValue::as_array) {
            for v in verdicts.iter().filter(|v| v.is_object()) {
                let id = v
                    .get("id")
                    .map_or_else(|| "undefined".to_owned(), JsValue::js_string);
                if !expected.contains(&id) {
                    out.unknown_verdict_ids.push(format!("{dispatch_id}:{id}"));
                    continue;
                }
                let value = verdict_value(v);
                match by_id.iter_mut().find(|(k, _)| *k == id) {
                    Some(slot) => slot.1 = value,
                    None => by_id.push((id, value)),
                }
            }
        }
        for (index, corpus_id) in corpus_ids.iter().enumerate() {
            let key = corpus_id.js_string();
            let verdict = by_id
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.clone());
            if verdict.is_none() {
                out.omitted_ids.push(format!("{dispatch_id}:{key}"));
            }
            let graded = verdict.filter(|v| v.get("refuted").and_then(JsValue::as_bool).is_some());
            let first = index == 0;
            out.rows.push(obj([
                ("trialId", format!("{dispatch_id}|{key}").into()),
                ("corpusId", corpus_id.clone()),
                ("tier", res.get("tier").cloned().unwrap_or(JsValue::Null)),
                (
                    "replicate",
                    res.get("replicate").cloned().unwrap_or(JsValue::Null),
                ),
                ("arm", "batched".into()),
                (
                    "dispatchId",
                    res.get("dispatchId").cloned().unwrap_or(JsValue::Null),
                ),
                (
                    "groupKey",
                    res.get("groupKey").cloned().unwrap_or(JsValue::Null),
                ),
                ("dispatchSize", corpus_ids.len().into()),
                ("positionInBatch", (index + 1).into()),
                ("verdict", graded.unwrap_or(JsValue::Null)),
                (
                    "error",
                    res.get("error")
                        .filter(|e| e.truthy())
                        .cloned()
                        .unwrap_or(JsValue::Null),
                ),
                (
                    "usage",
                    if first {
                        res.get("usage")
                            .filter(|u| u.truthy())
                            .cloned()
                            .unwrap_or_else(JsValue::empty_object)
                    } else {
                        JsValue::empty_object()
                    },
                ),
                (
                    "toolCalls",
                    if first {
                        res.get("toolCalls")
                            .filter(|t| t.truthy())
                            .cloned()
                            .unwrap_or(JsValue::Number(0.0))
                    } else {
                        JsValue::Number(0.0)
                    },
                ),
            ]));
        }
    }
    out
}

//! Scoring: false negatives and false positives over structurally different
//! denominators — never blended — with per-class and authority splits,
//! replicate self-consistency, token cost, and the batching anchoring signal.
//!
//! - FALSE NEGATIVE: `groundTruth.defect === true` and `refuted === true` (a
//!   real defect refuted: ships a bug). Denominator: defect-truth trials.
//! - FALSE POSITIVE: `groundTruth.defect === false` and `refuted === false` (a
//!   non-defect kept: costs a rework round). Denominator: non-defect trials.
//!
//! A trial with no boolean `refuted` is UNGRADED, in neither rate. Coercing it
//! to `false` would silently inflate the false-positive rate.

use std::collections::BTreeMap;

use serde::Serialize;

use super::corpus::{
    CorpusItem, CorpusSummary, DIVERGENCE_CLASS, HISTORICAL_ONLY_SEVERITIES, TOKEN_CLASSES,
    summarize_corpus,
};
use super::trials::{BatchPower, Group, MIN_BATCH_GROUP_SIZE};
use crate::measure::jsjson::{JsValue, js_str_cmp};
use crate::measure::jsnum::{js_round, serialize_js_number, serialize_opt_js_number, to_number};

/// `whole ? round(part/whole × 1000)/10 : null`.
pub fn rate(part: f64, whole: f64) -> Option<f64> {
    (whole != 0.0 && !whole.is_nan()).then(|| js_round((part / whole) * 1000.0) / 10.0)
}

/// `n ? round(total/n × 10)/10 : null`.
pub fn mean(total: f64, n: f64) -> Option<f64> {
    (n != 0.0 && !n.is_nan()).then(|| js_round((total / n) * 10.0) / 10.0)
}

/// One FN/FP rate set.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RateSet {
    /// Trials tallied.
    pub trials: usize,
    /// Trials with no boolean verdict.
    pub ungraded: usize,
    /// Graded trials on defect-truth items.
    pub defect_trials: usize,
    /// Graded trials on non-defect items.
    pub non_defect_trials: usize,
    /// Real defects refuted.
    pub false_negatives: usize,
    /// `false_negatives / defect_trials`.
    #[serde(serialize_with = "serialize_opt_js_number")]
    pub false_negative_rate: Option<f64>,
    /// Non-defects refuted.
    pub correct_refutations: usize,
    /// Non-defects kept.
    pub false_positives: usize,
    /// `false_positives / non_defect_trials`.
    #[serde(serialize_with = "serialize_opt_js_number")]
    pub false_positive_rate: Option<f64>,
    /// Real defects kept.
    pub correct_keeps: usize,
}

impl RateSet {
    fn tally(&mut self, defect: bool, refuted: Option<bool>) {
        self.trials += 1;
        let Some(refuted) = refuted else {
            self.ungraded += 1;
            return;
        };
        if defect {
            self.defect_trials += 1;
            if refuted {
                self.false_negatives += 1;
            } else {
                self.correct_keeps += 1;
            }
        } else {
            self.non_defect_trials += 1;
            if refuted {
                self.correct_refutations += 1;
            } else {
                self.false_positives += 1;
            }
        }
    }

    #[allow(clippy::cast_precision_loss)]
    fn finalize(&mut self) {
        self.false_negative_rate = rate(self.false_negatives as f64, self.defect_trials as f64);
        self.false_positive_rate = rate(self.false_positives as f64, self.non_defect_trials as f64);
    }
}

/// One per-class row.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ClassRow {
    /// The ground-truth class.
    pub class: String,
    /// Its rates.
    #[serde(flatten)]
    pub rates: RateSet,
}

/// Replicate self-consistency.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SelfConsistency {
    /// (item, bucket) pairs with ≥ 2 graded replicates.
    pub replicate_pairs: usize,
    /// Of those, pairs whose replicates disagreed.
    pub replicate_flips: usize,
    /// `flips / pairs`.
    #[serde(serialize_with = "serialize_opt_js_number")]
    pub flip_rate: Option<f64>,
}

/// Token volume (not price) and tool calls.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Cost {
    /// Every dispatched trial (historical-only and ungraded included).
    pub dispatched_trials: usize,
    /// Unique dispatches (a batched dispatch counts once).
    pub dispatches: usize,
    /// Trials on gating severities.
    pub graded_findings: usize,
    /// Output tokens.
    #[serde(serialize_with = "serialize_js_number")]
    pub output: f64,
    /// Uncached input tokens.
    #[serde(serialize_with = "serialize_js_number")]
    pub uncached_input: f64,
    /// Cache-write tokens.
    #[serde(serialize_with = "serialize_js_number")]
    pub cache_write: f64,
    /// Cache-read tokens.
    #[serde(serialize_with = "serialize_js_number")]
    pub cache_read: f64,
    /// All four classes.
    #[serde(serialize_with = "serialize_js_number")]
    pub total_tokens: f64,
    /// Per dispatched trial.
    #[serde(serialize_with = "serialize_opt_js_number")]
    pub mean_tokens_per_trial: Option<f64>,
    /// Per dispatch.
    #[serde(serialize_with = "serialize_opt_js_number")]
    pub mean_tokens_per_dispatch: Option<f64>,
    /// Per graded finding.
    #[serde(serialize_with = "serialize_opt_js_number")]
    pub mean_tokens_per_graded_finding: Option<f64>,
    /// Tool calls.
    #[serde(serialize_with = "serialize_js_number")]
    pub tool_calls: f64,
    /// Per dispatched trial.
    #[serde(serialize_with = "serialize_opt_js_number")]
    pub mean_tool_calls_per_trial: Option<f64>,
}

/// A bucket's token delta against the baseline tier.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TokenDelta {
    /// The baseline.
    pub baseline_tier: String,
    /// This bucket's mean tokens per trial.
    #[serde(serialize_with = "serialize_js_number")]
    pub mean_tokens_per_trial: f64,
    /// The baseline's.
    #[serde(serialize_with = "serialize_js_number")]
    pub baseline_mean_tokens_per_trial: f64,
    /// The difference, one decimal.
    #[serde(serialize_with = "serialize_js_number")]
    pub delta: f64,
    /// As a percentage of the baseline.
    #[serde(serialize_with = "serialize_opt_js_number")]
    pub percent: Option<f64>,
}

/// One report bucket: a tier, or `tier|arm`.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TierScore {
    /// `tier` or `tier|arm`.
    pub bucket: String,
    /// The tier, as recorded.
    pub tier: JsValue,
    /// The arm, when bucketed by one.
    pub arm: Option<String>,
    /// The DECISION-GRADE figure.
    pub authoritative_only: RateSet,
    /// Judgement-call items only.
    pub judgement_call_only: RateSet,
    /// Every graded item.
    pub all: RateSet,
    /// Per class, sorted by class.
    pub by_class: Vec<ClassRow>,
    /// Replicate agreement.
    pub self_consistency: SelfConsistency,
    /// Trials on historical-only severities.
    pub historical_only_trials: usize,
    /// Cost.
    pub cost: Cost,
    /// Against the baseline (`null` for the baseline itself).
    pub token_delta: Option<TokenDelta>,
}

/// The score report.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ScoreReport {
    /// Corpus composition.
    pub corpus: CorpusSummary,
    /// The baseline bucket/tier.
    pub baseline_tier: Option<String>,
    /// Per bucket, in first-appearance order.
    pub tiers: Vec<TierScore>,
    /// Trials on historical-only severities.
    pub historical_only_trials: usize,
    /// Trial corpus ids not in the corpus.
    pub unknown_corpus_ids: Vec<String>,
    /// The standing caveats.
    pub caveats: Vec<String>,
    /// Set when the batched arm was underpowered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub no_measurement: Option<bool>,
    /// The anchoring block, for a batched run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchoring: Option<Anchoring>,
    /// A decision line (never with `no_measurement`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision: Option<String>,
    /// The batch-power analysis, for a batched run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub batch_power: Option<BatchPower>,
}

/// The standing caveats (rendered verbatim, carried in the JSON too).
pub fn caveats() -> Vec<String> {
    vec![
        format!("The corpus is DELIBERATELY WEIGHTED toward {DIVERGENCE_CLASS} (the class where the tiers diverged). The aggregate is therefore NOT a population estimate of production finding mix — quote the per-class rates."),
        "False negatives and false positives have asymmetric cost and structurally different denominators. They are reported separately and are never averaged into an accuracy number.".to_owned(),
        "Token figures here are VOLUME, not price. Re-tiering changes price-per-token, not token volume (the initial A/B measured Sonnet at 52.9k vs Opus 48.4k per refuter). Any cost conclusion is a price-per-token argument and must be labelled as such.".to_owned(),
        "Historical `suggestion`-severity items are recorded but excluded from every headline rate: phase 6 landed NON_GATING_SEVERITIES = [suggestion], so no refuter is spawned for one again.".to_owned(),
        "Ground truth is adjudicated against the PINNED TREE recorded in each item's groundTruth.adjudicatedAgainstCommit — the tree a replay run actually reads — not against the tree the historical refuter read. A finding that was real then and is fixed now is `stale-fact`/defect:false, because that is the correct answer for a refuter reading this tree.".to_owned(),
    ]
}

/// A trial row's report bucket: the bare tier, or `tier|arm`.
pub fn bucket_key_for(t: &JsValue) -> String {
    let tier = t
        .get("tier")
        .map_or_else(|| "undefined".to_owned(), JsValue::js_string);
    match t.get("arm").filter(|a| a.truthy()) {
        Some(arm) => format!("{tier}|{}", arm.js_string()),
        None => tier,
    }
}

/// A trial row's boolean verdict, if graded.
pub fn refuted_of(t: &JsValue) -> Option<bool> {
    t.get("verdict")
        .and_then(|v| v.get("refuted"))
        .and_then(JsValue::as_bool)
}

fn js_number_or0(v: Option<&JsValue>) -> f64 {
    match v {
        Some(x) if x.truthy() => match x {
            JsValue::Number(n) => *n,
            JsValue::String(s) => to_number(s),
            JsValue::Bool(true) => 1.0,
            _ => f64::NAN,
        },
        _ => 0.0,
    }
}

struct Bucket {
    bucket: String,
    tier: JsValue,
    arm: Option<String>,
    authoritative: RateSet,
    judgement: RateSet,
    all: RateSet,
    by_class: Vec<(String, RateSet)>,
    usage: [f64; 4],
    tool_calls: f64,
    trials: usize,
    graded_findings: usize,
    dispatch_ids: Vec<String>,
    pairs: usize,
    flips: usize,
    historical_only: usize,
}

/// Options for [`score_trials`].
#[derive(Debug, Clone, Default)]
pub struct ScoreOptions {
    /// The baseline tier (default: the first bucket).
    pub baseline_tier: Option<String>,
    /// Stamp the report NO MEASUREMENT.
    pub no_measurement: bool,
    /// The anchoring block.
    pub anchoring: Option<Anchoring>,
    /// A decision line.
    pub decision: Option<String>,
    /// The batch-power analysis.
    pub batch_power: Option<BatchPower>,
}

/// Scores `trials` against `corpus`.
pub fn score_trials(corpus: &[CorpusItem], trials: &[JsValue], opts: ScoreOptions) -> ScoreReport {
    let mut unknown: Vec<String> = Vec::new();
    let mut buckets: Vec<Bucket> = Vec::new();
    let mut historical_only = 0;
    let mut replicate_verdicts: Vec<(String, String, Vec<Option<bool>>)> = Vec::new();

    for t in trials {
        let corpus_id = t
            .get("corpusId")
            .map_or_else(|| "undefined".to_owned(), JsValue::js_string);
        let Some(item) = corpus.iter().find(|i| i.id() == corpus_id) else {
            unknown.push(corpus_id);
            continue;
        };
        let key = bucket_key_for(t);
        let at = match buckets.iter().position(|b| b.bucket == key) {
            Some(at) => at,
            None => {
                buckets.push(Bucket {
                    bucket: key.clone(),
                    tier: t.get("tier").cloned().unwrap_or(JsValue::Null),
                    arm: t.get("arm").filter(|a| a.truthy()).map(JsValue::js_string),
                    authoritative: RateSet::default(),
                    judgement: RateSet::default(),
                    all: RateSet::default(),
                    by_class: Vec::new(),
                    usage: [0.0; 4],
                    tool_calls: 0.0,
                    trials: 0,
                    graded_findings: 0,
                    dispatch_ids: Vec::new(),
                    pairs: 0,
                    flips: 0,
                    historical_only: 0,
                });
                buckets.len() - 1
            }
        };
        let b = &mut buckets[at];
        let usage = t.get("usage").filter(|u| u.truthy());
        for (i, c) in TOKEN_CLASSES.iter().enumerate() {
            b.usage[i] += js_number_or0(usage.and_then(|u| u.get(c)));
        }
        b.tool_calls += js_number_or0(t.get("toolCalls"));
        b.trials += 1;
        let dispatch = match t.get("dispatchId") {
            None | Some(JsValue::Null) => t.get("trialId"),
            some => some,
        };
        let dispatch_key = dispatch.map_or_else(|| "undefined".to_owned(), JsValue::stringify);
        if !b.dispatch_ids.contains(&dispatch_key) {
            b.dispatch_ids.push(dispatch_key);
        }
        if HISTORICAL_ONLY_SEVERITIES.contains(&item.severity()) {
            b.historical_only += 1;
            historical_only += 1;
            continue;
        }
        b.graded_findings += 1;
        let refuted = refuted_of(t);
        let defect = item.defect();
        b.all.tally(defect, refuted);
        if item.authority() == "authoritative" {
            b.authoritative.tally(defect, refuted);
        } else {
            b.judgement.tally(defect, refuted);
        }
        let class = item.class().to_owned();
        match b.by_class.iter_mut().find(|(c, _)| *c == class) {
            Some(slot) => slot.1.tally(defect, refuted),
            None => {
                let mut set = RateSet::default();
                set.tally(defect, refuted);
                b.by_class.push((class, set));
            }
        }
        let rkey = format!("{corpus_id} {}", b.bucket);
        match replicate_verdicts.iter_mut().find(|(k, _, _)| *k == rkey) {
            Some(slot) => slot.2.push(refuted),
            None => replicate_verdicts.push((rkey, b.bucket.clone(), vec![refuted])),
        }
    }

    for (_, bucket, list) in &replicate_verdicts {
        let graded: Vec<bool> = list.iter().filter_map(|v| *v).collect();
        if graded.len() < 2 {
            continue;
        }
        if let Some(b) = buckets.iter_mut().find(|b| b.bucket == *bucket) {
            b.pairs += 1;
            if !graded.iter().all(|v| *v == graded[0]) {
                b.flips += 1;
            }
        }
    }

    #[allow(clippy::cast_precision_loss)]
    let mut tiers: Vec<TierScore> = buckets
        .into_iter()
        .map(|mut b| {
            b.authoritative.finalize();
            b.judgement.finalize();
            b.all.finalize();
            let mut by_class = b.by_class;
            by_class.sort_by(|x, y| js_str_cmp(&x.0, &y.0));
            let total: f64 = b.usage.iter().sum();
            let dispatches = b.dispatch_ids.len();
            TierScore {
                bucket: b.bucket,
                tier: b.tier,
                arm: b.arm,
                authoritative_only: b.authoritative,
                judgement_call_only: b.judgement,
                all: b.all,
                by_class: by_class
                    .into_iter()
                    .map(|(class, mut rates)| {
                        rates.finalize();
                        ClassRow { class, rates }
                    })
                    .collect(),
                self_consistency: SelfConsistency {
                    replicate_pairs: b.pairs,
                    replicate_flips: b.flips,
                    flip_rate: rate(b.flips as f64, b.pairs as f64),
                },
                historical_only_trials: b.historical_only,
                cost: Cost {
                    dispatched_trials: b.trials,
                    dispatches,
                    graded_findings: b.graded_findings,
                    output: b.usage[0],
                    uncached_input: b.usage[1],
                    cache_write: b.usage[2],
                    cache_read: b.usage[3],
                    total_tokens: total,
                    mean_tokens_per_trial: mean(total, b.trials as f64),
                    mean_tokens_per_dispatch: mean(total, dispatches as f64),
                    mean_tokens_per_graded_finding: mean(total, b.graded_findings as f64),
                    tool_calls: b.tool_calls,
                    mean_tool_calls_per_trial: mean(b.tool_calls, b.trials as f64),
                },
                token_delta: None,
            }
        })
        .collect();

    let baseline_tier = opts
        .baseline_tier
        .clone()
        .filter(|s| !s.is_empty())
        .or_else(|| tiers.first().map(|t| t.bucket.clone()));
    let baseline = baseline_tier.as_deref().and_then(|bt| {
        tiers
            .iter()
            .find(|t| t.bucket == bt)
            .or_else(|| tiers.iter().find(|t| t.tier.as_str() == Some(bt)))
            .map(|t| (t.bucket.clone(), t.cost.mean_tokens_per_trial))
    });
    for t in &mut tiers {
        let Some((base_bucket, Some(base_mean))) = &baseline else {
            continue;
        };
        if t.bucket == *base_bucket {
            continue;
        }
        let Some(mine) = t.cost.mean_tokens_per_trial else {
            continue;
        };
        let delta = mine - base_mean;
        t.token_delta = Some(TokenDelta {
            baseline_tier: baseline_tier.clone().unwrap_or_default(),
            mean_tokens_per_trial: mine,
            baseline_mean_tokens_per_trial: *base_mean,
            delta: js_round(delta * 10.0) / 10.0,
            percent: (*base_mean != 0.0).then(|| js_round((delta / base_mean) * 1000.0) / 10.0),
        });
    }

    let mut unknown_ids: Vec<String> = Vec::new();
    for id in unknown {
        if !unknown_ids.contains(&id) {
            unknown_ids.push(id);
        }
    }
    unknown_ids.sort_by(|a, b| js_str_cmp(a, b));
    ScoreReport {
        corpus: summarize_corpus(corpus),
        baseline_tier,
        tiers,
        historical_only_trials: historical_only,
        unknown_corpus_ids: unknown_ids,
        caveats: caveats(),
        no_measurement: opts.no_measurement.then_some(true),
        anchoring: opts.anchoring,
        decision: opts.decision.filter(|d| !d.is_empty()),
        batch_power: opts.batch_power,
    }
}

/// A graded/refuted tally.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Tally {
    /// Graded.
    pub graded: usize,
    /// Refuted.
    pub refuted: usize,
    /// `refuted / graded`.
    #[serde(serialize_with = "serialize_opt_js_number")]
    pub refuted_rate: Option<f64>,
}

/// A per-position tally.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PositionTally {
    /// 1-based position in the group's stable id order.
    pub position: usize,
    /// Graded.
    pub graded: usize,
    /// Refuted.
    pub refuted: usize,
    /// `refuted / graded`.
    #[serde(serialize_with = "serialize_opt_js_number")]
    pub refuted_rate: Option<f64>,
}

/// Refutation rate by position.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ByPosition {
    /// Position 1.
    pub first_position: Tally,
    /// Positions 2..n pooled.
    pub later_positions: Tally,
    /// Every position.
    pub by_position: Vec<PositionTally>,
    /// The anchoring signature: every later rate at or above the previous,
    /// the first strictly above position 1, the last strictly above too.
    pub rises_after_first: bool,
}

/// One arm's anchoring figures.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ArmAnchoring {
    /// The arm.
    pub arm: String,
    /// (group × tier × replicate) dispatches with ≥ 2 graded verdicts.
    pub dispatches_considered: usize,
    /// Of those, all verdicts identical.
    pub all_same_verdict: usize,
    /// `all_same / considered`.
    #[serde(serialize_with = "serialize_opt_js_number")]
    pub all_same_verdict_share: Option<f64>,
    /// The position breakdown.
    pub refutation_rate_by_position: ByPosition,
}

/// The anchoring block.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Anchoring {
    /// The qualifying floor.
    pub min_group_size: usize,
    /// Qualifying groups.
    pub qualifying_groups: usize,
    /// Their items.
    pub qualifying_items: usize,
    /// Per arm, sorted by arm.
    pub arms: Vec<ArmAnchoring>,
    /// How to read it.
    pub note: String,
}

/// The anchoring measurement over QUALIFYING groups only (a size-1 "batch" is
/// byte-for-byte a per-finding dispatch and can exhibit no anchoring).
pub fn score_anchoring(
    groups: &[Group],
    rows_by_arm: &BTreeMap<String, Vec<JsValue>>,
    min_group_size: usize,
) -> Anchoring {
    let qualifying: Vec<&Group> = groups.iter().filter(|g| g.size >= min_group_size).collect();
    let mut group_of: Vec<(String, String, usize)> = Vec::new();
    for g in &qualifying {
        for (i, id) in g.ids.iter().enumerate() {
            match group_of.iter_mut().find(|(x, _, _)| x == id) {
                Some(slot) => {
                    slot.1 = g.key.clone();
                    slot.2 = i + 1;
                }
                None => group_of.push((id.clone(), g.key.clone(), i + 1)),
            }
        }
    }
    #[allow(clippy::cast_precision_loss)]
    let arms = rows_by_arm
        .iter()
        .map(|(arm, rows)| {
            let mut dispatches: Vec<(String, Vec<Option<bool>>)> = Vec::new();
            let mut positions: Vec<PositionTally> = Vec::new();
            for row in rows {
                let id = row
                    .get("corpusId")
                    .map_or_else(|| "undefined".to_owned(), JsValue::js_string);
                let Some((_, group_key, pos)) = group_of.iter().find(|(x, _, _)| *x == id) else {
                    continue;
                };
                let refuted = refuted_of(row);
                let dkey = format!(
                    "{group_key} {} {}",
                    row.get("tier")
                        .map_or_else(|| "undefined".to_owned(), JsValue::js_string),
                    row.get("replicate")
                        .map_or_else(|| "undefined".to_owned(), JsValue::js_string)
                );
                match dispatches.iter_mut().find(|(k, _)| *k == dkey) {
                    Some(slot) => slot.1.push(refuted),
                    None => dispatches.push((dkey, vec![refuted])),
                }
                let at = match positions.iter().position(|p| p.position == *pos) {
                    Some(at) => at,
                    None => {
                        positions.push(PositionTally {
                            position: *pos,
                            graded: 0,
                            refuted: 0,
                            refuted_rate: None,
                        });
                        positions.len() - 1
                    }
                };
                if let Some(r) = refuted {
                    positions[at].graded += 1;
                    if r {
                        positions[at].refuted += 1;
                    }
                }
            }
            let mut considered = 0;
            let mut all_same = 0;
            for (_, list) in &dispatches {
                let graded: Vec<bool> = list.iter().filter_map(|v| *v).collect();
                if graded.len() < 2 {
                    continue;
                }
                considered += 1;
                if graded.iter().all(|v| *v == graded[0]) {
                    all_same += 1;
                }
            }
            positions.sort_by_key(|p| p.position);
            for p in &mut positions {
                p.refuted_rate = rate(p.refuted as f64, p.graded as f64);
            }
            let (first_graded, first_refuted) = positions
                .iter()
                .find(|p| p.position == 1)
                .map_or((0, 0), |p| (p.graded, p.refuted));
            let later_graded: usize = positions
                .iter()
                .filter(|p| p.position > 1)
                .map(|p| p.graded)
                .sum();
            let later_refuted: usize = positions
                .iter()
                .filter(|p| p.position > 1)
                .map(|p| p.refuted)
                .sum();
            let first_rate = rate(first_refuted as f64, first_graded as f64);
            let later_rates: Vec<f64> = positions
                .iter()
                .filter(|p| p.position > 1 && p.graded > 0)
                .filter_map(|p| p.refuted_rate)
                .collect();
            let rises = match (first_rate, later_rates.last()) {
                (Some(f), Some(last)) if first_graded > 0 => {
                    later_rates.iter().enumerate().all(|(i, r)| {
                        if i == 0 {
                            *r > f
                        } else {
                            *r >= later_rates[i - 1]
                        }
                    }) && *last > f
                }
                _ => false,
            };
            ArmAnchoring {
                arm: arm.clone(),
                dispatches_considered: considered,
                all_same_verdict: all_same,
                all_same_verdict_share: rate(all_same as f64, considered as f64),
                refutation_rate_by_position: ByPosition {
                    first_position: Tally {
                        graded: first_graded,
                        refuted: first_refuted,
                        refuted_rate: first_rate,
                    },
                    later_positions: Tally {
                        graded: later_graded,
                        refuted: later_refuted,
                        refuted_rate: rate(later_refuted as f64, later_graded as f64),
                    },
                    by_position: positions,
                    rises_after_first: rises,
                },
            }
        })
        .collect();
    Anchoring {
        min_group_size,
        qualifying_groups: qualifying.len(),
        qualifying_items: qualifying.iter().map(|g| g.size).sum(),
        arms,
        note: "A HIGHER all-same-verdict share for the batched arm plus a refutation rate that RISES after position 1 is the anchoring signature this phase exists to detect. Reported over qualifying groups only — a size-1 group is byte-for-byte a per-finding dispatch and can exhibit no anchoring.".to_owned(),
    }
}

/// The default anchoring floor.
pub const DEFAULT_ANCHORING_MIN_GROUP: usize = MIN_BATCH_GROUP_SIZE;

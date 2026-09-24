//! The determining-finding rank: where in a ranked CANDIDATE list does the
//! finding that decided the outcome sit?
//!
//! Ranked over the candidate list (what the finders emitted), not the
//! survivors: severity sorts first, so the top survivor is by construction
//! the determining one and a survivor rank would be a constant 1. A
//! refutation budget truncates the candidate list, so that is the ranking a
//! cap would apply.
//!
//! Unit statuses are decided PER UNIT, from evidence local to that unit. An
//! agent whose unit identity does not resolve (chiefly the JSON-shaped
//! `--implementation-plan` target) belongs to no unit, so it invalidates none:
//! it is counted as an orphan and reported. There is deliberately no
//! run-wide unrecoverable reason, and [`RANK_UNRECOVERABLE_REASONS`] is closed.
//!
//! The ranking (`rankFindings`), the eligibility test (`hasBlocking`), the
//! survival test (`survives`) and the AC-gap test (`acTableHasGap`) are the
//! canonical workflow functions, reached through [`ReviewRules`].

use serde::Serialize;

use super::fanout::{CountSummary, strip_retry, summarize_counts};
use super::{Finder, Refuter};
use crate::measure::jsjson::{JsObject, JsValue, js_str_cmp, quote, strict_equals};
use crate::measure::jsnum::{fmt_number, pct, serialize_js_number};
use crate::measure::review_rules::ReviewRules;

/// The closed unrecoverable-reason vocabulary, in fixed report order.
pub const RANK_UNRECOVERABLE_REASONS: [&str; 5] = [
    "unknown-disposition-above-determining",
    "unreadable-finder-transcript",
    "dimension-coverage-gap",
    "multi-round-unit",
    "ambiguous-finding-join",
];

/// The dimensions whose ABSENCE from a unit's own finder set is local evidence
/// that its candidate list is incomplete: a corpus-stable subset of the
/// always-on dimensions (`restraint` shipped part-way through the measured
/// window, so requiring it would mark every earlier plan unit incomplete).
/// [`coverage_dimensions`] checks it stays a subset of the real always-on set.
pub const COVERAGE_REQUIRED_DIMENSIONS: [(&str, &[&str]); 2] = [
    ("code", &["ac", "correctness"]),
    ("plan", &["coherence", "architectural-fit"]),
];

/// The N values the within-top-N figures report, in fixed order.
pub const WITHIN_TOP_N: [usize; 2] = [3, 5];

/// Validates [`COVERAGE_REQUIRED_DIMENSIONS`] against the always-on keys of
/// the canonical `DIMENSIONS` and returns it.
///
/// # Errors
///
/// When a required key is not always-on in `review.mjs`.
pub fn check_coverage_dimensions(
    always_on: &dyn Fn(&str) -> Vec<String>,
) -> Result<Vec<(String, Vec<String>)>, String> {
    let mut out = Vec::new();
    for (mode, keys) in COVERAGE_REQUIRED_DIMENSIONS {
        let on = always_on(mode);
        for k in keys {
            if !on.iter().any(|o| o == k) {
                return Err(format!(
                    "COVERAGE_REQUIRED_DIMENSIONS.{mode} names \"{k}\", which is not an always-on dimension in .claude/workflows/lib/review.mjs — the coverage check must stay a subset of the real always-on set"
                ));
            }
        }
        out.push((
            mode.to_owned(),
            keys.iter().map(|k| (*k).to_owned()).collect(),
        ));
    }
    Ok(out)
}

/// [`check_coverage_dimensions`] over the canonical dimension table.
///
/// # Errors
///
/// When the table cannot be read or a required key is not always-on.
pub fn coverage_dimensions(
    rules: &mut dyn ReviewRules,
) -> Result<Vec<(String, Vec<String>)>, String> {
    let code = rules.always_on_dimensions("code")?;
    let plan = rules.always_on_dimensions("plan")?;
    check_coverage_dimensions(&|mode| match mode {
        "code" => code.clone(),
        "plan" => plan.clone(),
        _ => Vec::new(),
    })
}

/// A finder's contribution to a unit.
#[derive(Debug, Clone, PartialEq)]
pub struct FinderEntry {
    /// Its dimension (prompt-derived, else label-derived).
    pub dim: Option<String>,
    /// A ` (retry N)` dispatch.
    pub is_retry: bool,
    /// Its findings (`None`: no `StructuredOutput`).
    pub findings: Option<Vec<JsValue>>,
    /// Its `ac` table.
    pub ac_table: Option<JsValue>,
}

/// A refuter's contribution to a unit.
#[derive(Debug, Clone, PartialEq)]
pub struct RefuterEntry {
    /// Its prompt-derived dimension.
    pub dim: Option<String>,
    /// The finding it graded.
    pub finding: Option<JsValue>,
    /// Its verdict.
    pub refuted: Option<bool>,
}

/// Whether a candidate's verdict is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    /// Graded, or legitimately ungraded (a non-gating pass-through).
    Resolved,
    /// Nothing can be said about its verdict; nothing is imputed.
    Unknown,
}

/// One candidate finding of a unit.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    /// The finding.
    pub finding: JsValue,
    /// Its finder's dimension.
    pub dim: Option<String>,
    /// `Some(refuted)` when a refuter graded it.
    pub verdict: Option<bool>,
    /// Whether its verdict is known.
    pub disposition: Disposition,
}

/// One review unit.
#[derive(Debug, Clone, PartialEq)]
pub struct Unit {
    /// `projectSlug|sessionId|runId|unitIdent`.
    pub key: String,
    /// `code` or `plan`, from the labels.
    pub mode: Option<String>,
    /// Its finders, in record order.
    pub finders: Vec<FinderEntry>,
    /// Its refuters, in record order.
    pub refuters: Vec<RefuterEntry>,
    /// The candidate list (after [`prepare_unit`]).
    pub candidates: Vec<Candidate>,
    /// The tier-independent reason the unit cannot be reconstructed.
    pub structural_reason: Option<&'static str>,
    /// Any finder's `ac` table carried a FAIL/PARTIAL.
    pub ac_table_gap: bool,
}

impl Unit {
    /// A unit with no candidates prepared yet.
    pub fn new(
        key: &str,
        mode: Option<&str>,
        finders: Vec<FinderEntry>,
        refuters: Vec<RefuterEntry>,
    ) -> Self {
        Self {
            key: key.to_owned(),
            mode: mode.map(str::to_owned),
            finders,
            refuters,
            candidates: Vec::new(),
            structural_reason: None,
            ac_table_gap: false,
        }
    }
}

/// Orphan agents: attributable to no unit.
#[derive(Debug, Clone, Default)]
pub struct Orphans {
    /// Finders.
    pub finders: usize,
    /// Refuters.
    pub refuters: usize,
    /// Distinct runs they sit in.
    pub runs: Vec<String>,
}

/// A stable structural key for a finding (sorted keys), used when an `id`
/// join is unusable.
pub fn structural_key(v: &JsValue) -> String {
    match v {
        JsValue::Array(a) => format!(
            "[{}]",
            a.iter().map(structural_key).collect::<Vec<_>>().join(",")
        ),
        JsValue::Object(o) => {
            let mut keys: Vec<&String> = o.keys().collect();
            keys.sort_by(|a, b| js_str_cmp(a, b));
            format!(
                "{{{}}}",
                keys.iter()
                    .map(|k| format!(
                        "{}:{}",
                        quote(k),
                        structural_key(o.get(k).unwrap_or(&JsValue::Null))
                    ))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
        JsValue::String(s) => quote(s),
        JsValue::Number(n) if n.is_finite() => fmt_number(*n),
        JsValue::Number(_) | JsValue::Null => "null".to_owned(),
        JsValue::Bool(b) => b.to_string(),
    }
}

/// Groups finders and refuters into units keyed by the prompt-derived unit
/// identity, sorted by key.
pub fn build_units(finders: &[Finder], refuters: &[Refuter]) -> (Vec<Unit>, Orphans) {
    let mut units: Vec<Unit> = Vec::new();
    let mut orphans = Orphans::default();
    let note_orphan_run = |orphans: &mut Orphans, run: String| {
        if !orphans.runs.contains(&run) {
            orphans.runs.push(run);
        }
    };
    fn unit_for<'u>(units: &'u mut Vec<Unit>, key: String, mode: Option<&str>) -> &'u mut Unit {
        let at = match units.iter().position(|u| u.key == key) {
            Some(at) => at,
            None => {
                units.push(Unit::new(&key, mode, Vec::new(), Vec::new()));
                units.len() - 1
            }
        };
        let u = &mut units[at];
        if u.mode.is_none() && mode.is_some() {
            u.mode = mode.map(str::to_owned);
        }
        u
    }

    for f in finders {
        let parts: Vec<&str> = f.record.label.split(':').collect();
        let mode = (parts.len() == 3 && parts[0] == "find").then(|| parts[1]);
        let (label_dim, is_retry) = if parts.len() == 3 {
            let (d, retry) = strip_retry(parts[2]);
            (Some(d), retry)
        } else {
            (None, false)
        };
        let Some(t) = f.transcript.as_ref() else {
            orphans.finders += 1;
            note_orphan_run(&mut orphans, f.record.run_key());
            continue;
        };
        let Some(unit_ident) = t.context.unit_ident.as_deref() else {
            orphans.finders += 1;
            note_orphan_run(&mut orphans, f.record.run_key());
            continue;
        };
        let dim = t
            .context
            .dim_key
            .clone()
            .filter(|d| !d.is_empty())
            .or_else(|| label_dim.map(str::to_owned));
        let key = format!("{}|{unit_ident}", f.record.run_key());
        unit_for(&mut units, key, mode).finders.push(FinderEntry {
            dim,
            is_retry,
            findings: t.findings.clone(),
            ac_table: t.ac_table.clone(),
        });
    }

    for r in refuters {
        let Some(unit_ident) = r.unit_ident.as_deref() else {
            orphans.refuters += 1;
            note_orphan_run(&mut orphans, r.record.run_key());
            continue;
        };
        let parts: Vec<&str> = r.record.label.split(':').collect();
        let mode = (parts.len() >= 2 && parts[0] == "refute").then(|| parts[1]);
        let key = format!("{}|{unit_ident}", r.record.run_key());
        unit_for(&mut units, key, mode).refuters.push(RefuterEntry {
            dim: r.dim_key.clone(),
            finding: r.finding.clone(),
            refuted: r.refuted,
        });
    }
    units.sort_by(|a, b| js_str_cmp(&a.key, &b.key));
    (units, orphans)
}

fn group_by_dim<T>(items: &[T], dim: impl Fn(&T) -> Option<&str>) -> Vec<(String, Vec<&T>)> {
    let mut groups: Vec<(String, Vec<&T>)> = Vec::new();
    for item in items {
        let k = dim(item).unwrap_or("").to_owned();
        match groups.iter_mut().find(|(g, _)| *g == k) {
            Some(slot) => slot.1.push(item),
            None => groups.push((k, vec![item])),
        }
    }
    groups
}

/// Joins each candidate to the refuter that graded it; returns whether any
/// join was ambiguous. A candidate with no refuter is resolved-and-ungraded
/// when its severity is non-gating, otherwise unknown; nothing is imputed.
pub fn attach_dispositions(u: &mut Unit, non_gating: &[String]) -> bool {
    let pools = group_by_dim(&u.refuters, |r| r.dim.as_deref());
    let mut ambiguous = false;
    for c in &mut u.candidates {
        let key = c.dim.as_deref().unwrap_or("");
        let pool: Vec<&RefuterEntry> = pools
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, p)| p.clone())
            .unwrap_or_default();
        let id = c.finding.get("id").filter(|v| !matches!(v, JsValue::Null));
        let by_id: Vec<&RefuterEntry> = match id {
            None => Vec::new(),
            Some(id) => pool
                .iter()
                .copied()
                .filter(|r| {
                    r.finding
                        .as_ref()
                        .and_then(|f| f.get("id"))
                        .is_some_and(|rid| strict_equals(rid, id))
                })
                .collect(),
        };
        let mut matched: Option<&RefuterEntry> = None;
        if by_id.len() == 1 {
            matched = Some(by_id[0]);
        } else {
            let wanted = structural_key(&c.finding);
            let deep: Vec<&RefuterEntry> = pool
                .iter()
                .copied()
                .filter(|r| {
                    r.finding
                        .as_ref()
                        .is_some_and(|f| structural_key(f) == wanted)
                })
                .collect();
            if deep.len() == 1 {
                matched = Some(deep[0]);
            } else if deep.len() > 1 || by_id.len() > 1 {
                ambiguous = true;
            }
        }
        if let Some(m) = matched {
            c.verdict = m.refuted;
            c.disposition = if m.refuted.is_some() {
                Disposition::Resolved
            } else {
                Disposition::Unknown
            };
            continue;
        }
        let severity = c.finding.get("severity").and_then(JsValue::as_str);
        c.verdict = None;
        c.disposition = if severity.is_some_and(|s| non_gating.iter().any(|n| n == s)) {
            Disposition::Resolved
        } else {
            Disposition::Unknown
        };
    }
    ambiguous
}

/// Resolves a unit's candidate list, dispositions and tier-independent
/// structural reason. Precedence: reasons that invalidate the candidate list
/// itself (`multi-round-unit`, `ambiguous-finding-join`,
/// `unreadable-finder-transcript`) come before `dimension-coverage-gap`,
/// because a walk over a wrong list yields a plausible wrong rank.
///
/// Pure: needs no canonical call (the `ac`-table diagnostic is computed by
/// [`mark_ac_table_gaps`]).
pub fn prepare_unit(u: &mut Unit, non_gating: &[String], coverage: &[(String, Vec<String>)]) {
    // A ` (retry N)` dispatch supersedes the previous attempt at its
    // dimension; two NON-retry finders at one dimension are a second review
    // round collapsing into one unit identity.
    let groups = group_by_dim(&u.finders, |f| f.dim.as_deref());
    let mut multi_round = false;
    let mut selected: Vec<FinderEntry> = Vec::new();
    for (_, group) in &groups {
        if group.iter().filter(|f| !f.is_retry).count() > 1 {
            multi_round = true;
        }
        if let Some(last) = group.last() {
            selected.push((*last).clone());
        }
    }
    u.candidates.clear();
    let mut unreadable = false;
    for f in &selected {
        match &f.findings {
            Some(list) if list.iter().all(JsValue::is_object) => {
                for finding in list {
                    u.candidates.push(Candidate {
                        finding: finding.clone(),
                        dim: f.dim.clone(),
                        verdict: None,
                        disposition: Disposition::Unknown,
                    });
                }
            }
            _ => unreadable = true,
        }
    }
    let ambiguous = attach_dispositions(u, non_gating);

    let finder_dims: Vec<Option<&str>> = selected.iter().map(|f| f.dim.as_deref()).collect();
    let required: &[String] = u
        .mode
        .as_deref()
        .and_then(|m| coverage.iter().find(|(mode, _)| mode == m))
        .map_or(&[], |(_, keys)| keys.as_slice());
    let coverage_gap = required
        .iter()
        .any(|d| !finder_dims.contains(&Some(d.as_str())))
        || u.refuters.iter().any(|r| {
            r.dim
                .as_deref()
                .is_some_and(|d| !d.is_empty() && !finder_dims.contains(&Some(d)))
        });

    u.structural_reason = if multi_round {
        Some("multi-round-unit")
    } else if ambiguous {
        Some("ambiguous-finding-join")
    } else if unreadable {
        Some("unreadable-finder-transcript")
    } else if coverage_gap {
        Some("dimension-coverage-gap")
    } else {
        None
    };
}

/// Sets each unit's `ac_table_gap`: whether any of its finders' `ac` tables
/// carries a FAIL/PARTIAL, per the canonical `acTableHasGap`.
///
/// # Errors
///
/// When the canonical call fails.
pub fn mark_ac_table_gaps(units: &mut [Unit], rules: &mut dyn ReviewRules) -> Result<(), String> {
    for u in units {
        u.ac_table_gap = false;
        for f in &u.finders {
            if rules.ac_table_has_gap(f.ac_table.as_ref())? {
                u.ac_table_gap = true;
                break;
            }
        }
    }
    Ok(())
}

/// A unit's outcome at one tier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnitOutcome {
    /// The determining finding's 1-based rank in the full candidate list.
    Determining(usize),
    /// Every candidate resolved and nothing gated.
    NonDetermining,
    /// The rank cannot be reconstructed, for this reason.
    Unrecoverable(&'static str),
}

fn verdict_value(refuted: Option<bool>) -> Option<JsValue> {
    refuted.map(|r| {
        let mut o = JsObject::new();
        o.insert("refuted", JsValue::Bool(r));
        JsValue::Object(o)
    })
}

/// Walks one unit's ranked candidates. Eligibility (`hasBlocking` at `tier`)
/// is checked BEFORE the disposition: a candidate that cannot gate at this
/// tier is inert, so its missing verdict decides nothing. The first eligible
/// unknown ranked above the determining finding makes the unit unrecoverable.
///
/// # Errors
///
/// When a canonical call fails.
pub fn determine_rank_for_unit(
    u: &Unit,
    tier: Option<&str>,
    rules: &mut dyn ReviewRules,
) -> Result<UnitOutcome, String> {
    if let Some(reason) = u.structural_reason {
        return Ok(UnitOutcome::Unrecoverable(reason));
    }
    let findings: Vec<JsValue> = u.candidates.iter().map(|c| c.finding.clone()).collect();
    let order = rules.rank_findings(&findings)?;
    for (i, &at) in order.iter().enumerate() {
        let c = &u.candidates[at];
        let eligible = rules.has_blocking(std::slice::from_ref(&c.finding), tier)?;
        if !eligible {
            continue;
        }
        if c.disposition == Disposition::Unknown {
            return Ok(UnitOutcome::Unrecoverable(
                "unknown-disposition-above-determining",
            ));
        }
        if rules.survives(&c.finding, verdict_value(c.verdict).as_ref())? {
            return Ok(UnitOutcome::Determining(i + 1));
        }
    }
    Ok(UnitOutcome::NonDetermining)
}

/// The unit partition at one tier.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UnitCounts {
    /// Every attributable unit.
    pub total: usize,
    /// Units with a determining finding.
    pub determining: usize,
    /// Fully resolved units where nothing gated.
    pub non_determining: usize,
    /// Units whose rank cannot be reconstructed.
    pub unrecoverable: usize,
    /// `determining + non_determining`.
    pub recoverable: usize,
    /// `recoverable / total`, one-decimal percent.
    #[serde(serialize_with = "serialize_js_number")]
    pub recoverable_share_percent: f64,
}

/// One unrecoverable-reason row.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ReasonCount {
    /// The reason.
    pub reason: String,
    /// Units.
    pub count: usize,
}

/// One rank-histogram row.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RankCount {
    /// The rank.
    pub rank: usize,
    /// Units.
    pub count: usize,
}

/// One within-top-N row.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WithinTop {
    /// N.
    pub n: usize,
    /// Determining units at rank ≤ N.
    pub count: usize,
    /// Of determining units.
    #[serde(serialize_with = "serialize_js_number")]
    pub percent_of_determining: f64,
    /// Of recoverable units.
    #[serde(serialize_with = "serialize_js_number")]
    pub percent_of_recoverable: f64,
}

/// One tier's summary.
#[derive(Debug, Clone, PartialEq)]
pub struct TierSummary {
    /// The partition.
    pub units: UnitCounts,
    /// Unrecoverable units by reason, in vocabulary order.
    pub unrecoverable_by_reason: Vec<ReasonCount>,
    /// Determining units by rank.
    pub rank_histogram: Vec<RankCount>,
    /// The rank distribution (absent with no determining unit).
    pub rank_summary: Option<CountSummary>,
    /// Within top 3 and 5.
    pub within_top: Vec<WithinTop>,
    /// Candidate-list sizes over recoverable units.
    pub candidate_set_size: Option<CountSummary>,
}

/// Summarizes every unit at `tier`.
///
/// # Errors
///
/// When a canonical call fails.
pub fn summarize_tier(
    units: &[Unit],
    tier: Option<&str>,
    rules: &mut dyn ReviewRules,
) -> Result<TierSummary, String> {
    let mut reasons: Vec<(&'static str, usize)> = Vec::new();
    let mut ranks = Vec::new();
    let mut sizes = Vec::new();
    let (mut determining, mut non_determining, mut unrecoverable) = (0, 0, 0);
    for u in units {
        #[allow(clippy::cast_precision_loss)]
        let size = u.candidates.len() as f64;
        match determine_rank_for_unit(u, tier, rules)? {
            UnitOutcome::Determining(rank) => {
                determining += 1;
                ranks.push(rank);
                sizes.push(size);
            }
            UnitOutcome::NonDetermining => {
                non_determining += 1;
                sizes.push(size);
            }
            UnitOutcome::Unrecoverable(reason) => {
                unrecoverable += 1;
                match reasons.iter_mut().find(|(r, _)| *r == reason) {
                    Some(slot) => slot.1 += 1,
                    None => reasons.push((reason, 1)),
                }
            }
        }
    }
    let total = units.len();
    let recoverable = determining + non_determining;
    let mut histogram: Vec<RankCount> = Vec::new();
    for &r in &ranks {
        match histogram.iter_mut().find(|h| h.rank == r) {
            Some(h) => h.count += 1,
            None => histogram.push(RankCount { rank: r, count: 1 }),
        }
    }
    histogram.sort_by_key(|h| h.rank);
    #[allow(clippy::cast_precision_loss)]
    let rank_values: Vec<f64> = ranks.iter().map(|&r| r as f64).collect();
    #[allow(clippy::cast_precision_loss)]
    let within_top = WITHIN_TOP_N
        .iter()
        .map(|&n| {
            let count = ranks.iter().filter(|&&r| r <= n).count();
            WithinTop {
                n,
                count,
                percent_of_determining: pct(count as f64, determining as f64),
                percent_of_recoverable: pct(count as f64, recoverable as f64),
            }
        })
        .collect();
    #[allow(clippy::cast_precision_loss)]
    Ok(TierSummary {
        units: UnitCounts {
            total,
            determining,
            non_determining,
            unrecoverable,
            recoverable,
            recoverable_share_percent: pct(recoverable as f64, total as f64),
        },
        unrecoverable_by_reason: RANK_UNRECOVERABLE_REASONS
            .iter()
            .filter_map(|r| {
                reasons
                    .iter()
                    .find(|(x, _)| x == r)
                    .map(|(_, c)| ReasonCount {
                        reason: (*r).to_owned(),
                        count: *c,
                    })
            })
            .collect(),
        rank_histogram: histogram,
        rank_summary: (!rank_values.is_empty()).then(|| summarize_counts(&rank_values)),
        within_top,
        candidate_set_size: (!sizes.is_empty()).then(|| summarize_counts(&sizes)),
    })
}

/// The pre-registered rule that turns the distribution into an answer.
#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CapVerdictRule {
    /// Supports a cap at or above this within-top-5 share.
    #[serde(serialize_with = "serialize_js_number")]
    pub supports_cap_at_or_above_percent: f64,
    /// Kills a cap below this within-top-5 share.
    #[serde(serialize_with = "serialize_js_number")]
    pub kills_cap_below_percent: f64,
    /// Supporting also needs this recoverable share.
    #[serde(serialize_with = "serialize_js_number")]
    pub min_recoverable_share_percent: f64,
    /// Below this many determining units the answer is inconclusive.
    #[serde(serialize_with = "serialize_js_number")]
    pub min_determining_units: f64,
    /// What the rule reads.
    pub basis: &'static str,
}

/// `CAP_VERDICT_RULE`.
pub const CAP_VERDICT_RULE: CapVerdictRule = CapVerdictRule {
    supports_cap_at_or_above_percent: 95.0,
    kills_cap_below_percent: 80.0,
    min_recoverable_share_percent: 50.0,
    min_determining_units: 20.0,
    basis: "withinTop n=5, as a percentage of DETERMINING units; a cap is only supported when enough units were recoverable to speak to it at all.",
};

/// The figures the cap verdict reads.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct CapVerdictInputs {
    /// Determining units.
    #[serde(serialize_with = "serialize_js_number")]
    pub determining: f64,
    /// Recoverable units.
    #[serde(serialize_with = "serialize_js_number")]
    pub recoverable: f64,
    /// All units.
    #[serde(serialize_with = "serialize_js_number")]
    pub total: f64,
    /// Recoverable share.
    #[serde(serialize_with = "serialize_js_number")]
    pub recoverable_share_percent: f64,
    /// Within-top-5 as a share of determining units.
    #[serde(serialize_with = "serialize_js_number")]
    pub within_top5_percent_of_determining: f64,
}

/// `supports-cap` | `kills-cap` | `inconclusive`. A missing figure counts as
/// zero (never "assume the best").
pub fn derive_cap_verdict(i: &CapVerdictInputs, rule: &CapVerdictRule) -> &'static str {
    let z = |x: f64| if x.is_nan() { 0.0 } else { x };
    let (determining, share, top5) = (
        z(i.determining),
        z(i.recoverable_share_percent),
        z(i.within_top5_percent_of_determining),
    );
    if determining < rule.min_determining_units {
        return "inconclusive";
    }
    if top5 < rule.kills_cap_below_percent {
        return "kills-cap";
    }
    if top5 >= rule.supports_cap_at_or_above_percent && share >= rule.min_recoverable_share_percent
    {
        return "supports-cap";
    }
    "inconclusive"
}

/// The `capVerdict` block.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CapVerdict {
    /// The derived verdict.
    pub verdict: &'static str,
    /// The rule.
    pub rule: CapVerdictRule,
    /// The figures it read.
    pub inputs: CapVerdictInputs,
}

/// The orphan diagnostic.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OrphanAgents {
    /// Orphan finders.
    pub finders: usize,
    /// Orphan refuters.
    pub refuters: usize,
    /// Distinct runs they sit in.
    pub runs_affected: usize,
}

/// The `largeTier` sensitivity block.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LargeTier {
    /// The partition at the widened blocker set.
    pub units: UnitCounts,
    /// Its histogram.
    pub rank_histogram: Vec<RankCount>,
    /// Its rank distribution.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rank_summary: Option<CountSummary>,
    /// Its within-top-N.
    pub within_top: Vec<WithinTop>,
}

/// The `determiningFindingRank` section.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RankBlock {
    /// The partition at the default blocker set.
    pub units: UnitCounts,
    /// Unrecoverable units by reason.
    pub unrecoverable_by_reason: Vec<ReasonCount>,
    /// Orphan agents.
    pub orphan_agents: OrphanAgents,
    /// Determining units by rank.
    pub rank_histogram: Vec<RankCount>,
    /// The rank distribution.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rank_summary: Option<CountSummary>,
    /// Within top 3 and 5.
    pub within_top: Vec<WithinTop>,
    /// Candidate-list sizes over recoverable units.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidate_set_size: Option<CountSummary>,
    /// The `large` tier (`['blocking','concern']`) variant.
    pub large_tier: LargeTier,
    /// Units whose `ac` table carried a FAIL/PARTIAL (a second outcome
    /// channel this measurement does not score).
    pub ac_table_gap_units: usize,
    /// The derived verdict.
    pub cap_verdict: CapVerdict,
}

/// The cap-verdict inputs of a default-tier summary.
pub fn cap_verdict_inputs(base: &TierSummary) -> CapVerdictInputs {
    #[allow(clippy::cast_precision_loss)]
    CapVerdictInputs {
        determining: base.units.determining as f64,
        recoverable: base.units.recoverable as f64,
        total: base.units.total as f64,
        recoverable_share_percent: base.units.recoverable_share_percent,
        within_top5_percent_of_determining: base
            .within_top
            .iter()
            .find(|w| w.n == 5)
            .map_or(0.0, |w| w.percent_of_determining),
    }
}

/// Builds the section. The tier is threaded through runtime context and is in
/// neither prompt, so the headline uses the default blocker set and
/// `largeTier` shows how much the answer moves at `['blocking','concern']`.
///
/// # Errors
///
/// When a canonical call fails.
pub fn build(
    finders: &[Finder],
    refuters: &[Refuter],
    rules: &mut dyn ReviewRules,
    non_gating: &[String],
    coverage: &[(String, Vec<String>)],
) -> Result<RankBlock, String> {
    let (mut units, orphans) = build_units(finders, refuters);
    for u in &mut units {
        prepare_unit(u, non_gating, coverage);
    }
    mark_ac_table_gaps(&mut units, rules)?;
    let base = summarize_tier(&units, None, rules)?;
    let large = summarize_tier(&units, Some("large"), rules)?;
    let inputs = cap_verdict_inputs(&base);
    Ok(RankBlock {
        units: base.units,
        unrecoverable_by_reason: base.unrecoverable_by_reason,
        orphan_agents: OrphanAgents {
            finders: orphans.finders,
            refuters: orphans.refuters,
            runs_affected: orphans.runs.len(),
        },
        rank_histogram: base.rank_histogram,
        rank_summary: base.rank_summary,
        within_top: base.within_top,
        candidate_set_size: base.candidate_set_size,
        large_tier: LargeTier {
            units: large.units,
            rank_histogram: large.rank_histogram,
            rank_summary: large.rank_summary,
            within_top: large.within_top,
        },
        ac_table_gap_units: units.iter().filter(|u| u.ac_table_gap).count(),
        cap_verdict: CapVerdict {
            verdict: derive_cap_verdict(&inputs, &CAP_VERDICT_RULE),
            rule: CAP_VERDICT_RULE,
            inputs,
        },
    })
}

#[cfg(test)]
mod tests {
    //! Pure tests need no JavaScript runtime. `inert_candidate_cannot_decide_unit`
    //! and `unknown_disposition_is_never_imputed` call the
    //! canonical ranking/gating functions through `NodeReviewRules` and need
    //! Node (`RDM_TEST_NODE` or `node` on PATH; a missing runtime fails them).

    use super::*;
    use crate::measure::review_rules::NodeReviewRules;

    fn finding(id: &str, severity: &str) -> JsValue {
        JsValue::parse(&format!(
            r#"{{"id":"{id}","severity":"{severity}","confidence":90,"summary":"seeded"}}"#
        ))
        .unwrap_or(JsValue::Null)
    }

    fn finder(dim: &str, findings: Option<Vec<JsValue>>, is_retry: bool) -> FinderEntry {
        FinderEntry {
            dim: Some(dim.to_owned()),
            is_retry,
            findings,
            ac_table: None,
        }
    }

    fn refuter(dim: &str, f: JsValue, refuted: Option<bool>) -> RefuterEntry {
        RefuterEntry {
            dim: Some(dim.to_owned()),
            finding: Some(f),
            refuted,
        }
    }

    fn coverage() -> Vec<(String, Vec<String>)> {
        COVERAGE_REQUIRED_DIMENSIONS
            .iter()
            .map(|(m, k)| ((*m).to_owned(), k.iter().map(|x| (*x).to_owned()).collect()))
            .collect()
    }

    fn reason_of(
        mode: &str,
        finders: Vec<FinderEntry>,
        refuters: Vec<RefuterEntry>,
    ) -> (Unit, Option<&'static str>) {
        let mut u = Unit::new("k", Some(mode), finders, refuters);
        prepare_unit(&mut u, &["suggestion".to_owned()], &coverage());
        let r = u.structural_reason;
        (u, r)
    }

    fn rules() -> NodeReviewRules {
        NodeReviewRules::start_default().unwrap_or_else(|e| panic!("{e}"))
    }

    #[test]
    fn retry_is_not_a_second_round() {
        let (u, reason) = reason_of(
            "code",
            vec![
                finder("ac", Some(vec![finding("stale", "blocking")]), false),
                finder("ac", Some(vec![finding("fresh", "blocking")]), true),
                finder("correctness", Some(vec![]), false),
            ],
            vec![refuter("ac", finding("fresh", "blocking"), Some(false))],
        );
        assert_eq!(reason, None, "a retry supersedes its prior attempt");
        let ids: Vec<&str> = u
            .candidates
            .iter()
            .filter_map(|c| c.finding.get("id").and_then(JsValue::as_str))
            .collect();
        assert_eq!(
            ids,
            ["fresh"],
            "the superseded attempt contributes no candidate"
        );
        let (_, twice) = reason_of(
            "code",
            vec![
                finder("ac", Some(vec![finding("a1", "blocking")]), false),
                finder("ac", Some(vec![finding("a2", "blocking")]), false),
                finder("correctness", Some(vec![]), false),
            ],
            vec![],
        );
        assert_eq!(
            twice,
            Some("multi-round-unit"),
            "two NON-retry finders are a second round"
        );
    }

    #[test]
    fn ambiguous_finding_join_is_unrecoverable() {
        let (_, reason) = reason_of(
            "code",
            vec![
                finder("ac", Some(vec![finding("dup", "blocking")]), false),
                finder("correctness", Some(vec![]), false),
            ],
            vec![
                refuter("ac", finding("dup", "blocking"), Some(true)),
                refuter("ac", finding("dup", "blocking"), Some(false)),
            ],
        );
        assert_eq!(reason, Some("ambiguous-finding-join"));
    }

    #[test]
    fn id_less_finding_joins_structurally() {
        let no_id = |summary: &str| {
            JsValue::parse(&format!(
                r#"{{"severity":"blocking","confidence":90,"summary":"{summary}"}}"#
            ))
            .unwrap_or(JsValue::Null)
        };
        let (u, reason) = reason_of(
            "code",
            vec![
                finder("ac", Some(vec![no_id("no id here")]), false),
                finder("correctness", Some(vec![]), false),
            ],
            vec![
                refuter("ac", no_id("no id here"), Some(true)),
                refuter("ac", no_id("something else"), Some(false)),
            ],
        );
        assert_eq!(reason, None, "one structural match is not ambiguous");
        assert_eq!(u.candidates[0].disposition, Disposition::Resolved);
        assert_eq!(
            u.candidates[0].verdict,
            Some(true),
            "the join carried the real verdict"
        );
    }

    #[test]
    fn unreadable_finder_transcript_is_not_empty() {
        let (_, reason) = reason_of(
            "code",
            vec![
                finder("ac", None, false),
                finder("correctness", Some(vec![finding("c1", "blocking")]), false),
            ],
            vec![],
        );
        assert_eq!(reason, Some("unreadable-finder-transcript"));
        for garbage in [
            vec![JsValue::String("not-an-object".to_owned())],
            vec![JsValue::Null],
            vec![JsValue::Array(vec![])],
        ] {
            let (_, reason) = reason_of(
                "code",
                vec![
                    finder("ac", Some(garbage.clone()), false),
                    finder("correctness", Some(vec![]), false),
                ],
                vec![],
            );
            assert_eq!(reason, Some("unreadable-finder-transcript"), "{garbage:?}");
        }
        // An EMPTY finder output is a legitimate zero, not unreadable.
        let (_, reason) = reason_of(
            "code",
            vec![
                finder("ac", Some(vec![]), false),
                finder("correctness", Some(vec![]), false),
            ],
            vec![],
        );
        assert_eq!(reason, None);
    }

    #[test]
    fn reason_vocabulary_is_closed_and_precedence_holds() {
        let d = || finding("d", "blocking");
        // dimension-coverage-gap, both halves.
        assert_eq!(
            reason_of("code", vec![finder("ac", Some(vec![]), false)], vec![]).1,
            Some("dimension-coverage-gap")
        );
        assert_eq!(
            reason_of(
                "plan",
                vec![finder("coherence", Some(vec![]), false)],
                vec![]
            )
            .1,
            Some("dimension-coverage-gap")
        );
        assert_eq!(
            reason_of(
                "code",
                vec![
                    finder("ac", Some(vec![]), false),
                    finder("correctness", Some(vec![]), false)
                ],
                vec![refuter("security", finding("s1", "blocking"), Some(false))]
            )
            .1,
            Some("dimension-coverage-gap"),
            "a refuter for a dimension with no finder"
        );
        // Precedence: every adjacent pair of the chain.
        assert_eq!(
            reason_of(
                "code",
                vec![
                    finder("ac", Some(vec![finding("x", "blocking")]), false),
                    finder("ac", Some(vec![finding("y", "blocking")]), false)
                ],
                vec![]
            )
            .1,
            Some("multi-round-unit"),
            "multi-round beats dimension-coverage-gap"
        );
        assert_eq!(
            reason_of(
                "code",
                vec![
                    finder("ac", Some(vec![d()]), false),
                    finder("ac", Some(vec![d()]), false),
                    finder("correctness", Some(vec![]), false)
                ],
                vec![
                    refuter("ac", d(), Some(true)),
                    refuter("ac", d(), Some(false))
                ]
            )
            .1,
            Some("multi-round-unit"),
            "multi-round beats ambiguous-finding-join"
        );
        assert_eq!(
            reason_of(
                "code",
                vec![
                    finder("ac", None, false),
                    finder("correctness", Some(vec![d()]), false)
                ],
                vec![
                    refuter("correctness", d(), Some(true)),
                    refuter("correctness", d(), Some(false))
                ]
            )
            .1,
            Some("ambiguous-finding-join"),
            "ambiguous-finding-join beats unreadable-finder-transcript"
        );
        assert_eq!(
            reason_of(
                "code",
                vec![finder("ac", Some(vec![d()]), false)],
                vec![
                    refuter("ac", d(), Some(true)),
                    refuter("ac", d(), Some(false))
                ]
            )
            .1,
            Some("ambiguous-finding-join"),
            "ambiguous-finding-join beats dimension-coverage-gap"
        );
        assert_eq!(
            reason_of("code", vec![finder("ac", None, false)], vec![]).1,
            Some("unreadable-finder-transcript"),
            "unreadable-finder-transcript beats dimension-coverage-gap"
        );
        // Every structural reason is in the closed vocabulary.
        for r in [
            "multi-round-unit",
            "ambiguous-finding-join",
            "unreadable-finder-transcript",
            "dimension-coverage-gap",
        ] {
            assert!(RANK_UNRECOVERABLE_REASONS.contains(&r));
        }
    }

    #[test]
    fn cap_verdict_four_branches_and_thresholds() {
        let r = CAP_VERDICT_RULE;
        let at = |determining: f64, share: f64, top5: f64| CapVerdictInputs {
            determining,
            recoverable: 0.0,
            total: 0.0,
            recoverable_share_percent: share,
            within_top5_percent_of_determining: top5,
        };
        let cases = [
            (
                at(r.min_determining_units - 1.0, 100.0, 100.0),
                "inconclusive",
            ),
            (at(0.0, 0.0, 0.0), "inconclusive"),
            (
                at(
                    r.min_determining_units,
                    100.0,
                    r.kills_cap_below_percent - 0.1,
                ),
                "kills-cap",
            ),
            (at(r.min_determining_units * 10.0, 100.0, 0.0), "kills-cap"),
            (
                at(
                    r.min_determining_units,
                    r.min_recoverable_share_percent,
                    r.supports_cap_at_or_above_percent,
                ),
                "supports-cap",
            ),
            (
                at(r.min_determining_units * 2.0, 100.0, 100.0),
                "supports-cap",
            ),
            (
                at(r.min_determining_units, 100.0, r.kills_cap_below_percent),
                "inconclusive",
            ),
            (
                at(
                    r.min_determining_units,
                    100.0,
                    r.supports_cap_at_or_above_percent - 0.1,
                ),
                "inconclusive",
            ),
            (
                at(
                    r.min_determining_units,
                    r.min_recoverable_share_percent - 0.1,
                    100.0,
                ),
                "inconclusive",
            ),
        ];
        let mut seen = std::collections::BTreeSet::new();
        for (inputs, want) in cases {
            assert_eq!(derive_cap_verdict(&inputs, &r), want, "{inputs:?}");
            seen.insert(want);
        }
        assert_eq!(
            seen.len(),
            3,
            "every verdict in the closed vocabulary is exercised"
        );
        assert_eq!(
            derive_cap_verdict(&CapVerdictInputs::default(), &r),
            "inconclusive",
            "missing figures cannot support a cap"
        );
    }

    #[test]
    fn coverage_dimensions_must_be_always_on() {
        let without_correctness = |mode: &str| -> Vec<String> {
            match mode {
                "code" => vec!["ac".to_owned(), "tests".to_owned()],
                _ => vec!["coherence".to_owned(), "architectural-fit".to_owned()],
            }
        };
        let e = check_coverage_dimensions(&without_correctness)
            .err()
            .unwrap_or_default();
        assert!(e.contains("\"correctness\""), "{e}");
        let all = |mode: &str| -> Vec<String> {
            match mode {
                "code" => vec!["ac".to_owned(), "correctness".to_owned()],
                _ => vec!["coherence".to_owned(), "architectural-fit".to_owned()],
            }
        };
        assert!(check_coverage_dimensions(&all).is_ok());
    }

    fn walk_unit(candidates: Vec<Candidate>) -> Unit {
        let mut u = Unit::new("k", Some("code"), vec![], vec![]);
        u.candidates = candidates;
        u
    }

    fn resolved(f: JsValue, refuted: bool) -> Candidate {
        Candidate {
            finding: f,
            dim: Some("ac".to_owned()),
            verdict: Some(refuted),
            disposition: Disposition::Resolved,
        }
    }

    fn unknown(f: JsValue) -> Candidate {
        Candidate {
            finding: f,
            dim: Some("ac".to_owned()),
            verdict: None,
            disposition: Disposition::Unknown,
        }
    }

    #[test]
    fn inert_candidate_cannot_decide_unit() {
        let mut rules = rules();
        let inert = walk_unit(vec![
            resolved(finding("b1", "blocking"), true),
            unknown(finding("s1", "suggestion")),
        ]);
        for tier in [None, Some("large")] {
            assert_eq!(
                determine_rank_for_unit(&inert, tier, &mut rules),
                Ok(UnitOutcome::NonDetermining),
                "an ungraded suggestion decides nothing at tier {tier:?}"
            );
        }
        let concern = walk_unit(vec![
            resolved(finding("b2", "blocking"), true),
            unknown(finding("c2", "concern")),
        ]);
        assert_eq!(
            determine_rank_for_unit(&concern, None, &mut rules),
            Ok(UnitOutcome::NonDetermining)
        );
        assert_eq!(
            determine_rank_for_unit(&concern, Some("large"), &mut rules),
            Ok(UnitOutcome::Unrecoverable(
                "unknown-disposition-above-determining"
            )),
            "the same concern IS eligible at the large tier"
        );
        let skip_then_gate = walk_unit(vec![
            resolved(finding("b5", "blocking"), true),
            unknown(finding("s2", "suggestion")),
            resolved(finding("b6", "blocking"), false),
        ]);
        assert_eq!(
            determine_rank_for_unit(&skip_then_gate, None, &mut rules),
            Ok(UnitOutcome::Determining(2)),
            "ranks index the FULL candidate list; the skipped suggestion does not renumber"
        );
        let _ = rules.shutdown();
    }

    #[test]
    fn unknown_disposition_is_never_imputed() {
        let mut rules = rules();
        let gating_unknown = walk_unit(vec![
            unknown(finding("b3", "blocking")),
            resolved(finding("b4", "blocking"), false),
        ]);
        for tier in [None, Some("large")] {
            assert_eq!(
                determine_rank_for_unit(&gating_unknown, tier, &mut rules),
                Ok(UnitOutcome::Unrecoverable(
                    "unknown-disposition-above-determining"
                )),
                "an ungraded blocking candidate is never imputed as not-refuted"
            );
        }
        // A gating candidate with NO refuter at all is unknown, not a
        // pass-through; a non-gating one is a legitimate pass-through.
        let (u, _) = reason_of(
            "code",
            vec![
                finder(
                    "ac",
                    Some(vec![finding("g", "blocking"), finding("s", "suggestion")]),
                    false,
                ),
                finder("correctness", Some(vec![]), false),
            ],
            vec![],
        );
        assert_eq!(u.candidates[0].disposition, Disposition::Unknown);
        assert_eq!(u.candidates[1].disposition, Disposition::Resolved);
        assert_eq!(
            determine_rank_for_unit(&u, None, &mut rules),
            Ok(UnitOutcome::Unrecoverable(
                "unknown-disposition-above-determining"
            ))
        );
        // Every structural reason is what the walk reports, at both tiers.
        let (multi, _) = reason_of(
            "code",
            vec![
                finder("ac", Some(vec![finding("x", "blocking")]), false),
                finder("ac", Some(vec![finding("y", "blocking")]), false),
                finder("correctness", Some(vec![]), false),
            ],
            vec![],
        );
        for tier in [None, Some("large")] {
            assert_eq!(
                determine_rank_for_unit(&multi, tier, &mut rules),
                Ok(UnitOutcome::Unrecoverable("multi-round-unit"))
            );
        }
        let _ = rules.shutdown();
    }
}

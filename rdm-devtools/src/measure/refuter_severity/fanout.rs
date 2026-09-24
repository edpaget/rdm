//! The two review-fanout distributions: findings per finder (by mode and
//! dimension, from each finder's OWN output) and refuters per review unit
//! (keyed by the unit identity in each refuter's own prompt).
//!
//! Neither keys anything on `phaseTitle`/`phaseIndex`: in a `plan-review` run
//! those are identical across every review unit (run `wf_55af7324-87c`: 96
//! refuters at one `phaseIndex` across 9 units), so grouping by them would
//! silently report one unit with 96 refuters.

use serde::Serialize;

use super::{Finder, Refuter};
use crate::measure::jsjson::js_str_cmp;
use crate::measure::jsnum::{pct, percentile, serialize_js_number};

/// `n/min/p50/p90/max` of a non-empty sample.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CountSummary {
    /// Sample size.
    pub n: usize,
    /// Minimum.
    #[serde(serialize_with = "serialize_js_number")]
    pub min: f64,
    /// Median.
    #[serde(serialize_with = "serialize_js_number")]
    pub p50: f64,
    /// 90th percentile.
    #[serde(serialize_with = "serialize_js_number")]
    pub p90: f64,
    /// Maximum.
    #[serde(serialize_with = "serialize_js_number")]
    pub max: f64,
}

/// Summarizes `values` (callers never pass an empty sample: an empty group is
/// omitted rather than reported as `n: 0`).
pub fn summarize_counts(values: &[f64]) -> CountSummary {
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    CountSummary {
        n: sorted.len(),
        min: sorted.first().copied().unwrap_or(0.0),
        p50: percentile(&sorted, 0.5),
        p90: percentile(&sorted, 0.9),
        max: sorted.last().copied().unwrap_or(0.0),
    }
}

/// One findings-per-finder row.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FinderRow {
    /// `code` or `plan`.
    pub mode: String,
    /// The dimension (retry suffix stripped).
    pub dim: String,
    /// `mode:dim`.
    pub key: String,
    /// The distribution.
    #[serde(flatten)]
    pub summary: CountSummary,
    /// Refuters whose PROMPT-derived dimension is this row's.
    pub refuters_dispatched: usize,
}

/// Findings per finder.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FindingsPerFinder {
    /// One row per `mode:dim`, sorted by key.
    pub rows: Vec<FinderRow>,
    /// Finders with no readable `StructuredOutput`.
    pub unreadable_finder_count: usize,
    /// Finders whose label is not `find:<mode>:<dim>`.
    pub unresolved_label_count: usize,
}

/// Refuters per review unit.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RefuterCountsByUnit {
    /// The distribution (all zero when no unit resolved).
    #[serde(flatten)]
    pub summary: CountSummary,
    /// Every refuter.
    pub total_refuters: usize,
    /// Refuters with a recovered unit.
    pub recovered_refuters: usize,
    /// Refuters never bucketed into any unit.
    pub unrecoverable_refuter_count: usize,
    /// `recovered / total`, one-decimal percent.
    #[serde(serialize_with = "serialize_js_number")]
    pub recovery_rate_percent: f64,
}

/// The `refuterFanout` section.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Fanout {
    /// Findings per finder.
    pub findings_per_finder: FindingsPerFinder,
    /// Refuters per unit.
    pub refuter_counts_by_unit: RefuterCountsByUnit,
}

/// Strips the Workflow runtime's ` (retry N)` label suffix, which names the
/// attempt, not the dimension.
pub fn strip_retry(dim: &str) -> (&str, bool) {
    if let Some(open) = dim.rfind(" (retry ")
        && let Some(inner) = dim[open + " (retry ".len()..].strip_suffix(')')
        && !inner.is_empty()
        && inner.bytes().all(|b| b.is_ascii_digit())
    {
        return (&dim[..open], true);
    }
    (dim, false)
}

/// Builds the section.
pub fn build(finders: &[Finder], refuters: &[Refuter]) -> Fanout {
    let mut unreadable = 0;
    let mut unresolved = 0;
    let mut groups: Vec<(String, String, String, Vec<f64>)> = Vec::new();
    for f in finders {
        let parts: Vec<&str> = f.record.label.split(':').collect();
        if parts.len() != 3 || parts[0] != "find" {
            unresolved += 1;
            continue;
        }
        let mode = parts[1];
        let (dim, _) = strip_retry(parts[2]);
        let Some(count) = f.transcript.as_ref().and_then(|t| t.findings_count()) else {
            unreadable += 1;
            continue;
        };
        let key = format!("{mode}:{dim}");
        #[allow(clippy::cast_precision_loss)]
        let value = count as f64;
        match groups.iter_mut().find(|g| g.2 == key) {
            Some(g) => g.3.push(value),
            None => groups.push((mode.to_owned(), dim.to_owned(), key, vec![value])),
        }
    }
    let mut rows: Vec<FinderRow> = groups
        .into_iter()
        .map(|(mode, dim, key, values)| FinderRow {
            mode,
            dim,
            key,
            summary: summarize_counts(&values),
            refuters_dispatched: 0,
        })
        .collect();
    rows.sort_by(|a, b| js_str_cmp(&a.key, &b.key));

    // A refuter's dimension comes from its OWN prompt, never its label (the
    // label carries the finder-supplied `f.id` whenever there is one); only
    // the mode is trusted from the label.
    for r in refuters {
        let Some(dim) = r.dim_key.as_deref().filter(|d| !d.is_empty()) else {
            continue;
        };
        let Some(mode) = r.record.label.split(':').nth(1).filter(|m| !m.is_empty()) else {
            continue;
        };
        let key = format!("{mode}:{dim}");
        if let Some(row) = rows.iter_mut().find(|row| row.key == key) {
            row.refuters_dispatched += 1;
        }
    }

    let mut recovered = 0;
    let mut unrecoverable = 0;
    let mut units: Vec<(String, f64)> = Vec::new();
    for r in refuters {
        match r.unit_ident.as_deref().filter(|u| !u.is_empty()) {
            Some(unit) => {
                recovered += 1;
                let key = format!("{}|{unit}", r.record.run_key());
                match units.iter_mut().find(|(k, _)| *k == key) {
                    Some(slot) => slot.1 += 1.0,
                    None => units.push((key, 1.0)),
                }
            }
            None => unrecoverable += 1,
        }
    }
    let values: Vec<f64> = units.iter().map(|(_, n)| *n).collect();
    let summary = if values.is_empty() {
        CountSummary {
            n: 0,
            min: 0.0,
            p50: 0.0,
            p90: 0.0,
            max: 0.0,
        }
    } else {
        summarize_counts(&values)
    };
    #[allow(clippy::cast_precision_loss)]
    let rate = pct(recovered as f64, refuters.len() as f64);
    Fanout {
        findings_per_finder: FindingsPerFinder {
            rows,
            unreadable_finder_count: unreadable,
            unresolved_label_count: unresolved,
        },
        refuter_counts_by_unit: RefuterCountsByUnit {
            summary,
            total_refuters: refuters.len(),
            recovered_refuters: recovered,
            unrecoverable_refuter_count: unrecoverable,
            recovery_rate_percent: rate,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_suffix_is_stripped() {
        assert_eq!(strip_retry("ac (retry 1)"), ("ac", true));
        assert_eq!(strip_retry("ac (retry 12)"), ("ac", true));
        assert_eq!(strip_retry("ac (retry x)"), ("ac (retry x)", false));
        assert_eq!(strip_retry("ac"), ("ac", false));
    }
}

#[cfg(test)]
mod join_tests {
    use super::*;
    use crate::measure::refuter_severity::transcripts::FinderTranscript;
    use crate::measure::sidecar::AgentRecord;

    fn record(label: &str) -> AgentRecord {
        AgentRecord {
            project_slug: "p".to_owned(),
            session_id: "s".to_owned(),
            run_id: "r".to_owned(),
            agent_id: Some("a".to_owned()),
            label: label.to_owned(),
            agent_class: label.split(':').next().unwrap_or("").to_owned(),
            model: "m".to_owned(),
            workflow_name: "w".to_owned(),
            agent_count: 1.0,
            output: 0.0,
            uncached_input: 0.0,
            cache_write: 0.0,
            cache_read: 0.0,
            deduped_request_count: 0.0,
            sidecar_only: false,
            cached: false,
            first_request_tokens: None,
        }
    }

    #[test]
    fn refuters_join_on_prompt_dimension_not_label() {
        let finders = [Finder {
            record: record("find:plan:unit-of-work"),
            transcript: Some(FinderTranscript {
                findings: Some(vec![]),
                ..FinderTranscript::default()
            }),
        }];
        // The label names `coherence` (the finder-supplied finding id), but the
        // prompt says the dimension is `unit-of-work`.
        let refuters = [Refuter {
            record: record("refute:plan:coherence"),
            severity: "blocking".to_owned(),
            refuted: Some(true),
            dim_key: Some("unit-of-work".to_owned()),
            unit_ident: Some("phase x/phase-1".to_owned()),
            finding: None,
        }];
        let f = build(&finders, &refuters);
        let rows = &f.findings_per_finder.rows;
        assert_eq!(rows.len(), 1, "no spurious plan:coherence row");
        assert_eq!(rows[0].key, "plan:unit-of-work");
        assert_eq!(
            rows[0].refuters_dispatched, 1,
            "joined on the prompt-derived dimension"
        );
        assert_eq!(f.refuter_counts_by_unit.recovered_refuters, 1);
    }
}

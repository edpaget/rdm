//! Rendering the score report. The text form puts FALSE NEGATIVES and FALSE
//! POSITIVES in two separately labelled blocks (so a skimmer cannot average
//! them) and puts each tier's token and tool-call columns on the same row as
//! its rates.

use super::corpus::HISTORICAL_ONLY_SEVERITIES;
use super::score::ScoreReport;
use super::trials::MIN_BATCH_GROUP_SIZE;
use crate::measure::jsnum::{to_fixed, to_locale_en_us};

fn pct_str(v: Option<f64>) -> String {
    v.map_or_else(|| "n/a".to_owned(), |x| format!("{}%", to_fixed(x, 1)))
}

fn num(v: Option<f64>) -> String {
    v.map_or_else(|| "null".to_owned(), to_locale_en_us)
}

fn frac(part: usize, whole: usize, r: Option<f64>) -> String {
    format!("{part}/{whole} ({})", pct_str(r))
}

/// `--format json`.
///
/// # Errors
///
/// Serialization failure (not expected).
pub fn format_json(report: &ScoreReport) -> Result<String, String> {
    serde_json::to_string_pretty(report).map_err(|e| e.to_string())
}

/// `--format text` (no trailing newline).
pub fn format_text(report: &ScoreReport) -> String {
    let mut out: Vec<String> = Vec::new();
    let c = &report.corpus;
    if report.no_measurement == Some(true) {
        out.push("NO MEASUREMENT — batched arm was underpowered".to_owned());
        out.push(format!("The qualifying (size >= {MIN_BATCH_GROUP_SIZE}) unit-scoped batch population did not clear the pre-registered floor, so the figures below describe a batched arm dominated by size-1 batches. That is not evidence and carries NO decision."));
        out.push(String::new());
    }
    out.push(format!(
        "Refuter agreement by model tier — {} corpus item(s), baseline tier \"{}\"",
        c.size,
        report.baseline_tier.as_deref().unwrap_or("null")
    ));
    let entries = |m: &crate::measure::jsjson::JsMap<usize>| {
        m.iter()
            .map(|(k, v)| format!("{k} {v}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    out.push(format!(
        "Composition: {} | authority: {} | provenance: {}",
        entries(&c.by_class),
        entries(&c.by_authority),
        entries(&c.by_provenance)
    ));
    out.push(String::new());

    out.push("## FALSE NEGATIVES — a real defect wrongly refuted -> SHIPS A DEFECT".to_owned());
    out.push(
        "Denominator: defect-truth trials only. Authoritative-only is the DECISION-GRADE figure."
            .to_owned(),
    );
    out.push("| tier | authoritative FN | judgement-call FN | all FN | output | uncached input | cache write | cache read | mean tokens/trial | mean tool calls/trial |".to_owned());
    out.push("|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|".to_owned());
    for t in &report.tiers {
        out.push(format!(
            "| {} |",
            [
                t.bucket.clone(),
                frac(
                    t.authoritative_only.false_negatives,
                    t.authoritative_only.defect_trials,
                    t.authoritative_only.false_negative_rate
                ),
                frac(
                    t.judgement_call_only.false_negatives,
                    t.judgement_call_only.defect_trials,
                    t.judgement_call_only.false_negative_rate
                ),
                frac(
                    t.all.false_negatives,
                    t.all.defect_trials,
                    t.all.false_negative_rate
                ),
                num(Some(t.cost.output)),
                num(Some(t.cost.uncached_input)),
                num(Some(t.cost.cache_write)),
                num(Some(t.cost.cache_read)),
                num(t.cost.mean_tokens_per_trial),
                num(t.cost.mean_tool_calls_per_trial),
            ]
            .join(" | ")
        ));
    }
    out.push(String::new());

    out.push("## FALSE POSITIVES — a non-defect kept -> COSTS A REWORK ROUND".to_owned());
    out.push(
        "Denominator: non-defect-truth trials only. A DIFFERENT denominator from the block above."
            .to_owned(),
    );
    out.push("| tier | authoritative FP | judgement-call FP | all FP | ungraded | mean tokens/trial | mean tool calls/trial |".to_owned());
    out.push("|---|---:|---:|---:|---:|---:|---:|".to_owned());
    for t in &report.tiers {
        out.push(format!(
            "| {} |",
            [
                t.bucket.clone(),
                frac(
                    t.authoritative_only.false_positives,
                    t.authoritative_only.non_defect_trials,
                    t.authoritative_only.false_positive_rate
                ),
                frac(
                    t.judgement_call_only.false_positives,
                    t.judgement_call_only.non_defect_trials,
                    t.judgement_call_only.false_positive_rate
                ),
                frac(
                    t.all.false_positives,
                    t.all.non_defect_trials,
                    t.all.false_positive_rate
                ),
                t.all.ungraded.to_string(),
                num(t.cost.mean_tokens_per_trial),
                num(t.cost.mean_tool_calls_per_trial),
            ]
            .join(" | ")
        ));
    }
    out.push(String::new());

    out.push("## SELF-CONSISTENCY — same item, same tier, replicate disagreement".to_owned());
    out.push("| tier | replicate pairs | flips | flip rate |".to_owned());
    out.push("|---|---:|---:|---:|".to_owned());
    for t in &report.tiers {
        out.push(format!(
            "| {} | {} | {} | {} |",
            t.bucket,
            t.self_consistency.replicate_pairs,
            t.self_consistency.replicate_flips,
            pct_str(t.self_consistency.flip_rate)
        ));
    }
    out.push(
        "A tier with a low FN rate but a high flip rate is not safer — it is lucky.".to_owned(),
    );
    out.push(String::new());

    out.push(
        "## PER-CLASS (the aggregate above is over a deliberately weighted corpus)".to_owned(),
    );
    for t in &report.tiers {
        out.push(format!("### {}", t.bucket));
        out.push(
            "| class | FN (defect-truth trials) | FP (non-defect-truth trials) | ungraded |"
                .to_owned(),
        );
        out.push("|---|---:|---:|---:|".to_owned());
        for row in &t.by_class {
            let r = &row.rates;
            out.push(format!(
                "| {} | {} | {} | {} |",
                row.class,
                frac(r.false_negatives, r.defect_trials, r.false_negative_rate),
                frac(
                    r.false_positives,
                    r.non_defect_trials,
                    r.false_positive_rate
                ),
                r.ungraded
            ));
        }
        out.push(String::new());
    }

    out.push("## TOKEN VOLUME vs BASELINE".to_owned());
    for t in &report.tiers {
        match &t.token_delta {
            None => out.push(format!(
                "- {}: baseline ({} mean tokens/trial, {} mean tool calls/trial).",
                t.bucket,
                num(t.cost.mean_tokens_per_trial),
                num(t.cost.mean_tool_calls_per_trial)
            )),
            Some(d) => out.push(format!(
                "- {}: {} mean tokens/trial vs {} for {} — a delta of {} ({}).",
                t.bucket,
                num(Some(d.mean_tokens_per_trial)),
                num(Some(d.baseline_mean_tokens_per_trial)),
                d.baseline_tier,
                num(Some(d.delta)),
                pct_str(d.percent)
            )),
        }
    }
    out.push(
        "Re-tiering changes PRICE-PER-TOKEN, not token VOLUME. These are volume figures only."
            .to_owned(),
    );
    out.push(String::new());

    if report.tiers.iter().any(|t| t.arm.is_some()) {
        out.push(
            "## TOKENS PER GRADED FINDING (the shape comparison — dispatches vs findings graded)"
                .to_owned(),
        );
        out.push("| bucket | dispatches | graded findings | total tokens | mean tokens/dispatch | mean tokens/graded finding |".to_owned());
        out.push("|---|---:|---:|---:|---:|---:|".to_owned());
        for t in &report.tiers {
            out.push(format!(
                "| {} | {} | {} | {} | {} | {} |",
                t.bucket,
                t.cost.dispatches,
                t.cost.graded_findings,
                num(Some(t.cost.total_tokens)),
                num(t.cost.mean_tokens_per_dispatch),
                num(t.cost.mean_tokens_per_graded_finding)
            ));
        }
        out.push(String::new());
    }

    if let Some(a) = &report.anchoring {
        out.push("## ANCHORING — does batching CORRELATE verdicts?".to_owned());
        out.push(format!(
            "Over qualifying groups only (size >= {}): {} group(s) / {} item(s).",
            a.min_group_size, a.qualifying_groups, a.qualifying_items
        ));
        out.push("| arm | dispatches considered | all-same verdict | all-same share | refuted @pos1 | refuted @pos2..n | rises after pos1 |".to_owned());
        out.push("|---|---:|---:|---:|---:|---:|---|".to_owned());
        for arm in &a.arms {
            let p = &arm.refutation_rate_by_position;
            out.push(format!(
                "| {} | {} | {} | {} | {} | {} | {} |",
                arm.arm,
                arm.dispatches_considered,
                arm.all_same_verdict,
                pct_str(arm.all_same_verdict_share),
                frac(
                    p.first_position.refuted,
                    p.first_position.graded,
                    p.first_position.refuted_rate
                ),
                frac(
                    p.later_positions.refuted,
                    p.later_positions.graded,
                    p.later_positions.refuted_rate
                ),
                if p.rises_after_first { "YES" } else { "no" }
            ));
        }
        out.push(a.note.clone());
        out.push(String::new());
    }

    if let Some(d) = &report.decision
        && report.no_measurement != Some(true)
    {
        out.push(format!("DECISION: {d}"));
        out.push(String::new());
    }
    if report.historical_only_trials > 0 {
        out.push(format!(
            "Excluded from every rate above: {} trial(s) on historical-only severities ({}).",
            report.historical_only_trials,
            HISTORICAL_ONLY_SEVERITIES.join(", ")
        ));
        out.push(String::new());
    }
    if !report.unknown_corpus_ids.is_empty() {
        out.push(format!(
            "WARNING: {} trial corpus id(s) are not in the corpus: {}",
            report.unknown_corpus_ids.len(),
            report.unknown_corpus_ids.join(", ")
        ));
        out.push(String::new());
    }
    out.push("## CAVEATS".to_owned());
    for line in &report.caveats {
        out.push(format!("- {line}"));
    }
    out.join("\n")
}

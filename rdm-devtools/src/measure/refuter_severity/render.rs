//! The `--format text` rendering (byte-compatible with the JS original).

use super::Report;
use crate::measure::jsnum::{fmt_number as n, to_locale_en_us as loc};

/// Renders the report (no trailing newline).
pub fn render_text(report: &Report) -> String {
    let mut out: Vec<String> = Vec::new();
    let c = &report.corpus;
    out.push(format!(
        "Refuter spend by graded finding severity — {} run(s), {} agent record(s)",
        c.run_count, c.agent_record_count
    ));
    out.push(format!(
        "Corpus: {}{}",
        c.projects_root,
        c.until
            .as_deref()
            .map(|u| format!(" (until {u})"))
            .unwrap_or_default()
    ));
    out.push(String::new());
    out.push("| severity | agents | graded | refuted | refuted rate | output | uncached input | cache write | cache read | all tokens | fresh tokens |".to_owned());
    out.push("|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|".to_owned());
    for r in &report.refute_by_severity {
        let b = r.bucket();
        out.push(format!(
            "| {} |",
            [
                r.key.clone(),
                n(r.agent_count),
                r.graded.to_string(),
                r.refuted.to_string(),
                format!("{}%", n(r.refuted_rate)),
                loc(r.output),
                loc(r.uncached_input),
                loc(r.cache_write),
                loc(r.cache_read),
                loc(b.all_tokens()),
                loc(b.fresh_tokens()),
            ]
            .join(" | ")
        ));
    }
    let t = &report.refute_totals;
    out.push(format!(
        "| **refute total** | **{}** | | | | {} | **{}** | **{}** |",
        n(t.agent_count),
        [t.output, t.uncached_input, t.cache_write, t.cache_read]
            .iter()
            .map(|x| loc(*x))
            .collect::<Vec<_>>()
            .join(" | "),
        loc(t.all_tokens()),
        loc(t.fresh_tokens())
    ));
    out.push(String::new());
    out.push(
        "`graded` counts refuters whose returned verdict was recoverable from the transcript; the"
            .to_owned(),
    );
    out.push("rate is over those, not over every dispatched refuter.".to_owned());
    out.push(String::new());
    let p = &report.projected;
    out.push(format!(
        "Projected drop (severities {}, measured over the historical corpus): {} refuter(s) not spawned — {}% of all refuters, {}% of refuter tokens, {}% of all lane tokens.",
        p.severities.join(", "),
        n(p.agents_not_spawned),
        n(p.percent_of_refute_agents),
        n(p.percent_of_refute_tokens),
        n(p.percent_of_lane_tokens)
    ));
    out.push(format!(
        "  {} tokens ({} excluding cache reads).",
        loc(p.all_tokens),
        loc(p.fresh_tokens)
    ));
    out.push(String::new());
    out.push(
        "No post-change lane corpus exists: every run above executed pre-change code. This is an"
            .to_owned(),
    );
    out.push(
        "exact accounting of what non-gating refutation actually cost, not a forecast.".to_owned(),
    );
    out.push(String::new());

    let fp = &report.refuter_fanout.findings_per_finder;
    out.push("Findings per finder, by mode and dimension:".to_owned());
    out.push(String::new());
    out.push("| mode | dim | n | min | p50 | p90 | max | refuters dispatched |".to_owned());
    out.push("|---|---|---:|---:|---:|---:|---:|---:|".to_owned());
    for row in &fp.rows {
        let s = &row.summary;
        out.push(format!(
            "| {} |",
            [
                row.mode.clone(),
                row.dim.clone(),
                s.n.to_string(),
                n(s.min),
                n(s.p50),
                n(s.p90),
                n(s.max),
                row.refuters_dispatched.to_string(),
            ]
            .join(" | ")
        ));
    }
    out.push(String::new());
    out.push(format!(
        "Unreadable finder transcripts: {}. Finders with an unresolved label: {}. Neither contributes to any row above.",
        fp.unreadable_finder_count, fp.unresolved_label_count
    ));
    out.push(String::new());

    let u = &report.refuter_fanout.refuter_counts_by_unit;
    out.push(
        "Refuters dispatched per review unit (unit identity from each refuter's own prompt):"
            .to_owned(),
    );
    out.push(String::new());
    out.push("| units | min | p50 | p90 | max |".to_owned());
    out.push("|---:|---:|---:|---:|---:|".to_owned());
    out.push(format!(
        "| {} | {} | {} | {} | {} |",
        u.summary.n,
        n(u.summary.min),
        n(u.summary.p50),
        n(u.summary.p90),
        n(u.summary.max)
    ));
    out.push(String::new());
    out.push(format!(
        "Unit recovered for {}/{} refuters ({}%); {} unrecoverable (never bucketed into any unit).",
        u.recovered_refuters,
        u.total_refuters,
        n(u.recovery_rate_percent),
        u.unrecoverable_refuter_count
    ));
    out.push(String::new());

    let d = &report.determining_finding_rank;
    out.push(
        "Rank of the outcome-determining finding, over each unit's full CANDIDATE list".to_owned(),
    );
    out.push(
        "(ranking among survivors is degenerate — severity sorts first, so it is always 1):"
            .to_owned(),
    );
    out.push(String::new());
    out.push("| rank | units |".to_owned());
    out.push("|---:|---:|".to_owned());
    for row in &d.rank_histogram {
        out.push(format!("| {} | {} |", row.rank, row.count));
    }
    if d.rank_histogram.is_empty() {
        out.push("| _(no determining units)_ | 0 |".to_owned());
    }
    out.push(String::new());
    out.push(
        "| units | determining | non-determining | unrecoverable | recoverable | recoverable share |"
            .to_owned(),
    );
    out.push("|---:|---:|---:|---:|---:|---:|".to_owned());
    out.push(format!(
        "| {} | {} | {} | {} | {} | {}% |",
        d.units.total,
        d.units.determining,
        d.units.non_determining,
        d.units.unrecoverable,
        d.units.recoverable,
        n(d.units.recoverable_share_percent)
    ));
    out.push(String::new());
    for w in &d.within_top {
        out.push(format!(
            "Determining finding within top {}: {} unit(s) — {}% of determining units, {}% of recoverable units, over a recoverable share of {}% ({}/{} units).",
            w.n,
            w.count,
            n(w.percent_of_determining),
            n(w.percent_of_recoverable),
            n(d.units.recoverable_share_percent),
            d.units.recoverable,
            d.units.total
        ));
    }
    out.push(String::new());
    if !d.unrecoverable_by_reason.is_empty() {
        out.push("| unrecoverable reason | units |".to_owned());
        out.push("|---|---:|".to_owned());
        for row in &d.unrecoverable_by_reason {
            out.push(format!("| {} | {} |", row.reason, row.count));
        }
        out.push(String::new());
    }
    out.push(format!(
        "Orphan agents (unit identity unresolvable, attributable to no unit and therefore invalidating none): {} finder(s), {} refuter(s), across {} run(s).",
        d.orphan_agents.finders, d.orphan_agents.refuters, d.orphan_agents.runs_affected
    ));
    out.push(format!(
        "Units whose `ac` table carried a FAIL/PARTIAL (a second outcome channel this measurement does not score): {}.",
        d.ac_table_gap_units
    ));
    out.push(format!(
        "Tier is not recoverable from either prompt; at the `large` blocker set (blocking+concern) the same corpus yields {} determining unit(s), {}.",
        d.large_tier.units.determining,
        d.large_tier
            .within_top
            .iter()
            .map(|w| format!("{} within top {}", w.count, w.n))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    out.push(format!(
        "Cap verdict (derived, not asserted): {}.",
        d.cap_verdict.verdict
    ));
    out.join("\n")
}

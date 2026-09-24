//! `rdm-measure refuter-severity`: what the lane spent refuting findings that
//! could never have changed an outcome, the review fan-out distributions, and
//! where in a ranked candidate list the outcome-determining finding sits.
//!
//! Replaces `scripts/measure-refuter-severity.mjs`; the three report sections
//! (`nonGatingRefutationSkip`, `refuterFanout`, `determiningFindingRank`) keep
//! their schemas.
//!
//! Each refuter's transcript opens with the prompt `refutePrompt` built, which
//! embeds the whole finding (severity included); joining that back to the
//! sidecar's per-agent usage yields agent counts and all four token classes
//! per severity. The measurement is over the historical corpus: an exact
//! accounting of what was spent, not a forecast.
//!
//! Measurement-owned logic (transcript parsing, the unit-identity rule,
//! aggregation, percentiles, the joins, dispositions, the cap-verdict rule and
//! the doc check/audit arithmetic) is ported here. The ranking and gating rule
//! (`rankFindings`, `survives`, `hasBlocking`, `acTableHasGap`), the dimension
//! table and the non-gating severity set are canonical workflow decisions and
//! are reached through [`ReviewRules`](super::review_rules::ReviewRules).
//!
//! Determinism: no clock, no RNG, no network.

pub mod doc;
pub mod extract;
pub mod fanout;
pub mod rank;
pub mod render;
pub mod transcripts;

use std::collections::HashMap;
use std::path::PathBuf;

use serde::Serialize;

use super::jsjson::{JsValue, js_str_cmp};
use super::jsnum::{pct, serialize_js_number};
use super::review_rules::ReviewRules;
use super::sidecar::{
    AgentRecord, Bucket, RunFilters, aggregate, aggregate_pairs, build_records, filter_until,
    find_workflow_run_files, locate_session_dirs, transcript_path_for,
};
use transcripts::{FinderTranscript, read_finder_transcript, read_refuter_transcript};

/// The six-lane run-set filter `docs/token-baseline.json`'s `runSet` uses.
pub const LANE_WORKFLOWS: [&str; 6] = [
    "autopilot",
    "dispatch-phase",
    "plan-review",
    "backlog",
    "estimate",
    "document",
];

/// The severities a finder may emit, in report order.
pub const SEVERITY_ORDER: [&str; 3] = ["blocking", "concern", "suggestion"];

/// A refuter with no transcript: its severity is unknown, never non-gating.
pub const NO_TRANSCRIPT: &str = "unrecoverable:no-transcript";
/// A refuter whose finding could not be recovered from its prompt.
pub const UNPARSEABLE: &str = "unrecoverable:unparseable";

/// A refuter record with what its transcript recovered.
#[derive(Debug, Clone)]
pub struct Refuter {
    /// The sidecar record.
    pub record: AgentRecord,
    /// The graded finding's severity, or one of the unrecoverable buckets.
    pub severity: String,
    /// The returned verdict.
    pub refuted: Option<bool>,
    /// The prompt-derived dimension (only when the severity was recovered).
    pub dim_key: Option<String>,
    /// The prompt-derived unit identity (only when the severity was recovered).
    pub unit_ident: Option<String>,
    /// The graded finding (only when the severity was recovered).
    pub finding: Option<JsValue>,
}

/// A finder record with its transcript (`None`: no transcript to read).
#[derive(Debug, Clone)]
pub struct Finder {
    /// The sidecar record.
    pub record: AgentRecord,
    /// The transcript, when one exists.
    pub transcript: Option<FinderTranscript>,
}

/// The corpus a report was measured over.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Corpus {
    /// The sidecar root, as given.
    pub projects_root: String,
    /// The `--until` window, when one applied.
    pub until: Option<String>,
    /// The workflow filter.
    pub lane_workflows: Vec<String>,
    /// In-scope runs.
    pub run_count: usize,
    /// Agent records.
    pub agent_record_count: usize,
}

/// One severity row: the bucket plus its verdict tally.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SeverityRow {
    /// The severity (or unrecoverable bucket).
    pub key: String,
    /// Refuters.
    #[serde(serialize_with = "serialize_js_number")]
    pub agent_count: f64,
    /// Deduped requests.
    #[serde(serialize_with = "serialize_js_number")]
    pub deduped_request_count: f64,
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
    /// Refuters with a recoverable verdict.
    pub graded: usize,
    /// Of those, `refuted: true`.
    pub refuted: usize,
    /// `refuted / graded`, one-decimal percent.
    #[serde(serialize_with = "serialize_js_number")]
    pub refuted_rate: f64,
}

impl SeverityRow {
    /// The row as an aggregation bucket.
    pub fn bucket(&self) -> Bucket {
        Bucket {
            key: self.key.clone(),
            agent_count: self.agent_count,
            deduped_request_count: self.deduped_request_count,
            output: self.output,
            uncached_input: self.uncached_input,
            cache_write: self.cache_write,
            cache_read: self.cache_read,
        }
    }
}

/// A bucket without its key.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Totals {
    /// Agents.
    #[serde(serialize_with = "serialize_js_number")]
    pub agent_count: f64,
    /// Deduped requests.
    #[serde(serialize_with = "serialize_js_number")]
    pub deduped_request_count: f64,
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
}

impl From<&Bucket> for Totals {
    fn from(b: &Bucket) -> Self {
        Self {
            agent_count: b.agent_count,
            deduped_request_count: b.deduped_request_count,
            output: b.output,
            uncached_input: b.uncached_input,
            cache_write: b.cache_write,
            cache_read: b.cache_read,
        }
    }
}

impl Totals {
    /// All four token classes.
    pub fn all_tokens(&self) -> f64 {
        self.output + self.uncached_input + self.cache_write + self.cache_read
    }

    /// Output + uncached input + cache write.
    pub fn fresh_tokens(&self) -> f64 {
        self.output + self.uncached_input + self.cache_write
    }
}

/// The drop the non-gating skip would have produced over the corpus.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Projected {
    /// The non-gating severities projected away.
    pub severities: Vec<String>,
    /// Refuters that would not have been spawned.
    #[serde(serialize_with = "serialize_js_number")]
    pub agents_not_spawned: f64,
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
    pub all_tokens: f64,
    /// Cache reads excluded.
    #[serde(serialize_with = "serialize_js_number")]
    pub fresh_tokens: f64,
    /// Share of refuter agents.
    #[serde(serialize_with = "serialize_js_number")]
    pub percent_of_refute_agents: f64,
    /// Share of refuter tokens.
    #[serde(serialize_with = "serialize_js_number")]
    pub percent_of_refute_tokens: f64,
    /// Share of all lane tokens.
    #[serde(serialize_with = "serialize_js_number")]
    pub percent_of_lane_tokens: f64,
}

/// The whole report.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    /// The measured corpus.
    pub corpus: Corpus,
    /// Refuter spend by graded severity.
    pub refute_by_severity: Vec<SeverityRow>,
    /// All refuters.
    pub refute_totals: Totals,
    /// Every lane agent.
    pub lane_totals: Totals,
    /// The non-gating projection.
    pub projected: Projected,
    /// Findings per finder and refuters per unit.
    pub refuter_fanout: fanout::Fanout,
    /// Where the determining finding ranks.
    pub determining_finding_rank: rank::RankBlock,
}

/// `{ instrument, ...report }` for `--format json`.
#[derive(Debug, Serialize)]
pub struct InstrumentReport<'a> {
    /// The producing command.
    pub instrument: &'static str,
    /// The report.
    #[serde(flatten)]
    pub report: &'a Report,
}

/// The `instrument` value this tool writes.
pub const INSTRUMENT: &str = "rdm-measure refuter-severity";

/// Folds each severity row's verdict tally in beside its token columns.
pub fn with_verdict_rates(rows: &[Bucket], refuters: &[Refuter]) -> Vec<SeverityRow> {
    rows.iter()
        .map(|row| {
            let mut graded = 0;
            let mut refuted = 0;
            for r in refuters.iter().filter(|r| r.severity == row.key) {
                if let Some(v) = r.refuted {
                    graded += 1;
                    if v {
                        refuted += 1;
                    }
                }
            }
            #[allow(clippy::cast_precision_loss)]
            let rate = pct(refuted as f64, graded as f64);
            SeverityRow {
                key: row.key.clone(),
                agent_count: row.agent_count,
                deduped_request_count: row.deduped_request_count,
                output: row.output,
                uncached_input: row.uncached_input,
                cache_write: row.cache_write,
                cache_read: row.cache_read,
                graded,
                refuted,
                refuted_rate: rate,
            }
        })
        .collect()
}

/// Report order: the three real severities, then any unexpected key, then the
/// two unrecoverable buckets.
pub fn sort_rows(rows: &mut [SeverityRow]) {
    let rank = |k: &str| -> i32 {
        if let Some(i) = SEVERITY_ORDER.iter().position(|s| *s == k) {
            return i32::try_from(i).unwrap_or(0);
        }
        match k {
            NO_TRANSCRIPT => 100,
            UNPARSEABLE => 101,
            _ => 50,
        }
    };
    rows.sort_by(|a, b| {
        rank(&a.key)
            .cmp(&rank(&b.key))
            .then_with(|| js_str_cmp(&a.key, &b.key))
    });
}

/// The drop over the rows whose severity is non-gating; unrecoverable rows
/// are never included (an unknown severity is not a non-gating one).
pub fn project_drop(
    rows: &[Bucket],
    refute_totals: &Totals,
    lane_totals: &Totals,
    non_gating: &[String],
) -> Projected {
    let mut sum = Bucket::empty("dropped");
    for r in rows.iter().filter(|r| non_gating.contains(&r.key)) {
        sum.agent_count += r.agent_count;
        sum.deduped_request_count += r.deduped_request_count;
        sum.output += r.output;
        sum.uncached_input += r.uncached_input;
        sum.cache_write += r.cache_write;
        sum.cache_read += r.cache_read;
    }
    Projected {
        severities: non_gating.to_vec(),
        agents_not_spawned: sum.agent_count,
        output: sum.output,
        uncached_input: sum.uncached_input,
        cache_write: sum.cache_write,
        cache_read: sum.cache_read,
        all_tokens: sum.all_tokens(),
        fresh_tokens: sum.fresh_tokens(),
        percent_of_refute_agents: pct(sum.agent_count, refute_totals.agent_count),
        percent_of_refute_tokens: pct(sum.all_tokens(), refute_totals.all_tokens()),
        percent_of_lane_tokens: pct(sum.all_tokens(), lane_totals.all_tokens()),
    }
}

/// Options for [`measure`].
#[derive(Debug, Clone)]
pub struct MeasureOptions {
    /// The sidecar root.
    pub root: PathBuf,
    /// The root as it should be reported.
    pub root_display: String,
    /// Ignore runs starting after this instant.
    pub until: Option<String>,
}

/// Reads every refuter and finder transcript for `records`.
pub fn classify(
    records: &[AgentRecord],
    session_dir_of: &HashMap<String, PathBuf>,
) -> (Vec<Refuter>, Vec<Finder>) {
    let transcript = |r: &AgentRecord| -> Option<PathBuf> {
        let dir = session_dir_of.get(&r.run_key())?;
        let id = r.agent_id.as_deref()?;
        Some(transcript_path_for(dir, &r.run_id, id)).filter(|p| p.exists())
    };
    let mut refuters = Vec::new();
    let mut finders = Vec::new();
    for r in records {
        match r.agent_class.as_str() {
            "refute" => {
                let mut out = Refuter {
                    record: r.clone(),
                    severity: NO_TRANSCRIPT.to_owned(),
                    refuted: None,
                    dim_key: None,
                    unit_ident: None,
                    finding: None,
                };
                if let Some(path) = transcript(r) {
                    let t = read_refuter_transcript(&path);
                    out.refuted = t.refuted;
                    match t.severity {
                        None => out.severity = UNPARSEABLE.to_owned(),
                        Some(sev) => {
                            // An unrecovered severity means the header could
                            // not be trusted, so neither can its context.
                            out.severity = sev;
                            out.dim_key = t.context.dim_key;
                            out.unit_ident = t.context.unit_ident;
                            out.finding = t.finding;
                        }
                    }
                }
                refuters.push(out);
            }
            "find" => finders.push(Finder {
                record: r.clone(),
                transcript: transcript(r).map(|p| read_finder_transcript(&p)),
            }),
            _ => {}
        }
    }
    (refuters, finders)
}

/// Measures the corpus under `options`.
///
/// # Errors
///
/// When the root cannot be read, `until` is not a date, or a canonical review
/// call fails.
pub fn measure(options: &MeasureOptions, rules: &mut dyn ReviewRules) -> Result<Report, String> {
    let non_gating = rules.non_gating_severities()?;
    let coverage = rank::coverage_dimensions(rules)?;
    let sessions = locate_session_dirs(&options.root)?;
    let filters = RunFilters {
        since: None,
        workflow_names: LANE_WORKFLOWS.iter().map(|s| (*s).to_owned()).collect(),
    };
    let mut runs = find_workflow_run_files(&sessions, &filters, &mut |_| {})?;
    if let Some(until) = options.until.as_deref().filter(|u| !u.is_empty()) {
        runs = filter_until(runs, until)?;
    }
    let mut session_dir_of = HashMap::new();
    for rf in &runs {
        session_dir_of.insert(rf.run_key(), rf.session_dir.clone());
    }
    let records = build_records(&runs, &mut |_| {});
    let (refuters, finders) = classify(&records, &session_dir_of);

    let buckets = aggregate_pairs(refuters.iter().map(|r| (r.severity.clone(), &r.record)));
    let mut rows = with_verdict_rates(&buckets, &refuters);
    let lane = aggregate(&records, |_| "all".to_owned())
        .into_iter()
        .next()
        .unwrap_or_else(|| Bucket::empty("all"));
    let refute = aggregate(refuters.iter().map(|r| &r.record), |_| "refute".to_owned())
        .into_iter()
        .next()
        .unwrap_or_else(|| Bucket::empty("refute"));
    let refute_totals = Totals::from(&refute);
    let lane_totals = Totals::from(&lane);
    let projected = project_drop(&buckets, &refute_totals, &lane_totals, &non_gating);
    sort_rows(&mut rows);
    let refuter_fanout = fanout::build(&finders, &refuters);
    let determining_finding_rank = rank::build(&finders, &refuters, rules, &non_gating, &coverage)?;
    Ok(Report {
        corpus: Corpus {
            projects_root: options.root_display.clone(),
            until: options.until.clone().filter(|u| !u.is_empty()),
            lane_workflows: LANE_WORKFLOWS.iter().map(|s| (*s).to_owned()).collect(),
            run_count: runs.len(),
            agent_record_count: records.len(),
        },
        refute_by_severity: rows,
        refute_totals,
        lane_totals,
        projected,
        refuter_fanout,
        determining_finding_rank,
    })
}

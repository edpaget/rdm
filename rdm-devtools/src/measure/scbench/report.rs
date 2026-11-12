//! `rdm-measure scbench report`: aggregates one run's SCBench outputs per arm,
//! with a seed-bootstrap confidence interval on every figure, sets each rdm
//! arm against each control arm, and optionally saves or compares against a
//! baseline ([`super::baseline`]).
//!
//! The report reads the run record (`<run>/rdm-scbench-run.json`) through its
//! own lenient view, so a phase 1-4 record (no `schema`, one entry per arm)
//! is still read, then each included unit's checkpoint tree
//! ([`super::outputs`]). It writes `<run>/rdm-scbench-report.json` (the
//! [`Report`]) and `<run>/rdm-scbench-report.md` (rendered from it), and
//! prints the chosen format. It never gates: the exit code is 0 whenever a
//! report was produced.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::baseline::{self, BaselineComparison};
use super::exec::write_file;
use super::lane::{Outcome, Reason};
use super::manifest::{self, ArmGroup, ArmImage};
use super::outputs::{self, EROSION, PassPolicy, ProblemOutput};
use super::plan::RECORD_FILE;
use super::stats;

/// The report JSON's file name, beside the run record.
pub const REPORT_JSON: &str = "rdm-scbench-report.json";
/// The rendered markdown report's file name, beside the run record.
pub const REPORT_MD: &str = "rdm-scbench-report.md";
/// The report JSON's schema version.
pub const REPORT_SCHEMA: u32 = 1;

/// What `scbench report` prints to stdout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// The rendered markdown (`rdm-scbench-report.md`).
    Markdown,
    /// The report JSON (`rdm-scbench-report.json`).
    Json,
}

/// Options for [`run`].
#[derive(Debug, Clone)]
pub struct ReportOptions {
    /// The run directory (`--out` of `scbench run`).
    pub run: PathBuf,
    /// A baseline to render the run beside (`--baseline`).
    pub baseline: Option<String>,
    /// Save the run as this baseline (`--save-baseline`).
    pub save_baseline: Option<String>,
    /// What to print.
    pub format: Format,
}

/// The five pins a run and a baseline are keyed by.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pins {
    /// The rdm checkout under test (the treatment).
    pub rdm_sha: Option<String>,
    /// The SCBench commit.
    pub scbench_sha: Option<String>,
    /// The `scb-problems` commit.
    pub problems_sha: Option<String>,
    /// The SCBench model name.
    pub model: Option<String>,
    /// The Claude Code version both agent images install.
    pub claude_code_version: Option<String>,
}

impl Pins {
    /// Every pin by name, in report order.
    pub fn named(&self) -> [(&'static str, Option<&str>); 5] {
        [
            ("rdm_sha", self.rdm_sha.as_deref()),
            ("scbench_sha", self.scbench_sha.as_deref()),
            ("problems_sha", self.problems_sha.as_deref()),
            ("model", self.model.as_deref()),
            ("claude_code_version", self.claude_code_version.as_deref()),
        ]
    }
}

/// How every interval was drawn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bootstrap {
    /// The resample unit (`seed`).
    pub unit: String,
    /// The interval method (`percentile`).
    pub method: String,
    /// Resamples per statistic.
    pub resamples: usize,
    /// The confidence level.
    pub level: f64,
    /// The generator (`splitmix64`).
    pub rng: String,
    /// How each statistic's generator is seeded from its key.
    pub seed_key: String,
}

/// One summary metric and which direction is better.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetricInfo {
    /// The metric key.
    pub key: String,
    /// `higher`, `lower`, or `null` for a context-only metric (`sloc`).
    pub better: Option<String>,
}

/// One per-seed value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sample {
    /// The seed (`null` for a phase 1-4 record, which has none).
    pub seed: Option<u64>,
    /// The value.
    pub value: f64,
}

/// A metric over an arm's seeds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stat {
    /// The plain sample mean.
    pub mean: f64,
    /// The interval's lower bound; `null` when `n < 2`.
    pub ci_low: Option<f64>,
    /// The interval's upper bound; `null` when `n < 2`.
    pub ci_high: Option<f64>,
    /// The number of seeds with a value.
    pub n: usize,
    /// Each seed's value, in seed order.
    pub samples: Vec<Sample>,
}

impl Stat {
    /// The values of [`Stat::samples`].
    pub fn values(&self) -> Vec<f64> {
        self.samples.iter().map(|s| s.value).collect()
    }
}

/// A difference of two groups' means, `a - b`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Delta {
    /// `mean(a) - mean(b)`.
    pub estimate: f64,
    /// The unpaired bootstrap interval's lower bound; `null` when either
    /// group has fewer than two seeds.
    pub ci_low: Option<f64>,
    /// The interval's upper bound.
    pub ci_high: Option<f64>,
    /// Group `a`'s seeds.
    pub n_a: usize,
    /// Group `b`'s seeds.
    pub n_b: usize,
    /// Whether the interval excludes 0 (a reading aid, not a
    /// multiple-comparison-corrected test).
    pub excludes_zero: bool,
}

/// One checkpoint of one problem, over an arm's seeds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckpointReport {
    /// The problem.
    pub problem: String,
    /// The checkpoint's directory name.
    pub checkpoint: String,
    /// Its `order`.
    pub order: i64,
    /// 1 when the checkpoint passed on a seed, 0 when not.
    pub passed: Option<Stat>,
    /// Each erosion metric at this checkpoint.
    pub metrics: BTreeMap<String, Option<Stat>>,
    /// Each erosion metric minus its value at the previous checkpoint of the
    /// same problem and seed.
    pub delta: BTreeMap<String, Option<Stat>>,
}

/// One arm's figures.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArmReport {
    /// The arm name.
    pub name: String,
    /// `rdm` or `control`.
    pub group: ArmGroup,
    /// The agent image (`stock` or `rdm`), when recorded.
    pub image: Option<ArmImage>,
    /// The seeds included, in order.
    pub seeds: Vec<Option<u64>>,
    /// Every summary metric ([`Report::metrics`]); `null` when no seed has a
    /// value.
    pub summary: BTreeMap<String, Option<Stat>>,
    /// Lane failures per reason, summed over seeds; `null` for an arm with no
    /// lane record.
    pub lane_failure_reasons: Option<BTreeMap<String, u64>>,
    /// Every checkpoint, problem by problem, in `(order, name)` order.
    pub checkpoints: Vec<CheckpointReport>,
}

/// One rdm arm against one control arm.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Comparison {
    /// The rdm arm.
    pub rdm: String,
    /// The control arm.
    pub control: String,
    /// Every summary metric's `rdm - control`; `null` when either side has
    /// no value.
    pub metrics: BTreeMap<String, Option<Delta>>,
}

/// A unit the report left out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Skipped {
    /// The arm.
    pub arm: String,
    /// The seed.
    pub seed: Option<u64>,
    /// Why it was left out.
    pub reason: String,
}

/// The whole report (`rdm-scbench-report.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    /// [`REPORT_SCHEMA`].
    pub schema: u32,
    /// The run directory.
    pub run: PathBuf,
    /// The run's pins.
    pub pins: Pins,
    /// The pass policy every checkpoint was judged by.
    pub pass_policy: String,
    /// The bootstrap parameters.
    pub bootstrap: Bootstrap,
    /// The summary metrics, in order.
    pub metrics: Vec<MetricInfo>,
    /// Each arm, in manifest order.
    pub arms: Vec<ArmReport>,
    /// Every rdm arm against every control arm, in arm order.
    pub comparisons: Vec<Comparison>,
    /// Units left out of every figure.
    pub skipped_runs: Vec<Skipped>,
    /// The comparison with `--baseline`, when given.
    pub baseline: Option<BaselineComparison>,
}

// --- The run record, read leniently --------------------------------------

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(super) struct RecordView {
    schema: Option<u32>,
    manifest: ManifestView,
    pub(super) manifest_path: Option<PathBuf>,
    rdm: ShaView,
    pub(super) observed: ObservedView,
    arms: Vec<ArmView>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ManifestView {
    scbench: ShaView,
    problems: ProblemsView,
    agent: AgentView,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ShaView {
    sha: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ProblemsView {
    sha: Option<String>,
    subset: Vec<String>,
    checkpoints: Option<BTreeMap<String, u64>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct AgentView {
    model: Option<String>,
    claude_code_version: Option<String>,
    pass_policy: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(super) struct ObservedView {
    pub(super) claude: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ArmView {
    name: String,
    group: Option<ArmGroup>,
    image: Option<ArmImage>,
    seed: Option<u64>,
    run_dir: PathBuf,
    lane: Option<LaneView>,
    finished: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct LaneView {
    problems: Vec<LaneProblemView>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct LaneProblemView {
    checkpoints: Vec<LaneCheckpointView>,
}

#[derive(Debug, Clone, Deserialize)]
struct LaneCheckpointView {
    outcome: Outcome,
    #[serde(default)]
    reason: Option<Reason>,
}

impl LaneView {
    fn failures(&self) -> impl Iterator<Item = &LaneCheckpointView> {
        self.problems
            .iter()
            .flat_map(|p| &p.checkpoints)
            .filter(|c| c.outcome == Outcome::LaneFailure)
    }
}

// --- Metrics ---------------------------------------------------------------

const PASS_RATE: &str = "pass_rate";
const LANE_FAILURES: &str = "lane_failures";
const USAGE: [&str; 9] = [
    "cost_usd",
    "tokens_input",
    "tokens_output",
    "tokens_cache_read",
    "tokens_cache_write",
    "tokens_total",
    "steps",
    "wall_clock_seconds",
    LANE_FAILURES,
];

fn erosion_better(e: &str) -> Option<&'static str> {
    match e {
        "sloc" => None,
        "mi_mean" => Some("higher"),
        _ => Some("lower"),
    }
}

/// Every summary metric and its better direction, in report order.
fn metric_infos() -> Vec<MetricInfo> {
    let info = |key: String, better: Option<&str>| MetricInfo {
        key,
        better: better.map(str::to_owned),
    };
    let mut out = vec![info(PASS_RATE.to_owned(), Some("higher"))];
    for suffix in ["final", "change"] {
        for e in EROSION {
            out.push(info(format!("{e}_{suffix}"), erosion_better(e)));
        }
    }
    for key in USAGE {
        out.push(info(key.to_owned(), Some("lower")));
    }
    out
}

/// One unit: one arm at one seed, read from its checkpoint tree.
struct Unit {
    seed: Option<u64>,
    problems: Vec<ProblemOutput>,
    lane: Option<LaneView>,
}

struct ArmUnits {
    name: String,
    group: ArmGroup,
    image: Option<ArmImage>,
    units: Vec<Unit>,
}

/// The unit's value of every summary metric it has.
fn unit_values(unit: &Unit, expected: Option<&BTreeMap<String, u64>>) -> BTreeMap<String, f64> {
    let mut v = BTreeMap::new();
    let mut passed = 0u64;
    let mut denominator = 0u64;
    for p in &unit.problems {
        let observed = p.checkpoints.len() as u64;
        let want = expected
            .and_then(|m| m.get(&p.problem))
            .copied()
            .unwrap_or(observed);
        denominator += want.max(observed);
        passed += p.checkpoints.iter().filter(|c| c.passed).count() as u64;
    }
    if denominator > 0 {
        #[allow(clippy::cast_precision_loss)]
        v.insert(PASS_RATE.to_owned(), passed as f64 / denominator as f64);
    }
    for e in EROSION {
        let (mut finals, mut changes) = (Vec::new(), Vec::new());
        for p in &unit.problems {
            let values: Vec<f64> = p
                .checkpoints
                .iter()
                .filter_map(|c| c.quality.as_ref()?.get(e))
                .collect();
            if let (Some(first), Some(last)) = (values.first(), values.last()) {
                finals.push(*last);
                changes.push(last - first);
            }
        }
        if let Some(m) = stats::mean(&finals) {
            v.insert(format!("{e}_final"), m);
        }
        if let Some(m) = stats::mean(&changes) {
            v.insert(format!("{e}_change"), m);
        }
    }
    let inferences: Vec<_> = unit
        .problems
        .iter()
        .flat_map(|p| &p.checkpoints)
        .filter_map(|c| c.inference)
        .collect();
    if !inferences.is_empty() {
        let sum = |f: &dyn Fn(&outputs::Inference) -> f64| inferences.iter().map(f).sum::<f64>();
        let tokens = [
            ("tokens_input", sum(&|i| i.tokens_input)),
            ("tokens_output", sum(&|i| i.tokens_output)),
            ("tokens_cache_read", sum(&|i| i.tokens_cache_read)),
            ("tokens_cache_write", sum(&|i| i.tokens_cache_write)),
        ];
        v.insert("cost_usd".to_owned(), sum(&|i| i.cost));
        v.insert("steps".to_owned(), sum(&|i| i.steps));
        v.insert("wall_clock_seconds".to_owned(), sum(&|i| i.elapsed));
        v.insert(
            "tokens_total".to_owned(),
            tokens.iter().map(|(_, t)| t).sum(),
        );
        for (k, t) in tokens {
            v.insert(k.to_owned(), t);
        }
    }
    if let Some(lane) = &unit.lane {
        #[allow(clippy::cast_precision_loss)]
        v.insert(LANE_FAILURES.to_owned(), lane.failures().count() as f64);
    }
    v
}

fn stat(key: &str, samples: Vec<Sample>) -> Option<Stat> {
    let xs: Vec<f64> = samples.iter().map(|s| s.value).collect();
    let mean = stats::mean(&xs)?;
    let ci = stats::mean_ci(key, &xs);
    Some(Stat {
        mean,
        ci_low: ci.map(|c| c.0),
        ci_high: ci.map(|c| c.1),
        n: xs.len(),
        samples,
    })
}

/// `a - b` over two stats' samples, with the unpaired bootstrap interval
/// drawn from `key`'s stream; `None` when either side is missing.
pub(super) fn delta(key: &str, a: Option<&Stat>, b: Option<&Stat>) -> Option<Delta> {
    let (a, b) = (a?, b?);
    let ci = stats::delta_ci(key, &a.values(), &b.values());
    Some(Delta {
        estimate: a.mean - b.mean,
        ci_low: ci.map(|c| c.0),
        ci_high: ci.map(|c| c.1),
        n_a: a.n,
        n_b: b.n,
        excludes_zero: ci.is_some_and(|(lo, hi)| lo > 0.0 || hi < 0.0),
    })
}

/// One unit's per-checkpoint values: `(passed, metrics, deltas)`.
type CheckpointValues = (
    f64,
    BTreeMap<&'static str, f64>,
    BTreeMap<&'static str, f64>,
);

/// One checkpoint's values on each seed that has it.
type SeedValues = Vec<(Option<u64>, CheckpointValues)>;

fn checkpoint_values(problem: &ProblemOutput) -> Vec<(String, i64, CheckpointValues)> {
    let mut prev: Option<BTreeMap<&'static str, f64>> = None;
    let mut out = Vec::new();
    for c in &problem.checkpoints {
        let metrics: BTreeMap<&'static str, f64> = EROSION
            .iter()
            .filter_map(|&e| Some((e, c.quality.as_ref()?.get(e)?)))
            .collect();
        let deltas = prev.as_ref().map_or_else(BTreeMap::new, |p| {
            metrics
                .iter()
                .filter_map(|(&e, &x)| Some((e, x - p.get(e)?)))
                .collect()
        });
        out.push((
            c.name.clone(),
            c.order,
            (if c.passed { 1.0 } else { 0.0 }, metrics.clone(), deltas),
        ));
        prev = Some(metrics);
    }
    out
}

fn arm_checkpoints(arm: &ArmUnits, subset: &[String]) -> Vec<CheckpointReport> {
    let mut out = Vec::new();
    for problem in subset {
        // (order, name) -> per-seed values
        let mut rows: BTreeMap<(i64, String), SeedValues> = BTreeMap::new();
        for unit in &arm.units {
            let Some(p) = unit.problems.iter().find(|p| &p.problem == problem) else {
                continue;
            };
            for (name, order, values) in checkpoint_values(p) {
                rows.entry((order, name))
                    .or_default()
                    .push((unit.seed, values));
            }
        }
        for ((order, name), seeds) in rows {
            let key = |metric: &str| format!("checkpoint/{}/{problem}/{name}/{metric}", arm.name);
            let passed = stat(
                &key("passed"),
                seeds
                    .iter()
                    .map(|(seed, v)| Sample {
                        seed: *seed,
                        value: v.0,
                    })
                    .collect(),
            );
            let mut metrics = BTreeMap::new();
            let mut deltas = BTreeMap::new();
            for e in EROSION {
                let pick = |which: usize| -> Vec<Sample> {
                    seeds
                        .iter()
                        .filter_map(|(seed, v)| {
                            let m = if which == 0 { &v.1 } else { &v.2 };
                            Some(Sample {
                                seed: *seed,
                                value: *m.get(e)?,
                            })
                        })
                        .collect()
                };
                metrics.insert(e.to_owned(), stat(&key(e), pick(0)));
                deltas.insert(e.to_owned(), stat(&key(&format!("delta.{e}")), pick(1)));
            }
            out.push(CheckpointReport {
                problem: problem.clone(),
                checkpoint: name,
                order,
                passed,
                metrics,
                delta: deltas,
            });
        }
    }
    out
}

/// Reads the run record at `<run>/rdm-scbench-run.json`.
///
/// # Errors
///
/// When it cannot be read or parsed, naming it.
pub(super) fn read_record(run: &Path) -> Result<RecordView, String> {
    let path = run.join(RECORD_FILE);
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("--run: cannot read the run record {}: {e}", path.display()))?;
    serde_json::from_str(&text)
        .map_err(|e| format!("--run: cannot parse the run record {}: {e}", path.display()))
}

/// Builds the report of the run at `run` from its record.
///
/// # Errors
///
/// When the record names no or an unknown pass policy, or a unit's SCBench
/// output cannot be read ([`outputs::read_unit`]).
pub(super) fn build(run: &Path, record: &RecordView) -> Result<Report, String> {
    let m = &record.manifest;
    let policy_name = m.agent.pass_policy.clone().ok_or_else(|| {
        format!(
            "the run record {} has no manifest.agent.pass_policy",
            run.join(RECORD_FILE).display()
        )
    })?;
    let policy =
        PassPolicy::parse(&policy_name).map_err(|e| format!("manifest.agent.pass_policy: {e}"))?;
    let schema2 = record.schema.is_some_and(|s| s >= 2);
    let mut arms: Vec<ArmUnits> = Vec::new();
    let mut skipped = Vec::new();
    for a in &record.arms {
        let included = if schema2 {
            a.finished.is_some()
        } else {
            a.run_dir.is_dir()
        };
        if !included {
            skipped.push(Skipped {
                arm: a.name.clone(),
                seed: a.seed,
                reason: if schema2 {
                    "not finished".to_owned()
                } else {
                    format!("no run directory {}", a.run_dir.display())
                },
            });
            continue;
        }
        let problems = outputs::read_unit(&a.run_dir, &m.problems.subset, policy)?;
        let idx = arms
            .iter()
            .position(|x| x.name == a.name)
            .unwrap_or_else(|| {
                // A phase 1-4 record has no `group`; one with no `image`
                // ran the stock image.
                arms.push(ArmUnits {
                    name: a.name.clone(),
                    group: manifest::group_for(a.group, a.image.unwrap_or_default()),
                    image: a.image,
                    units: Vec::new(),
                });
                arms.len() - 1
            });
        arms[idx].units.push(Unit {
            seed: a.seed,
            problems,
            lane: a.lane.clone(),
        });
    }
    for arm in &mut arms {
        arm.units.sort_by_key(|u| u.seed);
    }

    let infos = metric_infos();
    let mut reports = Vec::new();
    for arm in &arms {
        let values: Vec<(Option<u64>, BTreeMap<String, f64>)> = arm
            .units
            .iter()
            .map(|u| (u.seed, unit_values(u, m.problems.checkpoints.as_ref())))
            .collect();
        let summary = infos
            .iter()
            .map(|info| {
                let samples = values
                    .iter()
                    .filter_map(|(seed, v)| {
                        Some(Sample {
                            seed: *seed,
                            value: *v.get(&info.key)?,
                        })
                    })
                    .collect();
                (
                    info.key.clone(),
                    stat(&format!("arm/{}/{}", arm.name, info.key), samples),
                )
            })
            .collect();
        let lanes: Vec<&LaneView> = arm.units.iter().filter_map(|u| u.lane.as_ref()).collect();
        let lane_failure_reasons = (!lanes.is_empty()).then(|| {
            let mut reasons = BTreeMap::new();
            for c in lanes.iter().flat_map(|l| l.failures()) {
                let reason = c.reason.map_or("unknown", Reason::as_str);
                *reasons.entry(reason.to_owned()).or_insert(0) += 1;
            }
            reasons
        });
        reports.push(ArmReport {
            name: arm.name.clone(),
            group: arm.group,
            image: arm.image,
            seeds: arm.units.iter().map(|u| u.seed).collect(),
            summary,
            lane_failure_reasons,
            checkpoints: arm_checkpoints(arm, &m.problems.subset),
        });
    }

    let mut comparisons = Vec::new();
    for r in reports.iter().filter(|a| a.group == ArmGroup::Rdm) {
        for c in reports.iter().filter(|a| a.group == ArmGroup::Control) {
            let metrics = infos
                .iter()
                .map(|info| {
                    let key = format!("delta/{}/{}/{}", r.name, c.name, info.key);
                    let d = delta(
                        &key,
                        r.summary.get(&info.key).and_then(Option::as_ref),
                        c.summary.get(&info.key).and_then(Option::as_ref),
                    );
                    (info.key.clone(), d)
                })
                .collect();
            comparisons.push(Comparison {
                rdm: r.name.clone(),
                control: c.name.clone(),
                metrics,
            });
        }
    }

    Ok(Report {
        schema: REPORT_SCHEMA,
        run: run.to_path_buf(),
        pins: Pins {
            rdm_sha: record.rdm.sha.clone(),
            scbench_sha: m.scbench.sha.clone(),
            problems_sha: m.problems.sha.clone(),
            model: m.agent.model.clone(),
            claude_code_version: m.agent.claude_code_version.clone(),
        },
        pass_policy: policy_name,
        bootstrap: Bootstrap {
            unit: "seed".to_owned(),
            method: "percentile".to_owned(),
            resamples: stats::RESAMPLES,
            level: stats::LEVEL,
            rng: "splitmix64".to_owned(),
            seed_key: "sha256(\"rdm-scbench-bootstrap\\0\" + key)[0..8] LE".to_owned(),
        },
        metrics: infos,
        arms: reports,
        comparisons,
        skipped_runs: skipped,
        baseline: None,
    })
}

// --- Markdown --------------------------------------------------------------

/// `x` with at most three decimals, trailing zeros dropped.
fn num(x: f64) -> String {
    let s = format!("{x:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" {
        "0".to_owned()
    } else {
        s.to_owned()
    }
}

fn signed(x: f64) -> String {
    let s = num(x);
    if x > 0.0 && s != "0" {
        format!("+{s}")
    } else {
        s
    }
}

/// `mean [lo, hi]`, or `mean` without an interval, or `—`.
pub(super) fn cell(s: Option<&Stat>) -> String {
    match s {
        None => "—".to_owned(),
        Some(s) => match (s.ci_low, s.ci_high) {
            (Some(lo), Some(hi)) => format!("{} [{}, {}]", num(s.mean), num(lo), num(hi)),
            _ => num(s.mean),
        },
    }
}

/// `Δ [lo, hi]`, starred when the interval excludes 0, or `—`.
pub(super) fn delta_cell(d: Option<&Delta>) -> String {
    match d {
        None => "—".to_owned(),
        Some(d) => {
            let ci = match (d.ci_low, d.ci_high) {
                (Some(lo), Some(hi)) => format!(" [{}, {}]", signed(lo), signed(hi)),
                _ => String::new(),
            };
            let star = if d.excludes_zero { " *" } else { "" };
            format!("{}{ci}{star}", signed(d.estimate))
        }
    }
}

fn summary_of<'a>(arm: &'a ArmReport, key: &str) -> Option<&'a Stat> {
    arm.summary.get(key).and_then(Option::as_ref)
}

pub(super) fn table(out: &mut String, header: &[String], rows: &[Vec<String>]) {
    let line = |cells: &[String]| format!("| {} |\n", cells.join(" | "));
    out.push_str(&line(header));
    out.push_str(&line(&vec!["---".to_owned(); header.len()]));
    for row in rows {
        out.push_str(&line(row));
    }
    out.push('\n');
}

fn strings(xs: &[&str]) -> Vec<String> {
    xs.iter().map(|&x| x.to_owned()).collect()
}

/// Renders `report` as markdown.
pub fn render_markdown(report: &Report) -> String {
    let mut out = String::new();
    out.push_str("# SlopCodeBench report\n\n");
    out.push_str(&format!("Run: `{}`\n\n", report.run.display()));
    for (name, value) in report.pins.named() {
        out.push_str(&format!("- {name}: `{}`\n", value.unwrap_or("unknown")));
    }
    out.push_str(&format!("- pass_policy: `{}`\n\n", report.pass_policy));

    out.push_str("## Summary\n\n");
    let rows: Vec<Vec<String>> = report
        .arms
        .iter()
        .map(|a| {
            let mut row = vec![
                a.name.clone(),
                a.group.as_str().to_owned(),
                a.seeds.len().to_string(),
            ];
            for key in [
                PASS_RATE,
                "cost_usd",
                "tokens_total",
                "wall_clock_seconds",
                LANE_FAILURES,
            ] {
                row.push(cell(summary_of(a, key)));
            }
            row
        })
        .collect();
    table(
        &mut out,
        &strings(&[
            "arm",
            "group",
            "n",
            "pass rate",
            "cost (USD)",
            "tokens",
            "wall-clock (s)",
            "lane failures",
        ]),
        &rows,
    );

    out.push_str("## Erosion\n\n");
    let mut header = strings(&["arm"]);
    for suffix in ["final", "change"] {
        for e in EROSION {
            header.push(format!("{e}_{suffix}"));
        }
    }
    let rows: Vec<Vec<String>> = report
        .arms
        .iter()
        .map(|a| {
            std::iter::once(a.name.clone())
                .chain(header[1..].iter().map(|k| cell(summary_of(a, k))))
                .collect()
        })
        .collect();
    table(&mut out, &header, &rows);

    let mut problems: Vec<&str> = Vec::new();
    for c in report.arms.iter().flat_map(|a| &a.checkpoints) {
        if !problems.contains(&c.problem.as_str()) {
            problems.push(&c.problem);
        }
    }
    for problem in problems {
        out.push_str(&format!("## {problem} per checkpoint\n\n"));
        let mut header = strings(&["arm", "checkpoint", "pass"]);
        header.extend(EROSION.iter().map(|e| format!("{e} (Δ)")));
        let mut rows = Vec::new();
        for a in &report.arms {
            for c in a.checkpoints.iter().filter(|c| c.problem == problem) {
                let mut row = vec![
                    a.name.clone(),
                    c.checkpoint.clone(),
                    cell(c.passed.as_ref()),
                ];
                for e in EROSION {
                    let m = c.metrics.get(e).and_then(Option::as_ref);
                    let d = c.delta.get(e).and_then(Option::as_ref);
                    row.push(match d {
                        Some(d) => format!("{} (Δ {})", cell(m), signed(d.mean)),
                        None => cell(m),
                    });
                }
                rows.push(row);
            }
        }
        table(&mut out, &header, &rows);
    }

    if !report.comparisons.is_empty() {
        out.push_str("## rdm vs control\n\n");
        let mut header = strings(&["metric"]);
        header.extend(
            report
                .comparisons
                .iter()
                .map(|c| format!("{} − {}", c.rdm, c.control)),
        );
        let rows: Vec<Vec<String>> = report
            .metrics
            .iter()
            .map(|m| {
                std::iter::once(m.key.clone())
                    .chain(
                        report
                            .comparisons
                            .iter()
                            .map(|c| delta_cell(c.metrics.get(&m.key).and_then(Option::as_ref))),
                    )
                    .collect()
            })
            .collect();
        table(&mut out, &header, &rows);
        out.push_str(
            "`*`: the interval excludes 0 (not a multiple-comparison-corrected test).\n\n",
        );
    }

    if let Some(b) = &report.baseline {
        baseline::render_markdown(&mut out, b, &report.metrics);
    }

    if !report.skipped_runs.is_empty() {
        out.push_str("## Skipped runs\n\n");
        for s in &report.skipped_runs {
            let seed = s.seed.map_or_else(|| "-".to_owned(), |n| n.to_string());
            out.push_str(&format!("- {} seed {seed}: {}\n", s.arm, s.reason));
        }
        out.push('\n');
    }

    let b = &report.bootstrap;
    out.push_str(&format!(
        "Intervals: {}% {} bootstrap over {}s, {} resamples, {} seeded per statistic by {}.\n",
        num(b.level * 100.0),
        b.method,
        b.unit,
        b.resamples,
        b.rng,
        b.seed_key
    ));
    out
}

/// Runs `scbench report`, returning the process exit code (0 whenever a
/// report was produced, whatever a baseline comparison flags).
///
/// # Errors
///
/// When a baseline name is invalid, the run record or an SCBench output
/// cannot be read, a baseline cannot be loaded, a save is refused (an
/// existing name or a missing pin), or a file cannot be written.
pub fn run(
    opts: &ReportOptions,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<u8, String> {
    for (flag, name) in [
        ("--baseline", &opts.baseline),
        ("--save-baseline", &opts.save_baseline),
    ] {
        if let Some(name) = name {
            baseline::check_name(flag, name)?;
        }
    }
    let run = std::path::absolute(&opts.run).unwrap_or(opts.run.clone());
    let record = read_record(&run)?;
    let mut report = build(&run, &record)?;
    let dir = || baseline::dir(record.manifest_path.as_deref());
    if let Some(name) = &opts.baseline {
        let saved = baseline::load(&dir()?, name)?;
        report.baseline = Some(baseline::compare(&report, &saved));
    }
    let save = match &opts.save_baseline {
        Some(name) => Some(baseline::prepare(
            &dir()?,
            name,
            &report,
            record.observed.claude.clone(),
        )?),
        None => None,
    };

    let json = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())? + "\n";
    let md = render_markdown(&report);
    write_file(&run.join(REPORT_JSON), json.as_bytes())?;
    write_file(&run.join(REPORT_MD), md.as_bytes())?;
    if let Some((path, saved)) = save {
        let body = serde_json::to_string_pretty(&saved).map_err(|e| e.to_string())? + "\n";
        write_file(&path, body.as_bytes())?;
        let _ = writeln!(
            stderr,
            "scbench report: saved baseline {} to {}",
            saved.name,
            path.display()
        );
    }
    let _ = match opts.format {
        Format::Markdown => stdout.write_all(md.as_bytes()),
        Format::Json => stdout.write_all(json.as_bytes()),
    };
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_render_compactly() {
        assert_eq!(num(2.0), "2");
        assert_eq!(num(0.125), "0.125");
        assert_eq!(num(1.0 / 3.0), "0.333");
        assert_eq!(num(-0.0001), "0");
        assert_eq!(signed(1.5), "+1.5");
        assert_eq!(signed(-2.0), "-2");
    }

    #[test]
    fn every_summary_metric_has_a_direction_except_sloc() {
        let infos = metric_infos();
        assert_eq!(infos[0].key, "pass_rate");
        let get = |k: &str| {
            infos
                .iter()
                .find(|i| i.key == k)
                .unwrap_or_else(|| panic!("{k}"))
                .better
                .clone()
        };
        assert_eq!(get("mi_mean_change").as_deref(), Some("higher"));
        assert_eq!(get("sloc_final"), None);
        assert_eq!(get("lane_failures").as_deref(), Some("lower"));
        assert_eq!(infos.len(), 1 + 16 + 9);
    }
}

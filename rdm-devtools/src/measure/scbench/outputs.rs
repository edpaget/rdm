//! Reads one unit's SCBench checkpoint tree: for every checkpoint of every
//! problem, whether it passed and its erosion, cost and token figures.
//!
//! `slop-code run` writes `<run_dir>/<problem>/<checkpoint>/`
//! ([`lane::CHECKPOINT_FILE`] and [`INFERENCE_FILE`]); `slop-code eval` adds
//! [`EVALUATION_FILE`], [`QUALITY_FILE`] and the per-file and per-symbol
//! rows ([`FILES_FILE`], [`SYMBOLS_FILE`]). Only the fields the report uses
//! are read (the names SCBench's own `metrics/checkpoint/extractors.py`
//! reads), and unknown fields are ignored. A missing file is missing data; a
//! file that exists but does not parse is an error naming it.
//!
//! [`QUALITY_FILE`] scores the whole snapshot, tests the agent wrote
//! included. The per-file rows let the report recompute the same erosion
//! metrics for application code and for test code separately
//! ([`is_test_file`], [`QualitySplit`]).
//!
//! The token figures come from the checkpoint's agent stream
//! ([`STREAM_FILE`]) when it has one: its final `result` event's
//! `modelUsage`, the session's running totals over every turn, subagent and
//! `--continue` resume. SCBench's own `net_tokens` is the last turn's
//! main-thread `usage` alone, so it is the fallback only for a checkpoint
//! without a stream. Cost and steps stay SCBench's: its `usage.cost` is the
//! final result's cumulative `total_cost_usd`, and its `usage.steps` counts
//! every assistant message of every `claude` process in the invocation.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::Deserialize;
use serde::de::DeserializeOwned;

use super::lane;

/// SCBench's `EVALUATION_FILENAME` (`CorrectnessResults`).
pub const EVALUATION_FILE: &str = "evaluation.json";
/// SCBench's `QUALITY_DIR`/`QUALITY_METRIC_SAVENAME` (`SnapshotMetrics`).
pub const QUALITY_FILE: &str = "quality_analysis/overall_quality.json";
/// SCBench's per-file quality rows (`FileMetrics`, one JSON object a line).
pub const FILES_FILE: &str = "quality_analysis/files.jsonl";
/// SCBench's per-symbol quality rows (`SymbolMetrics`, one JSON object a
/// line).
pub const SYMBOLS_FILE: &str = "quality_analysis/symbols.jsonl";
/// SCBench's `HIGH_CC_THRESHOLD`: a function or method counts toward
/// `cc_high_count` when its complexity is above this.
pub const HIGH_CC_THRESHOLD: f64 = 10.0;
/// SCBench's `INFERENCE_RESULT_FILENAME` (`CheckpointInferenceResult`).
pub const INFERENCE_FILE: &str = "inference_result.json";
/// The claude_code agent's stream-json stdout (`STDOUT_FILENAME` under the
/// checkpoint's `agent` artifacts dir): every line of every `claude` process
/// the agent binary ran for the checkpoint.
pub const STREAM_FILE: &str = "agent/stdout.jsonl";

/// The erosion metrics, in report order.
pub const EROSION: [&str; 8] = [
    "cc_max",
    "cc_high_count",
    "mi_mean",
    "max_nesting_depth",
    "lint_errors",
    "single_use_functions",
    "trivial_wrappers",
    "sloc",
];

/// SCBench's `PassPolicy`: when a checkpoint's test counts pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassPolicy {
    /// `any`: always passes.
    Any,
    /// `any-case`: at least one test passed.
    AnyCase,
    /// `all-cases`: every executed test passed.
    AllCases,
    /// `all-non-error-cases`: every executed non-`Error` test passed.
    AllNonErrorCases,
    /// `core-cases`: every executed `Core` test passed.
    CoreCases,
    /// `any-core-cases`: at least one `Core` test passed.
    AnyCoreCases,
    /// `all-core-cases`: every executed `Core` test passed.
    AllCoreCases,
}

impl PassPolicy {
    /// Parses SCBench's policy name.
    ///
    /// # Errors
    ///
    /// When `name` is not one of SCBench's seven policies, naming it.
    pub fn parse(name: &str) -> Result<Self, String> {
        Ok(match name {
            "any" => Self::Any,
            "any-case" => Self::AnyCase,
            "all-cases" => Self::AllCases,
            "all-non-error-cases" => Self::AllNonErrorCases,
            "core-cases" => Self::CoreCases,
            "any-core-cases" => Self::AnyCoreCases,
            "all-core-cases" => Self::AllCoreCases,
            other => {
                return Err(format!(
                    "unknown pass policy \"{other}\"; SCBench's policies are any, any-case, \
                     all-cases, all-non-error-cases, core-cases, any-core-cases, all-core-cases"
                ));
            }
        })
    }

    /// Whether `pass` of `total` executed tests per group type (`Core`,
    /// `Functionality`, `Regression`, `Error`) satisfy the policy, as
    /// SCBench's `PassPolicy.check` decides.
    pub fn check(self, pass: &BTreeMap<String, u64>, total: &BTreeMap<String, u64>) -> bool {
        let get = |m: &BTreeMap<String, u64>, k: &str| m.get(k).copied().unwrap_or(0);
        let sum = |m: &BTreeMap<String, u64>, keep: &dyn Fn(&str) -> bool| -> u64 {
            ["Core", "Functionality", "Regression", "Error"]
                .into_iter()
                .filter(|k| keep(k))
                .map(|k| get(m, k))
                .sum()
        };
        let all = |_: &str| true;
        let non_error = |k: &str| k != "Error";
        let core = (get(pass, "Core"), get(total, "Core"));
        match self {
            Self::Any => true,
            Self::AnyCase => sum(pass, &all) > 0,
            Self::AllCases => sum(pass, &all) == sum(total, &all),
            Self::AllNonErrorCases => sum(pass, &non_error) == sum(total, &non_error),
            Self::CoreCases | Self::AllCoreCases => core.0 == core.1,
            Self::AnyCoreCases => core.0 > 0,
        }
    }
}

/// Whether the analysed file at `path` is test code rather than
/// application code. The one rule the report splits by:
///
/// - a `\` is read as `/`;
/// - any directory component named `tests` or `test` makes it a test file
///   (`tests/conftest.py`, `pkg/tests/util.py`, `src/test/helpers.py`);
/// - so does a file name `conftest.py`, `test_*.py` or `*_test.py`.
///
/// Everything else is application code, including `xjqlib/testing.py`,
/// `contest.py` and a top-level `tests.py`.
pub fn is_test_file(path: &str) -> bool {
    let path = path.replace('\\', "/");
    let mut parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    let Some(name) = parts.pop() else {
        return false;
    };
    parts.iter().any(|d| matches!(*d, "tests" | "test"))
        || name == "conftest.py"
        || (name.starts_with("test_") && name.ends_with(".py"))
        || name.ends_with("_test.py")
}

/// One checkpoint's quality figures (from [`QUALITY_FILE`], or recomputed
/// from a set of [`FILES_FILE`] / [`SYMBOLS_FILE`] rows).
#[derive(Debug, Clone, PartialEq)]
pub struct Quality {
    values: BTreeMap<&'static str, f64>,
    file_count: u64,
}

impl Quality {
    /// Erosion metric `key` (one of [`EROSION`]); `None` for `mi_mean` when
    /// the snapshot measured no file.
    pub fn get(&self, key: &str) -> Option<f64> {
        self.values.get(key).copied()
    }

    /// The number of files measured.
    pub fn file_count(&self) -> u64 {
        self.file_count
    }

    /// The erosion metrics of exactly `files` and `symbols`, by SCBench's
    /// own rules: line, lint, waste counts and MI summed over the files
    /// (`mi_mean` their mean, absent with no file); `cc_max`,
    /// `cc_high_count` (complexity above [`HIGH_CC_THRESHOLD`]) and
    /// `max_nesting_depth` over the `function` and `method` symbols, 0 with
    /// none.
    #[allow(clippy::cast_precision_loss)]
    pub fn from_rows(files: &[&FileRow], symbols: &[&SymbolRow]) -> Self {
        let sum = |f: &dyn Fn(&FileRow) -> f64| files.iter().map(|r| f(r)).sum::<f64>();
        let functions: Vec<&&SymbolRow> = symbols
            .iter()
            .filter(|s| matches!(s.kind.as_str(), "function" | "method"))
            .collect();
        let max =
            |f: &dyn Fn(&SymbolRow) -> f64| functions.iter().map(|s| f(s)).fold(0.0_f64, f64::max);
        let mut values = BTreeMap::from([
            ("cc_max", max(&|s| s.complexity)),
            (
                "cc_high_count",
                functions
                    .iter()
                    .filter(|s| s.complexity > HIGH_CC_THRESHOLD)
                    .count() as f64,
            ),
            ("max_nesting_depth", max(&|s| s.max_nesting_depth)),
            ("lint_errors", sum(&|r| r.lint_errors)),
            ("single_use_functions", sum(&|r| r.single_use_count)),
            ("trivial_wrappers", sum(&|r| r.trivial_wrapper_count)),
            ("sloc", sum(&|r| r.loc)),
        ]);
        if !files.is_empty() {
            values.insert(
                "mi_mean",
                sum(&|r| r.mi.unwrap_or(0.0)) / files.len() as f64,
            );
        }
        Self {
            values,
            file_count: files.len() as u64,
        }
    }
}

/// One row of [`FILES_FILE`]; only the fields the report uses.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct FileRow {
    /// The file's path in the snapshot.
    pub file_path: String,
    /// Source lines.
    #[serde(default)]
    pub loc: f64,
    /// Lint findings.
    #[serde(default)]
    pub lint_errors: f64,
    /// Maintainability index; `null` when SCBench computed none.
    #[serde(default)]
    pub mi: Option<f64>,
    /// Functions called from exactly one place.
    #[serde(default)]
    pub single_use_count: f64,
    /// Functions that only forward to another call.
    #[serde(default)]
    pub trivial_wrapper_count: f64,
}

/// One row of [`SYMBOLS_FILE`]; only the fields the report uses.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SymbolRow {
    /// The file the symbol is in.
    pub file_path: String,
    /// `function`, `method`, `class`, `variable`, ...
    #[serde(rename = "type", default)]
    pub kind: String,
    /// Cyclomatic complexity.
    #[serde(default)]
    pub complexity: f64,
    /// Deepest nesting inside the symbol.
    #[serde(default)]
    pub max_nesting_depth: f64,
}

/// A checkpoint's quality split by [`is_test_file`].
#[derive(Debug, Clone, PartialEq)]
pub struct QualitySplit {
    /// Every analysed file that is not test code.
    pub application: Quality,
    /// The test files alone.
    pub tests: Quality,
}

impl QualitySplit {
    /// Splits `files` and `symbols` (a symbol goes with its `file_path`).
    pub fn from_rows(files: &[FileRow], symbols: &[SymbolRow]) -> Self {
        let (tests, app): (Vec<&FileRow>, Vec<&FileRow>) =
            files.iter().partition(|f| is_test_file(&f.file_path));
        let (test_symbols, app_symbols): (Vec<&SymbolRow>, Vec<&SymbolRow>) =
            symbols.iter().partition(|s| is_test_file(&s.file_path));
        Self {
            application: Quality::from_rows(&app, &app_symbols),
            tests: Quality::from_rows(&tests, &test_symbols),
        }
    }

    /// The number of test files.
    pub fn tests_file_count(&self) -> u64 {
        self.tests.file_count()
    }

    /// Test lines per application line; `None` when there is no application
    /// line.
    pub fn code_ratio(&self) -> Option<f64> {
        let app = self.application.get("sloc")?;
        (app > 0.0).then(|| self.tests.get("sloc").unwrap_or(0.0) / app)
    }
}

/// A checkpoint's hidden `Regression` tests: SCBench's own counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegressionCounts {
    /// `pass_counts["Regression"]`.
    pub passed: u64,
    /// `total_counts["Regression"]`.
    pub total: u64,
}

/// One checkpoint's agent usage (from [`INFERENCE_FILE`], with the tokens
/// from [`STREAM_FILE`]'s final `result` when the checkpoint has a stream).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Inference {
    /// Agent wall-clock seconds for the checkpoint (`elapsed`), spanning the
    /// whole agent-binary invocation.
    pub elapsed: f64,
    /// `usage.cost`, in dollars: the final result's `total_cost_usd`, which
    /// covers every resume of the session.
    pub cost: f64,
    /// `usage.steps`: assistant messages with usage, subagents included,
    /// over every `claude` process of the invocation.
    pub steps: f64,
    /// Input tokens: the final result's `modelUsage` `inputTokens` summed
    /// over models, else `usage.net_tokens.input`.
    pub tokens_input: f64,
    /// Output tokens: `outputTokens`, else `usage.net_tokens.output`.
    pub tokens_output: f64,
    /// Cache-read tokens: `cacheReadInputTokens`, else
    /// `usage.net_tokens.cache_read`.
    pub tokens_cache_read: f64,
    /// Cache-write tokens: `cacheCreationInputTokens`, else
    /// `usage.net_tokens.cache_write`.
    pub tokens_cache_write: f64,
}

/// One checkpoint of one problem.
#[derive(Debug, Clone, PartialEq)]
pub struct CheckpointOutput {
    /// The checkpoint's directory name.
    pub name: String,
    /// Its `checkpoint.yaml` `order`.
    pub order: i64,
    /// Whether it passed: evaluated, no infrastructure failure, and the pass
    /// policy holds.
    pub passed: bool,
    /// Whether it has an [`EVALUATION_FILE`].
    pub evaluated: bool,
    /// Its whole-snapshot quality figures (tests included), when evaluated.
    pub quality: Option<Quality>,
    /// Its quality split into application and test code; `Some` only when
    /// both [`FILES_FILE`] and [`SYMBOLS_FILE`] exist.
    pub split: Option<QualitySplit>,
    /// Its `Regression` test counts, when evaluated with any.
    pub regression: Option<RegressionCounts>,
    /// Every test's outcome, keyed `<origin checkpoint>/<test name>`: `true`
    /// passed, `false` failed; skipped tests are left out.
    pub test_outcomes: BTreeMap<String, bool>,
    /// The keys of [`CheckpointOutput::test_outcomes`] that ran in a
    /// `*-Regression` group.
    pub regression_tests: BTreeSet<String>,
    /// Its agent usage, when recorded.
    pub inference: Option<Inference>,
}

/// One problem's checkpoints, in SCBench's `(order, name)` order.
#[derive(Debug, Clone, PartialEq)]
pub struct ProblemOutput {
    /// The problem name.
    pub problem: String,
    /// Its checkpoints; empty when the problem has no output directory.
    pub checkpoints: Vec<CheckpointOutput>,
}

#[derive(Deserialize)]
struct EvaluationJson {
    #[serde(default)]
    pass_counts: BTreeMap<String, u64>,
    #[serde(default)]
    total_counts: BTreeMap<String, u64>,
    #[serde(default)]
    infrastructure_failure: bool,
    #[serde(default)]
    tests: BTreeMap<String, TestGroup>,
}

/// One `evaluation.json` `tests` entry, keyed `<origin>-<GroupType>`.
#[derive(Default, Deserialize)]
struct TestGroup {
    #[serde(default)]
    passed: Vec<String>,
    #[serde(default)]
    failed: Vec<String>,
}

impl EvaluationJson {
    fn regression(&self) -> Option<RegressionCounts> {
        let total = *self.total_counts.get("Regression")?;
        Some(RegressionCounts {
            passed: self.pass_counts.get("Regression").copied().unwrap_or(0),
            total,
        })
    }

    /// Every test's outcome, and the ids that ran as `Regression`.
    fn outcomes(&self) -> (BTreeMap<String, bool>, BTreeSet<String>) {
        let mut out = BTreeMap::new();
        let mut regression = BTreeSet::new();
        for (key, group) in &self.tests {
            let (origin, kind) = key.rsplit_once('-').unwrap_or((key.as_str(), ""));
            for (names, passed) in [(&group.passed, true), (&group.failed, false)] {
                for name in names {
                    let id = format!("{origin}/{name}");
                    if kind == "Regression" {
                        regression.insert(id.clone());
                    }
                    out.insert(id, passed);
                }
            }
        }
        (out, regression)
    }
}

#[derive(Deserialize)]
struct QualityJson {
    file_count: u64,
    lines: LinesJson,
    lint: LintJson,
    functions: FunctionsJson,
    complexity: ComplexityJson,
    waste: WasteJson,
}

#[derive(Deserialize)]
struct LinesJson {
    loc: f64,
}

#[derive(Deserialize)]
struct LintJson {
    errors: f64,
}

#[derive(Deserialize)]
struct FunctionsJson {
    #[serde(default)]
    cc_max: f64,
    #[serde(default)]
    cc_high_count: f64,
    #[serde(default)]
    depth_max: f64,
}

#[derive(Deserialize)]
struct ComplexityJson {
    mi_sum: f64,
}

#[derive(Deserialize)]
struct WasteJson {
    single_use_functions: f64,
    trivial_wrappers: f64,
}

#[derive(Deserialize)]
struct InferenceJson {
    elapsed: f64,
    #[serde(default)]
    usage: UsageJson,
}

#[derive(Default, Deserialize)]
struct UsageJson {
    #[serde(default)]
    cost: f64,
    #[serde(default)]
    steps: f64,
    #[serde(default)]
    net_tokens: TokensJson,
}

#[derive(Default, Deserialize)]
struct TokensJson {
    #[serde(default)]
    input: f64,
    #[serde(default)]
    output: f64,
    #[serde(default)]
    cache_read: f64,
    #[serde(default)]
    cache_write: f64,
}

/// One line of [`STREAM_FILE`]; only a `result` event's usage is read.
#[derive(Deserialize)]
struct StreamEvent {
    #[serde(rename = "type", default)]
    kind: Option<String>,
    #[serde(default)]
    usage: Option<serde_json::Value>,
    #[serde(rename = "modelUsage", default)]
    model_usage: Option<BTreeMap<String, serde_json::Value>>,
}

/// The token totals of a stream's final `result` event.
#[derive(Debug, Clone, Copy, PartialEq)]
struct ResultTotals {
    input: f64,
    output: f64,
    cache_read: f64,
    cache_write: f64,
}

/// `v[key]` as a number, `0` when absent or not a number.
fn number(v: &serde_json::Value, key: &str) -> f64 {
    v.get(key)
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.0)
}

impl StreamEvent {
    /// The event's totals: `modelUsage` summed over its models (SCBench's own
    /// `modelUsage` mapping), else its `usage`; `None` with neither.
    fn totals(&self) -> Option<ResultTotals> {
        if let Some(models) = self.model_usage.as_ref().filter(|m| !m.is_empty()) {
            let mut t = ResultTotals {
                input: 0.0,
                output: 0.0,
                cache_read: 0.0,
                cache_write: 0.0,
            };
            for m in models.values().filter(|m| m.is_object()) {
                t.input += number(m, "inputTokens");
                t.output += number(m, "outputTokens");
                t.cache_read += number(m, "cacheReadInputTokens");
                t.cache_write += number(m, "cacheCreationInputTokens");
            }
            return Some(t);
        }
        let u = self.usage.as_ref().filter(|u| u.is_object())?;
        Some(ResultTotals {
            input: number(u, "input_tokens"),
            output: number(u, "output_tokens"),
            cache_read: number(u, "cache_read_input_tokens"),
            cache_write: number(u, "cache_creation_input_tokens"),
        })
    }
}

/// The token totals of the last `result` event in the stream at `path`;
/// `None` when the file does not exist or holds no result with usage. A
/// line that is not JSON is skipped, as SCBench skips it.
fn final_result(path: &Path) -> Result<Option<ResultTotals>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    Ok(text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .rev()
        .filter_map(|l| serde_json::from_str::<StreamEvent>(l).ok())
        .find(|e| e.kind.as_deref() == Some("result"))
        .and_then(|e| e.totals()))
}

/// Reads `path` as `T`; `None` when it does not exist.
fn read_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    serde_json::from_str(&text).map(Some).map_err(|e| {
        format!(
            "cannot parse {}: {e} (re-run `slop-code eval ... --overwrite` to regenerate it)",
            path.display()
        )
    })
}

/// Reads the JSON-lines file at `path` as rows of `T`; `None` when it does
/// not exist. Blank lines are skipped.
fn read_jsonl<T: DeserializeOwned>(path: &Path) -> Result<Option<Vec<T>>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    let mut rows = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        rows.push(serde_json::from_str(line).map_err(|e| {
            format!(
                "cannot parse {} line {}: {e} (re-run `slop-code eval ... --overwrite` to \
                 regenerate it)",
                path.display(),
                i + 1
            )
        })?);
    }
    Ok(Some(rows))
}

/// One outcome step of [`regressions`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RegressionStep {
    /// Regressions first seen at this checkpoint.
    pub new: u64,
    /// Regressions open after this checkpoint (new ones included).
    pub open: u64,
}

/// One problem's regressions over its checkpoints.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RegressionTrack {
    /// Per checkpoint, in [`ProblemOutput::checkpoints`] order; `None` for a
    /// checkpoint that was not evaluated.
    pub steps: Vec<Option<RegressionStep>>,
    /// Every regression onset.
    pub onsets: u64,
    /// Onsets still open at the last evaluated checkpoint.
    pub persisting: u64,
}

/// Tracks `problem`'s regressions: a test in a `*-Regression` group that
/// fails after its last observed outcome was a pass is one regression,
/// counted once at that checkpoint and open until the test passes again (a
/// later failure is then a new one). A test that never passed is no
/// regression, and an unevaluated checkpoint changes no outcome.
pub fn regressions(problem: &ProblemOutput) -> RegressionTrack {
    let mut last: BTreeMap<&str, bool> = BTreeMap::new();
    let mut open: BTreeSet<&str> = BTreeSet::new();
    let mut track = RegressionTrack::default();
    for c in &problem.checkpoints {
        if !c.evaluated {
            track.steps.push(None);
            continue;
        }
        let mut new = 0;
        for (id, &passed) in &c.test_outcomes {
            let id = id.as_str();
            if passed {
                open.remove(id);
            } else if c.regression_tests.contains(id)
                && last.get(id) == Some(&true)
                && open.insert(id)
            {
                new += 1;
            }
            last.insert(id, passed);
        }
        track.onsets += new;
        track.steps.push(Some(RegressionStep {
            new,
            open: open.len() as u64,
        }));
    }
    track.persisting = open.len() as u64;
    track
}

impl From<QualityJson> for Quality {
    #[allow(clippy::cast_precision_loss)]
    fn from(q: QualityJson) -> Self {
        let mut values = BTreeMap::from([
            ("cc_max", q.functions.cc_max),
            ("cc_high_count", q.functions.cc_high_count),
            ("max_nesting_depth", q.functions.depth_max),
            ("lint_errors", q.lint.errors),
            ("single_use_functions", q.waste.single_use_functions),
            ("trivial_wrappers", q.waste.trivial_wrappers),
            ("sloc", q.lines.loc),
        ]);
        if q.file_count > 0 {
            values.insert("mi_mean", q.complexity.mi_sum / q.file_count as f64);
        }
        Self {
            values,
            file_count: q.file_count,
        }
    }
}

/// Reads every checkpoint of `problems` under the unit's `run_dir`, deciding
/// each pass with `policy`.
///
/// # Errors
///
/// When a directory or `checkpoint.yaml` cannot be read, or an SCBench file
/// exists but does not parse (naming the file).
pub fn read_unit(
    run_dir: &Path,
    problems: &[String],
    policy: PassPolicy,
) -> Result<Vec<ProblemOutput>, String> {
    let mut out = Vec::new();
    for problem in problems {
        let mut checkpoints = Vec::new();
        for (order, name, dir) in lane::checkpoints(&run_dir.join(problem))? {
            let eval: Option<EvaluationJson> = read_json(&dir.join(EVALUATION_FILE))?;
            let quality: Option<QualityJson> = read_json(&dir.join(QUALITY_FILE))?;
            let inference: Option<InferenceJson> = read_json(&dir.join(INFERENCE_FILE))?;
            let files: Option<Vec<FileRow>> = read_jsonl(&dir.join(FILES_FILE))?;
            let symbols: Option<Vec<SymbolRow>> = read_jsonl(&dir.join(SYMBOLS_FILE))?;
            let split = match (files, symbols) {
                (Some(f), Some(s)) => Some(QualitySplit::from_rows(&f, &s)),
                _ => None,
            };
            let (test_outcomes, regression_tests) = eval
                .as_ref()
                .map(EvaluationJson::outcomes)
                .unwrap_or_default();
            let totals = match inference {
                Some(_) => final_result(&dir.join(STREAM_FILE))?,
                None => None,
            };
            checkpoints.push(CheckpointOutput {
                name,
                order,
                passed: eval.as_ref().is_some_and(|e| {
                    !e.infrastructure_failure && policy.check(&e.pass_counts, &e.total_counts)
                }),
                evaluated: eval.is_some(),
                quality: quality.map(Quality::from),
                split,
                regression: eval.as_ref().and_then(EvaluationJson::regression),
                test_outcomes,
                regression_tests,
                inference: inference.map(|i| {
                    let net = &i.usage.net_tokens;
                    let t = totals.unwrap_or(ResultTotals {
                        input: net.input,
                        output: net.output,
                        cache_read: net.cache_read,
                        cache_write: net.cache_write,
                    });
                    Inference {
                        elapsed: i.elapsed,
                        cost: i.usage.cost,
                        steps: i.usage.steps,
                        tokens_input: t.input,
                        tokens_output: t.output,
                        tokens_cache_read: t.cache_read,
                        tokens_cache_write: t.cache_write,
                    }
                }),
            });
        }
        out.push(ProblemOutput {
            problem: problem.clone(),
            checkpoints,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counts(pairs: &[(&str, u64)]) -> BTreeMap<String, u64> {
        pairs.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect()
    }

    #[test]
    fn every_pass_policy_follows_scbench() {
        use PassPolicy::*;
        // Core 2/2, Functionality 1/2, Error 0/1.
        let pass = counts(&[("Core", 2), ("Functionality", 1)]);
        let total = counts(&[("Core", 2), ("Functionality", 2), ("Error", 1)]);
        // Core 0/2, Functionality 1/1.
        let core_fail_pass = counts(&[("Functionality", 1)]);
        let core_fail_total = counts(&[("Core", 2), ("Functionality", 1)]);
        // Every executed test passed; Error 1/1.
        let all = counts(&[("Core", 1), ("Error", 1)]);
        // Core 2/2, Error 0/1 only.
        let err_pass = counts(&[("Core", 2)]);
        let err_total = counts(&[("Core", 2), ("Error", 1)]);
        let none = counts(&[]);
        let table = [
            (Any, [true, true, true, true, true]),
            (AnyCase, [true, true, true, true, false]),
            (AllCases, [false, false, true, false, true]),
            (AllNonErrorCases, [false, false, true, true, true]),
            (CoreCases, [true, false, true, true, true]),
            (AnyCoreCases, [true, false, true, true, false]),
            (AllCoreCases, [true, false, true, true, true]),
        ];
        for (policy, want) in table {
            let got = [
                policy.check(&pass, &total),
                policy.check(&core_fail_pass, &core_fail_total),
                policy.check(&all, &all),
                policy.check(&err_pass, &err_total),
                policy.check(&none, &none),
            ];
            assert_eq!(got, want, "{policy:?}");
        }
    }

    #[test]
    fn policy_names_parse_and_an_unknown_one_is_named() {
        for name in [
            "any",
            "any-case",
            "all-cases",
            "all-non-error-cases",
            "core-cases",
            "any-core-cases",
            "all-core-cases",
        ] {
            assert!(PassPolicy::parse(name).is_ok(), "{name}");
        }
        let e = PassPolicy::parse("most-cases").expect_err("unknown");
        assert!(e.contains("\"most-cases\""), "{e}");
    }

    fn quality(file_count: u64) -> Quality {
        Quality::from(QualityJson {
            file_count,
            lines: LinesJson { loc: 120.0 },
            lint: LintJson { errors: 3.0 },
            functions: FunctionsJson {
                cc_max: 9.0,
                cc_high_count: 1.0,
                depth_max: 4.0,
            },
            complexity: ComplexityJson { mi_sum: 150.0 },
            waste: WasteJson {
                single_use_functions: 2.0,
                trivial_wrappers: 1.0,
            },
        })
    }

    fn stream(dir: &Path, lines: &[&str]) -> std::path::PathBuf {
        let path = dir.join("stdout.jsonl");
        std::fs::write(&path, lines.join("\n") + "\n").expect("write stream");
        path
    }

    fn totals(input: f64, output: f64, cache_read: f64, cache_write: f64) -> ResultTotals {
        ResultTotals {
            input,
            output,
            cache_read,
            cache_write,
        }
    }

    #[test]
    fn final_result_keeps_the_last_result_and_skips_non_json() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = stream(
            tmp.path(),
            &[
                r#"{"type":"system","subtype":"init"}"#,
                r#"{"type":"result","usage":{"input_tokens":1},"modelUsage":{"m":{"inputTokens":10,"outputTokens":100,"cacheReadInputTokens":1000,"cacheCreationInputTokens":50}}}"#,
                "not json at all",
                r#"{"type":"assistant","message":{"usage":{"input_tokens":7}}}"#,
                r#"{"type":"result","usage":{"input_tokens":2},"modelUsage":{"m":{"inputTokens":15,"outputTokens":160,"cacheReadInputTokens":1600,"cacheCreationInputTokens":80}}}"#,
            ],
        );
        assert_eq!(
            final_result(&path).expect("read"),
            Some(totals(15.0, 160.0, 1600.0, 80.0))
        );
    }

    #[test]
    fn final_result_sums_model_usage_across_models() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = stream(
            tmp.path(),
            &[
                r#"{"type":"result","modelUsage":{"a":{"inputTokens":1,"outputTokens":2,"cacheReadInputTokens":3,"cacheCreationInputTokens":4},"b":{"inputTokens":10,"outputTokens":20,"cacheReadInputTokens":30,"cacheCreationInputTokens":40}}}"#,
            ],
        );
        assert_eq!(
            final_result(&path).expect("read"),
            Some(totals(11.0, 22.0, 33.0, 44.0))
        );
    }

    #[test]
    fn final_result_without_model_usage_falls_back_to_usage() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = stream(
            tmp.path(),
            &[
                r#"{"type":"result","usage":{"input_tokens":5,"output_tokens":6,"cache_read_input_tokens":7,"cache_creation_input_tokens":8},"modelUsage":{}}"#,
            ],
        );
        assert_eq!(
            final_result(&path).expect("read"),
            Some(totals(5.0, 6.0, 7.0, 8.0))
        );
    }

    #[test]
    fn no_stream_keeps_net_tokens() {
        let tmp = tempfile::tempdir().expect("tempdir");
        assert_eq!(
            final_result(&tmp.path().join("missing.jsonl")).expect("read"),
            None
        );
        let run_dir = tmp.path();
        let cp = run_dir.join("p/checkpoint_1");
        std::fs::create_dir_all(&cp).expect("mkdir");
        std::fs::write(cp.join("checkpoint.yaml"), "name: checkpoint_1\norder: 1\n")
            .expect("checkpoint.yaml");
        std::fs::write(
            cp.join(INFERENCE_FILE),
            r#"{"elapsed":1.0,"usage":{"cost":0.5,"steps":3,"net_tokens":{"input":1,"output":2,"cache_read":3,"cache_write":4}}}"#,
        )
        .expect("inference");
        let read = |dir: &Path| {
            read_unit(dir, &["p".to_owned()], PassPolicy::Any).expect("read unit")[0].checkpoints[0]
                .inference
                .expect("inference")
        };
        let i = read(run_dir);
        assert_eq!(
            (
                i.tokens_input,
                i.tokens_output,
                i.tokens_cache_read,
                i.tokens_cache_write
            ),
            (1.0, 2.0, 3.0, 4.0)
        );
        std::fs::create_dir_all(cp.join("agent")).expect("agent dir");
        std::fs::write(
            cp.join(STREAM_FILE),
            r#"{"type":"result","modelUsage":{"m":{"inputTokens":9,"outputTokens":8,"cacheReadInputTokens":7,"cacheCreationInputTokens":6}}}"#,
        )
        .expect("stream");
        let i = read(run_dir);
        assert_eq!(
            (
                i.tokens_input,
                i.tokens_output,
                i.tokens_cache_read,
                i.tokens_cache_write
            ),
            (9.0, 8.0, 7.0, 6.0)
        );
        assert_eq!((i.cost, i.steps), (0.5, 3.0));
    }

    #[test]
    fn mi_mean_divides_by_file_count_and_is_absent_without_files() {
        assert_eq!(quality(2).get("mi_mean"), Some(75.0));
        assert_eq!(quality(0).get("mi_mean"), None);
        assert_eq!(quality(0).get("cc_max"), Some(9.0));
        assert_eq!(quality(2).get("max_nesting_depth"), Some(4.0));
    }

    #[test]
    fn test_files_follow_the_one_rule() {
        for path in [
            "tests/conftest.py",
            "pkg/tests/util.py",
            "src/test/helpers.py",
            "test_a.py",
            "a/b/test_x.py",
            "a_test.py",
            "conftest.py",
            "tests\\win.py",
        ] {
            assert!(is_test_file(path), "{path}");
        }
        for path in [
            "xjq.py",
            "xjqlib/testing.py",
            "latest.py",
            "contest.py",
            "tests.py",
            "attest_x.py",
        ] {
            assert!(!is_test_file(path), "{path}");
        }
    }

    fn file(path: &str, loc: f64, lint: f64, mi: f64, single_use: f64, trivial: f64) -> FileRow {
        FileRow {
            file_path: path.to_owned(),
            loc,
            lint_errors: lint,
            mi: Some(mi),
            single_use_count: single_use,
            trivial_wrapper_count: trivial,
        }
    }

    fn symbol(path: &str, kind: &str, complexity: f64, depth: f64) -> SymbolRow {
        SymbolRow {
            file_path: path.to_owned(),
            kind: kind.to_owned(),
            complexity,
            max_nesting_depth: depth,
        }
    }

    fn metrics(q: &Quality) -> Vec<(&'static str, Option<f64>)> {
        EROSION.iter().map(|&e| (e, q.get(e))).collect()
    }

    #[test]
    fn a_split_gives_each_set_only_its_own_rows() {
        let files = [
            file("main.py", 100.0, 3.0, 60.0, 2.0, 1.0),
            file("lib/core.py", 200.0, 5.0, 40.0, 1.0, 0.0),
            file("tests/test_core.py", 300.0, 20.0, 30.0, 10.0, 0.0),
            file("tests/conftest.py", 50.0, 4.0, 50.0, 1.0, 0.0),
            file("test_cli.py", 70.0, 6.0, 40.0, 3.0, 1.0),
        ];
        let symbols = [
            symbol("main.py", "function", 11.0, 3.0),
            symbol("lib/core.py", "method", 10.0, 4.0),
            symbol("lib/core.py", "function", 12.0, 2.0),
            symbol("lib/core.py", "class", 30.0, 9.0),
            symbol("lib/core.py", "variable", 40.0, 8.0),
            symbol("tests/test_core.py", "function", 2.0, 1.0),
            symbol("tests/test_core.py", "function", 15.0, 5.0),
            symbol("tests/conftest.py", "function", 1.0, 1.0),
            symbol("test_cli.py", "method", 3.0, 2.0),
        ];
        let split = QualitySplit::from_rows(&files, &symbols);
        assert_eq!(
            metrics(&split.application),
            [
                ("cc_max", Some(12.0)),
                // 11 and 12 count; exactly 10 does not.
                ("cc_high_count", Some(2.0)),
                ("mi_mean", Some(50.0)),
                ("max_nesting_depth", Some(4.0)),
                ("lint_errors", Some(8.0)),
                ("single_use_functions", Some(3.0)),
                ("trivial_wrappers", Some(1.0)),
                ("sloc", Some(300.0)),
            ]
        );
        assert_eq!(
            metrics(&split.tests),
            [
                ("cc_max", Some(15.0)),
                ("cc_high_count", Some(1.0)),
                ("mi_mean", Some(40.0)),
                ("max_nesting_depth", Some(5.0)),
                ("lint_errors", Some(30.0)),
                ("single_use_functions", Some(14.0)),
                ("trivial_wrappers", Some(1.0)),
                ("sloc", Some(420.0)),
            ]
        );
        assert_eq!(split.application.file_count(), 2);
        assert_eq!(split.tests_file_count(), 3);
        assert_eq!(split.code_ratio(), Some(1.4));

        // The two sets together are the whole snapshot.
        let all_files: Vec<&FileRow> = files.iter().collect();
        let all_symbols: Vec<&SymbolRow> = symbols.iter().collect();
        let whole = Quality::from_rows(&all_files, &all_symbols);
        assert_eq!(
            metrics(&whole),
            [
                ("cc_max", Some(15.0)),
                ("cc_high_count", Some(3.0)),
                ("mi_mean", Some(44.0)),
                ("max_nesting_depth", Some(5.0)),
                ("lint_errors", Some(38.0)),
                ("single_use_functions", Some(17.0)),
                ("trivial_wrappers", Some(2.0)),
                ("sloc", Some(720.0)),
            ]
        );
    }

    #[test]
    fn an_empty_set_has_zeros_and_no_mi_mean() {
        let app_only = QualitySplit::from_rows(
            &[file("main.py", 100.0, 3.0, 60.0, 2.0, 1.0)],
            &[symbol("main.py", "function", 4.0, 2.0)],
        );
        assert_eq!(
            metrics(&app_only.tests),
            [
                ("cc_max", Some(0.0)),
                ("cc_high_count", Some(0.0)),
                ("mi_mean", None),
                ("max_nesting_depth", Some(0.0)),
                ("lint_errors", Some(0.0)),
                ("single_use_functions", Some(0.0)),
                ("trivial_wrappers", Some(0.0)),
                ("sloc", Some(0.0)),
            ]
        );
        assert_eq!(app_only.code_ratio(), Some(0.0));
        let tests_only =
            QualitySplit::from_rows(&[file("tests/test_a.py", 10.0, 1.0, 50.0, 0.0, 0.0)], &[]);
        assert_eq!(tests_only.code_ratio(), None);
        assert_eq!(tests_only.application.get("mi_mean"), None);
    }

    fn checkpoint_dir(run_dir: &Path, name: &str, order: u32) -> std::path::PathBuf {
        let cp = run_dir.join("p").join(name);
        std::fs::create_dir_all(cp.join("quality_analysis")).expect("mkdir");
        std::fs::write(
            cp.join("checkpoint.yaml"),
            format!("name: {name}\norder: {order}\n"),
        )
        .expect("checkpoint.yaml");
        cp
    }

    #[test]
    fn read_unit_splits_only_with_both_row_files_and_names_a_bad_line() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cp = checkpoint_dir(tmp.path(), "checkpoint_1", 1);
        let read = || read_unit(tmp.path(), &["p".to_owned()], PassPolicy::Any);
        assert_eq!(read().expect("read")[0].checkpoints[0].split, None);

        std::fs::write(
            cp.join(FILES_FILE),
            "{\"file_path\":\"main.py\",\"loc\":10,\"lint_errors\":1,\"mi\":70,\"extra\":true}\n\n\
             {\"file_path\":\"tests/test_main.py\",\"loc\":5,\"mi\":null}\n",
        )
        .expect("files");
        assert_eq!(read().expect("read")[0].checkpoints[0].split, None);

        std::fs::write(
            cp.join(SYMBOLS_FILE),
            "{\"file_path\":\"main.py\",\"type\":\"function\",\"complexity\":3,\"max_nesting_depth\":1}\n",
        )
        .expect("symbols");
        let split = read().expect("read")[0].checkpoints[0]
            .split
            .clone()
            .expect("split");
        assert_eq!(split.application.get("sloc"), Some(10.0));
        assert_eq!(split.application.get("cc_max"), Some(3.0));
        assert_eq!(split.tests.get("sloc"), Some(5.0));
        assert_eq!(split.tests.get("mi_mean"), Some(0.0));

        std::fs::write(
            cp.join(FILES_FILE),
            "{\"file_path\":\"main.py\",\"loc\":10}\n{not json\n",
        )
        .expect("bad files");
        let e = read().expect_err("malformed line");
        assert!(
            e.contains(&cp.join(FILES_FILE).display().to_string()) && e.contains("line 2"),
            "{e}"
        );
    }

    /// A checkpoint whose `test_outcomes` hold `(id, passed, regression)`;
    /// `None` is an unevaluated one.
    fn outcome_checkpoint(order: i64, tests: Option<&[(&str, bool, bool)]>) -> CheckpointOutput {
        let evaluated = tests.is_some();
        let tests = tests.unwrap_or(&[]);
        CheckpointOutput {
            name: format!("checkpoint_{order}"),
            order,
            passed: false,
            evaluated,
            quality: None,
            split: None,
            regression: None,
            test_outcomes: tests
                .iter()
                .map(|(id, p, _)| ((*id).to_owned(), *p))
                .collect(),
            regression_tests: tests
                .iter()
                .filter(|t| t.2)
                .map(|t| t.0.to_owned())
                .collect(),
            inference: None,
        }
    }

    fn track(checkpoints: Vec<CheckpointOutput>) -> RegressionTrack {
        regressions(&ProblemOutput {
            problem: "p".to_owned(),
            checkpoints,
        })
    }

    fn step(new: u64, open: u64) -> Option<RegressionStep> {
        Some(RegressionStep { new, open })
    }

    const T: &str = "checkpoint_1/test_t";

    #[test]
    fn a_persisting_regression_is_counted_once() {
        let t = track(vec![
            outcome_checkpoint(1, Some(&[(T, true, false)])),
            outcome_checkpoint(2, Some(&[(T, false, true)])),
            outcome_checkpoint(3, Some(&[(T, false, true)])),
            outcome_checkpoint(4, Some(&[(T, false, true)])),
        ]);
        assert_eq!(t.steps, [step(0, 0), step(1, 1), step(0, 1), step(0, 1)]);
        assert_eq!((t.onsets, t.persisting), (1, 1));
    }

    #[test]
    fn a_fixed_regression_closes_and_a_later_failure_is_a_new_one() {
        let fixed = vec![
            outcome_checkpoint(1, Some(&[(T, true, false)])),
            outcome_checkpoint(2, Some(&[(T, false, true)])),
            outcome_checkpoint(3, Some(&[(T, true, true)])),
        ];
        let t = track(fixed.clone());
        assert_eq!(t.steps, [step(0, 0), step(1, 1), step(0, 0)]);
        assert_eq!((t.onsets, t.persisting), (1, 0));

        let mut again = fixed;
        again.push(outcome_checkpoint(4, Some(&[(T, false, true)])));
        let t = track(again);
        assert_eq!(t.steps[3], step(1, 1));
        assert_eq!((t.onsets, t.persisting), (2, 1));
    }

    #[test]
    fn a_test_that_never_passed_is_no_regression() {
        let t = track(vec![
            outcome_checkpoint(1, Some(&[(T, false, false)])),
            outcome_checkpoint(2, Some(&[(T, false, true)])),
            outcome_checkpoint(3, Some(&[(T, false, true)])),
        ]);
        assert_eq!((t.onsets, t.persisting), (0, 0));
        assert_eq!(t.steps, [step(0, 0), step(0, 0), step(0, 0)]);
    }

    #[test]
    fn an_unevaluated_checkpoint_changes_no_outcome() {
        let t = track(vec![
            outcome_checkpoint(1, Some(&[(T, true, false)])),
            outcome_checkpoint(2, None),
            outcome_checkpoint(3, Some(&[(T, false, true)])),
        ]);
        assert_eq!(t.steps, [step(0, 0), None, step(1, 1)]);
        assert_eq!((t.onsets, t.persisting), (1, 1));
    }

    #[test]
    fn evaluation_tests_key_outcomes_by_origin_and_mark_regressions() {
        let eval: EvaluationJson = serde_json::from_str(
            r#"{"tests":{
                "checkpoint_1-Regression":{"passed":["a"],"failed":["b"],"skipped":["c"]},
                "checkpoint_2-Core":{"passed":["d"],"failed":[]}
              },
              "pass_counts":{"Core":1,"Regression":1},
              "total_counts":{"Core":1,"Regression":2}}"#,
        )
        .expect("evaluation");
        let (outcomes, regression) = eval.outcomes();
        assert_eq!(
            outcomes,
            BTreeMap::from([
                ("checkpoint_1/a".to_owned(), true),
                ("checkpoint_1/b".to_owned(), false),
                ("checkpoint_2/d".to_owned(), true),
            ])
        );
        assert_eq!(
            regression,
            BTreeSet::from(["checkpoint_1/a".to_owned(), "checkpoint_1/b".to_owned()])
        );
        assert_eq!(
            eval.regression(),
            Some(RegressionCounts {
                passed: 1,
                total: 2
            })
        );
    }
}

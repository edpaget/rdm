//! Reads one unit's SCBench checkpoint tree: for every checkpoint of every
//! problem, whether it passed and its erosion, cost and token figures.
//!
//! `slop-code run` writes `<run_dir>/<problem>/<checkpoint>/`
//! ([`lane::CHECKPOINT_FILE`] and [`INFERENCE_FILE`]); `slop-code eval` adds
//! [`EVALUATION_FILE`] and [`QUALITY_FILE`]. Only the fields the report uses
//! are read (the names SCBench's own `metrics/checkpoint/extractors.py`
//! reads), and unknown fields are ignored. A missing file is missing data; a
//! file that exists but does not parse is an error naming it.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;
use serde::de::DeserializeOwned;

use super::lane;

/// SCBench's `EVALUATION_FILENAME` (`CorrectnessResults`).
pub const EVALUATION_FILE: &str = "evaluation.json";
/// SCBench's `QUALITY_DIR`/`QUALITY_METRIC_SAVENAME` (`SnapshotMetrics`).
pub const QUALITY_FILE: &str = "quality_analysis/overall_quality.json";
/// SCBench's `INFERENCE_RESULT_FILENAME` (`CheckpointInferenceResult`).
pub const INFERENCE_FILE: &str = "inference_result.json";

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

/// One checkpoint's quality figures (from [`QUALITY_FILE`]).
#[derive(Debug, Clone, PartialEq)]
pub struct Quality {
    values: BTreeMap<&'static str, f64>,
}

impl Quality {
    /// Erosion metric `key` (one of [`EROSION`]); `None` for `mi_mean` when
    /// the snapshot measured no file.
    pub fn get(&self, key: &str) -> Option<f64> {
        self.values.get(key).copied()
    }
}

/// One checkpoint's agent usage (from [`INFERENCE_FILE`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Inference {
    /// Agent wall-clock seconds for the checkpoint (`elapsed`).
    pub elapsed: f64,
    /// `usage.cost`, in dollars.
    pub cost: f64,
    /// `usage.steps`.
    pub steps: f64,
    /// `usage.net_tokens.input`.
    pub tokens_input: f64,
    /// `usage.net_tokens.output`.
    pub tokens_output: f64,
    /// `usage.net_tokens.cache_read`.
    pub tokens_cache_read: f64,
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
    /// Its quality figures, when evaluated.
    pub quality: Option<Quality>,
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
        Self { values }
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
            checkpoints.push(CheckpointOutput {
                name,
                order,
                passed: eval.as_ref().is_some_and(|e| {
                    !e.infrastructure_failure && policy.check(&e.pass_counts, &e.total_counts)
                }),
                evaluated: eval.is_some(),
                quality: quality.map(Quality::from),
                inference: inference.map(|i| Inference {
                    elapsed: i.elapsed,
                    cost: i.usage.cost,
                    steps: i.usage.steps,
                    tokens_input: i.usage.net_tokens.input,
                    tokens_output: i.usage.net_tokens.output,
                    tokens_cache_read: i.usage.net_tokens.cache_read,
                    tokens_cache_write: i.usage.net_tokens.cache_write,
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

    #[test]
    fn mi_mean_divides_by_file_count_and_is_absent_without_files() {
        assert_eq!(quality(2).get("mi_mean"), Some(75.0));
        assert_eq!(quality(0).get("mi_mean"), None);
        assert_eq!(quality(0).get("cc_max"), Some(9.0));
        assert_eq!(quality(2).get("max_nesting_depth"), Some(4.0));
    }
}

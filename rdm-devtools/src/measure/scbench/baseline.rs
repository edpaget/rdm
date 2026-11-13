//! Baselines: a run's report saved under a name with its five pins, and the
//! comparison of a later run against it.
//!
//! A baseline lives at `<manifest dir>/baselines/<name>.json`, beside the
//! manifest the run loaded (the checkout under test's `bench/scbench/`), so a
//! committed baseline is a checked-in record. The name is an operator label;
//! the pins (rdm, SCBench and `scb-problems` SHAs, the model and the Claude
//! Code version) are the key. A comparison flags every non-rdm pin that
//! differs, since the rdm SHA is the treatment, and never gates.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::report::{self, Delta, MetricInfo, Pins, Report, Stat};

/// The directory beside the manifest that holds baselines.
pub const DIR: &str = "baselines";
/// The baseline file's schema version.
pub const BASELINE_SCHEMA: u32 = 1;

/// A saved baseline (`<manifest dir>/baselines/<name>.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Baseline {
    /// [`BASELINE_SCHEMA`].
    pub schema: u32,
    /// The operator's label.
    pub name: String,
    /// RFC 3339 time it was saved.
    pub saved: String,
    /// The five pins, all present.
    pub pins: Pins,
    /// The host `claude --version` the run observed (information only; the
    /// pinned version is `pins.claude_code_version`).
    pub observed_claude: Option<String>,
    /// The run directory it was saved from.
    pub source_run: PathBuf,
    /// The run's report, without a baseline section; its per-seed samples
    /// let a later comparison bootstrap against it.
    pub report: Report,
}

/// The current run's and the baseline's rdm SHA.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RdmShas {
    /// The current run's.
    pub current: Option<String>,
    /// The baseline's.
    pub baseline: Option<String>,
}

/// A non-rdm pin whose value differs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinDifference {
    /// The pin's name.
    pub pin: String,
    /// The current run's value.
    pub current: Option<String>,
    /// The baseline's value.
    pub baseline: Option<String>,
}

/// One metric of one arm, current beside baseline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BaselineMetric {
    /// The current run's figure.
    pub current: Option<Stat>,
    /// The baseline's figure.
    pub baseline: Option<Stat>,
    /// `current - baseline`; `null` when either is missing.
    pub delta: Option<Delta>,
}

/// One arm present in both runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BaselineArm {
    /// The arm name.
    pub name: String,
    /// Every summary metric.
    pub metrics: BTreeMap<String, BaselineMetric>,
}

/// Arms only one of the two runs has.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnmatchedArms {
    /// In the current run only.
    pub current_only: Vec<String>,
    /// In the baseline only.
    pub baseline_only: Vec<String>,
}

/// The report's `baseline` section.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BaselineComparison {
    /// The baseline's name.
    pub name: String,
    /// The baseline's pins.
    pub pins: Pins,
    /// Both rdm SHAs; a difference is the treatment, never flagged.
    pub rdm_sha: RdmShas,
    /// Every non-rdm pin that differs.
    pub pin_differences: Vec<PinDifference>,
    /// Whether no non-rdm pin differs.
    pub comparable: bool,
    /// The arms both runs have, in the current run's order.
    pub arms: Vec<BaselineArm>,
    /// The arms only one run has.
    pub unmatched_arms: UnmatchedArms,
    /// Whether the baseline's report splits application code from tests
    /// (report schema 2 or later). Without the split its application,
    /// tests and regression metrics have no baseline value; this is not a
    /// pin difference and leaves `comparable` alone.
    #[serde(default)]
    pub baseline_has_split: bool,
}

/// Refuses a baseline name that is not `[A-Za-z0-9][A-Za-z0-9._-]*`.
///
/// # Errors
///
/// When `name` does not match, naming `flag` and the name.
pub fn check_name(flag: &str, name: &str) -> Result<(), String> {
    let mut chars = name.chars();
    let ok = chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if ok {
        Ok(())
    } else {
        Err(format!(
            "{flag}: baseline name \"{name}\" must match [A-Za-z0-9][A-Za-z0-9._-]*"
        ))
    }
}

/// The baselines directory beside the manifest at `manifest_path`.
///
/// # Errors
///
/// When the run record names no manifest, or the manifest's directory does
/// not exist (a run copied from elsewhere), naming the path.
pub fn dir(manifest_path: Option<&Path>) -> Result<PathBuf, String> {
    let manifest = manifest_path.ok_or_else(|| {
        "the run record names no manifest_path, so it has no baselines directory".to_owned()
    })?;
    let parent = manifest.parent().unwrap_or(Path::new("."));
    if !parent.is_dir() {
        return Err(format!(
            "the run's manifest directory {} does not exist (was the run copied from \
             elsewhere?), so it has no baselines directory",
            parent.display()
        ));
    }
    Ok(parent.join(DIR))
}

fn path(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.json"))
}

/// Checks that `report` can be saved as baseline `name` in `dir` and returns
/// its path and record; nothing is written.
///
/// # Errors
///
/// When a pin is missing (naming it) or `<name>.json` already exists (naming
/// the path).
pub fn prepare(
    dir: &Path,
    name: &str,
    report: &Report,
    observed_claude: Option<String>,
) -> Result<(PathBuf, Baseline), String> {
    for (pin, value) in report.pins.named() {
        if value.is_none_or(str::is_empty) {
            return Err(format!(
                "--save-baseline: the run records no {pin}; a baseline needs all five pins \
                 (rdm_sha, scbench_sha, problems_sha, model, claude_code_version)"
            ));
        }
    }
    let path = path(dir, name);
    if path.exists() {
        return Err(format!(
            "--save-baseline: baseline {} already exists; delete it deliberately to replace it",
            path.display()
        ));
    }
    let mut saved = report.clone();
    saved.baseline = None;
    Ok((
        path,
        Baseline {
            schema: BASELINE_SCHEMA,
            name: name.to_owned(),
            saved: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            pins: report.pins.clone(),
            observed_claude,
            source_run: report.run.clone(),
            report: saved,
        },
    ))
}

/// Loads baseline `name` from `dir`.
///
/// # Errors
///
/// When it cannot be read or parsed, naming the path.
pub fn load(dir: &Path, name: &str) -> Result<Baseline, String> {
    let path = path(dir, name);
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("--baseline: cannot read {}: {e}", path.display()))?;
    serde_json::from_str(&text)
        .map_err(|e| format!("--baseline: cannot parse {}: {e}", path.display()))
}

/// Sets `current` beside `saved`.
pub fn compare(current: &Report, saved: &Baseline) -> BaselineComparison {
    let base = &saved.report;
    let mut pin_differences: Vec<PinDifference> = current
        .pins
        .named()
        .into_iter()
        .zip(saved.pins.named())
        .filter(|((pin, a), (_, b))| *pin != "rdm_sha" && a != b)
        .map(|((pin, a), (_, b))| PinDifference {
            pin: pin.to_owned(),
            current: a.map(str::to_owned),
            baseline: b.map(str::to_owned),
        })
        .collect();
    // A run over a `--problems` subset never reads as comparable with one
    // over a different problem set. A report saved before the problems were
    // recorded has none, and is not flagged.
    if !current.problems.is_empty()
        && !base.problems.is_empty()
        && current.problems != base.problems
    {
        pin_differences.push(PinDifference {
            pin: "problems".to_owned(),
            current: Some(current.problems.join(", ")),
            baseline: Some(base.problems.join(", ")),
        });
    }
    let arms = current
        .arms
        .iter()
        .filter_map(|a| {
            let b = base.arms.iter().find(|b| b.name == a.name)?;
            let metrics = current
                .metrics
                .iter()
                .map(|m| {
                    let cur = a.summary.get(&m.key).cloned().flatten();
                    let old = b.summary.get(&m.key).cloned().flatten();
                    let key = format!("baseline/{}/{}/{}", saved.name, a.name, m.key);
                    let delta = report::delta(&key, cur.as_ref(), old.as_ref());
                    (
                        m.key.clone(),
                        BaselineMetric {
                            current: cur,
                            baseline: old,
                            delta,
                        },
                    )
                })
                .collect();
            Some(BaselineArm {
                name: a.name.clone(),
                metrics,
            })
        })
        .collect();
    let only = |xs: &Report, ys: &Report| -> Vec<String> {
        xs.arms
            .iter()
            .filter(|x| !ys.arms.iter().any(|y| y.name == x.name))
            .map(|x| x.name.clone())
            .collect()
    };
    BaselineComparison {
        name: saved.name.clone(),
        pins: saved.pins.clone(),
        rdm_sha: RdmShas {
            current: current.pins.rdm_sha.clone(),
            baseline: saved.pins.rdm_sha.clone(),
        },
        comparable: pin_differences.is_empty(),
        pin_differences,
        arms,
        unmatched_arms: UnmatchedArms {
            current_only: only(current, base),
            baseline_only: only(base, current),
        },
        baseline_has_split: base.schema >= 2,
    }
}

/// Appends the baseline section to a markdown report.
pub(super) fn render_markdown(out: &mut String, b: &BaselineComparison, metrics: &[MetricInfo]) {
    out.push_str(&format!("## Baseline `{}`\n\n", b.name));
    for d in &b.pin_differences {
        out.push_str(&format!(
            "> **not comparable: {} differs ({} vs {})**\n\n",
            d.pin,
            d.current.as_deref().unwrap_or("unknown"),
            d.baseline.as_deref().unwrap_or("unknown"),
        ));
    }
    out.push_str(&format!(
        "rdm: `{}` (current) vs `{}` (baseline)\n\n",
        b.rdm_sha.current.as_deref().unwrap_or("unknown"),
        b.rdm_sha.baseline.as_deref().unwrap_or("unknown"),
    ));
    if !b.baseline_has_split {
        out.push_str(&format!(
            "> baseline `{}` predates the application/tests split (report schema 1): \
             application, tests and regression metrics have no baseline value; compare the \
             SCBench snapshot (includes tests) rows.\n\n",
            b.name
        ));
    }
    for arm in &b.arms {
        out.push_str(&format!("### {}\n\n", arm.name));
        let header = ["metric", "current", "baseline", "Δ"].map(str::to_owned);
        let rows: Vec<Vec<String>> = metrics
            .iter()
            .filter_map(|m| {
                let x = arm.metrics.get(&m.key)?;
                Some(vec![
                    m.label(),
                    report::cell(x.current.as_ref()),
                    report::cell(x.baseline.as_ref()),
                    report::delta_cell(x.delta.as_ref()),
                ])
            })
            .collect();
        report::table(out, &header, &rows);
    }
    let u = &b.unmatched_arms;
    if !u.current_only.is_empty() || !u.baseline_only.is_empty() {
        out.push_str(&format!(
            "Unmatched arms: current only [{}]; baseline only [{}].\n\n",
            u.current_only.join(", "),
            u.baseline_only.join(", ")
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_labels_never_paths() {
        for ok in ["first", "v1.2", "a_b-c", "0x"] {
            assert_eq!(check_name("--baseline", ok), Ok(()), "{ok}");
        }
        for bad in ["", ".hidden", "../x", "a/b", "-x", "a b"] {
            let e = check_name("--save-baseline", bad).expect_err(bad);
            assert!(e.contains("--save-baseline"), "{e}");
        }
    }

    fn report_over(problems: &[&str]) -> Report {
        let some = |s: &str| Some(s.to_owned());
        Report {
            schema: report::REPORT_SCHEMA,
            run: PathBuf::from("/runs/r"),
            pins: Pins {
                rdm_sha: some("a"),
                scbench_sha: some("b"),
                problems_sha: some("c"),
                model: some("m"),
                claude_code_version: some("v"),
            },
            problems: problems.iter().map(|&p| p.to_owned()).collect(),
            problems_filter: None,
            pass_policy: "all-cases".to_owned(),
            bootstrap: report::Bootstrap {
                unit: "seed".to_owned(),
                method: "percentile".to_owned(),
                resamples: 1,
                level: 0.95,
                rng: "splitmix64".to_owned(),
                seed_key: "k".to_owned(),
            },
            metrics: Vec::new(),
            arms: Vec::new(),
            comparisons: Vec::new(),
            cost_per_lift: None,
            skipped_runs: Vec::new(),
            baseline: None,
        }
    }

    fn saved(report: Report) -> Baseline {
        Baseline {
            schema: BASELINE_SCHEMA,
            name: "b".to_owned(),
            saved: "2026-01-01T00:00:00Z".to_owned(),
            pins: report.pins.clone(),
            observed_claude: None,
            source_run: PathBuf::from("/runs/b"),
            report,
        }
    }

    #[test]
    fn a_differing_problem_set_is_not_comparable() {
        let c = compare(
            &report_over(&["xjq"]),
            &saved(report_over(&["mvvault", "xjq"])),
        );
        assert_eq!(
            c.pin_differences,
            [PinDifference {
                pin: "problems".to_owned(),
                current: Some("xjq".to_owned()),
                baseline: Some("mvvault, xjq".to_owned()),
            }]
        );
        assert!(!c.comparable);

        let same = compare(
            &report_over(&["mvvault", "xjq"]),
            &saved(report_over(&["mvvault", "xjq"])),
        );
        assert!(same.comparable, "{:?}", same.pin_differences);
        let unrecorded = compare(&report_over(&["xjq"]), &saved(report_over(&[])));
        assert!(unrecorded.comparable, "{:?}", unrecorded.pin_differences);
    }

    /// A baseline saved by a schema-1 report: no `scope` on its metrics, no
    /// split or regression fields on its checkpoints.
    const SCHEMA_1_BASELINE: &str = r#"{
      "schema": 1, "name": "old", "saved": "2026-01-01T00:00:00Z",
      "pins": {"rdm_sha": "a", "scbench_sha": "b", "problems_sha": "c", "model": "m",
               "claude_code_version": "v"},
      "observed_claude": null, "source_run": "/runs/old",
      "report": {
        "schema": 1, "run": "/runs/old",
        "pins": {"rdm_sha": "a", "scbench_sha": "b", "problems_sha": "c", "model": "m",
                 "claude_code_version": "v"},
        "pass_policy": "all-cases",
        "bootstrap": {"unit": "seed", "method": "percentile", "resamples": 1, "level": 0.95,
                      "rng": "splitmix64", "seed_key": "k"},
        "metrics": [{"key": "pass_rate", "better": "higher"},
                    {"key": "cc_max_final", "better": "lower"}],
        "arms": [{"name": "x", "group": "control", "image": "stock", "seeds": [1],
                  "summary": {"pass_rate": null, "cc_max_final": null},
                  "lane_failure_reasons": null,
                  "checkpoints": [{"problem": "p", "checkpoint": "checkpoint_1", "order": 1,
                                   "passed": null, "metrics": {"cc_max": null},
                                   "delta": {"cc_max": null}}]}],
        "comparisons": [], "skipped_runs": [], "baseline": null
      }
    }"#;

    #[test]
    fn a_schema_1_baseline_loads_and_is_flagged_as_predating_the_split() {
        let old: Baseline = serde_json::from_str(SCHEMA_1_BASELINE).expect("schema-1 baseline");
        assert_eq!(old.report.metrics[1].scope, None);
        assert!(old.report.arms[0].checkpoints[0].application.is_empty());
        let c = compare(&report_over(&[]), &old);
        assert!(!c.baseline_has_split);
        assert!(c.comparable);
        let mut md = String::new();
        render_markdown(&mut md, &c, &[]);
        assert!(md.contains("predates the application/tests split"), "{md}");

        let new = compare(&report_over(&[]), &saved(report_over(&[])));
        assert!(new.baseline_has_split);
        let mut md = String::new();
        render_markdown(&mut md, &new, &[]);
        assert!(!md.contains("predates"), "{md}");
    }
}

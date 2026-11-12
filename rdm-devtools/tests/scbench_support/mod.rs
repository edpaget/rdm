//! SCBench per-checkpoint output fixtures, written in SCBench's full shapes
//! at the pinned commit (`af5a9cbc`): `checkpoint.yaml`, `evaluation.json`
//! (`CorrectnessResults`), `quality_analysis/overall_quality.json`
//! (`SnapshotMetrics`) and `inference_result.json`
//! (`CheckpointInferenceResult`). They are built from SCBench's source, not
//! captured from a run, so only a live run confirms SCBench writes exactly
//! these.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

/// A checkpoint's evaluation outcome.
#[derive(Clone, Copy)]
pub struct Eval {
    /// Every executed test passed (else one `Core` test failed).
    pub passed: bool,
    /// `infrastructure_failure`.
    pub infra: bool,
}

/// The quality figures the report reads, plus `mi_sum` / `file_count`.
#[derive(Clone, Copy)]
pub struct Quality {
    pub cc_max: f64,
    pub cc_high_count: f64,
    pub depth_max: f64,
    pub lint_errors: f64,
    pub single_use_functions: f64,
    pub trivial_wrappers: f64,
    pub loc: f64,
    pub mi_sum: f64,
    pub file_count: u64,
}

/// The agent usage the report reads.
#[derive(Clone, Copy)]
pub struct Inference {
    pub elapsed: f64,
    pub cost: f64,
    pub steps: u64,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

/// One checkpoint directory's contents; `None` leaves that file out.
#[derive(Clone)]
pub struct Checkpoint {
    pub problem: String,
    pub name: String,
    pub order: u32,
    pub eval: Option<Eval>,
    pub quality: Option<Quality>,
    pub inference: Option<Inference>,
}

fn evaluation(c: &Checkpoint, e: Eval) -> Value {
    let core_passed = if e.passed { 4 } else { 3 };
    let ids = |n: usize, what: &str| -> Vec<String> {
        (0..n)
            .map(|i| format!("tests/test_{}.py::test_{what}_{i}", c.name))
            .collect()
    };
    json!({
        "schema_version": 1,
        "problem_name": c.problem,
        "problem_version": 1,
        "checkpoint_name": c.name,
        "checkpoint_version": 1,
        "duration": 2.5,
        "entrypoint": "python main.py",
        "tests": {
            format!("{}-Core", c.name): {
                "passed": ids(core_passed, "core"),
                "failed": ids(4 - core_passed, "core_failing"),
                "skipped": [],
            },
            "checkpoint_0-Regression": {
                "passed": ids(2, "regression"),
                "failed": [],
                "skipped": [],
            },
        },
        "pass_counts": { "Core": core_passed, "Regression": 2 },
        "total_counts": { "Core": 4, "Regression": 2 },
        "pytest_exit_code": if e.passed { 0 } else { 1 },
        "pytest_collected": 6,
        "test_collection_hash": null,
        "infrastructure_failure": e.infra,
    })
}

fn quality(q: Quality) -> Value {
    json!({
        "file_count": q.file_count,
        "lines": {
            "total_lines": q.loc + 20.0,
            "loc": q.loc,
            "comments": 12,
            "multi_comment": 4,
            "single_comment": 8,
        },
        "lint": { "errors": q.lint_errors, "fixable": 0, "counts": { "E501": q.lint_errors } },
        "symbols": {
            "total": 10, "functions": 6, "methods": 2, "classes": 1, "variables": 1,
            "type_aliases": 0, "statements": 80, "expressions_top_level": 4,
            "expressions": 120, "imports": 5, "max_imports": 3, "globals": 1,
        },
        "functions": {
            "count": 8, "cc_sum": 30, "cc_max": q.cc_max, "cc_mean": 3.75, "cc_std": 1.5,
            "cc_high_count": q.cc_high_count, "cc_extreme_count": 0, "high_cc_mean": 12.0,
            "extreme_cc_mean": 0.0, "cc_normalized": 0.25, "cc_concentration": 0.4,
            "cc_top20": 0.5, "depth_max": q.depth_max, "lines_sum": 90, "lines_mean": 11.25,
        },
        "classes": {
            "count": 1, "method_counts_sum": 2, "method_counts_mean": 2.0,
            "attribute_counts_sum": 3, "attribute_counts_mean": 3.0,
        },
        "complexity": {
            "cc_ratings": { "A": 6, "B": 1, "C": 1, "D": 0, "E": 0, "F": 0 },
            "mi_ratings": { "A": q.file_count, "B": 0, "C": 0 },
            "cc_sum": 30,
            // Deliberately not functions.cc_max: the report reads the latter.
            "cc_max": q.cc_max + 100.0,
            "num_complex": 1,
            "mi_sum": q.mi_sum,
            "mi_min": 40.0,
        },
        "waste": {
            "single_use_functions": q.single_use_functions,
            "trivial_wrappers": q.trivial_wrappers,
            "unused_variables": 0,
        },
        "graph": null,
        "source_files": ["main.py"],
    })
}

fn inference(c: &Checkpoint, i: Inference, dir: &Path) -> Value {
    let tokens = json!({
        "input": i.input,
        "output": i.output,
        "cache_read": i.cache_read,
        "cache_write": i.cache_write,
        "reasoning": 0,
    });
    json!({
        "started": "2026-01-01T00:00:00Z",
        "completed": "2026-01-01T00:10:00Z",
        "elapsed": i.elapsed,
        "usage": {
            "cost": i.cost,
            "steps": i.steps,
            "net_tokens": tokens,
            "current_tokens": tokens,
        },
        "had_error": false,
        "error_message": null,
        "checkpoint_path": dir.join(&c.problem).join(&c.name),
        "snapshot_dir": "snapshot",
        "artifacts_dir": "agent",
    })
}

/// The files of checkpoint `c` relative to the unit's run directory
/// `run_dir`, with their text.
pub fn files(run_dir: &Path, c: &Checkpoint) -> Vec<(PathBuf, String)> {
    let dir = PathBuf::from(&c.problem).join(&c.name);
    let pretty = |v: &Value| serde_json::to_string_pretty(v).expect("json");
    let mut out = vec![(
        dir.join("checkpoint.yaml"),
        format!(
            "name: {}\norder: {}\nstate: Core Tests\nspecification: spec.md\n",
            c.name, c.order
        ),
    )];
    if let Some(e) = c.eval {
        out.push((dir.join("evaluation.json"), pretty(&evaluation(c, e))));
    }
    if let Some(q) = c.quality {
        out.push((
            dir.join("quality_analysis/overall_quality.json"),
            pretty(&quality(q)),
        ));
    }
    if let Some(i) = c.inference {
        out.push((
            dir.join("inference_result.json"),
            pretty(&inference(c, i, run_dir)),
        ));
    }
    out
}

/// Writes checkpoint `c` under the unit's run directory `run_dir`.
pub fn write(run_dir: &Path, c: &Checkpoint) {
    for (rel, text) in files(run_dir, c) {
        let path = run_dir.join(rel);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&path, text).expect("write fixture");
    }
}

/// A plain passing checkpoint with every file.
pub fn simple(problem: &str, name: &str, order: u32) -> Checkpoint {
    Checkpoint {
        problem: problem.to_owned(),
        name: name.to_owned(),
        order,
        eval: Some(Eval {
            passed: true,
            infra: false,
        }),
        quality: Some(Quality {
            cc_max: 5.0,
            cc_high_count: 0.0,
            depth_max: 2.0,
            lint_errors: 1.0,
            single_use_functions: 1.0,
            trivial_wrappers: 0.0,
            loc: 50.0,
            mi_sum: 80.0,
            file_count: 1,
        }),
        inference: Some(Inference {
            elapsed: 10.0,
            cost: 0.5,
            steps: 3,
            input: 100,
            output: 10,
            cache_read: 1,
            cache_write: 1,
        }),
    }
}

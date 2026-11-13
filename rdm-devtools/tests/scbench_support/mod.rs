//! SCBench per-checkpoint output fixtures, written in SCBench's full shapes
//! at the pinned commit (`af5a9cbc`): `checkpoint.yaml`, `evaluation.json`
//! (`CorrectnessResults`), `quality_analysis/overall_quality.json`
//! (`SnapshotMetrics`), `inference_result.json`
//! (`CheckpointInferenceResult`), and on request the per-file and per-symbol
//! rows `quality_analysis/files.jsonl` / `symbols.jsonl` (`FileMetrics`,
//! `SymbolMetrics`). They are built from SCBench's source, not
//! captured from a run, so only a live run confirms SCBench writes exactly
//! these.
//!
//! Beside them: the unit's run-level `checkpoint_results.jsonl` rows, an
//! SCBench model file, and a checkpoint's Claude trace tree under
//! `agent/workspace/projects/` (the shapes of a real run's traces).

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
    /// The claude_code agent's stream-json stdout, `agent/stdout.jsonl`.
    pub stream: Option<String>,
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
    if let Some(s) = &c.stream {
        out.push((dir.join("agent/stdout.jsonl"), s.clone()));
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
        stream: None,
    }
}

/// One `quality_analysis/files.jsonl` row.
#[derive(Clone, Copy)]
pub struct FileFixture {
    pub path: &'static str,
    pub loc: f64,
    pub lint_errors: f64,
    pub mi: f64,
    pub single_use: f64,
    pub trivial: f64,
}

/// One `quality_analysis/symbols.jsonl` row.
#[derive(Clone, Copy)]
pub struct SymbolFixture {
    pub path: &'static str,
    /// `function`, `method`, `class` or `variable`.
    pub kind: &'static str,
    pub complexity: f64,
    pub depth: f64,
}

fn checkpoint_dir(run_dir: &Path, problem: &str, checkpoint: &str) -> PathBuf {
    run_dir.join(problem).join(checkpoint)
}

/// Writes `files.jsonl` and `symbols.jsonl` for one checkpoint, in SCBench's
/// row shapes (with fields the report does not read).
pub fn write_quality_rows(
    run_dir: &Path,
    problem: &str,
    checkpoint: &str,
    files: &[FileFixture],
    symbols: &[SymbolFixture],
) {
    let dir = checkpoint_dir(run_dir, problem, checkpoint).join("quality_analysis");
    std::fs::create_dir_all(&dir).expect("mkdir");
    let lines = |rows: Vec<Value>| -> String {
        rows.iter()
            .map(|r| serde_json::to_string(r).expect("json") + "\n")
            .collect()
    };
    let file_rows = files
        .iter()
        .map(|f| {
            json!({
                "file_path": f.path, "loc": f.loc, "total_lines": f.loc + 10.0,
                "comments": 1, "lint_errors": f.lint_errors, "lint_fixable": 0,
                "mi": f.mi, "depth": 1, "is_entry_language": true,
                "single_use_count": f.single_use, "trivial_wrapper_count": f.trivial,
                "unused_variable_count": 0,
            })
        })
        .collect();
    let symbol_rows = symbols
        .iter()
        .enumerate()
        .map(|(i, s)| {
            json!({
                "name": format!("sym_{i}"), "type": s.kind, "start": i, "end": i + 5,
                "complexity": s.complexity, "branches": 0, "statements": 3,
                "max_nesting_depth": s.depth, "lines": 5, "sloc": 4,
                "file_path": s.path, "rating": "A",
            })
        })
        .collect();
    std::fs::write(dir.join("files.jsonl"), lines(file_rows)).expect("files.jsonl");
    std::fs::write(dir.join("symbols.jsonl"), lines(symbol_rows)).expect("symbols.jsonl");
}

/// Rewrites one checkpoint's `evaluation.json` `tests` to `groups` (each
/// `(group key, passed names, failed names)`, the key `<origin>-<Type>`),
/// with `pass_counts` and `total_counts` summed per group type to match.
pub fn write_test_results(
    run_dir: &Path,
    problem: &str,
    checkpoint: &str,
    groups: &[(&str, &[&str], &[&str])],
) {
    let path = checkpoint_dir(run_dir, problem, checkpoint).join("evaluation.json");
    let mut eval: Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("evaluation.json"))
            .expect("evaluation json");
    let mut tests = serde_json::Map::new();
    let mut pass = serde_json::Map::new();
    let mut total = serde_json::Map::new();
    for (key, passed, failed) in groups {
        tests.insert(
            (*key).to_owned(),
            json!({ "passed": passed, "failed": failed, "skipped": [] }),
        );
        let kind = key.rsplit_once('-').expect("<origin>-<Type>").1;
        let add = |m: &mut serde_json::Map<String, Value>, n: usize| {
            let now = m.get(kind).and_then(Value::as_u64).unwrap_or(0);
            m.insert(kind.to_owned(), json!(now + n as u64));
        };
        add(&mut pass, passed.len());
        add(&mut total, passed.len() + failed.len());
    }
    eval["tests"] = Value::Object(tests);
    eval["pass_counts"] = Value::Object(pass);
    eval["total_counts"] = Value::Object(total);
    std::fs::write(&path, serde_json::to_string_pretty(&eval).expect("json"))
        .expect("write evaluation.json");
}

/// One `checkpoint_results.jsonl` row's scb-check composites; `None` leaves
/// the field out.
#[derive(Clone, Copy)]
pub struct ScbRow {
    pub problem: &'static str,
    pub checkpoint: &'static str,
    pub erosion: Option<f64>,
    pub verbosity: Option<f64>,
    pub cloned_pct: Option<f64>,
}

/// Writes the unit's `<run_dir>/checkpoint_results.jsonl`, each row with the
/// other fields SCBench writes beside the composites.
pub fn write_checkpoint_results(run_dir: &Path, rows: &[ScbRow]) {
    let text: String = rows
        .iter()
        .map(|r| {
            let mut v = json!({
                "problem": r.problem, "version": 1, "checkpoint": r.checkpoint,
                "path": run_dir.join(r.problem), "state": "ran", "strict_pass_rate": 1.0,
            });
            for (k, x) in [
                ("erosion", r.erosion),
                ("verbosity", r.verbosity),
                ("cloned_pct", r.cloned_pct),
            ] {
                if let Some(x) = x {
                    v[k] = json!(x);
                }
            }
            serde_json::to_string(&v).expect("json") + "\n"
        })
        .collect();
    std::fs::create_dir_all(run_dir).expect("mkdir");
    std::fs::write(run_dir.join("checkpoint_results.jsonl"), text).expect("results");
}

/// Writes an SCBench model file for `claude-test-1` at `path`, priced input
/// 4, output 20, cache read 0.2 and 5-minute cache write 5 USD/MTok.
pub fn write_model_file(path: &Path) {
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(
        path,
        "internal_name: claude-test-1\nprovider: anthropic\nthinking_style: effort\n\
         aliases:\n  - claude-test-1\n  - test-1\npricing:\n  input: 4\n  output: 20\n  \
         cache_read: 0.20\n  cache_write: 5.00\nagent_specific:\n  claude_code:\n    \
         endpoint: anthropic\n",
    )
    .expect("model file");
}

/// One message's usage: input, output, cache read, 5-minute and 1-hour
/// cache writes.
#[derive(Clone, Copy, Default)]
pub struct TraceUsage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub write_5m: u64,
    pub write_1h: u64,
}

/// One trace-file `assistant` line of message `id` by `model` with `usage`
/// and the `tool_use` blocks `(id, name, input)`.
pub fn assistant_line(
    id: &str,
    model: &str,
    usage: TraceUsage,
    tools: &[(&str, &str, Value)],
) -> String {
    let mut content = vec![json!({ "type": "text", "text": "working" })];
    content.extend(tools.iter().map(
        |(id, name, input)| json!({ "type": "tool_use", "id": id, "name": name, "input": input }),
    ));
    json!({
        "parentUuid": null, "isSidechain": false, "type": "assistant",
        "uuid": format!("u-{id}"), "timestamp": "2026-01-01T00:00:00Z",
        "message": {
            "model": model, "id": id, "type": "message", "role": "assistant",
            "content": content, "stop_reason": "tool_use",
            "usage": {
                "input_tokens": usage.input,
                "output_tokens": usage.output,
                "cache_read_input_tokens": usage.cache_read,
                "cache_creation_input_tokens": usage.write_5m + usage.write_1h,
                "cache_creation": {
                    "ephemeral_5m_input_tokens": usage.write_5m,
                    "ephemeral_1h_input_tokens": usage.write_1h,
                },
                "service_tier": "standard",
            },
        },
    })
    .to_string()
}

/// One trace file of a checkpoint: its path under the session (`None` for
/// the main transcript `<session>.jsonl`, else relative to `<session>/`)
/// and its text.
pub struct TraceFile {
    pub path: Option<&'static str>,
    pub text: String,
}

/// Writes a checkpoint's Claude trace tree: session `session` of project
/// `-workspace` under `<checkpoint>/agent/workspace/projects/`.
pub fn write_trace(
    run_dir: &Path,
    problem: &str,
    checkpoint: &str,
    session: &str,
    files: &[TraceFile],
) {
    let project =
        checkpoint_dir(run_dir, problem, checkpoint).join("agent/workspace/projects/-workspace");
    for f in files {
        let path = match f.path {
            None => project.join(format!("{session}.jsonl")),
            Some(rel) => project.join(session).join(rel),
        };
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&path, &f.text).expect("trace file");
    }
}

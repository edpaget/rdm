//! `rdm-measure scbench report` over fixture runs: a run record written as
//! `scbench run` writes it and, per unit, an SCBench checkpoint tree in
//! SCBench's full shapes ([`scbench_support`]). Nothing here runs SCBench or
//! spawns anything but `rdm-measure`; every assertion reads the report JSON,
//! the baseline JSON or the files written.

mod measure_support;
mod scbench_support;

use std::path::{Path, PathBuf};
use std::process::Output;

use measure_support::{fails, json, ok};
use scbench_support::{Checkpoint, Eval, FileFixture, Inference, Quality, SymbolFixture};
use serde_json::{Value, json};

const RDM_SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const OTHER_RDM_SHA: &str = "dddddddddddddddddddddddddddddddddddddddd";
const SCBENCH_SHA: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const OTHER_SCBENCH_SHA: &str = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
const PROBLEMS_SHA: &str = "cccccccccccccccccccccccccccccccccccccccc";

/// The pins a fixture record carries.
#[derive(Clone)]
struct RecordPins {
    rdm_sha: Option<&'static str>,
    scbench_sha: &'static str,
    model: &'static str,
}

impl Default for RecordPins {
    fn default() -> Self {
        Self {
            rdm_sha: Some(RDM_SHA),
            scbench_sha: SCBENCH_SHA,
            model: "opus-5.5",
        }
    }
}

/// One unit of a fixture run.
struct UnitSpec {
    arm: &'static str,
    group: Option<&'static str>,
    image: &'static str,
    seed: Option<u64>,
    finished: bool,
    checkpoints: Vec<Checkpoint>,
    lane: Option<Value>,
    /// `(signal, step)` of an interruption that stopped the unit; its run
    /// directory then also holds a truncated `evaluation.json`.
    interrupted: Option<(&'static str, &'static str)>,
}

/// A temp tree holding the stand-in rdm checkout's `bench/scbench/` (the
/// manifest directory, where baselines land) and the runs.
struct Fixture {
    _tmp: tempfile::TempDir,
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = std::fs::canonicalize(tmp.path()).expect("canonical tmp");
        std::fs::create_dir_all(root.join("rdm/bench/scbench")).expect("manifest dir");
        Self { _tmp: tmp, root }
    }

    fn manifest_path(&self) -> PathBuf {
        self.root.join("rdm/bench/scbench/manifest.toml")
    }

    fn baseline(&self, name: &str) -> PathBuf {
        self.root
            .join("rdm/bench/scbench/baselines")
            .join(format!("{name}.json"))
    }

    /// Writes a schema-2 run `name` (or a phase 1-4 record with
    /// `schema2: false`) and each unit's checkpoint tree; returns its dir.
    fn write_run(
        &self,
        name: &str,
        schema2: bool,
        pins: &RecordPins,
        units: &[UnitSpec],
    ) -> PathBuf {
        let dir = self.root.join("runs").join(name);
        let mut arms = Vec::new();
        let mut arm_names: Vec<&str> = Vec::new();
        for u in units {
            if !arm_names.contains(&u.arm) {
                arm_names.push(u.arm);
            }
            let run_dir = match u.seed {
                Some(s) if schema2 => dir.join(u.arm).join(format!("seed-{s}/output")),
                _ => dir.join(u.arm).join("output"),
            };
            std::fs::create_dir_all(&run_dir).expect("run dir");
            for c in &u.checkpoints {
                scbench_support::write(&run_dir, c);
            }
            if u.interrupted.is_some() {
                // The half-written evaluation an interrupted eval leaves.
                let eval = run_dir.join("mvvault/checkpoint_1/evaluation.json");
                std::fs::create_dir_all(eval.parent().expect("parent")).expect("mkdir");
                std::fs::write(&eval, "{\"pass").expect("truncated evaluation");
            }
            let mut arm = json!({
                "name": u.arm,
                "image": u.image,
                "graded_dir": if u.image == "rdm" { json!("/workspace") } else { Value::Null },
                "agent_config": {},
                "run_config": {},
                "run_dir": run_dir,
                "lane": u.lane,
            });
            if let Some(g) = u.group {
                arm["group"] = json!(g);
            }
            if let Some((signal, step)) = u.interrupted {
                arm["interrupted"] = json!({
                    "signal": signal,
                    "step": step,
                    "at": "2026-01-01T00:30:00.000Z",
                    "killed": false,
                    "removed_containers": ["c1"],
                });
            }
            if schema2 {
                arm["seed"] = json!(u.seed);
                arm["finished"] = if u.finished {
                    json!("2026-01-01T01:00:00.000Z")
                } else {
                    Value::Null
                };
            }
            arms.push(arm);
        }
        let mut problems = json!({
            "repo": "https://example.invalid/scb-problems",
            "sha": PROBLEMS_SHA,
            "subset": ["mvvault", "xjq"],
        });
        if schema2 {
            problems["checkpoints"] = json!({ "mvvault": 2, "xjq": 2 });
        }
        let mut record = json!({
            "manifest": {
                "scbench": { "repo": "https://example.invalid/scbench", "sha": pins.scbench_sha },
                "problems": problems,
                "agent": {
                    "provider": "claude_code_oauth",
                    "model": pins.model,
                    "model_file": "models/opus-5.5.yaml",
                    "thinking": "xhigh",
                    "claude_code_version": "2.1.322",
                    "pass_policy": "all-cases",
                    "environment": "docker-python3.12-uv",
                    "step_limit": 100,
                    "cost_limit": 0.0,
                    "net_cost_limit": 0.0,
                },
                "arm": arm_names.iter().map(|n| json!({ "name": n, "prompt": "p.jinja" })).collect::<Vec<_>>(),
            },
            "manifest_path": self.manifest_path(),
            "rdm": { "sha": pins.rdm_sha },
            "observed": { "uv": "uv 0.0.0", "docker": "99.0.0", "claude": "2.1.322 (Claude Code)" },
            "model_file": null,
            "arms": arms,
            "commands": [],
        });
        if schema2 {
            record["schema"] = json!(2);
            record["seeds"] = json!([1, 2, 3]);
            record["selected_arms"] = json!(arm_names);
        }
        std::fs::create_dir_all(&dir).expect("run dir");
        std::fs::write(
            dir.join("rdm-scbench-run.json"),
            serde_json::to_string_pretty(&record).expect("record json"),
        )
        .expect("write record");
        dir
    }
}

fn report(run: &Path, extra: &[&str]) -> Output {
    let run = run.display().to_string();
    let mut args = vec!["scbench", "report", "--run", run.as_str()];
    args.extend_from_slice(extra);
    measure_support::run(&args)
}

fn report_json(run: &Path, extra: &[&str]) -> Value {
    let mut args = vec!["--format", "json"];
    args.extend_from_slice(extra);
    json(&ok(&report(run, &args)))
}

// --- The main fixture ------------------------------------------------------

/// `(name, group, image)` of the fixture arms; the index is `A` below.
const ARMS: [(&str, &str, &str); 3] = [
    ("just-solve", "control", "stock"),
    ("anti_slop", "control", "stock"),
    ("full-lane", "rdm", "rdm"),
];

/// How a variant of the main fixture shifts its values.
#[derive(Clone, Copy, Default)]
struct Shift {
    /// Added to every checkpoint's `usage.cost`.
    cost: f64,
}

/// Arm `a`, seed `s` of the main fixture. Problems mvvault (`P` = 0) and xjq
/// (`P` = 1), checkpoints `c` = 1, 2 each, so four per unit:
///
/// - the first `passes(a, s)` checkpoints (mvvault 1, 2, xjq 1, 2) pass:
///   just-solve `s`, anti_slop `s - 1`, full-lane `s + 1`
/// - `functions.cc_max` = base(a) + s + 2c + P with base 10, 12, 4;
///   `cc_high_count` = c + s; `depth_max` = 2 + c; `lint.errors` = s·c;
///   `single_use_functions` = c; `trivial_wrappers` = s;
///   `lines.loc` = 100c + 10s; two files with MI 50 + 10c − s each
/// - per checkpoint: `elapsed` 30 + s, `cost` 0.5 + 0.25s + 0.5a (+ shift),
///   5 steps, tokens 1000s / 100 / 10 / 1
fn unit_checkpoints(a: usize, s: u64, shift: Shift) -> Vec<Checkpoint> {
    let passes = match a {
        0 => s,
        1 => s - 1,
        _ => s + 1,
    };
    let base = [10.0, 12.0, 4.0][a];
    let sf = s as f64;
    let mut out = Vec::new();
    for (pi, problem) in ["mvvault", "xjq"].into_iter().enumerate() {
        for c in 1..=2u32 {
            let k = (pi * 2) as u64 + u64::from(c) - 1;
            let cf = f64::from(c);
            out.push(Checkpoint {
                problem: problem.to_owned(),
                name: format!("checkpoint_{c}"),
                order: c,
                eval: Some(Eval {
                    passed: k < passes,
                    infra: false,
                }),
                quality: Some(Quality {
                    cc_max: base + sf + 2.0 * cf + pi as f64,
                    cc_high_count: cf + sf,
                    depth_max: 2.0 + cf,
                    lint_errors: sf * cf,
                    single_use_functions: cf,
                    trivial_wrappers: sf,
                    loc: 100.0 * cf + 10.0 * sf,
                    mi_sum: 2.0 * (50.0 + 10.0 * cf - sf),
                    file_count: 2,
                }),
                inference: Some(Inference {
                    elapsed: 30.0 + sf,
                    cost: 0.5 + 0.25 * sf + 0.5 * a as f64 + shift.cost,
                    steps: 5,
                    input: 1000 * s,
                    output: 100,
                    cache_read: 10,
                    cache_write: 1,
                }),
                stream: None,
            });
        }
    }
    out
}

/// The full-lane lane record for seed `s`: `s - 1` lane failures (seed 2
/// parked; seed 3 parked and escalated).
fn lane(s: u64) -> Value {
    let row = |name: &str, reason: Option<&str>| {
        json!({
            "checkpoint": name,
            "order": 1,
            "plan_repo": null,
            "phase": null,
            "status": null,
            "outcome": if reason.is_some() { "lane-failure" } else { "landed" },
            "reason": reason,
            "detail": null,
        })
    };
    let reasons: Vec<Option<&str>> = match s {
        1 => vec![None, None, None, None],
        2 => vec![Some("parked"), None, None, None],
        _ => vec![Some("parked"), Some("escalated"), None, None],
    };
    json!({ "problems": [
        { "problem": "mvvault", "plan_repo": null,
          "checkpoints": [row("checkpoint_1", reasons[0]), row("checkpoint_2", reasons[1])] },
        { "problem": "xjq", "plan_repo": null,
          "checkpoints": [row("checkpoint_1", reasons[2]), row("checkpoint_2", reasons[3])] },
    ]})
}

fn main_units(shift: Shift) -> Vec<UnitSpec> {
    let mut units = Vec::new();
    for s in 1..=3 {
        for (a, (arm, group, image)) in ARMS.into_iter().enumerate() {
            units.push(UnitSpec {
                arm,
                group: Some(group),
                image,
                seed: Some(s),
                finished: true,
                checkpoints: unit_checkpoints(a, s, shift),
                lane: (image == "rdm").then(|| lane(s)),
                interrupted: None,
            });
        }
    }
    units
}

fn arm<'a>(report: &'a Value, name: &str) -> &'a Value {
    report["arms"]
        .as_array()
        .expect("arms")
        .iter()
        .find(|a| a["name"] == name)
        .unwrap_or_else(|| panic!("no arm {name}"))
}

fn values(stat: &Value) -> Vec<f64> {
    stat["samples"]
        .as_array()
        .expect("samples")
        .iter()
        .map(|s| s["value"].as_f64().expect("value"))
        .collect()
}

/// `stat` has `n` samples equal to `want` and their mean, and an interval
/// within them around the mean.
fn assert_stat(stat: &Value, want: &[f64], ctx: &str) {
    assert_eq!(values(stat), want, "{ctx}: {stat}");
    assert_eq!(stat["n"], want.len(), "{ctx}");
    let mean = want.iter().sum::<f64>() / want.len() as f64;
    assert_eq!(stat["mean"].as_f64(), Some(mean), "{ctx}");
    let lo = stat["ci_low"].as_f64().expect("ci_low");
    let hi = stat["ci_high"].as_f64().expect("ci_high");
    let min = want.iter().copied().fold(f64::INFINITY, f64::min);
    let max = want.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    assert!(
        min <= lo && lo <= mean && mean <= hi && hi <= max,
        "{ctx}: {min} <= {lo} <= {mean} <= {hi} <= {max}"
    );
}

#[test]
fn report_summarises_each_arm_with_seed_cis() {
    let f = Fixture::new();
    let run = f.write_run(
        "a",
        true,
        &RecordPins::default(),
        &main_units(Shift::default()),
    );
    let r = report_json(&run, &[]);
    let names: Vec<_> = r["arms"]
        .as_array()
        .expect("arms")
        .iter()
        .map(|a| a["name"].clone())
        .collect();
    assert_eq!(
        names,
        [json!("just-solve"), json!("anti_slop"), json!("full-lane")]
    );
    let tokens = [4444.0, 8444.0, 12444.0];
    let wall = [124.0, 128.0, 132.0];
    for (name, pass, cost) in [
        ("just-solve", [0.25, 0.5, 0.75], [3.0, 4.0, 5.0]),
        ("anti_slop", [0.0, 0.25, 0.5], [5.0, 6.0, 7.0]),
        ("full-lane", [0.5, 0.75, 1.0], [7.0, 8.0, 9.0]),
    ] {
        let a = arm(&r, name);
        assert_eq!(a["seeds"], json!([1, 2, 3]), "{name}");
        let s = &a["summary"];
        assert_stat(&s["pass_rate"], &pass, &format!("{name} pass_rate"));
        assert_stat(&s["cost_usd"], &cost, &format!("{name} cost_usd"));
        assert_stat(&s["tokens_total"], &tokens, &format!("{name} tokens_total"));
        assert_stat(
            &s["wall_clock_seconds"],
            &wall,
            &format!("{name} wall_clock"),
        );
    }
    let fl = arm(&r, "full-lane");
    assert_stat(
        &fl["summary"]["lane_failures"],
        &[0.0, 1.0, 2.0],
        "lane_failures",
    );
    assert_eq!(
        fl["lane_failure_reasons"],
        json!({ "parked": 2, "escalated": 1 })
    );
    let js = arm(&r, "just-solve");
    assert_eq!(js["summary"]["lane_failures"], Value::Null);
    assert_eq!(js["lane_failure_reasons"], Value::Null);
    assert_eq!(js["group"], "control");
    assert_eq!(fl["group"], "rdm");
    assert_eq!(r["skipped_runs"], json!([]));
    assert_eq!(r["pass_policy"], "all-cases");
    assert_eq!(r["bootstrap"]["resamples"], 10000);
}

#[test]
fn report_gives_per_checkpoint_erosion_and_deltas_with_cis() {
    let f = Fixture::new();
    let run = f.write_run(
        "a",
        true,
        &RecordPins::default(),
        &main_units(Shift::default()),
    );
    let r = report_json(&run, &[]);
    let js = arm(&r, "just-solve");
    let cps: Vec<(String, String)> = js["checkpoints"]
        .as_array()
        .expect("checkpoints")
        .iter()
        .map(|c| {
            (
                c["problem"].as_str().expect("p").to_owned(),
                c["checkpoint"].as_str().expect("c").to_owned(),
            )
        })
        .collect();
    let want: Vec<(String, String)> = [
        ("mvvault", "checkpoint_1"),
        ("mvvault", "checkpoint_2"),
        ("xjq", "checkpoint_1"),
        ("xjq", "checkpoint_2"),
    ]
    .map(|(p, c)| (p.to_owned(), c.to_owned()))
    .into();
    assert_eq!(cps, want);
    let cp2 = &js["checkpoints"][1];
    assert_eq!(cp2["order"], 2);
    let m = &cp2["metrics"];
    assert_stat(&m["cc_max"], &[15.0, 16.0, 17.0], "cc_max");
    assert_stat(&m["cc_high_count"], &[3.0, 4.0, 5.0], "cc_high_count");
    assert_stat(&m["mi_mean"], &[69.0, 68.0, 67.0], "mi_mean");
    assert_stat(&m["max_nesting_depth"], &[4.0, 4.0, 4.0], "nesting");
    assert_stat(&m["lint_errors"], &[2.0, 4.0, 6.0], "lint_errors");
    assert_stat(&m["single_use_functions"], &[2.0, 2.0, 2.0], "single_use");
    assert_stat(&m["trivial_wrappers"], &[1.0, 2.0, 3.0], "trivial_wrappers");
    assert_stat(&cp2["delta"]["cc_max"], &[2.0, 2.0, 2.0], "delta.cc_max");
    // Seed 1 passes only mvvault checkpoint_1; seeds 2 and 3 pass checkpoint_2.
    assert_stat(&cp2["passed"], &[0.0, 1.0, 1.0], "passed");
    assert_eq!(js["checkpoints"][0]["delta"]["cc_max"], Value::Null);

    let s = &js["summary"];
    assert_stat(&s["cc_max_final"], &[15.5, 16.5, 17.5], "cc_max_final");
    assert_stat(&s["cc_max_change"], &[2.0, 2.0, 2.0], "cc_max_change");
    assert_stat(&s["mi_mean_final"], &[69.0, 68.0, 67.0], "mi_mean_final");
}

#[test]
fn report_compares_each_rdm_arm_with_each_control() {
    let f = Fixture::new();
    let run = f.write_run(
        "a",
        true,
        &RecordPins::default(),
        &main_units(Shift::default()),
    );
    let r = report_json(&run, &[]);
    let pairs: Vec<_> = r["comparisons"]
        .as_array()
        .expect("comparisons")
        .iter()
        .map(|c| (c["rdm"].clone(), c["control"].clone()))
        .collect();
    assert_eq!(
        pairs,
        [
            (json!("full-lane"), json!("just-solve")),
            (json!("full-lane"), json!("anti_slop")),
        ]
    );
    let keys: Vec<Value> = r["metrics"]
        .as_array()
        .expect("metrics")
        .iter()
        .map(|m| m["key"].clone())
        .collect();
    for c in r["comparisons"].as_array().expect("comparisons") {
        let got: Vec<Value> = c["metrics"]
            .as_object()
            .expect("metrics")
            .keys()
            .map(|k| json!(k))
            .collect();
        let mut want = keys.clone();
        want.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
        assert_eq!(got, want);
        assert_eq!(c["metrics"]["lane_failures"], Value::Null);
    }
    let vs_js = &r["comparisons"][0]["metrics"];
    for (metric, estimate) in [
        ("pass_rate", 0.25),
        ("cost_usd", 4.0),
        ("cc_max_final", -6.0),
        ("steps", 0.0),
    ] {
        let d = &vs_js[metric];
        assert_eq!(d["estimate"].as_f64(), Some(estimate), "{metric}: {d}");
        let lo = d["ci_low"].as_f64().expect("ci_low");
        let hi = d["ci_high"].as_f64().expect("ci_high");
        assert!(lo <= estimate && estimate <= hi, "{metric}: {d}");
        assert_eq!((d["n_a"].clone(), d["n_b"].clone()), (json!(3), json!(3)));
    }
    assert_eq!(vs_js["cc_max_final"]["excludes_zero"], true);
    assert_eq!(vs_js["steps"]["excludes_zero"], false);
    let vs_as = &r["comparisons"][1]["metrics"];
    assert_eq!(vs_as["pass_rate"]["estimate"].as_f64(), Some(0.5));
}

#[test]
fn report_pass_follows_the_pass_policy() {
    let f = Fixture::new();
    let mut infra = scbench_support::simple("mvvault", "checkpoint_2", 2);
    infra.eval = Some(Eval {
        passed: true,
        infra: true,
    });
    let mut unevaluated = scbench_support::simple("xjq", "checkpoint_1", 1);
    unevaluated.eval = None;
    let mut failing = scbench_support::simple("xjq", "checkpoint_2", 2);
    failing.eval = Some(Eval {
        passed: false,
        infra: false,
    });
    let run = f.write_run(
        "policy",
        true,
        &RecordPins::default(),
        &[UnitSpec {
            arm: "just-solve",
            group: Some("control"),
            image: "stock",
            seed: Some(1),
            finished: true,
            checkpoints: vec![
                scbench_support::simple("mvvault", "checkpoint_1", 1),
                infra,
                unevaluated,
                failing,
            ],
            lane: None,
            interrupted: None,
        }],
    );
    let r = report_json(&run, &[]);
    let js = arm(&r, "just-solve");
    let passed: Vec<_> = js["checkpoints"]
        .as_array()
        .expect("checkpoints")
        .iter()
        .map(|c| (c["checkpoint"].clone(), c["passed"]["mean"].clone()))
        .collect();
    assert_eq!(
        passed,
        [
            (json!("checkpoint_1"), json!(1.0)),
            (json!("checkpoint_2"), json!(0.0)),
            (json!("checkpoint_1"), json!(0.0)),
            (json!("checkpoint_2"), json!(0.0)),
        ]
    );
    assert_eq!(js["summary"]["pass_rate"]["mean"], json!(0.25));
}

/// A full-lane checkpoint the wrapper resumed once: its `agent/stdout.jsonl`
/// holds two `claude` processes of one session, and its
/// `inference_result.json` is what SCBench writes for that stream (the last
/// result's cumulative `total_cost_usd`, every assistant message as a step,
/// the last result's per-turn `usage` as `net_tokens`). The report's cost
/// and steps cover both processes, and its tokens are the final result's
/// `modelUsage`.
#[test]
fn resumed_checkpoint_usage_covers_every_process() {
    let f = Fixture::new();
    let assistant = |id: &str| {
        json!({
            "type": "assistant",
            "message": { "id": id, "usage": { "input_tokens": 1, "output_tokens": 1 } },
        })
        .to_string()
    };
    let init = json!({ "type": "system", "subtype": "init", "session_id": "s1" }).to_string();
    let stream = [
        init.clone(),
        assistant("msg-1"),
        assistant("msg-2"),
        json!({
            "type": "result",
            "session_id": "s1",
            "total_cost_usd": 1.0,
            "usage": { "input_tokens": 2, "output_tokens": 20,
                       "cache_read_input_tokens": 200, "cache_creation_input_tokens": 5 },
            "modelUsage": { "claude-opus-5-5": {
                "inputTokens": 10, "outputTokens": 100,
                "cacheReadInputTokens": 1000, "cacheCreationInputTokens": 50 } },
        })
        .to_string(),
        init,
        assistant("msg-3"),
        json!({
            "type": "result",
            "session_id": "s1",
            "total_cost_usd": 1.5,
            "usage": { "input_tokens": 3, "output_tokens": 30,
                       "cache_read_input_tokens": 300, "cache_creation_input_tokens": 7 },
            "modelUsage": { "claude-opus-5-5": {
                "inputTokens": 15, "outputTokens": 160,
                "cacheReadInputTokens": 1600, "cacheCreationInputTokens": 80 } },
        })
        .to_string(),
    ]
    .join("\n")
        + "\n";
    let mut cp = scbench_support::simple("mvvault", "checkpoint_1", 1);
    cp.inference = Some(Inference {
        elapsed: 60.0,
        cost: 1.5,
        steps: 3,
        input: 3,
        output: 30,
        cache_read: 300,
        cache_write: 7,
    });
    cp.stream = Some(stream);
    let run = f.write_run(
        "resumed",
        true,
        &RecordPins::default(),
        &[UnitSpec {
            arm: "full-lane",
            group: Some("rdm"),
            image: "rdm",
            seed: Some(1),
            finished: true,
            checkpoints: vec![cp],
            lane: None,
            interrupted: None,
        }],
    );
    let r = report_json(&run, &[]);
    let s = &arm(&r, "full-lane")["summary"];
    for (key, want) in [
        ("cost_usd", 1.5),
        ("steps", 3.0),
        ("tokens_input", 15.0),
        ("tokens_output", 160.0),
        ("tokens_cache_read", 1600.0),
        ("tokens_cache_write", 80.0),
        ("tokens_total", 15.0 + 160.0 + 1600.0 + 80.0),
    ] {
        assert_eq!(values(&s[key]), [want], "{key}: {}", s[key]);
    }
}

/// A checkpoint the agent never reached counts as a failure: the pass-rate
/// denominator is the manifest's checkpoint count (mvvault 2, xjq 2), not
/// the checkpoints on disk.
#[test]
fn report_pass_rate_counts_unreached_checkpoints_as_failures() {
    let f = Fixture::new();
    let unit = |seed: u64, checkpoints: Vec<Checkpoint>| UnitSpec {
        arm: "just-solve",
        group: Some("control"),
        image: "stock",
        seed: Some(seed),
        finished: true,
        checkpoints,
        lane: None,
        interrupted: None,
    };
    let run = f.write_run(
        "unreached",
        true,
        &RecordPins::default(),
        &[
            // mvvault checkpoint_2 missing and no xjq directory: 1 of 4.
            unit(
                1,
                vec![scbench_support::simple("mvvault", "checkpoint_1", 1)],
            ),
            // xjq checkpoint_2 missing: 3 of 4.
            unit(
                2,
                vec![
                    scbench_support::simple("mvvault", "checkpoint_1", 1),
                    scbench_support::simple("mvvault", "checkpoint_2", 2),
                    scbench_support::simple("xjq", "checkpoint_1", 1),
                ],
            ),
        ],
    );
    assert!(!run.join("just-solve/seed-1/output/xjq").exists());
    let r = report_json(&run, &[]);
    assert_eq!(
        arm(&r, "just-solve")["summary"]["pass_rate"]["samples"],
        json!([
            { "seed": 1, "value": 0.25 },
            { "seed": 2, "value": 0.75 },
        ])
    );
}

#[test]
fn report_is_deterministic() {
    let f = Fixture::new();
    let run = f.write_run(
        "a",
        true,
        &RecordPins::default(),
        &main_units(Shift::default()),
    );
    ok(&report(&run, &[]));
    let first = std::fs::read(run.join("rdm-scbench-report.json")).expect("report");
    ok(&report(&run, &[]));
    let second = std::fs::read(run.join("rdm-scbench-report.json")).expect("report");
    assert_eq!(first, second);
}

#[test]
fn report_skips_unfinished_runs() {
    let f = Fixture::new();
    let mut units = main_units(Shift::default());
    let unfinished = units
        .iter_mut()
        .find(|u| u.arm == "full-lane" && u.seed == Some(3))
        .expect("unit");
    unfinished.finished = false;
    let run = f.write_run("a", true, &RecordPins::default(), &units);
    let r = report_json(&run, &[]);
    let fl = arm(&r, "full-lane");
    assert_eq!(fl["seeds"], json!([1, 2]));
    assert_eq!(fl["summary"]["pass_rate"]["n"], 2);
    assert_eq!(values(&fl["summary"]["pass_rate"]), [0.5, 0.75]);
    assert_eq!(
        r["skipped_runs"],
        json!([{ "arm": "full-lane", "seed": 3, "reason": "not finished" }])
    );
}

#[test]
fn report_lists_an_interrupted_unit_as_not_finished() {
    let f = Fixture::new();
    let mut units = main_units(Shift::default());
    let interrupted = units
        .iter_mut()
        .find(|u| u.arm == "full-lane" && u.seed == Some(3))
        .expect("unit");
    interrupted.finished = false;
    interrupted.interrupted = Some(("SIGTERM", "run"));
    // One checkpoint's inference files only, beside the truncated evaluation.
    interrupted.checkpoints.truncate(1);
    for c in &mut interrupted.checkpoints {
        c.eval = None;
        c.quality = None;
    }
    let run = f.write_run("a", true, &RecordPins::default(), &units);
    let o = report(&run, &["--format", "json"]);
    assert_eq!(o.status.code(), Some(0));
    let r = json(&ok(&o));
    assert_eq!(
        r["skipped_runs"],
        json!([{
            "arm": "full-lane",
            "seed": 3,
            "reason": "not finished: interrupted by SIGTERM during run",
        }])
    );
    assert_eq!(arm(&r, "full-lane")["seeds"], json!([1, 2]));
    let md = ok(&report(&run, &[]));
    assert!(
        md.contains("## Skipped runs\n\n- full-lane seed 3: not finished: interrupted by SIGTERM during run\n"),
        "{md}"
    );
}

#[test]
fn report_reads_a_phase_1_to_4_record() {
    let f = Fixture::new();
    let units: Vec<UnitSpec> = [
        ("just-solve", "stock", 0usize),
        ("just-solve-rdm", "rdm", 2),
    ]
    .into_iter()
    .map(|(name, image, a)| UnitSpec {
        arm: name,
        group: None,
        image,
        seed: None,
        finished: true,
        checkpoints: unit_checkpoints(a, 2, Shift::default()),
        lane: None,
        interrupted: None,
    })
    .collect();
    let run = f.write_run("old", false, &RecordPins::default(), &units);
    let r = report_json(&run, &[]);
    for (name, group, pass) in [
        ("just-solve", "control", 0.5),
        ("just-solve-rdm", "rdm", 0.75),
    ] {
        let a = arm(&r, name);
        assert_eq!(a["group"], group, "{name}");
        assert_eq!(a["seeds"], json!([null]), "{name}");
        let p = &a["summary"]["pass_rate"];
        assert_eq!(p["n"], 1, "{name}");
        assert_eq!(p["ci_low"], Value::Null, "{name}");
        assert_eq!(p["ci_high"], Value::Null, "{name}");
        // No manifest checkpoint counts: the denominator is the four observed.
        assert_eq!(
            p["samples"],
            json!([{ "seed": null, "value": pass }]),
            "{name}"
        );
    }
    let d = &r["comparisons"][0]["metrics"]["pass_rate"];
    assert_eq!(d["estimate"].as_f64(), Some(0.25));
    assert_eq!(d["ci_low"], Value::Null);
}

#[test]
fn report_writes_markdown_and_json() {
    let f = Fixture::new();
    let run = f.write_run(
        "a",
        true,
        &RecordPins::default(),
        &main_units(Shift::default()),
    );
    let stdout = ok(&report(&run, &[]));
    let md = std::fs::read_to_string(run.join("rdm-scbench-report.md")).expect("md report");
    assert_eq!(stdout, md);
    let report =
        json(&std::fs::read_to_string(run.join("rdm-scbench-report.json")).expect("json report"));
    assert_eq!(report["schema"], 2);
    assert_eq!(report["run"], json!(run));
}

#[test]
fn report_names_a_malformed_scbench_file() {
    let f = Fixture::new();
    let run = f.write_run(
        "a",
        true,
        &RecordPins::default(),
        &main_units(Shift::default()),
    );
    let bad = run.join("anti_slop/seed-2/output/xjq/checkpoint_1/evaluation.json");
    std::fs::write(&bad, "{not json").expect("corrupt");
    let err = fails(&report(&run, &[]));
    assert!(err.contains(&bad.display().to_string()), "stderr: {err}");
    assert!(!run.join("rdm-scbench-report.json").exists());
}

// --- Baselines -------------------------------------------------------------

#[test]
fn save_baseline_writes_the_five_pins() {
    let f = Fixture::new();
    let run = f.write_run(
        "a",
        true,
        &RecordPins::default(),
        &main_units(Shift::default()),
    );
    ok(&report(&run, &["--save-baseline", "first"]));
    let path = f.baseline("first");
    let b = json(&std::fs::read_to_string(&path).expect("baseline written"));
    assert_eq!(b["name"], "first");
    assert_eq!(
        b["pins"],
        json!({
            "rdm_sha": RDM_SHA,
            "scbench_sha": SCBENCH_SHA,
            "problems_sha": PROBLEMS_SHA,
            "model": "opus-5.5",
            "claude_code_version": "2.1.322",
        })
    );
    assert_eq!(b["observed_claude"], "2.1.322 (Claude Code)");
    assert_eq!(b["source_run"], json!(run));
    assert_eq!(b["report"]["baseline"], Value::Null);
    for a in b["report"]["arms"].as_array().expect("arms") {
        assert_eq!(
            a["summary"]["pass_rate"]["samples"]
                .as_array()
                .map(Vec::len),
            Some(3),
            "{}",
            a["name"]
        );
    }
}

#[test]
fn save_baseline_refuses_an_existing_name_and_a_bad_name() {
    let f = Fixture::new();
    let run = f.write_run(
        "a",
        true,
        &RecordPins::default(),
        &main_units(Shift::default()),
    );
    ok(&report(&run, &["--save-baseline", "first"]));
    let path = f.baseline("first");
    let before = std::fs::read(&path).expect("baseline");
    let err = fails(&report(&run, &["--save-baseline", "first"]));
    assert!(err.contains(&path.display().to_string()), "stderr: {err}");
    assert_eq!(std::fs::read(&path).expect("baseline"), before);

    let other = f.write_run(
        "b",
        true,
        &RecordPins::default(),
        &main_units(Shift::default()),
    );
    let err = fails(&report(&other, &["--save-baseline", "../x"]));
    assert!(
        err.contains("--save-baseline") && err.contains("../x"),
        "stderr: {err}"
    );
    assert!(!f.root.join("rdm/bench/scbench/x.json").exists());
    assert!(!f.baseline("x").exists());
    assert!(!other.join("rdm-scbench-report.json").exists());
    assert!(!other.join("rdm-scbench-report.md").exists());
}

#[test]
fn save_baseline_refuses_a_missing_pin() {
    let f = Fixture::new();
    let pins = RecordPins {
        rdm_sha: None,
        ..RecordPins::default()
    };
    let run = f.write_run("a", true, &pins, &main_units(Shift::default()));
    let err = fails(&report(&run, &["--save-baseline", "first"]));
    assert!(err.contains("rdm_sha"), "stderr: {err}");
    assert!(!f.baseline("first").exists());
}

#[test]
fn baseline_comparison_places_runs_side_by_side() {
    let f = Fixture::new();
    let a = f.write_run(
        "a",
        true,
        &RecordPins::default(),
        &main_units(Shift::default()),
    );
    ok(&report(&a, &["--save-baseline", "first"]));
    let pins = RecordPins {
        rdm_sha: Some(OTHER_RDM_SHA),
        ..RecordPins::default()
    };
    let b = f.write_run("b", true, &pins, &main_units(Shift { cost: 1.0 }));
    let o = report(&b, &["--baseline", "first", "--format", "json"]);
    assert_eq!(o.status.code(), Some(0));
    let r = json(&ok(&o));
    let cmp = &r["baseline"];
    assert_eq!(cmp["name"], "first");
    assert_eq!(cmp["comparable"], true);
    assert_eq!(cmp["pin_differences"], json!([]));
    assert_eq!(
        cmp["rdm_sha"],
        json!({ "current": OTHER_RDM_SHA, "baseline": RDM_SHA })
    );
    assert_eq!(
        cmp["unmatched_arms"],
        json!({ "current_only": [], "baseline_only": [] })
    );
    for (name, pass, cost_a) in [
        ("just-solve", 0.5, 4.0),
        ("anti_slop", 0.25, 6.0),
        ("full-lane", 0.75, 8.0),
    ] {
        let x = cmp["arms"]
            .as_array()
            .expect("arms")
            .iter()
            .find(|x| x["name"] == name)
            .unwrap_or_else(|| panic!("no baseline arm {name}"));
        let m = &x["metrics"];
        assert_eq!(
            m["pass_rate"]["current"]["mean"].as_f64(),
            Some(pass),
            "{name}"
        );
        assert_eq!(
            m["pass_rate"]["baseline"]["mean"].as_f64(),
            Some(pass),
            "{name}"
        );
        assert_eq!(
            m["pass_rate"]["delta"]["estimate"].as_f64(),
            Some(0.0),
            "{name}"
        );
        // Run B pays 1 more per checkpoint, four checkpoints per seed.
        let cost = &m["cost_usd"];
        assert_eq!(cost["baseline"]["mean"].as_f64(), Some(cost_a), "{name}");
        assert_eq!(
            cost["current"]["mean"].as_f64(),
            Some(cost_a + 4.0),
            "{name}"
        );
        let d = &cost["delta"];
        assert_eq!(d["estimate"].as_f64(), Some(4.0), "{name}");
        let (lo, hi) = (
            d["ci_low"].as_f64().expect("lo"),
            d["ci_high"].as_f64().expect("hi"),
        );
        assert!(lo <= 4.0 && 4.0 <= hi, "{name}: {d}");
    }
}

#[test]
fn baseline_comparison_flags_differing_pins() {
    let f = Fixture::new();
    let a = f.write_run(
        "a",
        true,
        &RecordPins::default(),
        &main_units(Shift::default()),
    );
    ok(&report(&a, &["--save-baseline", "first"]));
    let pins = RecordPins {
        scbench_sha: OTHER_SCBENCH_SHA,
        model: "opus-6",
        ..RecordPins::default()
    };
    let mut units = main_units(Shift::default());
    units.push(UnitSpec {
        arm: "extra",
        group: Some("control"),
        image: "stock",
        seed: Some(1),
        finished: true,
        checkpoints: unit_checkpoints(0, 1, Shift::default()),
        lane: None,
        interrupted: None,
    });
    let c = f.write_run("c", true, &pins, &units);
    let o = report(&c, &["--baseline", "first", "--format", "json"]);
    assert_eq!(o.status.code(), Some(0));
    let r = json(&ok(&o));
    let cmp = &r["baseline"];
    assert_eq!(cmp["comparable"], false);
    assert_eq!(
        cmp["pin_differences"],
        json!([
            { "pin": "scbench_sha", "current": OTHER_SCBENCH_SHA, "baseline": SCBENCH_SHA },
            { "pin": "model", "current": "opus-6", "baseline": "opus-5.5" },
        ])
    );
    assert_eq!(
        cmp["unmatched_arms"],
        json!({ "current_only": ["extra"], "baseline_only": [] })
    );
}

// --- Application code, tests and regressions --------------------------------

/// A file row with `loc`, `lint`, `mi`, `single_use`, `trivial`.
fn file(
    path: &'static str,
    loc: f64,
    lint: f64,
    mi: f64,
    single_use: f64,
    trivial: f64,
) -> FileFixture {
    FileFixture {
        path,
        loc,
        lint_errors: lint,
        mi,
        single_use,
        trivial,
    }
}

fn symbol(path: &'static str, kind: &'static str, complexity: f64, depth: f64) -> SymbolFixture {
    SymbolFixture {
        path,
        kind,
        complexity,
        depth,
    }
}

/// The split fixture's rows at checkpoint `k` (1-3). Both arms have the
/// application files `main.py` (growing 10 lines a checkpoint) and
/// `lib/core.py`; just-solve's `main.py` carries 10 more lint findings.
/// Only full-lane has test files: `tests/test_core.py`, `tests/conftest.py`
/// and `test_cli.py`.
fn split_rows(k: u32, with_tests: bool) -> (Vec<FileFixture>, Vec<SymbolFixture>) {
    let main_loc = 100.0 + 10.0 * f64::from(k - 1);
    let main_lint = if with_tests { 3.0 } else { 13.0 };
    let mut files = vec![
        file("main.py", main_loc, main_lint, 60.0, 2.0, 1.0),
        file("lib/core.py", 200.0, 5.0, 40.0, 1.0, 0.0),
    ];
    let mut symbols = vec![
        symbol("main.py", "function", 11.0, 3.0),
        symbol("lib/core.py", "method", 10.0, 4.0),
        symbol("lib/core.py", "function", 12.0, 2.0),
        symbol("lib/core.py", "class", 30.0, 9.0),
        symbol("lib/core.py", "variable", 0.0, 0.0),
    ];
    if with_tests {
        files.extend([
            file("tests/test_core.py", 300.0, 20.0, 30.0, 10.0, 0.0),
            file("tests/conftest.py", 50.0, 4.0, 50.0, 1.0, 0.0),
            file("test_cli.py", 70.0, 6.0, 40.0, 3.0, 1.0),
        ]);
        symbols.extend([
            symbol("tests/test_core.py", "function", 2.0, 1.0),
            symbol("tests/test_core.py", "function", 15.0, 5.0),
            symbol("tests/conftest.py", "function", 1.0, 1.0),
            symbol("test_cli.py", "method", 3.0, 2.0),
        ]);
    }
    (files, symbols)
}

/// One `evaluation.json` test group: `(key, passed, failed)`.
type TestGroup<'a> = (&'a str, &'a [&'a str], &'a [&'a str]);

/// A run of full-lane (rdm) and just-solve (control), one seed each, over
/// mvvault checkpoints 1-3. Their `overall_quality.json` is
/// [`scbench_support::simple`]'s (`cc_max` 5, `sloc` 50, ...), deliberately
/// unlike the split figures. Full-lane's hidden test `t_b` passes at
/// checkpoint 1 and fails as a regression at checkpoints 2 and 3;
/// just-solve's never fails.
fn split_run(f: &Fixture, name: &str, pins: &RecordPins) -> PathBuf {
    let checkpoints: Vec<Checkpoint> = (1..=3)
        .map(|k| scbench_support::simple("mvvault", &format!("checkpoint_{k}"), k))
        .collect();
    let unit = |arm, group, image| UnitSpec {
        arm,
        group: Some(group),
        image,
        seed: Some(1),
        finished: true,
        checkpoints: checkpoints.clone(),
        lane: None,
        interrupted: None,
    };
    let run = f.write_run(
        name,
        true,
        pins,
        &[
            unit("full-lane", "rdm", "rdm"),
            unit("just-solve", "control", "stock"),
        ],
    );
    for (arm, rdm) in [("full-lane", true), ("just-solve", false)] {
        let out = run.join(arm).join("seed-1/output");
        let failing: &[&str] = if rdm { &["t_b"] } else { &[] };
        let passing: &[&str] = if rdm { &["t_a"] } else { &["t_a", "t_b"] };
        let results: [Vec<TestGroup>; 3] = [
            vec![("checkpoint_1-Core", &["t_a", "t_b"], &[])],
            vec![
                ("checkpoint_1-Regression", passing, failing),
                ("checkpoint_2-Core", &["t_c"], &[]),
            ],
            vec![
                ("checkpoint_1-Regression", passing, failing),
                ("checkpoint_2-Regression", &["t_c"], &[]),
                ("checkpoint_3-Core", &["t_d"], &[]),
            ],
        ];
        for (i, groups) in results.iter().enumerate() {
            let k = i as u32 + 1;
            let cp = format!("checkpoint_{k}");
            let (files, symbols) = split_rows(k, rdm);
            scbench_support::write_quality_rows(&out, "mvvault", &cp, &files, &symbols);
            scbench_support::write_test_results(&out, "mvvault", &cp, groups);
        }
    }
    run
}

/// The single value of a one-seed stat, `None` for `null`.
fn one(stat: &Value) -> Option<f64> {
    if stat.is_null() {
        return None;
    }
    let v = values(stat);
    assert_eq!(v.len(), 1, "{stat}");
    Some(v[0])
}

#[test]
fn application_metrics_count_only_application_files_and_tests_only_test_files() {
    let f = Fixture::new();
    let run = split_run(&f, "split", &RecordPins::default());
    let r = report_json(&run, &[]);
    let fl = arm(&r, "full-lane");
    let s = &fl["summary"];
    // main.py + lib/core.py at checkpoint 3; the class (cc 30, depth 9) and
    // the cc-10 method do not count toward cc_max / cc_high_count.
    for (key, want) in [
        ("app_cc_max_final", 12.0),
        ("app_cc_high_count_final", 2.0),
        ("app_mi_mean_final", 50.0),
        ("app_max_nesting_depth_final", 4.0),
        ("app_lint_errors_final", 8.0),
        ("app_single_use_functions_final", 3.0),
        ("app_trivial_wrappers_final", 1.0),
        ("app_sloc_final", 320.0),
        ("app_sloc_change", 20.0),
        ("app_lint_errors_change", 0.0),
        ("tests_cc_max_final", 15.0),
        ("tests_cc_high_count_final", 1.0),
        ("tests_mi_mean_final", 40.0),
        ("tests_max_nesting_depth_final", 5.0),
        ("tests_lint_errors_final", 30.0),
        ("tests_single_use_functions_final", 14.0),
        ("tests_trivial_wrappers_final", 1.0),
        ("tests_sloc_final", 420.0),
        ("tests_file_count_final", 3.0),
        ("tests_code_ratio_final", 420.0 / 320.0),
    ] {
        assert_eq!(one(&s[key]), Some(want), "full-lane {key}: {}", s[key]);
    }
    let cp3 = &fl["checkpoints"][2];
    assert_eq!(cp3["checkpoint"], "checkpoint_3");
    assert_eq!(one(&cp3["application"]["sloc"]), Some(320.0));
    assert_eq!(one(&cp3["application"]["lint_errors"]), Some(8.0));
    assert_eq!(one(&cp3["application_delta"]["sloc"]), Some(10.0));
    assert_eq!(one(&cp3["tests"]["sloc"]), Some(420.0));
    assert_eq!(one(&cp3["tests"]["file_count"]), Some(3.0));
    assert_eq!(
        fl["checkpoints"][0]["application_delta"]["sloc"],
        Value::Null
    );

    // just-solve wrote no tests: an empty test set, no test MI.
    let js = &arm(&r, "just-solve")["summary"];
    assert_eq!(one(&js["app_lint_errors_final"]), Some(18.0));
    assert_eq!(one(&js["tests_sloc_final"]), Some(0.0));
    assert_eq!(one(&js["tests_file_count_final"]), Some(0.0));
    assert_eq!(one(&js["tests_code_ratio_final"]), Some(0.0));
    assert_eq!(js["tests_mi_mean_final"], Value::Null);
}

#[test]
fn scbench_snapshot_figures_keep_their_keys_labelled_as_including_tests() {
    let f = Fixture::new();
    let run = split_run(&f, "split", &RecordPins::default());
    let r = report_json(&run, &[]);
    let s = &arm(&r, "full-lane")["summary"];
    // overall_quality.json's figures, not the per-file ones.
    for (key, want) in [
        ("cc_max_final", 5.0),
        ("sloc_final", 50.0),
        ("lint_errors_final", 1.0),
        ("sloc_change", 0.0),
    ] {
        assert_eq!(one(&s[key]), Some(want), "{key}: {}", s[key]);
    }
    assert_eq!(
        one(&arm(&r, "full-lane")["checkpoints"][2]["metrics"]["cc_max"]),
        Some(5.0)
    );
    let scope = |key: &str| {
        r["metrics"]
            .as_array()
            .expect("metrics")
            .iter()
            .find(|m| m["key"] == key)
            .unwrap_or_else(|| panic!("no metric {key}"))["scope"]
            .clone()
    };
    assert_eq!(scope("cc_max_final"), "snapshot");
    assert_eq!(scope("sloc_change"), "snapshot");
    assert_eq!(scope("app_cc_max_final"), "application");
    assert_eq!(scope("tests_sloc_final"), "tests");
    assert_eq!(scope("pass_rate"), Value::Null);
    let md = ok(&report(&run, &[]));
    assert!(
        md.contains("| cc_max_final (SCBench snapshot, includes tests) |"),
        "{md}"
    );
}

#[test]
fn regressions_are_counted_once_and_compared_with_the_control() {
    let f = Fixture::new();
    let run = split_run(&f, "split", &RecordPins::default());
    let r = report_json(&run, &[]);
    let fl = arm(&r, "full-lane");
    let s = &fl["summary"];
    // Regression tests: 1 of 2 at checkpoint 2, 2 of 3 at checkpoint 3.
    assert_eq!(one(&s["regression_pass_rate"]), Some(3.0 / 5.0));
    assert_eq!(one(&s["regressions"]), Some(1.0));
    assert_eq!(one(&s["regressions_persisting"]), Some(1.0));
    let per_cp = |field: &str| -> Vec<Option<f64>> {
        fl["checkpoints"]
            .as_array()
            .expect("checkpoints")
            .iter()
            .map(|c| one(&c[field]))
            .collect()
    };
    assert_eq!(per_cp("regression_passed"), [None, Some(1.0), Some(2.0)]);
    assert_eq!(per_cp("regression_total"), [None, Some(2.0), Some(3.0)]);
    assert_eq!(per_cp("regressions_new"), [Some(0.0), Some(1.0), Some(0.0)]);
    assert_eq!(
        per_cp("regressions_open"),
        [Some(0.0), Some(1.0), Some(1.0)]
    );

    let js = &arm(&r, "just-solve")["summary"];
    assert_eq!(one(&js["regression_pass_rate"]), Some(1.0));
    assert_eq!(one(&js["regressions"]), Some(0.0));

    let vs = &r["comparisons"][0];
    assert_eq!(
        (vs["rdm"].clone(), vs["control"].clone()),
        (json!("full-lane"), json!("just-solve"))
    );
    let estimate = |key: &str| {
        vs["metrics"][key]["estimate"]
            .as_f64()
            .unwrap_or_else(|| panic!("{key}"))
    };
    assert_eq!(estimate("app_lint_errors_final"), -10.0);
    assert_eq!(estimate("regressions"), 1.0);
    assert!((estimate("regression_pass_rate") + 0.4).abs() < 1e-12);
    assert_eq!(estimate("tests_sloc_final"), 420.0);
}

#[test]
fn markdown_reports_application_tests_regressions_and_the_snapshot_apart() {
    let f = Fixture::new();
    let run = split_run(&f, "split", &RecordPins::default());
    let md = ok(&report(&run, &[]));
    let at = |heading: &str| {
        md.find(heading)
            .unwrap_or_else(|| panic!("no {heading}: {md}"))
    };
    let order = [
        at("## Summary\n"),
        at("## Application code\n"),
        at("## Tests\n"),
        at("## Regression tests\n"),
        at("## SCBench snapshot (includes tests)\n"),
        at("## mvvault per checkpoint\n"),
        at("## rdm vs control\n"),
    ];
    assert!(order.windows(2).all(|w| w[0] < w[1]), "{order:?}\n{md}");
    assert!(!md.contains("No per-file quality data"), "{md}");
    assert!(md.contains("counts once"), "{md}");
    // full-lane checkpoint 2: 1 of 2 regression tests, one new regression.
    assert!(
        md.contains("| full-lane | checkpoint_2 | 0 | 1/2 | 1 |"),
        "{md}"
    );
}

#[test]
fn a_run_without_per_file_rows_reports_no_split() {
    let f = Fixture::new();
    let run = f.write_run(
        "a",
        true,
        &RecordPins::default(),
        &main_units(Shift::default()),
    );
    let r = report_json(&run, &[]);
    let s = &arm(&r, "full-lane")["summary"];
    assert_eq!(s["app_cc_max_final"], Value::Null);
    assert_eq!(s["tests_sloc_final"], Value::Null);
    assert_eq!(s["cc_max_final"]["n"], 3);
    let md = ok(&report(&run, &[]));
    assert!(
        md.contains(
            "No per-file quality data (quality_analysis/files.jsonl, symbols.jsonl) in this run."
        ),
        "{md}"
    );
}

#[test]
fn a_baseline_from_before_the_split_still_compares_on_the_snapshot() {
    let f = Fixture::new();
    let a = split_run(&f, "a", &RecordPins::default());
    ok(&report(&a, &["--save-baseline", "old"]));
    // Rewrite it as a schema-1 report saved it: no scope, no split or
    // regression metrics or checkpoint fields.
    let path = f.baseline("old");
    let mut b = json(&std::fs::read_to_string(&path).expect("baseline"));
    let new_key =
        |k: &str| k.starts_with("app_") || k.starts_with("tests_") || k.starts_with("regression");
    let report = &mut b["report"];
    report["schema"] = json!(1);
    let metrics: Vec<Value> = report["metrics"]
        .as_array()
        .expect("metrics")
        .iter()
        .filter(|m| !new_key(m["key"].as_str().expect("key")))
        .map(|m| json!({ "key": m["key"], "better": m["better"] }))
        .collect();
    report["metrics"] = json!(metrics);
    for arm in report["arms"].as_array_mut().expect("arms") {
        arm["summary"]
            .as_object_mut()
            .expect("summary")
            .retain(|k, _| !new_key(k));
        for c in arm["checkpoints"].as_array_mut().expect("checkpoints") {
            let c = c.as_object_mut().expect("checkpoint");
            for field in [
                "application",
                "application_delta",
                "tests",
                "regression_passed",
                "regression_total",
                "regressions_new",
                "regressions_open",
            ] {
                assert!(c.remove(field).is_some(), "{field}");
            }
        }
    }
    for c in report["comparisons"].as_array_mut().expect("comparisons") {
        c["metrics"]
            .as_object_mut()
            .expect("metrics")
            .retain(|k, _| !new_key(k));
    }
    std::fs::write(&path, serde_json::to_string_pretty(&b).expect("json")).expect("rewrite");

    let pins = RecordPins {
        rdm_sha: Some(OTHER_RDM_SHA),
        ..RecordPins::default()
    };
    let current = split_run(&f, "b", &pins);
    let r = report_json(&current, &["--baseline", "old"]);
    let cmp = &r["baseline"];
    assert_eq!(cmp["baseline_has_split"], false);
    assert_eq!(cmp["comparable"], true);
    let fl = cmp["arms"]
        .as_array()
        .expect("arms")
        .iter()
        .find(|x| x["name"] == "full-lane")
        .expect("full-lane");
    let m = &fl["metrics"];
    assert_eq!(m["cc_max_final"]["delta"]["estimate"].as_f64(), Some(0.0));
    assert_eq!(m["app_cc_max_final"]["baseline"], Value::Null);
    assert_eq!(m["app_cc_max_final"]["delta"], Value::Null);
    assert_eq!(one(&m["app_cc_max_final"]["current"]), Some(12.0));
    assert_eq!(m["regressions"]["baseline"], Value::Null);
    let md = ok(&self::report(&current, &["--baseline", "old"]));
    assert!(
        md.contains("> baseline `old` predates the application/tests split (report schema 1)"),
        "{md}"
    );

    // A baseline saved by this schema has the split.
    ok(&self::report(&current, &["--save-baseline", "new"]));
    let r = report_json(&a, &["--baseline", "new"]);
    assert_eq!(r["baseline"]["baseline_has_split"], true);
}

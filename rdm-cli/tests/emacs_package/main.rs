//! The Emacs package suite: byte-compiles `editors/emacs/*.el` with warnings
//! as errors and runs every `editors/emacs/test/*-test.el` in a child Emacs,
//! through the batch driver `editors/emacs/test/run.el`.
//!
//! Each pass gets a fresh [`SeededPlan`] and runs under its [`Sandbox`]
//! (no inherited `RDM_*`, private `HOME`/XDG dirs), so the developer's
//! `$RDM_ROOT` is never visible. The real-binary ERT tests find the binary
//! under test and the fixture through `RDM_TEST_BIN`, `RDM_TEST_ROOT` and
//! `RDM_TEST_PROJECT`; `EDITOR`/`VISUAL` are `false`.
//!
//! - [`emacs_suite_bare`] runs `emacs -Q`: every test tagged for a soft
//!   dependency (evil, magit, consult, embark, markdown-mode) must be
//!   skipped.
//! - [`emacs_suite_with_init_dir`] runs with `--init-directory
//!   $RDM_EMACS_INIT_DIR` (built from `editors/emacs/test/ci-init/init.el`),
//!   loading its `init.el` first: every soft-dependency-tagged test must
//!   run and pass, and evil, consult, embark and markdown-mode must each
//!   have one.
//!
//! `emacs` is `$RDM_EMACS`, else the first `emacs` on `PATH`. Without one,
//! or without `RDM_EMACS_INIT_DIR` for the second pass, a test prints a skip
//! notice and passes — unless `RDM_EMACS_REQUIRED` is set (CI), when it
//! fails.

#[macro_use]
#[path = "../common/workflow_support.rs"]
mod workflow_support;

#[path = "../git_test_support.rs"]
mod git_test_support;

#[path = "../common/plan_fixture.rs"]
mod plan_fixture;

#[path = "../common/seeded_plan.rs"]
mod seeded_plan;

use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::seeded_plan::{PROJECT, SESSION, SeededPlan};
use crate::workflow_support::repo_root;

/// How long one child Emacs pass may run before it is killed.
const PASS_DEADLINE: Duration = Duration::from_secs(120);

/// The soft dependencies whose tagged tests the passes police.
const SOFT_DEPS: &[&str] = &["evil", "magit", "consult", "embark", "markdown-mode"];

/// The soft dependencies the suite already has tagged tests for; the
/// init-directory pass requires at least one of each to run.
const TESTED_SOFT_DEPS: &[&str] = &["evil", "magit", "consult", "embark", "markdown-mode"];

/// Whether a missing `emacs` or init directory fails instead of skipping.
fn required() -> bool {
    std::env::var_os("RDM_EMACS_REQUIRED").is_some_and(|v| !v.is_empty())
}

/// Skips (or, under `RDM_EMACS_REQUIRED`, fails) with `why`.
fn skip(why: &str) {
    assert!(!required(), "RDM_EMACS_REQUIRED is set but {why}");
    eprintln!("SKIP emacs_package: {why}");
}

/// The Emacs to run: `$RDM_EMACS`, else `emacs` on `PATH`.
fn find_emacs() -> Option<PathBuf> {
    if let Some(e) = std::env::var_os("RDM_EMACS").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(e));
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join("emacs"))
        .find(|candidate| candidate.is_file())
}

/// One test's record from the driver's results file.
struct TestResult {
    name: String,
    tags: Vec<String>,
    result: String,
}

/// Runs one pass: `emacs <pre_args> --batch [-l init] -l run.el`, bounded by
/// [`PASS_DEADLINE`]; returns the per-test results, panicking with the
/// child's output on a non-zero exit.
fn run_pass(emacs: &Path, pre_args: &[&str], init: Option<&Path>) -> Vec<TestResult> {
    let plan = SeededPlan::new(PROJECT).unwrap_or_else(|f| panic!("{f}"));
    let out_dir = plan.dir.path().join("emacs-out");
    std::fs::create_dir_all(&out_dir).expect("create output dir");
    let results = out_dir.join("results.json");
    let stdout_path = out_dir.join("stdout");
    let stderr_path = out_dir.join("stderr");
    let run_el = repo_root().join("editors/emacs/test/run.el");
    // The ERT tests' temp files (stubs, logs, byte-compile output) land
    // inside the fixture's TempDir and go with it.
    let tmp = out_dir.join("tmp");
    std::fs::create_dir_all(&tmp).expect("create child TMPDIR");

    let mut cmd: Command = plan.sandbox.command(emacs);
    cmd.args(pre_args).arg("--batch");
    if let Some(init) = init {
        cmd.arg("-l").arg(init);
    }
    cmd.arg("-l")
        .arg(&run_el)
        .current_dir(plan.dir.path())
        .env("RDM_TEST_BIN", env!("CARGO_BIN_EXE_rdm"))
        .env("RDM_TEST_ROOT", &plan.plan)
        .env("RDM_TEST_PROJECT", &plan.project)
        .env("RDM_SESSION", SESSION)
        .env("RDM_EMACS_RESULTS", &results)
        .env("TMPDIR", &tmp)
        .env("EDITOR", "false")
        .env("VISUAL", "false")
        .stdin(Stdio::null())
        .stdout(File::create(&stdout_path).expect("create stdout file"))
        .stderr(File::create(&stderr_path).expect("create stderr file"));

    let status = wait_bounded(&mut cmd);
    let output = format!(
        "--- stdout ---\n{}\n--- stderr ---\n{}",
        std::fs::read_to_string(&stdout_path).unwrap_or_default(),
        std::fs::read_to_string(&stderr_path).unwrap_or_default()
    );
    let status = status.unwrap_or_else(|why| panic!("emacs pass {why}\n{output}"));
    assert!(status.success(), "emacs pass failed ({status})\n{output}");

    let text = std::fs::read_to_string(&results)
        .unwrap_or_else(|e| panic!("reading {}: {e}\n{output}", results.display()));
    let parsed: Value = serde_json::from_str(&text).expect("results file is JSON");
    let records: Vec<TestResult> = parsed
        .as_array()
        .expect("results file is an array")
        .iter()
        .map(|r| TestResult {
            name: r["name"].as_str().unwrap_or_default().to_owned(),
            tags: r["tags"]
                .as_array()
                .map(|t| {
                    t.iter()
                        .filter_map(|s| s.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default(),
            result: r["result"].as_str().unwrap_or_default().to_owned(),
        })
        .collect();
    assert!(!records.is_empty(), "the suite ran no tests\n{output}");
    for r in &records {
        assert_ne!(r.result, "failed", "{} failed\n{output}", r.name);
    }
    records
}

/// Spawns `cmd` and waits at most [`PASS_DEADLINE`], killing it on expiry.
fn wait_bounded(cmd: &mut Command) -> Result<ExitStatus, String> {
    let mut child = cmd.spawn().map_err(|e| format!("could not start: {e}"))?;
    let deadline = Instant::now() + PASS_DEADLINE;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("timed out after {PASS_DEADLINE:?}"));
            }
            Err(e) => return Err(format!("wait failed: {e}")),
        }
    }
}

/// The records tagged for soft dependency `dep`.
fn tagged<'r>(records: &'r [TestResult], dep: &str) -> Vec<&'r TestResult> {
    records
        .iter()
        .filter(|r| r.tags.iter().any(|t| t == dep))
        .collect()
}

#[test]
fn emacs_suite_bare() {
    let Some(emacs) = find_emacs() else {
        skip("no `emacs` on PATH (set RDM_EMACS to point at one)");
        return;
    };
    let records = run_pass(&emacs, &["-Q"], None);
    let evil = tagged(&records, "evil");
    assert!(!evil.is_empty(), "no evil-tagged test in the suite");
    for dep in SOFT_DEPS {
        for r in tagged(&records, dep) {
            assert_eq!(
                r.result, "skipped",
                "{} is tagged {dep} and must skip under emacs -Q",
                r.name
            );
        }
    }
    // The real-binary tests must run, not skip, when the suite supplies
    // the binary and fixture.
    for r in records.iter().filter(|r| r.name.contains("/real-")) {
        assert_eq!(r.result, "passed", "{} did not run", r.name);
    }
}

#[test]
fn emacs_suite_with_init_dir() {
    let Some(emacs) = find_emacs() else {
        skip("no `emacs` on PATH (set RDM_EMACS to point at one)");
        return;
    };
    let Some(dir) = std::env::var_os("RDM_EMACS_INIT_DIR").filter(|v| !v.is_empty()) else {
        skip("RDM_EMACS_INIT_DIR is unset; the soft-dependency pass did not run");
        return;
    };
    let dir = PathBuf::from(dir);
    assert!(
        dir.is_dir(),
        "RDM_EMACS_INIT_DIR={} is not a directory",
        dir.display()
    );
    let init = dir.join("init.el");
    let dir_arg = dir.to_string_lossy().into_owned();
    let records = run_pass(
        &emacs,
        &["-q", "--init-directory", &dir_arg],
        init.is_file().then_some(init.as_path()),
    );
    for dep in TESTED_SOFT_DEPS {
        assert!(
            !tagged(&records, dep).is_empty(),
            "no {dep}-tagged test in the suite"
        );
    }
    for dep in SOFT_DEPS {
        for r in tagged(&records, dep) {
            assert_eq!(
                r.result, "passed",
                "{} is tagged {dep} and must run with RDM_EMACS_INIT_DIR",
                r.name
            );
        }
    }
}

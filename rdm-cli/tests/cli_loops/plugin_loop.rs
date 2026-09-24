//! The argv/JSON contract an editor integration relies on when it shells
//! out to rdm: a create → list → update → show → search → info round trip
//! read back as JSON, `create` waiting for stdin EOF, and errors and
//! warnings staying on stderr so `--format json` stdout always parses.
//!
//! The body-resolution rules (`--body` beats stdin, update never reads
//! stdin, `--body ""` refused with a `--clear-body` pointer, `--clear-body`
//! empties) are covered by `cli_task.rs`'s `body_flag_beats_stdin`,
//! `task_update_body_flag_beats_stdin`, `task_update_tags_ignores_stdin`,
//! `task_update_status_ignores_stdin`, `task_update_empty_body_refuses_clobber`
//! and `task_update_clear_body_succeeds`.

use std::process::Stdio;
use std::time::{Duration, Instant};

use rdm_devtools::process::{Hooks, ProcessSpec, Session, run_bounded};
use serde_json::Value;

use crate::seeded_plan::{SESSION, SeededPlan};
use crate::{field, items, must, stderr, stdout};

const PROJECT: &str = "plugin-loop";
const STDIN_BODY: &str =
    "Body content supplied entirely over stdin for the plugin round-trip test.";

fn fixture() -> SeededPlan {
    must(SeededPlan::new(PROJECT).and_then(SeededPlan::with_code_repo))
}

fn json(fx: &SeededPlan, args: &[&str]) -> Value {
    must(fx.json(args))
}

/// The bounded spec for `rdm --root <plan> <args>` under the fixture sandbox.
fn spec(fx: &SeededPlan, args: &[&str], timeout: Duration) -> ProcessSpec {
    let mut spec = ProcessSpec::new(crate::plan_fixture::rdm_bin())
        .arg("--root")
        .arg(&fx.plan)
        .args(args)
        .cwd(fx.dir.path())
        .timeout(timeout);
    for key in crate::plan_fixture::Sandbox::removals() {
        spec = spec.env_remove(key);
    }
    for (key, value) in fx.sandbox.vars() {
        spec = spec.env(key, value);
    }
    spec.env("RDM_SESSION", SESSION)
}

#[test]
fn editor_round_trip() {
    let fx = fixture();
    let mut create = fx.command_in(
        fx.dir.path(),
        &[
            "task",
            "create",
            "widget-loop",
            "--title",
            "Widget Loop Task",
            "--no-edit",
            "--project",
            PROJECT,
        ],
    );
    let mut child = create
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn create");
    {
        use std::io::Write;
        let mut stdin = child.stdin.take().expect("stdin");
        writeln!(stdin, "{STDIN_BODY}").expect("write body");
    }
    let out = child.wait_with_output().expect("wait create");
    assert!(
        out.status.success(),
        "create with a stdin body: {}",
        stderr(&out)
    );

    let listed = json(
        &fx,
        &["task", "list", "--format", "json", "--project", PROJECT],
    );
    let task = items(&listed)
        .into_iter()
        .find(|t| t["slug"] == "widget-loop")
        .unwrap_or_else(|| panic!("task list shows the new task: {listed}"));
    assert_eq!(task["title"], "Widget Loop Task");

    must(fx.ok(&[
        "task",
        "update",
        "widget-loop",
        "--status",
        "in-progress",
        "--no-edit",
        "--project",
        PROJECT,
    ]));
    let shown = json(
        &fx,
        &[
            "task",
            "show",
            "widget-loop",
            "--format",
            "json",
            "--project",
            PROJECT,
        ],
    );
    assert_eq!(shown["status"], "in-progress");
    assert_eq!(
        shown["body"].as_str().map(str::trim_end),
        Some(STDIN_BODY),
        "the stdin body round-trips: {shown}"
    );

    let found = json(
        &fx,
        &[
            "search",
            "Widget Lop Task",
            "--format",
            "json",
            "--project",
            PROJECT,
        ],
    );
    assert!(
        field(&found, "identifier").contains(&"widget-loop".to_owned()),
        "a one-letter typo still finds the task: {found}"
    );

    let info = json(&fx, &["info", "--format", "json", "--project", PROJECT]);
    assert_eq!(info["project"], PROJECT);
}

#[test]
fn create_waits_for_stdin_eof() {
    let fx = fixture();
    let blocked = [
        "task",
        "create",
        "stdin-blocks",
        "--title",
        "Stdin Blocks",
        "--no-edit",
        "--project",
        PROJECT,
    ];
    let show = |slug: &str| {
        must(fx.run(&[
            "task",
            "show",
            slug,
            "--format",
            "json",
            "--project",
            PROJECT,
        ]))
    };
    // stdin is a pipe the test holds open and never writes to.
    let session = Session::spawn(&spec(&fx, &blocked, Duration::from_secs(30))).expect("spawn");
    std::thread::sleep(Duration::from_secs(1));
    assert!(
        !show("stdin-blocks").status.success(),
        "create finished before its stdin reached EOF"
    );
    let started = Instant::now();
    session
        .shutdown()
        .unwrap_or_else(|e| panic!("create did not exit 0 promptly once stdin closed: {e}"));
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(
        show("stdin-blocks").status.success(),
        "the task exists once stdin closed"
    );

    // stdin already at EOF: returns at once.
    let quick = spec(
        &fx,
        &[
            "task",
            "create",
            "stdin-eof-immediately",
            "--title",
            "Stdin EOF Immediately",
            "--no-edit",
            "--project",
            PROJECT,
        ],
        Duration::from_secs(10),
    );
    let started = Instant::now();
    let out = run_bounded(&quick, Hooks::new()).expect("create with stdin at EOF");
    assert!(out.status.success(), "{out:?}");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "create with stdin at EOF returns at once"
    );
    assert!(show("stdin-eof-immediately").status.success());
}

#[test]
fn errors_and_warnings_stay_on_stderr() {
    let fx = fixture();
    let bad = must(fx.run(&[
        "task",
        "show",
        "definitely-not-a-real-slug",
        "--project",
        PROJECT,
    ]));
    assert!(!bad.status.success(), "a missing slug fails");
    let err = stderr(&bad);
    assert!(
        !err.trim().is_empty(),
        "the failure explains itself on stderr"
    );
    assert!(!err.contains("panicked"), "no panic leaks: {err}");
    assert!(
        bad.stdout.is_empty(),
        "stdout stays empty: {}",
        stdout(&bad)
    );

    // needs_review_warning reads the invoking process's cwd: a branched code
    // repo with no new commit triggers it.
    let code = must(fx.code()).to_owned();
    must(
        fx.sandbox
            .git(&code, &["checkout", "--quiet", "-b", "other-branch"]),
    );
    must(fx.ok(&[
        "task",
        "create",
        "warning-target",
        "--title",
        "Warning Target",
        "--no-edit",
        "--project",
        PROJECT,
    ]));
    let warned = must(fx.run_in(
        &code,
        &[
            "task",
            "update",
            "warning-target",
            "--status",
            "needs-review",
            "--no-edit",
            "--project",
            PROJECT,
        ],
    ));
    assert!(warned.status.success(), "{}", stderr(&warned));
    let warning = stderr(&warned);
    assert!(
        warning.lines().any(|l| l.starts_with("warning:")),
        "the needs-review warning is printed on stderr: {warning}"
    );
    assert!(
        !stdout(&warned).lines().any(|l| l.starts_with("warning:")),
        "the warning never reaches stdout"
    );
    let shown = must(fx.run_in(
        &code,
        &[
            "task",
            "show",
            "warning-target",
            "--format",
            "json",
            "--project",
            PROJECT,
        ],
    ));
    let parsed: Value = serde_json::from_slice(&shown.stdout)
        .unwrap_or_else(|e| panic!("JSON stdout stays clean: {e}: {}", stdout(&shown)));
    assert_eq!(parsed["slug"], "warning-target");
}

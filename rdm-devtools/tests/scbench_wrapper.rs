//! `bench/scbench/agent/claude-rdm.sh`, the rdm arms' agent binary, run
//! with SlopCodeBench's argument shape against fake `claude` and `rdm`
//! executables in a temp dir. Nothing here touches a real plan repo, a real
//! `claude` or Docker; every assertion reads the fakes' recorded argv and
//! environment, the files they and the wrapper wrote, and the wrapper's
//! exit status and output.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// The fake credential the wrapper must pass through and never print.
const TOKEN: &str = "fake-oauth-token-do-not-print";
/// The prompt SlopCodeBench passes after `--`.
const PROMPT: &str = "PROMPT";

/// SlopCodeBench's argv for a fresh checkpoint attempt.
fn scbench_args() -> Vec<String> {
    [
        "--output-format",
        "stream-json",
        "--verbose",
        "--model",
        "m",
        "--max-turns",
        "1000",
        "--permission-mode",
        "bypassPermissions",
        "--plugin-dir",
        "/opt/rdm/plugin",
        "--print",
        "--",
        PROMPT,
    ]
    .map(str::to_owned)
    .to_vec()
}

/// SlopCodeBench's argv for its own `retry()` of the attempt.
fn scbench_retry_args() -> Vec<String> {
    let mut args = scbench_args();
    args.insert(3, "--continue".to_owned());
    args
}

/// `rdm phase list --format json`'s shape for `phases` `(number, status)`.
fn phases_json(phases: &[(u32, &str)]) -> String {
    let rows: Vec<serde_json::Value> = phases
        .iter()
        .map(|(n, status)| {
            let stem = format!("phase-{n}-checkpoint-{n}");
            serde_json::json!({
                "number": n,
                "stem": stem,
                "ref": format!("solution/{stem}"),
                "review_ref": format!("phase/solution/{stem}"),
                "title": format!("Checkpoint {n}"),
                "status": status,
            })
        })
        .collect();
    serde_json::to_string_pretty(&rows).expect("phases json")
}

/// One scenario: the plan repo's phases before the first call, the phases
/// each call leaves (`None` leaves them as they are) and each call's exit.
#[derive(Default)]
struct Scenario {
    initial: Option<Vec<(u32, &'static str)>>,
    after_call: Vec<Option<Vec<(u32, &'static str)>>>,
    exits: Vec<i32>,
    env: Vec<(&'static str, &'static str)>,
    baseline: Option<&'static str>,
}

struct Harness {
    _tmp: tempfile::TempDir,
    root: PathBuf,
}

impl Harness {
    fn new(s: &Scenario) -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = std::fs::canonicalize(tmp.path()).expect("canonical tmp");
        for d in ["bin", "home", "plan", "tmp", "log", "script"] {
            std::fs::create_dir_all(root.join(d)).expect("mkdir");
        }
        if let Some(p) = &s.initial {
            std::fs::write(root.join("plan/phases.json"), phases_json(p)).expect("initial");
        }
        for (i, p) in s.after_call.iter().enumerate() {
            if let Some(p) = p {
                std::fs::write(
                    root.join(format!("script/phases.{}.json", i + 1)),
                    phases_json(p),
                )
                .expect("phases script");
            }
        }
        for (i, e) in s.exits.iter().enumerate() {
            std::fs::write(root.join(format!("script/exit.{}", i + 1)), e.to_string())
                .expect("exit script");
        }
        if let Some(b) = s.baseline {
            std::fs::write(root.join("tmp/claude-rdm.baseline"), format!("{b}\n"))
                .expect("baseline");
        }
        let r = root.display();
        write_exe(
            &root.join("bin/claude"),
            &format!(
                r#"#!/bin/sh
r='{r}'
k=$(( $(cat "$r/count" 2>/dev/null || echo 0) + 1 ))
echo "$k" >"$r/count"
printf '%s\0' "$@" >"$r/log/claude.$k"
printf 'CEILING=%s\nFORCE=%s\n' "${{CLAUDE_CODE_PRINT_BG_WAIT_CEILING_MS-unset}}" \
    "${{FORCE_AUTO_BACKGROUND_TASKS-unset}}" >"$r/log/claude.$k.env"
: >"$r/plan/call-$k"
if [ -f "$r/script/phases.$k.json" ]; then
    cp "$r/script/phases.$k.json" "$r/plan/phases.json"
fi
echo '{{"type":"result","call":'"$k"'}}'
e=0
if [ -f "$r/script/exit.$k" ]; then e=$(cat "$r/script/exit.$k"); fi
exit "$e"
"#
            ),
        );
        write_exe(
            &root.join("bin/rdm"),
            &format!(
                r#"#!/bin/sh
r='{r}'
if [ "$1" = --root ] && [ "$3" = init ]; then
    mkdir -p "$2/.git"
    echo "fake rdm: initialized $2"
    exit 0
fi
if [ "$*" = "--root $r/plan phase list --roadmap solution --project bench --format json" ]; then
    if [ -f "$r/plan/phases.json" ]; then
        cat "$r/plan/phases.json"
        exit 0
    fi
    echo "error: failed to list phases: roadmap not found: solution" >&2
    exit 1
fi
echo "fake rdm: unexpected arguments: $*" >&2
exit 2
"#
            ),
        );
        Self { _tmp: tmp, root }
    }

    fn run(&self, args: &[String], s: &Scenario) -> Output {
        let script =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../bench/scbench/agent/claude-rdm.sh");
        let mut cmd = Command::new("sh");
        cmd.arg(script)
            .args(args)
            .env_clear()
            .env(
                "PATH",
                format!("{}:/usr/bin:/bin", self.root.join("bin").display()),
            )
            .env("HOME", self.root.join("home"))
            .env("TMPDIR", self.root.join("tmp"))
            .env("RDM_ROOT", self.root.join("plan"))
            .env("RDM_PROJECT", "bench")
            .env("RDM_ROADMAP", "solution")
            .env("RDM_PLAN_EXPORT_SUBDIR", "projects/-workspace/rdm-plan")
            .env("FORCE_AUTO_BACKGROUND_TASKS", "1")
            .env("CLAUDE_CODE_OAUTH_TOKEN", TOKEN);
        for (k, v) in &s.env {
            cmd.env(k, v);
        }
        let out = cmd.output().expect("run claude-rdm.sh");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(!stdout.contains(TOKEN), "stdout leaks the token: {stdout}");
        assert!(!stderr.contains(TOKEN), "stderr leaks the token: {stderr}");
        let want: String = (1..=self.calls())
            .map(|k| format!("{{\"type\":\"result\",\"call\":{k}}}\n"))
            .collect();
        assert_eq!(
            stdout, want,
            "stdout holds only claude's stream; stderr: {stderr}"
        );
        out
    }

    fn calls(&self) -> usize {
        std::fs::read_to_string(self.root.join("count"))
            .map_or(0, |s| s.trim().parse().expect("count"))
    }

    fn argv(&self, k: usize) -> Vec<String> {
        let raw = std::fs::read(self.root.join(format!("log/claude.{k}"))).expect("argv log");
        raw.split(|b| *b == 0)
            .filter(|s| !s.is_empty())
            .map(|s| String::from_utf8(s.to_vec()).expect("utf8 arg"))
            .collect()
    }

    fn env_log(&self, k: usize) -> String {
        std::fs::read_to_string(self.root.join(format!("log/claude.{k}.env"))).expect("env log")
    }

    fn export_dir(&self) -> PathBuf {
        self.root.join("home/.claude/projects/-workspace/rdm-plan")
    }
}

fn write_exe(path: &Path, text: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, text).expect("write fake");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// Call `k`'s argv is the resume of `original`: `--continue` (unless
/// `original` holds one), `original` up to `--`, then `--` and a
/// continuation prompt other than [`PROMPT`].
fn assert_resume_argv(argv: &[String], original: &[String], add_continue: bool) {
    let sep = original.iter().position(|a| a == "--").expect("--");
    let mut want: Vec<String> = Vec::new();
    if add_continue {
        want.push("--continue".to_owned());
    }
    want.extend_from_slice(&original[..sep]);
    want.push("--".to_owned());
    assert_eq!(argv.len(), want.len() + 1, "{argv:?}");
    assert_eq!(&argv[..want.len()], want.as_slice(), "{argv:?}");
    let prompt = &argv[want.len()];
    assert!(!prompt.is_empty() && prompt != PROMPT, "{prompt}");
}

#[test]
fn resumes_once_until_the_phase_settles() {
    let s = Scenario {
        after_call: vec![Some(vec![(1, "in-progress")]), Some(vec![(1, "done")])],
        exits: vec![0, 0],
        ..Scenario::default()
    };
    let h = Harness::new(&s);
    let args = scbench_args();
    let out = h.run(&args, &s);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(h.calls(), 2);
    assert_eq!(h.argv(1), args);
    assert_resume_argv(&h.argv(2), &args, true);
    assert!(
        stderr(&out).contains("claude-rdm: phase 1 is in-progress; resuming (1/6)"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn scbench_retry_is_not_given_a_second_continue() {
    // The previous attempt created phase 2 over a baseline of 1; a
    // recomputed baseline (2) would never see phase 2 settle.
    let s = Scenario {
        initial: Some(vec![(1, "done"), (2, "in-progress")]),
        after_call: vec![None, Some(vec![(1, "done"), (2, "done")])],
        exits: vec![0, 0],
        baseline: Some("1"),
        ..Scenario::default()
    };
    let h = Harness::new(&s);
    let args = scbench_retry_args();
    let out = h.run(&args, &s);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(h.calls(), 2);
    let argv = h.argv(2);
    assert_eq!(argv.iter().filter(|a| *a == "--continue").count(), 1);
    assert_resume_argv(&argv, &args, false);
    assert_eq!(
        std::fs::read_to_string(h.root.join("tmp/claude-rdm.baseline")).expect("baseline"),
        "1\n"
    );
}

#[test]
fn a_checkpoint_without_its_new_phase_resumes() {
    let s = Scenario {
        initial: Some(vec![(1, "done")]),
        after_call: vec![None, Some(vec![(1, "done"), (2, "done")])],
        exits: vec![0, 0],
        ..Scenario::default()
    };
    let h = Harness::new(&s);
    let out = h.run(&scbench_args(), &s);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(h.calls(), 2);
    assert!(
        stderr(&out).contains("claude-rdm: phase none is missing; resuming (1/6)"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn raises_the_print_background_ceiling_only_when_unset() {
    let s = Scenario {
        after_call: vec![Some(vec![(1, "done")])],
        ..Scenario::default()
    };
    let h = Harness::new(&s);
    ok_status(&h.run(&scbench_args(), &s));
    assert_eq!(h.env_log(1), "CEILING=7200000\nFORCE=1\n");

    let preset = Scenario {
        after_call: vec![Some(vec![(1, "done")])],
        env: vec![("CLAUDE_CODE_PRINT_BG_WAIT_CEILING_MS", "5")],
        ..Scenario::default()
    };
    let h = Harness::new(&preset);
    ok_status(&h.run(&scbench_args(), &preset));
    assert_eq!(h.env_log(1), "CEILING=5\nFORCE=1\n");
}

fn ok_status(out: &Output) {
    assert_eq!(out.status.code(), Some(0), "{}", stderr(out));
}

/// AC2's scenario: never settles, `RDM_RESUME_MAX=2`, exits 11, 12, 13.
fn never_settles() -> Scenario {
    Scenario {
        after_call: vec![Some(vec![(1, "in-progress")])],
        exits: vec![11, 12, 13],
        env: vec![("RDM_RESUME_MAX", "2")],
        ..Scenario::default()
    }
}

#[test]
fn stops_after_resume_max_with_the_last_exit_status() {
    let s = never_settles();
    let h = Harness::new(&s);
    let args = scbench_args();
    let out = h.run(&args, &s);
    assert_eq!(out.status.code(), Some(13), "{}", stderr(&out));
    assert_eq!(h.calls(), 3);
    assert_resume_argv(&h.argv(2), &args, true);
    assert_resume_argv(&h.argv(3), &args, true);
    assert!(
        stderr(&out).contains("claude-rdm: checkpoint not settled after 2 resume(s)"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn a_settled_first_call_does_not_resume() {
    for status in ["done", "blocked", "wont-fix"] {
        let s = Scenario {
            after_call: vec![Some(vec![(1, status)])],
            ..Scenario::default()
        };
        let h = Harness::new(&s);
        let out = h.run(&scbench_args(), &s);
        assert_eq!(out.status.code(), Some(0), "{status}: {}", stderr(&out));
        assert_eq!(h.calls(), 1, "{status}");
        assert!(
            !stderr(&out).contains("resuming"),
            "{status}: {}",
            stderr(&out)
        );
    }
}

#[test]
fn a_reviewed_phase_does_not_settle() {
    let s = Scenario {
        after_call: vec![Some(vec![(1, "reviewed")]), Some(vec![(1, "done")])],
        ..Scenario::default()
    };
    let h = Harness::new(&s);
    let out = h.run(&scbench_args(), &s);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(h.calls(), 2);
    assert!(
        stderr(&out).contains("claude-rdm: phase 1 is reviewed; resuming (1/6)"),
        "{}",
        stderr(&out)
    );
}

/// A scenario whose phase never settles, whose first call exits 7, with
/// `env` over the defaults.
fn never_settles_with(env: Vec<(&'static str, &'static str)>) -> Scenario {
    Scenario {
        after_call: vec![Some(vec![(1, "in-progress")])],
        exits: vec![7, 8, 9],
        env,
        ..Scenario::default()
    }
}

#[test]
fn an_invalid_resume_max_does_not_resume() {
    let s = never_settles_with(vec![("RDM_RESUME_MAX", "abc")]);
    let h = Harness::new(&s);
    let out = h.run(&scbench_args(), &s);
    assert_eq!(out.status.code(), Some(7), "{}", stderr(&out));
    assert_eq!(h.calls(), 1);
    assert!(
        stderr(&out)
            .contains("claude-rdm: RDM_RESUME_MAX=abc is not a non-negative integer; not resuming"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn a_zero_resume_max_does_not_resume() {
    let s = never_settles_with(vec![("RDM_RESUME_MAX", "0")]);
    let h = Harness::new(&s);
    let out = h.run(&scbench_args(), &s);
    assert_eq!(out.status.code(), Some(7), "{}", stderr(&out));
    assert_eq!(h.calls(), 1);
    assert!(
        stderr(&out).contains("claude-rdm: checkpoint not settled after 0 resume(s)"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn arguments_without_a_separator_are_not_resumed() {
    let s = never_settles_with(Vec::new());
    let h = Harness::new(&s);
    let args: Vec<String> = ["--model", "m", "--print", PROMPT]
        .map(str::to_owned)
        .to_vec();
    let out = h.run(&args, &s);
    assert_eq!(out.status.code(), Some(7), "{}", stderr(&out));
    assert_eq!(h.calls(), 1);
    assert_eq!(h.argv(1), args);
    assert!(
        stderr(&out)
            .contains("claude-rdm: phase 1 is in-progress; no -- in the arguments, not resuming"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn every_claude_exit_exports_the_plan_repo() {
    let s = never_settles();
    let h = Harness::new(&s);
    h.run(&scbench_args(), &s);
    let mut names: Vec<String> = std::fs::read_dir(h.export_dir())
        .expect("export dir")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, ["000001.tar", "000002.tar", "000003.tar"]);
    for k in 1..=3 {
        let into = h.root.join(format!("extract-{k}"));
        std::fs::create_dir_all(&into).expect("mkdir");
        let status = Command::new("tar")
            .arg("-xf")
            .arg(h.export_dir().join(format!("{k:06}.tar")))
            .arg("-C")
            .arg(&into)
            .status()
            .expect("tar");
        assert!(status.success(), "tar -xf {k}");
        assert!(
            into.join(format!("call-{k}")).exists(),
            "export {k} holds call-{k}"
        );
        assert!(
            !into.join(format!("call-{}", k + 1)).exists(),
            "export {k} was written before call {}",
            k + 1
        );
        assert!(
            into.join(".git").is_dir(),
            "export {k} holds the plan repo's .git"
        );
    }
}

//! `rdm-smoke codex-coexistence`, driven as a real child process against the
//! `rdm-devtools-fixture` fake `codex` and fake `rdm` roles (symlinked under
//! those names, each answering from a scenario file beside it).
//!
//! No test here needs a JavaScript runtime, a real Codex, credentials, the
//! network or the real `~/.claude`: every evidence root lands in a per-test
//! `TMPDIR`, and the "login file" is a non-secret marker. The real catalog and
//! live invocation run only in the `#[ignore]`d `codex_coexistence_live`.

mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::Duration;

use common::{ReapGuard, gone_within, read_pid, smoke, wait_for_file};
use rustix::process::{Pid, Signal, kill_process};
use serde_json::{Value, json};
use tempfile::TempDir;

const MARKER: &str = "NONSECRET-AUTH-MARKER-7f3c";

struct Fixture {
    tmp: TempDir,
}

impl Fixture {
    fn new() -> Self {
        let f = Self {
            tmp: tempfile::tempdir().expect("tempdir"),
        };
        for d in ["bin", "evidence"] {
            std::fs::create_dir_all(f.tmp.path().join(d)).expect("mkdir");
        }
        for role in ["codex", "rdm"] {
            std::os::unix::fs::symlink(common::fixture(), f.bin(role)).expect("symlink");
            f.scenario(role, json!({}));
        }
        std::fs::write(f.auth(), format!("{{\"token\":\"{MARKER}\"}}\n")).expect("auth");
        f
    }

    fn bin(&self, role: &str) -> PathBuf {
        self.tmp.path().join("bin").join(role)
    }

    fn record(&self) -> PathBuf {
        self.tmp.path().join("calls.jsonl")
    }

    fn pidfile(&self) -> PathBuf {
        self.tmp.path().join("child.pid")
    }

    fn auth(&self) -> PathBuf {
        self.tmp.path().join("auth.json")
    }

    fn scenario(&self, role: &str, mut v: Value) {
        v["record"] = json!(self.record());
        v["pidfile"] = json!(self.pidfile());
        std::fs::write(
            self.tmp
                .path()
                .join("bin")
                .join(format!("{role}.scenario.json")),
            v.to_string(),
        )
        .expect("scenario");
    }

    fn command(&self, extra: &[&str]) -> Command {
        let mut c = Command::new(smoke());
        c.arg("codex-coexistence")
            .arg("--rdm")
            .arg(self.bin("rdm"))
            .arg("--codex")
            .arg(self.bin("codex"))
            .args(extra)
            .env("TMPDIR", self.tmp.path().join("evidence"))
            .stdin(Stdio::null());
        c
    }

    fn run(&self, extra: &[&str]) -> Output {
        self.command(extra).output().expect("rdm-smoke runs")
    }

    fn run_live(&self, extra: &[&str]) -> Output {
        let auth = self.auth();
        let mut args = vec!["--copy-auth-from", auth.to_str().expect("utf-8")];
        args.extend_from_slice(extra);
        self.run(&args)
    }

    fn calls(&self, role: &str) -> Vec<Value> {
        std::fs::read_to_string(self.record())
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str::<Value>(l).expect("record"))
            .filter(|c| c["role"] == role)
            .collect()
    }
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

/// The evidence root printed on the first line.
fn root_of(out: &Output) -> PathBuf {
    let stdout = text(&out.stdout);
    let line = stdout.lines().next().unwrap_or_default();
    PathBuf::from(
        line.strip_prefix("Coexistence fixture: ")
            .unwrap_or_else(|| panic!("no evidence line: {stdout}")),
    )
}

fn assert_exit(out: &Output, code: i32) -> String {
    let err = text(&out.stderr);
    assert_eq!(
        out.status.code(),
        Some(code),
        "stdout:\n{}\nstderr:\n{err}",
        text(&out.stdout)
    );
    err
}

fn result_json(root: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(root.join("result.json")).expect("result.json"))
        .expect("json")
}

#[test]
fn discovery_only_passes_and_reports_live_not_run() {
    let f = Fixture::new();
    let out = f.run(&[]);
    assert_exit(&out, 0);
    let root = root_of(&out);
    let stdout = text(&out.stdout);
    assert!(stdout.contains("live invocation NOT RUN"), "{stdout}");
    assert!(stdout.contains(&format!("Evidence: {}", root.display())));
    let r = result_json(&root);
    assert_eq!(r["liveInvocation"], false, "never reported as a live pass");
    assert_eq!(r["authCopyRemoved"], false);
    assert_eq!(r["discoveredCopies"], 3);
    assert!(
        f.calls("codex").iter().all(|c| c["argv"][0] != "exec"),
        "no exec without consent"
    );
    let catalog: Value =
        serde_json::from_str(&std::fs::read_to_string(root.join("catalog.json")).expect("catalog"))
            .expect("json");
    assert_eq!(catalog.as_array().map(Vec::len), Some(3));
}

#[test]
fn live_path_passes_with_marker_auth_and_removes_copy() {
    let f = Fixture::new();
    let out = f.run_live(&[]);
    assert_exit(&out, 0);
    let root = root_of(&out);
    assert!(text(&out.stdout).contains("live repository invocation passed"));
    assert!(
        !root.join("config/auth.json").exists(),
        "the private login copy is removed"
    );
    assert!(f.auth().exists(), "the source login file is untouched");
    let exec: Vec<Value> = f
        .calls("codex")
        .into_iter()
        .filter(|c| c["argv"][0] == "exec")
        .collect();
    assert_eq!(exec.len(), 1);
    assert_eq!(
        exec[0]["authCopyPresent"], true,
        "the copy existed while exec ran"
    );
    let argv: Vec<&str> = exec[0]["argv"]
        .as_array()
        .expect("argv")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(
        argv[..5],
        ["exec", "--ephemeral", "--sandbox", "read-only", "--json"]
    );
    let r = result_json(&root);
    assert_eq!(
        (
            r["liveInvocation"].as_bool(),
            r["authCopyRemoved"].as_bool()
        ),
        (Some(true), Some(true))
    );
    assert_eq!(
        r["selectedPath"],
        "source/.agents/skills/rdm-roadmap/SKILL.md"
    );
}

#[test]
fn child_env_is_isolated_and_rdm_vars_scrubbed() {
    let f = Fixture::new();
    let out = f
        .command(&[])
        .env("RDM_ROOT", "/should/not/leak")
        .env("RDM_PROJECT", "leak")
        .env("RDM_ANYTHING", "leak")
        .env("GIT_DIR", "/should/not/leak/.git")
        .env("GIT_INDEX_FILE", "/should/not/leak/index")
        .output()
        .expect("runs");
    assert_exit(&out, 0);
    let root = root_of(&out);
    let calls: Vec<Value> = f.calls("codex").into_iter().chain(f.calls("rdm")).collect();
    assert!(calls.len() >= 4);
    for c in &calls {
        let keys: Vec<&str> = c["envKeys"]
            .as_array()
            .expect("keys")
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert!(
            !keys.iter().any(|k| k.starts_with("RDM_")),
            "an RDM_ variable leaked: {keys:?}"
        );
        assert_eq!(c["env"]["HOME"], json!(root.join("home")));
        assert_eq!(c["env"]["CODEX_HOME"], json!(root.join("config")));
        assert_eq!(
            c["env"]["XDG_CONFIG_HOME"],
            json!(root.join("home/.config"))
        );
        assert_eq!(c["env"]["GIT_CONFIG_GLOBAL"], "/dev/null");
        assert_eq!(c["env"]["GIT_CONFIG_SYSTEM"], "/dev/null");
    }
}

#[test]
fn rdm_invoked_with_codex_skills_project_and_out() {
    let f = Fixture::new();
    let out = f.run(&[]);
    assert_exit(&out, 0);
    let root = root_of(&out);
    let rdm = f.calls("rdm");
    assert_eq!(rdm.len(), 1);
    let source = root.join("source");
    assert_eq!(
        rdm[0]["argv"],
        json!([
            "agent-config",
            "codex",
            "--skills",
            "--project",
            "coexistence",
            "--out",
            source
        ])
    );
    assert_eq!(rdm[0]["cwd"], json!(source));
}

fn fails_naming(f: &Fixture, scenario: Value, needle: &str) -> PathBuf {
    f.scenario("codex", scenario);
    let out = f.run(&[]);
    let err = assert_exit(&out, 1);
    let root = root_of(&out);
    assert!(err.contains(needle), "{err}");
    assert!(
        err.contains(&format!("Evidence: {}", root.display())),
        "{err}"
    );
    root
}

#[test]
fn catalog_missing_copy_fails_naming_path() {
    let f = Fixture::new();
    let root_marker = "/home/.agents/skills/";
    f.scenario("codex", json!({"appServer": {"omit": root_marker}}));
    let out = f.run(&[]);
    let err = assert_exit(&out, 1);
    let root = root_of(&out);
    assert!(
        err.contains(&format!(
            "Missing enabled skill: {}",
            root.join("home/.agents/skills/rdm-roadmap/SKILL.md")
                .display()
        )),
        "{err}"
    );
}

#[test]
fn catalog_disabled_copy_fails() {
    let f = Fixture::new();
    fails_naming(
        &f,
        json!({"appServer": {"disable": "plugins/cache"}}),
        "Missing enabled skill:",
    );
}

#[test]
fn catalog_errors_fail() {
    let f = Fixture::new();
    fails_naming(
        &f,
        json!({"appServer": {"catalogErrors": [{"message": "bad skill"}]}}),
        "the skill catalog reported errors",
    );
}

#[test]
fn app_server_error_reply_fails() {
    let f = Fixture::new();
    fails_naming(
        &f,
        json!({"appServer": {"errorReply": true}}),
        "Codex skill discovery returned an error",
    );
}

#[test]
fn app_server_early_exit_fails() {
    let f = Fixture::new();
    fails_naming(
        &f,
        json!({"appServer": {"exitBeforeReply": true}}),
        "Codex exited before discovery",
    );
}

#[test]
fn discovery_timeout_kills_and_reaps_app_server() {
    let f = Fixture::new();
    f.scenario("codex", json!({"appServer": {"hang": true}}));
    let out = f.run(&["--discovery-timeout-secs", "1"]);
    let err = assert_exit(&out, 124);
    assert!(err.contains("Codex skill discovery timed out"), "{err}");
    let pid = read_pid(&f.pidfile()).expect("the hanging app-server wrote its pid");
    let mut guard = ReapGuard::default();
    guard.track(pid);
    assert!(
        gone_within(pid, Duration::from_secs(3)),
        "the app-server survived its deadline"
    );
    guard.clear();
}

#[test]
fn live_wrong_selected_path_fails_and_removes_auth_copy() {
    let f = Fixture::new();
    f.scenario("codex", json!({"exec": {"wrongPath": true}}));
    let out = f.run_live(&[]);
    let err = assert_exit(&out, 1);
    assert!(err.contains("not the repository skill"), "{err}");
    assert!(!root_of(&out).join("config/auth.json").exists());
    assert!(
        !root_of(&out).join("result.json").exists(),
        "no result for a failed run"
    );
}

#[test]
fn live_competing_copy_answer_fails() {
    let f = Fixture::new();
    f.scenario(
        "codex",
        json!({"exec": {"planGate": "needs-plan-review, per USER_COPY_ONLY"}}),
    );
    let out = f.run_live(&[]);
    let err = assert_exit(&out, 1);
    assert!(err.contains("quotes a competing copy"), "{err}");
}

#[test]
fn protected_byte_change_fails_even_when_live_fails() {
    let f = Fixture::new();
    f.scenario(
        "codex",
        json!({"exec": {"mutateProtected": true, "exit": 7}}),
    );
    let out = f.run_live(&[]);
    let err = assert_exit(&out, 1);
    let root = root_of(&out);
    assert!(
        err.contains(&format!(
            "Changed competing copy: {}",
            root.join("home/.agents/skills/rdm-roadmap/SKILL.md")
                .display()
        )),
        "{err}"
    );
    assert!(
        err.contains("after:"),
        "the live failure is still named: {err}"
    );
    assert!(!root.join("config/auth.json").exists());
}

#[test]
fn exec_timeout_kills_group_and_removes_auth_copy() {
    let f = Fixture::new();
    f.scenario("codex", json!({"exec": {"hang": true}}));
    let out = f.run_live(&["--exec-timeout-secs", "1"]);
    let err = assert_exit(&out, 124);
    assert!(err.contains("did not finish within 1s"), "{err}");
    assert!(
        !root_of(&out).join("config/auth.json").exists(),
        "the login copy is removed on timeout"
    );
    let pid = read_pid(&f.pidfile()).expect("pid");
    assert!(
        gone_within(pid, Duration::from_secs(3)),
        "the exec group survived its deadline"
    );
}

fn interrupted_during_exec(signal: Signal, code: i32) {
    let f = Fixture::new();
    f.scenario("codex", json!({"exec": {"hang": true}}));
    let auth = f.auth();
    let child = f
        .command(&["--copy-auth-from", auth.to_str().expect("utf-8")])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    let mut guard = ReapGuard::default();
    assert!(
        wait_for_file(&f.pidfile(), Duration::from_secs(60)),
        "exec never started"
    );
    let exec_pid = read_pid(&f.pidfile()).expect("pid");
    guard.track(exec_pid);
    let smoke_pid = Pid::from_raw(i32::try_from(child.id()).expect("pid")).expect("pid");
    kill_process(smoke_pid, signal).expect("signal");
    let out = child.wait_with_output().expect("wait");
    let err = assert_exit(&out, code);
    assert!(err.contains("interrupted by signal"), "{err}");
    let root = root_of(&out);
    assert!(
        !root.join("config/auth.json").exists(),
        "the login copy is removed on a signal"
    );
    assert!(
        gone_within(exec_pid, Duration::from_secs(3)),
        "the exec child was not reaped"
    );
    guard.clear();
}

#[test]
fn sigterm_during_exec_removes_auth_copy_and_reaps() {
    interrupted_during_exec(Signal::TERM, 143);
}

#[test]
fn sigint_during_exec_removes_auth_copy_and_reaps() {
    interrupted_during_exec(Signal::INT, 130);
}

fn mode(p: &Path) -> u32 {
    std::fs::metadata(p).expect("metadata").permissions().mode() & 0o777
}

#[test]
fn evidence_files_private_and_result_schema_preserved() {
    let f = Fixture::new();
    let out = f.run_live(&[]);
    assert_exit(&out, 0);
    let root = root_of(&out);
    assert_eq!(mode(&root), 0o700, "the evidence root is private");
    for file in [
        "catalog.json",
        "result.json",
        "events.jsonl",
        "schema.json",
        "answer.json",
    ] {
        assert_eq!(mode(&root.join(file)), 0o600, "{file}");
    }
    for seeded in [
        "home/.agents/skills/rdm-roadmap/SKILL.md",
        "home/plugins/rdm-coexistence/skills/rdm-roadmap/SKILL.md",
        "home/plugins/rdm-coexistence/.codex-plugin/plugin.json",
        "home/.agents/plugins/marketplace.json",
    ] {
        assert_eq!(mode(&root.join(seeded)), 0o600, "{seeded}");
    }
    let raw = std::fs::read_to_string(root.join("result.json")).expect("result");
    let keys: Vec<&str> = raw
        .lines()
        .filter_map(|l| l.trim().strip_prefix('"').and_then(|r| r.split('"').next()))
        .collect();
    assert_eq!(
        keys,
        [
            "codexVersion",
            "discoveredCopies",
            "liveInvocation",
            "selectedPath",
            "competingCopiesUnchanged",
            "authCopyRemoved"
        ],
        "result.json keeps the JS schema and key order"
    );
    assert_eq!(result_json(&root)["codexVersion"], "codex-cli 0.0.0-fake");
}

fn files_under(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).expect("read_dir").flatten() {
        let p = e.path();
        if p.is_dir() {
            files_under(&p, out);
        } else {
            out.push(p);
        }
    }
}

#[test]
fn auth_marker_never_appears_in_output_or_evidence() {
    let f = Fixture::new();
    let out = f.run_live(&[]);
    assert_exit(&out, 0);
    assert!(!text(&out.stdout).contains(MARKER) && !text(&out.stderr).contains(MARKER));
    let mut files = Vec::new();
    files_under(&root_of(&out), &mut files);
    assert!(files.len() >= 8);
    for p in files {
        let bytes = std::fs::read(&p).unwrap_or_default();
        assert!(
            !String::from_utf8_lossy(&bytes).contains(MARKER),
            "{} carries the login",
            p.display()
        );
    }
    assert!(
        !std::fs::read_to_string(f.record())
            .unwrap_or_default()
            .contains(MARKER)
    );
}

/// The real thing: a real Codex catalog and, with `RDM_CODEX_AUTH_FILE`, a live
/// invocation. Opt-in only; fails (never passes silently) without its
/// prerequisites.
#[test]
#[ignore = "live Codex: set RDM_CODEX_BIN and RDM_BIN, optionally RDM_CODEX_AUTH_FILE; run with --run-ignored only"]
fn codex_coexistence_live() {
    let (Some(codex), Some(rdm)) = (
        std::env::var_os("RDM_CODEX_BIN"),
        std::env::var_os("RDM_BIN"),
    ) else {
        panic!(
            "live Codex: set RDM_CODEX_BIN and RDM_BIN, optionally RDM_CODEX_AUTH_FILE; run with --run-ignored only"
        );
    };
    let mut c = Command::new(smoke());
    c.arg("codex-coexistence")
        .arg("--rdm")
        .arg(&rdm)
        .arg("--codex")
        .arg(&codex);
    let auth = std::env::var_os("RDM_CODEX_AUTH_FILE").filter(|a| !a.is_empty());
    if let Some(a) = &auth {
        c.arg("--copy-auth-from").arg(a);
    }
    let out = c.output().expect("rdm-smoke runs");
    println!("{}", text(&out.stdout));
    assert_exit(&out, 0);
    let stdout = text(&out.stdout);
    if auth.is_none() {
        assert!(stdout.contains("live invocation NOT RUN"), "{stdout}");
    } else {
        assert!(
            stdout.contains("live repository invocation passed"),
            "{stdout}"
        );
    }
}

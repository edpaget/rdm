//! Fixtures, fake binaries, readers and process guards for the `codex_process`
//! binary.
//!
//! - [`Root`]: a canonical per-test temp root with its own [`Sandbox`] (every
//!   `git`/`rdm` child a test spawns goes through it) and a `tmp/` that the
//!   Node host uses as `TMPDIR`, so the runtime's own temp dirs stay inside the
//!   test root.
//! - [`host`]: a [`Host`] whose Node child runs under the sandbox: the
//!   sandbox's removals (every inherited `RDM_*`, `REPO_REDIRECT_VARS`,
//!   `CODEX_HOME`, harness session variables) and variables (`HOME`, XDG,
//!   `/dev/null` git config, a fixed git identity), then the test's extras.
//!   Only the child's environment is ever set; the test's never is.
//! - The fake binaries ([`FAKE_CODEX`], [`FAKE_RDM`], [`FAKE_RUNNER_CODEX`])
//!   are POSIX `sh`. Each records its call — pid, argv (one per line), stdin,
//!   `pwd -P`, environment — under `calls/<pid>/` and replays a response Rust
//!   wrote. What a fake does is chosen by those Rust-written files or by the
//!   argument the code under test passes, never decided by the fake.
//! - [`Reaper`]: on drop, kills the process group of every pid a fake
//!   recorded (production spawns each fake `detached`, so pid = pgid and the
//!   group holds its descendants) and every recorded descendant, on every
//!   path including a failed assertion.
//! - Evidence (`journal.jsonl`, `manifest.json`, recorded calls, event lines)
//!   is parsed structurally; nothing here matches text against a pattern.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use rdm_devtools::workflow::{Host, HostConfig, JsError, WorkflowError};
use serde_json::Value;
use tempfile::TempDir;

use crate::plan_fixture::Sandbox;

/// The deadline every Node host in this binary runs under.
pub const HOST_TIMEOUT: Duration = Duration::from_secs(30);

/// The workspace root (read only).
pub fn repo() -> PathBuf {
    crate::workflow_support::repo_root()
}

/// A canonical per-test temp root.
pub struct Root {
    /// The canonical root path (symlinks resolved, so it compares equal to
    /// what the runtime's `realpath` checks produce).
    pub path: PathBuf,
    /// The isolated user context for every child.
    pub sandbox: Sandbox,
    dir: TempDir,
}

impl Root {
    /// A fresh root with `sandbox/` and `tmp/` under it.
    pub fn new() -> Self {
        let dir = TempDir::new().expect("temp root");
        let path = dir.path().canonicalize().expect("canonical temp root");
        let sandbox = Sandbox::new(&path.join("sandbox")).expect("sandbox");
        fs::create_dir_all(path.join("tmp")).expect("tmp dir");
        Self { path, sandbox, dir }
    }

    /// A path under the root.
    pub fn join(&self, rel: &str) -> PathBuf {
        self.path.join(rel)
    }

    /// Creates and returns a directory under the root.
    pub fn mkdir(&self, rel: &str) -> PathBuf {
        let p = self.join(rel);
        fs::create_dir_all(&p).expect("create fixture dir");
        p
    }

    /// `git` under the sandbox in `dir`, requiring success; trimmed stdout.
    pub fn git(&self, dir: &Path, args: &[&str]) -> String {
        self.sandbox
            .git(dir, args)
            .unwrap_or_else(|f| panic!("{f}"))
            .trim()
            .to_owned()
    }

    /// A git repo at `rel` on branch `main` with one commit of `seed`.
    pub fn seeded_repo(&self, rel: &str) -> PathBuf {
        let dir = self.mkdir(rel);
        self.git(&dir, &["init", "-q", "-b", "main"]);
        fs::write(dir.join("seed"), "seed").expect("seed file");
        self.git(&dir, &["add", "seed"]);
        self.git(&dir, &["commit", "-qm", "seed"]);
        dir
    }
}

/// A [`HostConfig`] for `root`: the sandbox, `TMPDIR` inside the root, the
/// binary's deadline, then `extra` (applied last, so it may re-set a key the
/// sandbox removes).
pub fn host_config(root: &Root, extra: &[(&str, OsString)]) -> HostConfig {
    let mut config = HostConfig::new().timeout(HOST_TIMEOUT);
    for key in Sandbox::removals() {
        config = config.env_remove(key);
    }
    for (key, value) in root.sandbox.vars() {
        config = config.env(key, value);
    }
    config = config.env("TMPDIR", root.join("tmp"));
    for (key, value) in extra {
        config = config.env(*key, value.clone());
    }
    config
}

/// Starts a [`Host`] for `root` (see [`host_config`]).
pub fn host(root: &Root, extra: &[(&str, OsString)]) -> Host {
    Host::start(&host_config(root, extra)).unwrap_or_else(|e| panic!("start the Node host: {e}"))
}

/// Writes `text` to `path` as an owner-only executable.
pub fn write_exec(path: &Path, text: &str) {
    fs::write(path, text).expect("write fake binary");
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).expect("chmod fake binary");
}

/// The JavaScript error in `r`, panicking on a value or an infrastructure
/// failure.
pub fn js_err(r: Result<Value, WorkflowError>, what: &str) -> JsError {
    match r {
        Ok(v) => panic!("{what}: expected a JavaScript rejection, got {v}"),
        Err(WorkflowError::Js(e)) => e,
        Err(e) => panic!("{what}: infrastructure failure: {e}"),
    }
}

/// The value in `r`, panicking with the JavaScript error or failure.
pub fn js_ok(r: Result<Value, WorkflowError>, what: &str) -> Value {
    r.unwrap_or_else(|e| panic!("{what}: {e}"))
}

/// Whether `pid` still exists (`kill -0`).
pub fn alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Sends `sig` (a `kill` signal name) to `target` (a pid, or `-pgid`).
pub fn signal(target: &str, sig: &str) {
    let _ = Command::new("kill")
        .args([&format!("-{sig}"), "--", target])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Polls `cond` every 10 ms until it holds or `within` passes; returns
/// whether it held.
pub fn until(within: Duration, mut cond: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + within;
    loop {
        if cond() {
            return true;
        }
        if Instant::now() >= end {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Polls until `pid` is gone, for at most `within`.
pub fn gone_within(pid: u32, within: Duration) -> bool {
    until(within, || !alive(pid))
}

/// Reads a pid file.
pub fn read_pid(path: &Path) -> Option<u32> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// One recorded fake invocation.
#[derive(Debug, Clone)]
pub struct Call {
    /// The fake's pid (also its process-group id).
    pub pid: u32,
    /// Its argv after the program name.
    pub argv: Vec<String>,
    /// Its stdin, when the fake records it.
    pub prompt: String,
    /// `pwd -P`, when recorded.
    pub cwd: String,
    /// Its environment, when recorded.
    pub env: BTreeMap<String, String>,
    /// The `--output-schema` it was given, when recorded.
    pub schema: Option<Value>,
    /// Its recorded descendant, if it started one.
    pub descendant: Option<u32>,
    /// The recording directory.
    pub dir: PathBuf,
}

impl Call {
    /// The argument following `flag`.
    pub fn after(&self, flag: &str) -> Option<&str> {
        let i = self.argv.iter().position(|a| a == flag)?;
        self.argv.get(i + 1).map(String::as_str)
    }

    /// Whether `arg` is one of its arguments.
    pub fn has(&self, arg: &str) -> bool {
        self.argv.iter().any(|a| a == arg)
    }

    /// Every `-c` configuration value it was given.
    pub fn configs(&self) -> Vec<&str> {
        self.argv
            .windows(2)
            .filter(|w| w[0] == "-c")
            .map(|w| w[1].as_str())
            .collect()
    }
}

fn parse_env(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|l| l.split_once('='))
        .filter(|(k, _)| !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect()
}

/// Every call recorded under `<fixture>/calls`, in pid order. A call whose
/// recording is still incomplete (no `argv` yet) is skipped.
pub fn calls(fixture: &Path) -> Vec<Call> {
    let Ok(entries) = fs::read_dir(fixture.join("calls")) else {
        return Vec::new();
    };
    let mut out: Vec<Call> = entries
        .filter_map(Result::ok)
        .filter_map(|e| {
            let dir = e.path();
            let pid = e.file_name().to_str()?.parse().ok()?;
            let argv = fs::read_to_string(dir.join("argv")).ok()?;
            let read = |name: &str| fs::read_to_string(dir.join(name)).unwrap_or_default();
            Some(Call {
                pid,
                argv: argv.lines().map(str::to_owned).collect(),
                prompt: read("prompt"),
                cwd: read("cwd").trim_end().to_owned(),
                env: parse_env(&read("env")),
                schema: serde_json::from_str(&read("schema.json")).ok(),
                descendant: read_pid(&dir.join("descendant")),
                dir,
            })
        })
        .collect();
    out.sort_by_key(|c| c.pid);
    out
}

/// Kills every recorded fake group and descendant on drop.
pub struct Reaper {
    fixtures: Vec<PathBuf>,
    pid_files: Vec<PathBuf>,
}

impl Reaper {
    /// Guards the calls recorded under each of `fixtures`.
    pub fn new(fixtures: &[&Path]) -> Self {
        Self {
            fixtures: fixtures.iter().map(|p| p.to_path_buf()).collect(),
            pid_files: Vec::new(),
        }
    }

    /// Also kills the process named by `path` (and its group) on drop.
    pub fn pid_file(mut self, path: PathBuf) -> Self {
        self.pid_files.push(path);
        self
    }
}

impl Drop for Reaper {
    fn drop(&mut self) {
        let mut targets = Vec::new();
        for fixture in &self.fixtures {
            let Ok(entries) = fs::read_dir(fixture.join("calls")) else {
                continue;
            };
            for dir in entries.filter_map(Result::ok).map(|e| e.path()) {
                if let Some(pid) = dir
                    .file_name()
                    .and_then(|n| n.to_str())
                    .and_then(|n| n.parse::<u32>().ok())
                {
                    targets.push(pid);
                }
                for name in ["pid", "descendant", "child"] {
                    targets.extend(read_pid(&dir.join(name)));
                }
            }
        }
        targets.extend(self.pid_files.iter().filter_map(|p| read_pid(p)));
        for pid in targets {
            signal(&format!("-{pid}"), "KILL");
            signal(&pid.to_string(), "KILL");
        }
    }
}

/// `<run>/journal.jsonl`, one parsed record per line.
pub fn journal(run: &Path) -> Vec<Value> {
    fs::read_to_string(run.join("journal.jsonl"))
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("journal line {l:?}: {e}")))
        .collect()
}

/// The journal records of `kind`.
pub fn records<'a>(journal: &'a [Value], kind: &str) -> Vec<&'a Value> {
    journal.iter().filter(|e| e["type"] == kind).collect()
}

/// `<run>/manifest.json`.
pub fn manifest(run: &Path) -> Value {
    let text = fs::read_to_string(run.join("manifest.json")).expect("read manifest.json");
    serde_json::from_str(&text).expect("parse manifest.json")
}

/// The transport fake `codex`: records the call, then replays the response
/// directory `$FAKE_RESPONSE` (files `stderr`, `behaviour`, `output`,
/// `stdout`, `exit`, each optional).
pub const FAKE_CODEX: &str = r#"#!/bin/sh
# Transport-only fake codex: record this call, replay the Rust-written response.
call="$FAKE_ROOT/calls/$$"
mkdir -p "$call"
echo $$ > "$call/pid"
pwd -P > "$call/cwd"
env > "$call/env"
: > "$call/argv"
prev=
for arg in "$@"; do
  printf '%s\n' "$arg" >> "$call/argv"
  case "$prev" in
    --output-schema) cp "$arg" "$call/schema.json" ;;
    -o) out="$arg" ;;
  esac
  prev="$arg"
done
cat > "$call/prompt"
R="$FAKE_RESPONSE"
if [ -f "$R/stderr" ]; then cat "$R/stderr" >&2; fi
case "$(cat "$R/behaviour" 2>/dev/null)" in
  hang) exec sleep 600 ;;
  kill-self) kill -KILL $$ ;;
  spawn-and-hang) sleep 600 & echo $! > "$call/descendant"; wait; exit 0 ;;
esac
if [ -f "$R/output" ]; then cp "$R/output" "$out"; fi
if [ -f "$R/stdout" ]; then cat "$R/stdout"; fi
if [ -f "$R/exit" ]; then exit "$(cat "$R/exit")"; fi
exit 0
"#;

/// The fake `rdm` for `createRun`: records the call under
/// `$CODEX_FIXTURE_DIR/calls`, performs the process shape its first argument
/// names (the argv the code under test passes), or replays
/// `$CODEX_FIXTURE_DIR/reply`.
pub const FAKE_RDM: &str = r#"#!/bin/sh
# Transport-only fake rdm: record this call; replay the Rust-written reply.
call="$CODEX_FIXTURE_DIR/calls/$$"
mkdir -p "$call"
echo $$ > "$call/pid"
: > "$call/args"
for arg in "$@"; do printf '%s\n' "$arg" >> "$call/args"; done
pwd -P > "$call/cwd"
env > "$call/env"
mv "$call/args" "$call/argv"
case "$1" in
  fail) echo written > "$RDM_ROOT/effect"; exit 1 ;;
  descendant|orphan)
    (sleep 1; : > "$RDM_ROOT/late-effect") &
    echo $! > "$call/child"
    if [ "$1" = descendant ]; then exec sleep 30; fi ;;
  overflow) head -c 9437184 /dev/zero | tr '\0' x; exit 0 ;;
  sleep) exec sleep 30 ;;
esac
cat "$CODEX_FIXTURE_DIR/reply"
"#;

/// The fake `codex` for whole-runtime runs (first on the runtime's `PATH`):
/// records the call and `start`/`end` event lines, then replays
/// `$CODEX_FIXTURE_DIR/responses/<key>` and `$CODEX_FIXTURE_DIR/stream`.
/// The key follows from what the runtime sent: an estimator prompt's
/// `Phase stem:` line, the refuter schema (`refuted`), the AC schema (`ac`),
/// or the finder prompt's dimension (falling back to `finder`). Optional
/// Rust-written files: `delay-<role>` (seconds before answering) and
/// `spawn-descendant`; `FIXTURE_HOLD=1` holds the call on its descendant.
pub const FAKE_RUNNER_CODEX: &str = r#"#!/bin/sh
# Transport-only fake codex: record this call, replay the Rust-written response.
F="$CODEX_FIXTURE_DIR"
call="$F/calls/$$"
mkdir -p "$call"
echo $$ > "$call/pid"
: > "$call/args"
prev=
for arg in "$@"; do
  printf '%s\n' "$arg" >> "$call/args"
  case "$prev" in
    --output-schema) schema="$arg" ;;
    -o) out="$arg" ;;
  esac
  prev="$arg"
done
cat > "$call/prompt"
cp "$schema" "$call/schema.json"
stem=$(sed -n 's/^Phase stem: \([^ ]*\).*$/\1/p' "$call/prompt" | head -n 1)
dimension=$(sed -n 's/^Your single dimension is [^(]*(\([^)]*\))\..*$/\1/p' "$call/prompt" | head -n 1)
if [ -n "$stem" ]; then role=estimator; key="estimate-$stem"
elif grep -q '"refuted"' "$call/schema.json"; then role=refuter; key=refuter
elif grep -q '"ac"' "$call/schema.json"; then role=finder; key=ac
else role=finder; key="finder-$dimension"; [ -f "$F/responses/$key" ] || key=finder
fi
if [ -f "$F/spawn-descendant" ]; then sleep 600 & echo $! > "$call/descendant"; fi
mv "$call/args" "$call/argv"
printf 'start %s %s\n' $$ "$role" >> "$F/events"
if [ "$FIXTURE_HOLD" = 1 ]; then wait; exit 0; fi
if [ -f "$F/delay-$role" ]; then sleep "$(cat "$F/delay-$role")"; fi
cp "$F/responses/$key" "$out"
cat "$F/stream"
printf 'end %s %s\n' $$ "$role" >> "$F/events"
"#;

/// One `start`/`end` line a runner fake appended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    /// `start` or `end`.
    pub kind: String,
    /// The fake's pid.
    pub pid: u32,
    /// `finder`, `refuter` or `estimator`.
    pub role: String,
}

/// Every event line under `<fixture>/events`, in append order.
pub fn events(fixture: &Path) -> Vec<Event> {
    fs::read_to_string(fixture.join("events"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let mut parts = l.split(' ');
            Some(Event {
                kind: parts.next()?.to_owned(),
                pid: parts.next()?.parse().ok()?,
                role: parts.next()?.to_owned(),
            })
        })
        .collect()
}

/// The highest number of calls live at once, from `start`/`end` order.
pub fn peak(events: &[Event]) -> usize {
    let (mut live, mut peak) = (0usize, 0usize);
    for e in events {
        if e.kind == "start" {
            live += 1;
            peak = peak.max(live);
        } else {
            live = live.saturating_sub(1);
        }
    }
    peak
}

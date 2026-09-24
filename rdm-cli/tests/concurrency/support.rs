//! Process, barrier and reader support for the `concurrency` binary.
//!
//! - [`Plan`]: a per-test plan repo at `<TempDir>/plan` with its own
//!   [`Sandbox`], driven by one binary (the real `rdm` or an isolated mutant).
//!   Every invocation pins an explicit session unless the test deliberately
//!   takes rung 2 from its own process ([`Plan::run_bare`]): a bare call leases
//!   the test's pid in that repo, and every later driver would ascend to it.
//! - [`Proc`]: a guarded background child. It stays in the test's process
//!   group (nextest kills that group on a timeout), its output goes to files,
//!   and dropping it kills and reaps it, so every assertion or panic path
//!   tears parked children down.
//! - [`Parked`]: a child parked at an `RDM_HARNESS_*_BARRIER` seam. Readiness
//!   is the `<marker>.parked` file the seam creates, never elapsed time.
//! - [`ShellDriver`]: a generated POSIX `sh` script, used only where a real
//!   shell is the process topology under test (the long-lived parent that
//!   anchors a rung-2 lease, the per-call wrapper). It carries no assertion,
//!   wait or branch; every argument is single-quoted by [`sh_quote`].
//! - Readers go through the product (`rdm session journal`, `task show`) or
//!   git. Raw reads remain only where the file is the evidence: lease files,
//!   a journal's line count, the hook log.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use serde_json::Value;
use tempfile::TempDir;

use crate::plan_fixture::{BARE_PATH, Sandbox, rdm_bin};
use crate::workflow_support::{Failure, infra};

/// How long a child may take to reach its barrier.
pub const PARK_DEADLINE: Duration = Duration::from_secs(30);
/// How long a child may take to exit once nothing holds it.
pub const EXIT_DEADLINE: Duration = Duration::from_secs(30);
/// The polling interval of every event wait.
const POLL: Duration = Duration::from_millis(10);

/// Unwraps a fixture result, panicking with the failure.
pub fn must<T>(r: Result<T, Failure>) -> T {
    r.unwrap_or_else(|f| panic!("{f}"))
}

/// The binary under test.
pub fn real_bin() -> PathBuf {
    PathBuf::from(rdm_bin())
}

/// Single-quotes `s` for a POSIX shell.
pub fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// How a process ended, with everything it printed.
#[derive(Debug)]
pub struct Finished {
    /// Its exit status.
    pub status: ExitStatus,
    /// Its stdout.
    pub stdout: String,
    /// Its stderr.
    pub stderr: String,
}

impl Finished {
    /// Whether it exited 0.
    pub fn success(&self) -> bool {
        self.status.success()
    }

    /// Stdout followed by stderr.
    pub fn all(&self) -> String {
        format!("{}{}", self.stdout, self.stderr)
    }

    /// A one-line summary for failure messages.
    pub fn describe(&self) -> String {
        format!(
            "exit {:?}\n--- stdout ---\n{}\n--- stderr ---\n{}",
            self.status.code(),
            self.stdout,
            self.stderr
        )
    }
}

/// A guarded background child whose output goes to files.
pub struct Proc {
    label: String,
    child: Child,
    stdin: Option<ChildStdin>,
    status: Option<ExitStatus>,
    out: PathBuf,
    err: PathBuf,
}

impl Proc {
    fn spawn(
        label: &str,
        mut cmd: Command,
        out: PathBuf,
        err: PathBuf,
        gated: bool,
    ) -> Result<Self, Failure> {
        cmd.stdout(Stdio::from(File::create(&out).map_err(infra)?))
            .stderr(Stdio::from(File::create(&err).map_err(infra)?))
            .stdin(if gated { Stdio::piped() } else { Stdio::null() });
        let mut child = cmd
            .spawn()
            .map_err(|e| Failure::Infra(format!("spawning {label}: {e}")))?;
        let stdin = child.stdin.take();
        Ok(Self {
            label: label.to_owned(),
            child,
            stdin,
            status: None,
            out,
            err,
        })
    }

    /// The child's pid.
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Writes one newline to a gated child's stdin, keeping it open.
    pub fn release_gate(&mut self) -> Result<(), Failure> {
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| Failure::Infra(format!("{} has no gate", self.label)))?;
        stdin.write_all(b"\n").map_err(infra)?;
        stdin.flush().map_err(infra)
    }

    /// The exit status, if the child has exited (reaping it).
    pub fn try_status(&mut self) -> Result<Option<ExitStatus>, Failure> {
        if self.status.is_none() {
            self.status = self.child.try_wait().map_err(infra)?;
        }
        Ok(self.status)
    }

    fn output(&self) -> (String, String) {
        (
            fs::read_to_string(&self.out).unwrap_or_default(),
            fs::read_to_string(&self.err).unwrap_or_default(),
        )
    }

    /// Waits up to `deadline` for the child to exit; on the deadline it is
    /// killed and reaped and the wait fails.
    pub fn wait_bounded(&mut self, deadline: Duration) -> Result<Finished, Failure> {
        self.stdin = None;
        let until = Instant::now() + deadline;
        loop {
            if let Some(status) = self.try_status()? {
                let (stdout, stderr) = self.output();
                return Ok(Finished {
                    status,
                    stdout,
                    stderr,
                });
            }
            if Instant::now() >= until {
                let _ = self.child.kill();
                self.status = self.child.wait().ok();
                let (stdout, stderr) = self.output();
                return Err(Failure::Infra(format!(
                    "{} did not exit within {deadline:?}; killed\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}",
                    self.label
                )));
            }
            std::thread::sleep(POLL);
        }
    }
}

impl Drop for Proc {
    fn drop(&mut self) {
        if self.status.is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

/// A child parked at a harness barrier.
pub struct Parked {
    /// The parked child.
    pub proc: Proc,
    release: PathBuf,
}

impl Parked {
    /// Creates the release file the barrier polls for.
    pub fn release(&self) -> Result<(), Failure> {
        fs::write(&self.release, b"").map_err(infra)
    }

    /// Releases the child and waits for it to exit.
    pub fn release_and_finish(mut self) -> Result<Finished, Failure> {
        self.release()?;
        self.proc.wait_bounded(EXIT_DEADLINE)
    }
}

/// Waits until `cond` holds, failing with `what` after `deadline`.
pub fn wait_until(
    what: &str,
    deadline: Duration,
    mut cond: impl FnMut() -> Result<bool, Failure>,
) -> Result<(), Failure> {
    let until = Instant::now() + deadline;
    loop {
        if cond()? {
            return Ok(());
        }
        if Instant::now() >= until {
            return Err(Failure::Infra(format!(
                "timed out after {deadline:?} waiting for {what}"
            )));
        }
        std::thread::sleep(POLL);
    }
}

/// `rdm session id --format json`, parsed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionOut {
    /// The resolved id.
    pub id: String,
    /// The rung that produced it.
    pub rung: u64,
    /// The measured resolution cost.
    pub resolve_micros: u64,
    /// Whether this invocation created a lease.
    pub lease_bootstrapped: bool,
}

impl SessionOut {
    /// Parses the JSON line of one `session id --format json` invocation.
    pub fn parse(stdout: &str) -> Result<Self, Failure> {
        let line = stdout
            .lines()
            .find(|l| l.starts_with('{'))
            .ok_or_else(|| Failure::Infra(format!("no JSON line in: {stdout}")))?;
        let v: Value = serde_json::from_str(line).map_err(infra)?;
        Ok(Self {
            id: v["id"]
                .as_str()
                .ok_or_else(|| Failure::Infra(format!("no id in {line}")))?
                .to_owned(),
            rung: v["rung"].as_u64().unwrap_or(0),
            resolve_micros: v["resolve_micros"].as_u64().unwrap_or(u64::MAX),
            lease_bootstrapped: v["lease_bootstrapped"].as_bool().unwrap_or(false),
        })
    }
}

/// A plan repo at `<TempDir>/plan` with its own sandbox, driven by one
/// binary.
pub struct Plan {
    /// The plan repo root.
    pub root: PathBuf,
    /// The user context every process runs under.
    pub sandbox: Sandbox,
    /// The binary under test.
    pub bin: PathBuf,
    seq: AtomicUsize,
    /// The owning temp directory (dropped last).
    pub dir: TempDir,
}

impl Plan {
    /// An empty temp tree; nothing is initialised.
    pub fn empty(bin: &Path) -> Self {
        let dir = TempDir::new().expect("tempdir");
        let root = dir.path().join("plan");
        fs::create_dir_all(&root).expect("create plan dir");
        let sandbox = must(Sandbox::new(&dir.path().join("sandbox")));
        Self {
            root,
            sandbox,
            bin: bin.to_owned(),
            seq: AtomicUsize::new(0),
            dir,
        }
    }

    /// `init --default-project <default_project>`, then each of `seed`, then
    /// one seed commit, all in session `seed` (rung 1, so no lease exists).
    pub fn seeded(bin: &Path, default_project: &str, seed: &[&[&str]]) -> Self {
        let plan = Self::empty(bin);
        plan.ok(
            Some("seed"),
            &["init", "--default-project", default_project],
        );
        for args in seed {
            plan.ok(Some("seed"), args);
        }
        plan.ok(Some("seed"), &["commit", "-m", "seed: init plan repo"]);
        plan
    }

    /// A seeded repo with the single project `demo`.
    pub fn demo(bin: &Path) -> Self {
        Self::seeded(bin, "demo", &[])
    }

    /// A fresh path under the temp dir, unique within this plan.
    pub fn scratch(&self, name: &str) -> PathBuf {
        let n = self.seq.fetch_add(1, Ordering::SeqCst);
        let dir = self.dir.path().join("scratch");
        fs::create_dir_all(&dir).expect("create scratch dir");
        dir.join(format!("{n}-{name}"))
    }

    /// A path inside the plan repo.
    pub fn path(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    /// `rdm --root <plan> <args>` under the sandbox, from the temp dir, with
    /// stdin closed; `session` pins `RDM_SESSION`, `None` leaves it unset.
    pub fn cmd(&self, session: Option<&str>, args: &[&str]) -> Command {
        let mut cmd = self.sandbox.command(&self.bin);
        cmd.arg("--root")
            .arg(&self.root)
            .args(args)
            .current_dir(self.dir.path())
            .stdin(Stdio::null());
        if let Some(s) = session {
            cmd.env("RDM_SESSION", s);
        }
        cmd
    }

    /// Runs `cmd` to completion.
    pub fn output(&self, mut cmd: Command) -> Finished {
        let out = cmd.output().expect("spawn rdm");
        Finished {
            status: out.status,
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        }
    }

    /// Runs `rdm <args>` in `session`, whatever the status.
    pub fn run(&self, session: Option<&str>, args: &[&str]) -> Finished {
        self.output(self.cmd(session, args))
    }

    /// Runs `rdm <args>` with no session variable from the test process
    /// itself — the one deliberate route to rung 2 at the test's own pid.
    pub fn run_bare(&self, args: &[&str]) -> Finished {
        self.run(None, args)
    }

    /// Runs `rdm <args>` in `session`, requiring success; returns stdout.
    pub fn ok(&self, session: Option<&str>, args: &[&str]) -> String {
        let out = self.run(session, args);
        assert!(
            out.success(),
            "`rdm {}` failed: {}",
            args.join(" "),
            out.describe()
        );
        out.stdout
    }

    /// Runs a read-only `rdm` command in the `reader` session and parses its
    /// stdout as JSON.
    pub fn json(&self, args: &[&str]) -> Value {
        let text = self.ok(Some("reader"), args);
        serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("`rdm {}` json: {e}: {text}", args.join(" ")))
    }

    /// Spawns `cmd` as a guarded background child.
    pub fn spawn(&self, label: &str, cmd: Command) -> Result<Proc, Failure> {
        let base = self.scratch(label);
        Proc::spawn(
            label,
            cmd,
            base.with_extension("out"),
            base.with_extension("err"),
            false,
        )
    }

    /// Spawns `cmd` with the barrier variable `var` naming a fresh release
    /// file, and waits for the `<release>.parked` file the seam creates. A
    /// child that exits first, or never parks, fails the wait.
    pub fn park(&self, var: &str, label: &str, mut cmd: Command) -> Result<Parked, Failure> {
        let release = self.scratch(&format!("{label}-go"));
        let mut ready = release.clone().into_os_string();
        ready.push(".parked");
        let ready = PathBuf::from(ready);
        cmd.env(var, &release);
        let mut proc = self.spawn(label, cmd)?;
        let until = Instant::now() + PARK_DEADLINE;
        loop {
            if ready.exists() {
                return Ok(Parked { proc, release });
            }
            if proc.try_status()?.is_some() {
                let (stdout, stderr) = proc.output();
                return Err(Failure::Infra(format!(
                    "{label} exited instead of parking at {var}, so the processes never \
                     interleaved\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}"
                )));
            }
            if Instant::now() >= until {
                return Err(Failure::Infra(format!(
                    "{label} did not park at {var} within {PARK_DEADLINE:?}"
                )));
            }
            std::thread::sleep(POLL);
        }
    }

    /// `git <args>` in the plan repo under the sandbox, requiring success;
    /// returns stdout.
    pub fn git(&self, args: &[&str]) -> String {
        must(self.sandbox.git(&self.root, args))
    }

    /// The plan repo's HEAD.
    pub fn head(&self) -> String {
        self.git(&["rev-parse", "HEAD"]).trim().to_owned()
    }

    /// `git status --porcelain --untracked-files=all`.
    pub fn porcelain(&self) -> String {
        self.git(&["status", "--porcelain", "--untracked-files=all"])
    }

    /// Every path in `rev`'s tree.
    pub fn tree(&self, rev: &str) -> BTreeSet<String> {
        self.git(&["ls-tree", "-r", "--name-only", rev])
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// The paths `rev` itself changed.
    pub fn commit_files(&self, rev: &str) -> BTreeSet<String> {
        self.git(&["show", "--name-only", "--pretty=format:", rev])
            .lines()
            .filter(|l| !l.is_empty())
            .map(str::to_owned)
            .collect()
    }

    /// The contents of `path` at `rev`, if the tree holds it.
    pub fn show(&self, rev: &str, path: &str) -> Option<String> {
        self.tree(rev)
            .contains(path)
            .then(|| self.git(&["show", &format!("{rev}:{path}")]))
    }

    /// What changeset `id` currently claims (the tombstone-aware fold), as
    /// path → kind, read through `rdm session journal`.
    pub fn journal(&self, id: &str) -> BTreeMap<String, String> {
        let v = self.json(&["session", "journal", "--id", id, "--format", "json"]);
        v["paths"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|e| {
                Some((
                    e["path"].as_str()?.to_owned(),
                    e["kind"].as_str()?.to_owned(),
                ))
            })
            .collect()
    }

    /// The raw line count of changeset `id`'s journal file (`None` when it
    /// does not exist) — evidence of what compaction did, which the fold
    /// deliberately hides.
    pub fn journal_lines(&self, id: &str) -> Option<usize> {
        fs::read_to_string(self.changesets_dir().join(format!("{id}.jsonl")))
            .ok()
            .map(|t| t.lines().count())
    }

    /// `<git-dir>/rdm/changesets`.
    pub fn changesets_dir(&self) -> PathBuf {
        self.root.join(".git/rdm/changesets")
    }

    /// `<git-dir>/rdm/leases`.
    pub fn leases_dir(&self) -> PathBuf {
        self.root.join(".git/rdm/leases")
    }

    /// Every lease file, as pid → the id it records.
    pub fn leases(&self) -> BTreeMap<u32, String> {
        let Ok(entries) = fs::read_dir(self.leases_dir()) else {
            return BTreeMap::new();
        };
        entries
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                let pid = name.strip_suffix(".lease")?.parse().ok()?;
                let text = fs::read_to_string(e.path()).ok()?;
                let v: Value = serde_json::from_str(&text).ok()?;
                Some((pid, v["id"].as_str().unwrap_or_default().to_owned()))
            })
            .collect()
    }

    /// `task show <slug> --project <project> --format json`.
    pub fn task(&self, project: &str, slug: &str) -> Value {
        self.json(&[
            "task",
            "show",
            slug,
            "--project",
            project,
            "--format",
            "json",
        ])
    }

    /// A task's tags.
    pub fn tags(&self, project: &str, slug: &str) -> Vec<String> {
        self.task(project, slug)["tags"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|t| t.as_str().map(str::to_owned))
            .collect()
    }

    /// `git commit --allow-empty` carrying `message` (for `Done:` lines).
    pub fn git_commit_empty(&self, message: &str) {
        self.git(&["commit", "--allow-empty", "--quiet", "-m", message]);
    }
}

/// One step of a [`ShellDriver`].
enum Step {
    /// The binary as the driver's own child.
    Direct(Vec<String>),
    /// The binary inside its own wrapper shell.
    Wrapped(Vec<String>),
    /// `read -r _`: blocks until the test releases the gate.
    Gate,
}

/// The fixed wrapper body: all data arrives as positional parameters, so
/// nothing is `eval`ed. The trailing `:` keeps the shell from exec-ing the
/// last command into itself, so the wrapper is a live process for the whole
/// call and the binary is its child.
const WRAPPER: &str = r#"p=$1 o=$2 e=$3 s=$4; shift 4; echo "$$ $PPID" >"$p"; "$@" >"$o" 2>"$e" </dev/null; echo $? >"$s"; :"#;

/// A generated POSIX `sh` driver: a long-lived parent running `rdm` steps.
#[derive(Default)]
pub struct ShellDriver {
    steps: Vec<Step>,
    env: Vec<(String, String)>,
}

impl ShellDriver {
    /// An empty driver.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets `key=value` on the driver (so every step inherits it).
    pub fn env(mut self, key: &str, value: &str) -> Self {
        self.env.push((key.to_owned(), value.to_owned()));
        self
    }

    /// Runs `rdm <args>` as the driver's child.
    pub fn direct(mut self, args: &[&str]) -> Self {
        self.steps
            .push(Step::Direct(args.iter().map(|a| (*a).to_owned()).collect()));
        self
    }

    /// Runs `rdm <args>` inside its own wrapper shell.
    pub fn wrapped(mut self, args: &[&str]) -> Self {
        self.steps.push(Step::Wrapped(
            args.iter().map(|a| (*a).to_owned()).collect(),
        ));
        self
    }

    /// Blocks the driver until [`Driver::release_gate`].
    pub fn gate(mut self) -> Self {
        self.steps.push(Step::Gate);
        self
    }

    /// Writes the script and starts it under the plan's sandbox with
    /// `PATH=/usr/bin:/bin` and the binary named by absolute path.
    pub fn spawn(self, plan: &Plan, label: &str) -> Result<Driver, Failure> {
        let dir = plan.scratch(label);
        fs::create_dir_all(&dir).map_err(infra)?;
        let q = |p: &Path| sh_quote(&p.to_string_lossy());
        let mut rdm = vec![q(&plan.bin), sh_quote("--root"), q(&plan.root)];
        let mut script = String::new();
        let mut wrapped = Vec::new();
        let mut n = 0;
        let gated = self.steps.iter().any(|s| matches!(s, Step::Gate));
        for step in &self.steps {
            let args = match step {
                Step::Gate => {
                    script.push_str("read -r _\n");
                    continue;
                }
                Step::Direct(a) | Step::Wrapped(a) => a,
            };
            let base = dir.join(n.to_string());
            let file = |ext: &str| q(&base.with_extension(ext));
            let words: Vec<String> = args.iter().map(|a| sh_quote(a)).collect();
            rdm.truncate(3);
            rdm.extend(words);
            let argv = rdm.join(" ");
            match step {
                Step::Direct(_) => {
                    script.push_str(&format!(
                        "{argv} >{} 2>{} </dev/null; echo $? >{}\n",
                        file("out"),
                        file("err"),
                        file("status")
                    ));
                    wrapped.push(false);
                }
                _ => {
                    script.push_str(&format!(
                        "sh -c {} wrapper {} {} {} {} {argv} </dev/null\n",
                        sh_quote(WRAPPER),
                        file("pid"),
                        file("out"),
                        file("err"),
                        file("status")
                    ));
                    wrapped.push(true);
                }
            }
            n += 1;
        }
        script.push_str(":\n");
        let path = dir.join("driver.sh");
        fs::write(&path, &script).map_err(infra)?;
        let mut cmd = plan.sandbox.command("/bin/sh");
        cmd.arg(&path)
            .env("PATH", BARE_PATH)
            .current_dir(plan.dir.path());
        for (k, v) in &self.env {
            cmd.env(k, v);
        }
        let proc = Proc::spawn(
            label,
            cmd,
            dir.join("driver.out"),
            dir.join("driver.err"),
            gated,
        )?;
        Ok(Driver { proc, dir, wrapped })
    }

    /// Spawns the driver and waits for it to finish.
    pub fn run(self, plan: &Plan, label: &str) -> Result<DriverRun, Failure> {
        self.spawn(plan, label)?.finish()
    }
}

/// A running [`ShellDriver`].
pub struct Driver {
    /// The driver process.
    pub proc: Proc,
    dir: PathBuf,
    wrapped: Vec<bool>,
}

/// One finished step of a driver.
#[derive(Debug)]
pub struct StepResult {
    /// The step's exit status.
    pub status: i32,
    /// Its stdout.
    pub stdout: String,
    /// Its stderr.
    pub stderr: String,
    /// For a wrapped step, the wrapper's pid and its parent's pid.
    pub wrapper: Option<(u32, u32)>,
}

impl StepResult {
    /// Stdout followed by stderr.
    pub fn all(&self) -> String {
        format!("{}{}", self.stdout, self.stderr)
    }

    /// The step's `session id --format json` output.
    pub fn session(&self) -> Result<SessionOut, Failure> {
        SessionOut::parse(&self.stdout)
    }
}

/// A finished driver and its steps.
#[derive(Debug)]
pub struct DriverRun {
    /// The driver's pid.
    pub pid: u32,
    /// Every command step, in order.
    pub steps: Vec<StepResult>,
}

impl DriverRun {
    /// Every step's `session id` output, requiring each step to be one.
    pub fn sessions(&self) -> Result<Vec<SessionOut>, Failure> {
        self.steps.iter().map(StepResult::session).collect()
    }

    /// Every step's combined output.
    pub fn all(&self) -> String {
        self.steps.iter().map(StepResult::all).collect()
    }
}

impl Driver {
    /// The driver's pid (the parent every direct step runs under).
    pub fn pid(&self) -> u32 {
        self.proc.pid()
    }

    /// Releases the next `read -r _` gate.
    pub fn release_gate(&mut self) -> Result<(), Failure> {
        self.proc.release_gate()
    }

    fn step_status(&self, i: usize) -> Option<i32> {
        fs::read_to_string(self.dir.join(i.to_string()).with_extension("status"))
            .ok()?
            .trim()
            .parse()
            .ok()
    }

    /// Waits for command step `i` (0-based) to have finished.
    pub fn wait_step(&mut self, i: usize) -> Result<(), Failure> {
        let label = self.proc.label.clone();
        wait_until(&format!("{label} step {i}"), EXIT_DEADLINE, || {
            if self.step_status(i).is_some() {
                return Ok(true);
            }
            if self.proc.try_status()?.is_some() {
                return Err(Failure::Infra(format!(
                    "{label} exited before step {i} finished"
                )));
            }
            Ok(false)
        })
    }

    /// Waits for the driver to exit and collects every step.
    pub fn finish(mut self) -> Result<DriverRun, Failure> {
        let driver = self.proc.wait_bounded(EXIT_DEADLINE)?;
        if !driver.success() {
            return Err(Failure::Infra(format!(
                "driver {} failed: {}",
                self.proc.label,
                driver.describe()
            )));
        }
        let mut steps = Vec::new();
        for (i, wrapped) in self.wrapped.iter().enumerate() {
            let base = self.dir.join(i.to_string());
            let read = |ext: &str| fs::read_to_string(base.with_extension(ext)).unwrap_or_default();
            let status = self.step_status(i).ok_or_else(|| {
                Failure::Infra(format!(
                    "step {i} of {} left no status: {}",
                    self.proc.label,
                    driver.describe()
                ))
            })?;
            let wrapper = if *wrapped {
                let text = read("pid");
                let mut it = text.split_whitespace().filter_map(|w| w.parse().ok());
                match (it.next(), it.next()) {
                    (Some(pid), Some(ppid)) => Some((pid, ppid)),
                    _ => {
                        return Err(Failure::Infra(format!(
                            "wrapped step {i} recorded no pids: {text:?}"
                        )));
                    }
                }
            } else {
                None
            };
            steps.push(StepResult {
                status,
                stdout: read("out"),
                stderr: read("err"),
                wrapper,
            });
        }
        let pid = self.proc.pid();
        Ok(DriverRun { pid, steps })
    }
}

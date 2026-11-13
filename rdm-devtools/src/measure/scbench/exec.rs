//! Executes a [`RunPlan`]: spawns its commands in order and keeps the run
//! record current after every one.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::containers;
use super::environment;
use super::lane::{self, LaneRecord};
use super::manifest::{ArmGroup, ArmImage, Manifest};
use super::plan::{self, CommandSpec, Paths, RunPlan, Tools};
use super::redact;
use crate::process::{self, Interrupts};

/// The run record's schema version: 2 records one [`ArmRecord`] per arm and
/// seed. A record without `schema` (phases 1-4) has one per arm, at seed 1.
pub const RECORD_SCHEMA: u32 = 2;

/// The run record written to `<out>/rdm-scbench-run.json`.
#[derive(Debug, Serialize)]
pub struct RunRecord {
    /// [`RECORD_SCHEMA`].
    pub schema: u32,
    /// Every pin, as parsed, with the selected arms and, under `--problems`,
    /// the selected problems (`problems.subset` and `problems.checkpoints`
    /// narrowed to them).
    pub manifest: Manifest,
    /// The manifest file the run loaded.
    pub manifest_path: PathBuf,
    /// The rdm checkout under test.
    pub rdm: RdmInfo,
    /// Tool versions the probes observed.
    pub observed: Observed,
    /// The installed model file, once copied.
    pub model_file: Option<ModelFileRecord>,
    /// The seeds each selected arm runs with (`1..=n`).
    pub seeds: Vec<u64>,
    /// The arms `--arms` selected, in manifest order.
    pub selected_arms: Vec<String>,
    /// The problems `--problems` selected, in manifest order; `null` without
    /// `--problems`.
    pub problems_filter: Option<Vec<String>>,
    /// Each unit's (arm and seed's) generated configs and SCBench run
    /// directory, in run order.
    pub arms: Vec<ArmRecord>,
    /// Every command spawned, in order.
    pub commands: Vec<CommandRecord>,
}

/// The rdm checkout's identity.
#[derive(Debug, Default, Serialize)]
pub struct RdmInfo {
    /// `HEAD` of the checkout holding the manifest.
    pub sha: Option<String>,
}

/// The trimmed stdout of each version probe.
#[derive(Debug, Default, Serialize)]
pub struct Observed {
    /// `uv --version`.
    pub uv: Option<String>,
    /// The Docker server version.
    pub docker: Option<String>,
    /// The host `claude --version`.
    pub claude: Option<String>,
}

/// The copied model file.
#[derive(Debug, Serialize)]
pub struct ModelFileRecord {
    /// The manifest's model file.
    pub source: PathBuf,
    /// Where it was installed in the SCBench checkout.
    pub installed: PathBuf,
    /// SHA-256 of its bytes.
    pub sha256: String,
}

/// One unit's configuration: an arm at one seed.
#[derive(Debug, Serialize)]
pub struct ArmRecord {
    /// The arm name.
    pub name: String,
    /// The unit's SCBench seed.
    pub seed: u64,
    /// The agent image the arm runs in.
    pub image: ArmImage,
    /// The arm's treatment group, which the ablation comparison keys on.
    pub group: ArmGroup,
    /// SCBench's graded directory (the environment's `docker.workdir`), read
    /// for an rdm arm; `null` for a stock arm.
    pub graded_dir: Option<String>,
    /// The agent config written for the arm.
    pub agent_config: Value,
    /// The run config written for the arm.
    pub run_config: Value,
    /// SCBench's output directory for the unit.
    pub run_dir: PathBuf,
    /// For an rdm arm, once its run finished: each checkpoint's copied plan
    /// repo and lane classification (`lane::collect`); `null` for a stock
    /// arm.
    pub lane: Option<LaneRecord>,
    /// RFC 3339 time at which the unit's eval exited 0; `null` until then, so
    /// a report skips a unit that never finished.
    pub finished: Option<String>,
    /// Set when a signal stopped the run during this unit; left out of the
    /// record otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interrupted: Option<Interruption>,
}

/// How a signal stopped a unit.
#[derive(Debug, Serialize)]
pub struct Interruption {
    /// `SIGINT`, `SIGTERM` or `SIGHUP`.
    pub signal: String,
    /// The step it arrived in: `run`, `lane` or `eval`.
    pub step: String,
    /// RFC 3339 time the interruption was recorded.
    pub at: String,
    /// Whether `slop-code` outlived the grace period and was killed.
    pub killed: bool,
    /// The containers removed because they mounted the step's scratch
    /// directory.
    pub removed_containers: Vec<String>,
}

/// One spawned command.
#[derive(Debug, Serialize)]
pub struct CommandRecord {
    /// Program and arguments.
    pub argv: Vec<String>,
    /// Working directory, when set.
    pub cwd: Option<PathBuf>,
    /// The extra environment it was given.
    pub env: BTreeMap<String, String>,
    /// Its exit code (`null` when it failed to spawn or died by signal).
    pub exit: Option<i32>,
    /// RFC 3339 start time.
    pub started: String,
    /// RFC 3339 finish time.
    pub finished: String,
    /// The signal forwarded to it, when the run was interrupted while it ran.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interrupted: Option<String>,
    /// Whether it outlived the grace period and was killed.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub killed: bool,
    /// For a `slop-code` run or eval, the scratch directory given to it as
    /// `TMPDIR`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tmpdir: Option<PathBuf>,
}

/// How a spawned command's stdio is wired.
#[derive(Clone, Copy)]
pub(super) enum Io {
    /// Null stdin, captured stdout, inherited stderr.
    Capture,
    /// Null stdin, inherited stdout/stderr.
    Quiet,
    /// Fully inherited stdio (the long SCBench runs the operator watches).
    Inherit,
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// `SIGINT`, `SIGTERM`, `SIGHUP`, or `signal <n>` for any other.
pub(super) fn signal_name(sig: i32) -> String {
    use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};
    match sig {
        SIGINT => "SIGINT".to_owned(),
        SIGTERM => "SIGTERM".to_owned(),
        SIGHUP => "SIGHUP".to_owned(),
        n => format!("signal {n}"),
    }
}

struct Executor<'a> {
    record: RunRecord,
    path: PathBuf,
    interrupts: &'a Interrupts,
    stop_grace: Duration,
    docker: String,
}

/// The outcome of one spawn.
pub(super) struct Ran {
    /// The exit code (`None` when it failed to spawn or died by signal).
    pub(super) exit: Option<i32>,
    /// Captured stdout (empty unless [`Io::Capture`]).
    pub(super) stdout: String,
    spawn_error: Option<String>,
    /// The signal forwarded to a `slop-code` spawn, when one arrived.
    pub(super) interrupted: Option<i32>,
    /// Whether that spawn outlived the grace period and was killed.
    pub(super) killed: bool,
    grace: Duration,
}

impl Ran {
    /// Whether it exited 0.
    pub(super) fn ok(&self) -> bool {
        self.exit == Some(0)
    }

    /// Whether the program could not be started at all.
    pub(super) fn spawn_failed(&self) -> bool {
        self.spawn_error.is_some()
    }

    /// `exited <n>`, `was killed by a signal`, `failed to spawn: <e>`, or
    /// for an interrupted spawn `was stopped by SIGTERM` / `was killed after
    /// the <n>s grace`.
    pub(super) fn describe(&self) -> String {
        match (&self.spawn_error, self.exit) {
            (Some(e), _) => format!("failed to spawn: {e}"),
            (None, _) if self.killed => {
                format!("was killed after the {}s grace", self.grace.as_secs())
            }
            (None, _) if self.interrupted.is_some() => format!(
                "was stopped by {}",
                signal_name(self.interrupted.unwrap_or_default())
            ),
            (None, Some(code)) => format!("exited {code}"),
            (None, None) => "was killed by a signal".to_owned(),
        }
    }

    fn spawned(exit: Option<i32>, stdout: String) -> Self {
        Self {
            exit,
            stdout,
            spawn_error: None,
            interrupted: None,
            killed: false,
            grace: Duration::ZERO,
        }
    }

    fn spawn_failed_with(e: &std::io::Error) -> Self {
        Self {
            exit: None,
            stdout: String::new(),
            spawn_error: Some(e.to_string()),
            interrupted: None,
            killed: false,
            grace: Duration::ZERO,
        }
    }
}

impl Executor<'_> {
    fn save(&self) -> Result<(), String> {
        let body = serde_json::to_string_pretty(&self.record).map_err(|e| e.to_string())?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, format!("{body}\n"))
            .map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &self.path)
            .map_err(|e| format!("cannot write {}: {e}", self.path.display()))
    }

    /// Spawns `spec`, unless a signal has arrived: then it spawns nothing and
    /// fails `interrupted by <SIG> before `<argv>``. That stops probes,
    /// provisioning and the lane's git and tar steps once the run is being
    /// stopped.
    fn spawn(&mut self, spec: &CommandSpec, io: Io) -> Result<Ran, String> {
        if let Some(sig) = self.interrupts.received() {
            return Err(format!(
                "interrupted by {} before `{}`",
                signal_name(sig),
                spec.argv().join(" ")
            ));
        }
        self.spawn_teardown(spec, io)
    }

    /// [`Executor::spawn`] without the signal check, for the docker calls
    /// that tear an interrupted step down.
    fn spawn_teardown(&mut self, spec: &CommandSpec, io: Io) -> Result<Ran, String> {
        let (ran, record) = spawn_command(spec, io);
        self.record.commands.push(record);
        self.save()?;
        Ok(ran)
    }

    /// Spawns a `slop-code` run or eval in its own process group with stdin
    /// at `/dev/null`, inherited stdout/stderr and `TMPDIR` at `scratch`,
    /// forwarding a signal to it ([`process::run_forwarding`]), and records
    /// it.
    fn spawn_slop_code(&mut self, spec: &CommandSpec, scratch: &Path) -> Result<Ran, String> {
        let mut cmd = command(spec);
        cmd.stdin(Stdio::null()).env("TMPDIR", scratch);
        let started = now();
        let ran = match process::run_forwarding(&mut cmd, self.interrupts, self.stop_grace) {
            Ok(f) => Ran {
                interrupted: f.interrupted,
                killed: f.killed,
                grace: self.stop_grace,
                ..Ran::spawned(f.status.code(), String::new())
            },
            Err(e) => Ran {
                interrupted: self.interrupts.received(),
                ..Ran::spawn_failed_with(&e)
            },
        };
        self.record.commands.push(CommandRecord {
            argv: spec.argv(),
            cwd: spec.cwd.clone(),
            env: spec.env.clone(),
            exit: ran.exit,
            started,
            finished: now(),
            interrupted: ran.interrupted.map(signal_name),
            killed: ran.killed,
            tmpdir: Some(scratch.to_path_buf()),
        });
        self.save()?;
        Ok(ran)
    }

    /// Runs one `slop-code` step (`run` or `eval`) of `stop`'s unit: refuses
    /// to start once a signal has arrived; spawns it with a fresh scratch
    /// `TMPDIR`; when it was interrupted, removes the containers mounting the
    /// scratch; scrubs the unit's `seed-<n>` directory whatever happened;
    /// removes the scratch; then records and returns an interruption as the
    /// error.
    fn slop_code_step(
        &mut self,
        stop: &Stop<'_>,
        step: &str,
        spec: &CommandSpec,
        secret: Option<&redact::Secret>,
        stderr: &mut dyn Write,
    ) -> Result<Ran, String> {
        if let Some(sig) = self.interrupts.received() {
            return Err(self.interrupted(stop, step, sig, None));
        }
        let base = std::env::temp_dir();
        let scratch = tempfile::Builder::new()
            .prefix("scb")
            .rand_bytes(6)
            .tempdir_in(&base)
            .map_err(|e| {
                format!(
                    "arm {}: cannot create a scratch directory under {}: {e}",
                    stop.unit,
                    base.display()
                )
            })?;
        let ran = self.spawn_slop_code(spec, scratch.path());
        let interrupted = match &ran {
            Ok(r) => r.interrupted,
            Err(_) => self.interrupts.received(),
        };
        let removed = if interrupted.is_some() {
            self.teardown_containers(scratch.path(), stderr)
        } else {
            Vec::new()
        };
        redact::scrub_tree(stop.seed_dir, secret, "scbench run", stderr);
        let path = scratch.path().to_path_buf();
        if let Err(e) = scratch.close() {
            redact::scrub_tree(&path, secret, "scbench run", stderr);
            let _ = writeln!(
                stderr,
                "scbench run: could not remove scratch dir {} ({}); remove it by hand",
                path.display(),
                e.kind()
            );
        }
        let ran = ran?;
        if let Some(sig) = interrupted {
            let killed = ran.killed;
            return Err(self.interrupted(stop, step, sig, Some((killed, removed))));
        }
        Ok(ran)
    }

    /// Records that `sig` stopped `stop`'s unit during `step` and returns the
    /// error naming the unit. `slop` is `None` when no `slop-code` command
    /// was running, else whether it was killed and the containers removed.
    fn interrupted(
        &mut self,
        stop: &Stop<'_>,
        step: &str,
        sig: i32,
        slop: Option<(bool, Vec<String>)>,
    ) -> String {
        let signal = signal_name(sig);
        let what = match &slop {
            None => "no slop-code command was running".to_owned(),
            Some((killed, removed)) => {
                let how = if *killed {
                    format!(
                        "did not exit within the {}s grace and was killed",
                        self.stop_grace.as_secs()
                    )
                } else {
                    "exited on the signal".to_owned()
                };
                format!(
                    "slop-code {how}; removed {} container(s) this run started",
                    removed.len()
                )
            }
        };
        let (killed, removed_containers) = slop.unwrap_or_default();
        self.record.arms[stop.index].interrupted = Some(Interruption {
            signal: signal.clone(),
            step: step.to_owned(),
            at: now(),
            killed,
            removed_containers,
        });
        let mut message = format!(
            "arm {}: interrupted by {signal} during {step} (problems {}); {what}; the partial \
             output stays under {}; rdm-measure scbench report --run {} lists the unit as not \
             finished",
            stop.unit,
            stop.problems.join(", "),
            stop.seed_dir.display(),
            stop.out.display()
        );
        if let Err(e) = self.save() {
            message.push_str(&format!("; the run record could not be updated: {e}"));
        }
        message
    }

    /// Removes the containers that mount `scratch` (see [`containers`]) and
    /// returns the ids actually removed. A docker failure is reported on
    /// `stderr` and the teardown carries on as far as it can; it never fails
    /// the caller. A container that vanished between the listing and the
    /// inspection, or a malformed inspect line, does not stop the others from
    /// being attributed, and each attributed container is removed on its own.
    fn teardown_containers(&mut self, scratch: &Path, stderr: &mut dyn Write) -> Vec<String> {
        let docker = self.docker.clone();
        let listed = match self.spawn_teardown(&containers::list(&docker), Io::Capture) {
            Ok(r) if r.ok() => r
                .stdout
                .split_whitespace()
                .map(str::to_owned)
                .collect::<Vec<_>>(),
            Ok(r) => {
                teardown_note(stderr, "list", &r.describe(), scratch);
                return Vec::new();
            }
            Err(e) => {
                teardown_note(stderr, "list", &e, scratch);
                return Vec::new();
            }
        };
        let mut ours = Vec::new();
        if !listed.is_empty() {
            let r = match self.spawn_teardown(&containers::inspect(&docker, &listed), Io::Capture) {
                Ok(r) => r,
                Err(e) => {
                    teardown_note(stderr, "inspect", &e, scratch);
                    return Vec::new();
                }
            };
            if !r.ok() {
                if r.stdout.trim().is_empty() {
                    teardown_note(stderr, "inspect", &r.describe(), scratch);
                    return Vec::new();
                }
                // Exits non-zero when any listed container is gone by now,
                // but still prints the others.
                teardown_note(
                    stderr,
                    "inspect all the listed",
                    &format!("{}; the ones it printed are still attributed", r.describe()),
                    scratch,
                );
            }
            let found = containers::attributed(&r.stdout, scratch);
            if !found.malformed.is_empty() {
                let lines: Vec<String> = found.malformed.iter().map(usize::to_string).collect();
                teardown_note(
                    stderr,
                    "inspect",
                    &format!(
                        "line(s) {} of the output were not `<id> <mounts JSON>` and were skipped",
                        lines.join(", ")
                    ),
                    scratch,
                );
            }
            ours = found.ours;
        }
        if ours.is_empty() {
            let _ = writeln!(
                stderr,
                "scbench run: no container mounted the scratch directory {}; none removed",
                scratch.display()
            );
            return Vec::new();
        }
        let mut removed = Vec::new();
        let mut failures = Vec::new();
        for id in &ours {
            match self.spawn_teardown(&containers::remove(&docker, id), Io::Capture) {
                Ok(r) if r.ok() => removed.push(id.clone()),
                Ok(r) => failures.push(format!("{id} {}", r.describe())),
                Err(e) => failures.push(format!("{id}: {e}")),
            }
        }
        if !failures.is_empty() {
            teardown_note(stderr, "remove", &failures.join("; "), scratch);
        }
        if !removed.is_empty() {
            let _ = writeln!(
                stderr,
                "scbench run: removed {} container(s) mounting {}",
                removed.len(),
                scratch.display()
            );
        }
        removed
    }

    fn probe(&mut self, tool: &str, spec: &CommandSpec) -> Result<String, String> {
        let ran = self.spawn(spec, Io::Capture)?;
        if !ran.ok() {
            return Err(format!(
                "version probe for {tool} (`{}`) {}",
                spec.argv().join(" "),
                ran.describe()
            ));
        }
        Ok(ran.stdout.trim().to_owned())
    }
}

/// Reports a docker teardown step that could not `what` the containers.
fn teardown_note(stderr: &mut dyn Write, what: &str, how: &str, scratch: &Path) {
    let _ = writeln!(
        stderr,
        "scbench run: could not {what} containers ({how}); check `docker ps` for SlopCodeBench \
         containers mounting {}",
        scratch.display()
    );
}

/// The unit a step belongs to, for an interruption's record and message.
struct Stop<'a> {
    /// The unit's index in [`RunRecord::arms`].
    index: usize,
    /// `<arm> seed <n>`.
    unit: &'a str,
    /// The unit's `seed-<n>` directory.
    seed_dir: &'a Path,
    /// The manifest's problem subset (one `slop-code run` drives all of it).
    problems: &'a [String],
    /// The run's `--out`.
    out: &'a Path,
}

/// The [`Command`] for `spec`: program, arguments, extra environment and
/// working directory; stdio untouched.
fn command(spec: &CommandSpec) -> Command {
    let mut cmd = Command::new(&spec.program);
    cmd.args(&spec.args).envs(&spec.env);
    if let Some(cwd) = &spec.cwd {
        cmd.current_dir(cwd);
    }
    cmd
}

/// Spawns `spec` with `io` and waits for it, returning the outcome and its
/// record.
pub(super) fn spawn_command(spec: &CommandSpec, io: Io) -> (Ran, CommandRecord) {
    let mut cmd = command(spec);
    match io {
        Io::Capture => {
            cmd.stdin(Stdio::null()).stdout(Stdio::piped());
        }
        Io::Quiet => {
            cmd.stdin(Stdio::null());
        }
        Io::Inherit => {}
    }
    let started = now();
    let result = match io {
        Io::Capture => cmd.output().map(|o| {
            (
                o.status.code(),
                String::from_utf8_lossy(&o.stdout).into_owned(),
            )
        }),
        Io::Quiet | Io::Inherit => cmd.status().map(|s| (s.code(), String::new())),
    };
    let ran = match result {
        Ok((exit, stdout)) => Ran::spawned(exit, stdout),
        Err(e) => Ran::spawn_failed_with(&e),
    };
    let record = CommandRecord {
        argv: spec.argv(),
        cwd: spec.cwd.clone(),
        env: spec.env.clone(),
        exit: ran.exit,
        started,
        finished: now(),
        interrupted: None,
        killed: false,
        tmpdir: None,
    };
    (ran, record)
}

/// Provisions checkout `c` (named `name` in errors) at its pin: clone or
/// re-point, fetch, then detach at the SHA. `spawn` runs each step.
///
/// # Errors
///
/// When a step fails, naming it; or whatever `spawn` returns.
pub(super) fn provision(
    name: &str,
    c: &plan::Checkout,
    spawn: &mut dyn FnMut(&CommandSpec) -> Result<Ran, String>,
) -> Result<(), String> {
    let steps: Vec<&CommandSpec> = if c.dir.exists() {
        vec![&c.set_url, &c.fetch, &c.checkout]
    } else {
        vec![&c.clone, &c.fetch, &c.checkout]
    };
    for spec in steps {
        let ran = spawn(spec)?;
        if !ran.ok() {
            return Err(format!(
                "provisioning {name} (`{}`) {}",
                spec.argv().join(" "),
                ran.describe()
            ));
        }
    }
    Ok(())
}

/// Creates `path`'s parent directory, then writes `bytes` to `path`.
///
/// # Errors
///
/// When either fails, naming the path.
pub(super) fn write_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    std::fs::write(path, bytes).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// Writes `value` to `path` as pretty JSON (which YAML readers accept).
///
/// # Errors
///
/// When the file cannot be written, naming it.
pub(super) fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    let body = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    write_file(path, format!("{body}\n").as_bytes())
}

/// Runs `manifest` (loaded from `manifest_path`, holding the selected arms)
/// against `paths` over `seeds` to completion, stopping at the first failing
/// step, and returns the plan it ran. The plan is built once the rdm SHA is
/// resolved, so rdm arms carry it;
/// `template` is the unrendered [`super::rdm_image::TEMPLATE`] and `prompts`
/// the unrendered manifest prompts (see [`plan::build_plan`]). Between an rdm
/// arm's run and its eval, its plan repos are copied out and classified
/// (`lane::collect`) and a `<landed>/<total>` line goes to `stderr`. An arm's
/// agent config, agent template and prompt are written once, before its
/// first unit; each unit writes its own run config. After each unit's run and
/// eval, whatever its exit status, the unit's `seed-<n>` directory is
/// scrubbed of the provider's credential ([`redact::scrub_tree`]); the run
/// scrub comes before the plan repos are copied out, so their copies are
/// made from clean tars.
///
/// Each `slop-code` run and eval runs in its own process group, with stdin at
/// `/dev/null` and `TMPDIR` at a fresh scratch directory
/// ([`process::run_forwarding`]). When `interrupts` records a signal, it is
/// forwarded to that group; after `stop_grace` the group is killed; the
/// containers mounting the scratch directory, and only those, are removed
/// ([`containers`]); the unit is scrubbed; the interruption is recorded on
/// the unit ([`ArmRecord::interrupted`]) and returned as the error. A signal
/// between steps spawns nothing more and is recorded the same way.
///
/// # Errors
///
/// When a probe, provisioning step, arm command or plan-repo extraction
/// fails, an rdm arm's plan root lies in SCBench's graded directory, or a
/// file cannot be written. An arm failure reads
/// `arm <name> seed <n>: <run|eval> exited <code>`; an interruption reads
/// `arm <name> seed <n>: interrupted by <SIG> during <run|lane|eval> ...`.
#[allow(clippy::too_many_arguments)]
pub fn execute(
    manifest: &Manifest,
    manifest_path: &Path,
    paths: &Paths,
    tools: &Tools,
    seeds: &[u64],
    problems_filter: Option<&[String]>,
    template: Option<&str>,
    prompts: &BTreeMap<String, String>,
    interrupts: &Interrupts,
    stop_grace: Duration,
    stderr: &mut dyn Write,
) -> Result<RunPlan, String> {
    let secret = redact::Secret::for_provider(&manifest.agent.provider);
    let out_dir = &paths.out_dir;
    std::fs::create_dir_all(out_dir)
        .map_err(|e| format!("cannot create --out {}: {e}", out_dir.display()))?;
    let mut ex = Executor {
        record: RunRecord {
            schema: RECORD_SCHEMA,
            manifest: manifest.clone(),
            manifest_path: manifest_path.to_path_buf(),
            rdm: RdmInfo::default(),
            observed: Observed::default(),
            model_file: None,
            seeds: seeds.to_vec(),
            selected_arms: manifest.arms.iter().map(|a| a.name.clone()).collect(),
            problems_filter: problems_filter.map(<[String]>::to_vec),
            arms: Vec::new(),
            commands: Vec::new(),
        },
        path: out_dir.join(plan::RECORD_FILE),
        interrupts,
        stop_grace,
        docker: tools.docker.clone(),
    };
    ex.save()?;

    let rdm_sha = plan::rdm_sha_command(paths);
    let ran = ex.spawn(&rdm_sha, Io::Capture)?;
    if !ran.ok() {
        return Err(format!(
            "cannot resolve the rdm SHA (`{}`) {}",
            rdm_sha.argv().join(" "),
            ran.describe()
        ));
    }
    let sha = ran.stdout.trim().to_owned();
    let plan = plan::build_plan(manifest, paths, tools, seeds, Some(&sha), template, prompts);
    ex.record.rdm.sha = Some(sha);
    ex.record.arms = plan
        .arms
        .iter()
        .map(|a| ArmRecord {
            name: a.name.clone(),
            seed: a.seed,
            image: a.image,
            group: a.group,
            graded_dir: None,
            agent_config: a.agent_config.clone(),
            run_config: a.run_config.clone(),
            run_dir: a.run_dir.clone(),
            lane: None,
            finished: None,
            interrupted: None,
        })
        .collect();
    ex.save()?;

    ex.record.observed.uv = Some(ex.probe("uv", &plan.probes.uv)?);
    ex.save()?;
    ex.record.observed.docker = Some(ex.probe("docker", &plan.probes.docker)?);
    ex.save()?;
    ex.record.observed.claude = Some(ex.probe("claude", &plan.probes.claude)?);
    ex.save()?;

    if let Some(parent) = plan.scbench.dir.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create --cache-dir {}: {e}", parent.display()))?;
    }
    provision("slop-code-bench", &plan.scbench, &mut |spec| {
        ex.spawn(spec, Io::Quiet)
    })?;
    provision("scb-problems", &plan.problems, &mut |spec| {
        ex.spawn(spec, Io::Quiet)
    })?;

    let m = &plan.model_file;
    let bytes = std::fs::read(&m.source)
        .map_err(|e| format!("cannot read model file {}: {e}", m.source.display()))?;
    write_file(&m.installed, &bytes).map_err(|e| format!("cannot install model file: {e}"))?;
    ex.record.model_file = Some(ModelFileRecord {
        source: m.source.clone(),
        installed: m.installed.clone(),
        sha256: Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    });
    ex.save()?;

    if let Some(rdm) = &manifest.rdm
        && plan.arms.iter().any(|a| a.image == ArmImage::Rdm)
    {
        let env = environment::load(&environment::path(
            &plan.scbench.dir,
            &manifest.agent.environment,
        ))?;
        for arm in &mut ex.record.arms {
            if arm.image == ArmImage::Rdm {
                arm.graded_dir = Some(env.docker.workdir.clone());
            }
        }
        ex.save()?;
        let rdm_arms: BTreeSet<&str> = plan
            .arms
            .iter()
            .filter(|a| a.image == ArmImage::Rdm)
            .map(|a| a.name.as_str())
            .collect();
        for arm in rdm_arms {
            environment::refuse_graded_plan_root(
                &format!("arm {arm}"),
                &rdm.plan_root,
                &env.docker.workdir,
            )?;
        }
    }

    let mut written = BTreeSet::new();
    for (i, arm) in plan.arms.iter().enumerate() {
        let unit = format!("{} seed {}", arm.name, arm.seed);
        if written.insert(arm.name.as_str()) {
            write_json(&arm.agent_config_path, &arm.agent_config)?;
            if let (Some(path), Some(text)) = (&arm.docker_template_path, &arm.docker_template) {
                write_file(path, text.as_bytes())?;
            }
            if let (Some(path), Some(text)) = (&arm.prompt_path, &arm.prompt) {
                write_file(path, text.as_bytes())?;
            }
        }
        write_json(&arm.run_config_path, &arm.run_config)?;
        let seed_dir = arm.run_dir.parent().unwrap_or(&arm.run_dir);
        let stop = Stop {
            index: i,
            unit: &unit,
            seed_dir,
            problems: &manifest.problems.subset,
            out: out_dir,
        };
        let ran = ex.slop_code_step(&stop, "run", &arm.run, secret.as_ref(), stderr)?;
        if !ran.ok() {
            return Err(format!("arm {unit}: run {}", ran.describe()));
        }
        if let (Some(dir), Some(rdm)) = (&arm.plan_repos_dir, &manifest.rdm) {
            let collected = lane::collect(
                &arm.name,
                &manifest.problems.subset,
                &arm.run_dir,
                dir,
                &rdm.project,
                &mut |spec| ex.spawn(spec, Io::Quiet),
            );
            // A signal during the lane's git and tar steps makes them fail;
            // report the interruption, not that failure.
            if let Some(sig) = interrupts.received() {
                redact::scrub_tree(seed_dir, secret.as_ref(), "scbench run", stderr);
                return Err(ex.interrupted(&stop, "lane", sig, None));
            }
            let record = collected?;
            let (landed, total) = record.landed();
            ex.record.arms[i].lane = Some(record);
            ex.save()?;
            let _ = writeln!(
                stderr,
                "scbench run: {unit}: {landed}/{total} checkpoints landed"
            );
        }
        let ran = ex.slop_code_step(&stop, "eval", &arm.eval, secret.as_ref(), stderr)?;
        if !ran.ok() {
            return Err(format!("arm {unit}: eval {}", ran.describe()));
        }
        ex.record.arms[i].finished = Some(now());
        ex.save()?;
    }
    Ok(plan)
}

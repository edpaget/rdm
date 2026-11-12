//! Executes a [`RunPlan`]: spawns its commands in order and keeps the run
//! record current after every one.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::environment;
use super::lane::{self, LaneRecord};
use super::manifest::{ArmGroup, ArmImage, Manifest};
use super::plan::{self, CommandSpec, Paths, RunPlan, Tools};

/// The run record's schema version: 2 records one [`ArmRecord`] per arm and
/// seed. A record without `schema` (phases 1-4) has one per arm, at seed 1.
pub const RECORD_SCHEMA: u32 = 2;

/// The run record written to `<out>/rdm-scbench-run.json`.
#[derive(Debug, Serialize)]
pub struct RunRecord {
    /// [`RECORD_SCHEMA`].
    pub schema: u32,
    /// Every pin, as parsed, with the selected arms.
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

struct Executor {
    record: RunRecord,
    path: PathBuf,
}

/// The outcome of one spawn.
pub(super) struct Ran {
    /// The exit code (`None` when it failed to spawn or died by signal).
    pub(super) exit: Option<i32>,
    /// Captured stdout (empty unless [`Io::Capture`]).
    pub(super) stdout: String,
    spawn_error: Option<String>,
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

    /// `exited <n>`, `was killed by a signal` or `failed to spawn: <e>`.
    pub(super) fn describe(&self) -> String {
        match (&self.spawn_error, self.exit) {
            (Some(e), _) => format!("failed to spawn: {e}"),
            (None, Some(code)) => format!("exited {code}"),
            (None, None) => "was killed by a signal".to_owned(),
        }
    }
}

impl Executor {
    fn save(&self) -> Result<(), String> {
        let body = serde_json::to_string_pretty(&self.record).map_err(|e| e.to_string())?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, format!("{body}\n"))
            .map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &self.path)
            .map_err(|e| format!("cannot write {}: {e}", self.path.display()))
    }

    fn spawn(&mut self, spec: &CommandSpec, io: Io) -> Result<Ran, String> {
        let (ran, record) = spawn_command(spec, io);
        self.record.commands.push(record);
        self.save()?;
        Ok(ran)
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

/// Spawns `spec` with `io` and waits for it, returning the outcome and its
/// record.
pub(super) fn spawn_command(spec: &CommandSpec, io: Io) -> (Ran, CommandRecord) {
    let mut cmd = Command::new(&spec.program);
    cmd.args(&spec.args).envs(&spec.env);
    if let Some(cwd) = &spec.cwd {
        cmd.current_dir(cwd);
    }
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
        Ok((exit, stdout)) => Ran {
            exit,
            stdout,
            spawn_error: None,
        },
        Err(e) => Ran {
            exit: None,
            stdout: String::new(),
            spawn_error: Some(e.to_string()),
        },
    };
    let record = CommandRecord {
        argv: spec.argv(),
        cwd: spec.cwd.clone(),
        env: spec.env.clone(),
        exit: ran.exit,
        started,
        finished: now(),
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
/// first unit; each unit writes its own run config.
///
/// # Errors
///
/// When a probe, provisioning step, arm command or plan-repo extraction
/// fails, an rdm arm's plan root lies in SCBench's graded directory, or a
/// file cannot be written. An arm failure reads
/// `arm <name> seed <n>: <run|eval> exited <code>`.
#[allow(clippy::too_many_arguments)]
pub fn execute(
    manifest: &Manifest,
    manifest_path: &Path,
    paths: &Paths,
    tools: &Tools,
    seeds: &[u64],
    template: Option<&str>,
    prompts: &BTreeMap<String, String>,
    stderr: &mut dyn Write,
) -> Result<RunPlan, String> {
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
            arms: Vec::new(),
            commands: Vec::new(),
        },
        path: out_dir.join(plan::RECORD_FILE),
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
        let ran = ex.spawn(&arm.run, Io::Inherit)?;
        if !ran.ok() {
            return Err(format!("arm {unit}: run {}", ran.describe()));
        }
        if let (Some(dir), Some(rdm)) = (&arm.plan_repos_dir, &manifest.rdm) {
            let record = lane::collect(
                &arm.name,
                &manifest.problems.subset,
                &arm.run_dir,
                dir,
                &rdm.project,
                &mut |spec| ex.spawn(spec, Io::Quiet),
            )?;
            let (landed, total) = record.landed();
            ex.record.arms[i].lane = Some(record);
            ex.save()?;
            let _ = writeln!(
                stderr,
                "scbench run: {unit}: {landed}/{total} checkpoints landed"
            );
        }
        let ran = ex.spawn(&arm.eval, Io::Inherit)?;
        if !ran.ok() {
            return Err(format!("arm {unit}: eval {}", ran.describe()));
        }
        ex.record.arms[i].finished = Some(now());
        ex.save()?;
    }
    Ok(plan)
}

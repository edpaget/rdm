//! Executes a [`RunPlan`]: spawns its commands in order and keeps the run
//! record current after every one.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::manifest::Manifest;
use super::plan::{CommandSpec, RunPlan};

/// The run record written to `<out>/rdm-scbench-run.json`.
#[derive(Debug, Serialize)]
pub struct RunRecord {
    /// Every pin, as parsed.
    pub manifest: Manifest,
    /// The manifest file the run loaded.
    pub manifest_path: PathBuf,
    /// The rdm checkout under test.
    pub rdm: RdmInfo,
    /// Tool versions the probes observed.
    pub observed: Observed,
    /// The installed model file, once copied.
    pub model_file: Option<ModelFileRecord>,
    /// Each arm's generated configs and SCBench run directory.
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

/// One arm's configuration.
#[derive(Debug, Serialize)]
pub struct ArmRecord {
    /// The arm name.
    pub name: String,
    /// The agent config written for the arm.
    pub agent_config: Value,
    /// The run config written for the arm.
    pub run_config: Value,
    /// SCBench's output directory for the arm.
    pub run_dir: PathBuf,
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

#[derive(Clone, Copy)]
enum Io {
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
struct Ran {
    exit: Option<i32>,
    stdout: String,
    spawn_error: Option<String>,
}

impl Ran {
    fn ok(&self) -> bool {
        self.exit == Some(0)
    }

    fn describe(&self) -> String {
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
        self.record.commands.push(CommandRecord {
            argv: spec.argv(),
            cwd: spec.cwd.clone(),
            env: spec.env.clone(),
            exit: ran.exit,
            started,
            finished: now(),
        });
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

    fn provision(&mut self, name: &str, c: &super::plan::Checkout) -> Result<(), String> {
        let steps: Vec<&CommandSpec> = if c.dir.exists() {
            vec![&c.set_url, &c.fetch, &c.checkout]
        } else {
            vec![&c.clone, &c.fetch, &c.checkout]
        };
        for spec in steps {
            let ran = self.spawn(spec, Io::Quiet)?;
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
}

fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    let body = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    std::fs::write(path, format!("{body}\n"))
        .map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// Runs `plan` for `manifest` (loaded from `manifest_path`) to completion,
/// stopping at the first failing step.
///
/// # Errors
///
/// When a probe, provisioning step or arm command fails, or a file cannot be
/// written. An arm failure reads `arm <name>: <run|eval> exited <code>`.
pub fn execute(manifest: &Manifest, manifest_path: &Path, plan: &RunPlan) -> Result<(), String> {
    let out_dir = plan
        .record_path
        .parent()
        .ok_or("the run record has no parent directory")?;
    std::fs::create_dir_all(out_dir)
        .map_err(|e| format!("cannot create --out {}: {e}", out_dir.display()))?;
    let mut ex = Executor {
        record: RunRecord {
            manifest: manifest.clone(),
            manifest_path: manifest_path.to_path_buf(),
            rdm: RdmInfo::default(),
            observed: Observed::default(),
            model_file: None,
            arms: plan
                .arms
                .iter()
                .map(|a| ArmRecord {
                    name: a.name.clone(),
                    agent_config: a.agent_config.clone(),
                    run_config: a.run_config.clone(),
                    run_dir: a.run_dir.clone(),
                })
                .collect(),
            commands: Vec::new(),
        },
        path: plan.record_path.clone(),
    };
    ex.save()?;

    let ran = ex.spawn(&plan.rdm_sha, Io::Capture)?;
    if !ran.ok() {
        return Err(format!(
            "cannot resolve the rdm SHA (`{}`) {}",
            plan.rdm_sha.argv().join(" "),
            ran.describe()
        ));
    }
    ex.record.rdm.sha = Some(ran.stdout.trim().to_owned());

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
    ex.provision("slop-code-bench", &plan.scbench)?;
    ex.provision("scb-problems", &plan.problems)?;

    let m = &plan.model_file;
    let bytes = std::fs::read(&m.source)
        .map_err(|e| format!("cannot read model file {}: {e}", m.source.display()))?;
    if let Some(parent) = m.installed.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    std::fs::write(&m.installed, &bytes)
        .map_err(|e| format!("cannot install model file {}: {e}", m.installed.display()))?;
    ex.record.model_file = Some(ModelFileRecord {
        source: m.source.clone(),
        installed: m.installed.clone(),
        sha256: Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    });
    ex.save()?;

    for arm in &plan.arms {
        if let Some(dir) = arm.agent_config_path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        }
        write_json(&arm.agent_config_path, &arm.agent_config)?;
        write_json(&arm.run_config_path, &arm.run_config)?;
        for (step, spec) in [("run", &arm.run), ("eval", &arm.eval)] {
            let ran = ex.spawn(spec, Io::Inherit)?;
            if !ran.ok() {
                return Err(format!("arm {}: {step} {}", arm.name, ran.describe()));
            }
        }
    }
    Ok(())
}

//! The opt-in live Codex coexistence smoke flow behind
//! `rdm-smoke codex-coexistence` (replaces `scripts/verify-codex-coexistence.mjs`).
//!
//! It proves that a repository's rdm skills, a user-scope copy and an
//! installed-plugin copy of the same skill coexist in Codex without any of the
//! competing copies being modified, and — only with explicit consent — that a
//! live `codex exec` selects the repository copy. Everything happens in a
//! private temporary installation:
//!
//! 1. A private evidence root (mode 0700, realpath'd) holding `home/`,
//!    `config/` (`CODEX_HOME`) and `source/`.
//! 2. Children run with `HOME`/`CODEX_HOME`/`XDG_CONFIG_HOME` redirected into
//!    it, `GIT_CONFIG_{GLOBAL,SYSTEM}=/dev/null`, and every `RDM_*` variable
//!    (and any repository-locating `GIT_DIR`-style variable) removed.
//! 3. Competing USER and PLUGIN `rdm-roadmap` skills, a plugin manifest and a
//!    personal marketplace are seeded (mode 0600).
//! 4. `git init`, then `codex plugin add rdm-coexistence@personal --json`.
//! 5. The protected bytes (user skill, plugin skill, installed skill,
//!    marketplace, plugin manifest) are snapshotted.
//! 6. `rdm agent-config codex --skills --project coexistence --out source`, and
//!    the repository skill is snapshotted.
//! 7. The skill-catalog protocol over `codex app-server --stdio`: `initialize`
//!    (id 1), the `initialized` notification, then `skills/list` (id 2) —
//!    non-JSON lines are skipped; an `error` message, an early exit or the
//!    discovery deadline fail; the child is killed and reaped on every path.
//! 8. The catalog must report no errors and all three copies enabled (the
//!    plugin one carrying `pluginId == rdm-coexistence@personal`);
//!    `catalog.json` is written.
//! 9. With `--copy-auth-from` only: `codex exec --ephemeral --sandbox read-only
//!    …` under a bounded run whose prepare/cleanup hooks copy the login file to
//!    `config/auth.json` (0600) and remove it on every catchable path
//!    (including SIGINT/SIGTERM and the exec deadline). The trace must show a
//!    successful read of the repository skill yielding `needs-plan-review`,
//!    and the answer must select the repository skill, name the plan gate and
//!    quote neither competing copy. Without consent the live step is NOT RUN
//!    and `result.json` records `liveInvocation: false`.
//! 10. On success and on failure alike, every protected file and the
//!     repository skill must be byte-identical to its snapshot.
//! 11. `result.json` (unchanged schema) is written, and the evidence root is
//!     always reported.
//!
//! No credentials are ever printed or written to the evidence.

use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};

use crate::measure::jsjson::{JsValue, obj};
use crate::process::{Hooks, ProcessSpec, RunError, Session, SessionError, run_bounded};

/// Default skill-discovery deadline.
pub const DEFAULT_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(30);
/// Default live-exec deadline.
pub const DEFAULT_EXEC_TIMEOUT: Duration = Duration::from_secs(120);
/// Deadline for each setup command (git, plugin add, rdm, --version).
const SETUP_TIMEOUT: Duration = Duration::from_secs(120);

/// The plugin id the installed copy must carry.
pub const PLUGIN_ID: &str = "rdm-coexistence@personal";

/// What to run.
#[derive(Debug, Clone)]
pub struct Options {
    /// The rdm binary under test.
    pub rdm: PathBuf,
    /// The Codex binary.
    pub codex: PathBuf,
    /// The login file to copy privately for the live invocation — the explicit
    /// credentials consent. `None`: live invocation is NOT RUN.
    pub copy_auth_from: Option<PathBuf>,
    /// Skill-discovery deadline.
    pub discovery_timeout: Duration,
    /// Live-exec deadline.
    pub exec_timeout: Duration,
}

/// Why the flow failed.
#[derive(Debug)]
pub enum Failure {
    /// A check failed (exit 1).
    Check(String),
    /// A deadline passed (exit 124).
    TimedOut(String),
    /// SIGINT/SIGTERM arrived (exit 128 + signal).
    Interrupted(i32),
}

impl Failure {
    /// The process exit code for this failure.
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::Check(_) => 1,
            Self::TimedOut(_) => 124,
            Self::Interrupted(sig) => u8::try_from(128 + sig).unwrap_or(1),
        }
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Check(m) | Self::TimedOut(m) => f.write_str(m),
            Self::Interrupted(sig) => {
                write!(f, "interrupted by signal {sig}; every child was killed")
            }
        }
    }
}

fn check(msg: impl Into<String>) -> Failure {
    Failure::Check(msg.into())
}

/// A successful run's evidence.
#[derive(Debug, Clone)]
pub struct Outcome {
    /// The live invocation ran and passed.
    pub live: bool,
    /// The `result.json` written.
    pub result: Value,
}

/// The private evidence layout.
#[derive(Debug, Clone)]
pub struct Layout {
    /// The evidence root (0700).
    pub root: PathBuf,
    /// `HOME` for every child.
    pub home: PathBuf,
    /// `CODEX_HOME`.
    pub config: PathBuf,
    /// The repository the skills are emitted into.
    pub source: PathBuf,
}

impl Layout {
    /// The competing user-scope skill.
    pub fn user_skill(&self) -> PathBuf {
        self.home.join(".agents/skills/rdm-roadmap/SKILL.md")
    }
    /// The plugin source directory.
    pub fn plugin(&self) -> PathBuf {
        self.home.join("plugins/rdm-coexistence")
    }
    /// The competing plugin skill (source copy).
    pub fn plugin_skill(&self) -> PathBuf {
        self.plugin().join("skills/rdm-roadmap/SKILL.md")
    }
    /// The plugin manifest.
    pub fn plugin_manifest(&self) -> PathBuf {
        self.plugin().join(".codex-plugin/plugin.json")
    }
    /// The personal marketplace.
    pub fn marketplace(&self) -> PathBuf {
        self.home.join(".agents/plugins/marketplace.json")
    }
    /// The repository skill rdm emits.
    pub fn repository_skill(&self) -> PathBuf {
        self.source.join(".agents/skills/rdm-roadmap/SKILL.md")
    }
    /// The private login copy (exists only during the live exec).
    pub fn auth_copy(&self) -> PathBuf {
        self.config.join("auth.json")
    }
}

/// Creates the private evidence root under the system temp dir (honouring
/// `TMPDIR`). It is kept: it is the evidence.
///
/// # Errors
///
/// When the directory cannot be created.
pub fn create_layout() -> io::Result<Layout> {
    let dir = tempfile::Builder::new()
        .prefix("rdm-codex-coexistence-")
        .tempdir()?;
    let root = fs::canonicalize(dir.keep())?;
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
    let layout = Layout {
        home: root.join("home"),
        config: root.join("config"),
        source: root.join("source"),
        root,
    };
    for d in [&layout.home, &layout.config, &layout.source] {
        fs::create_dir_all(d)?;
    }
    Ok(layout)
}

/// Writes `bytes` to `path` with mode 0600, creating parents.
///
/// # Errors
///
/// The I/O error.
pub fn put(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut f = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(bytes)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}

fn put_json(path: &Path, value: &Value) -> io::Result<()> {
    let text = serde_json::to_string_pretty(value).map_err(io::Error::other)?;
    put(path, format!("{text}\n").as_bytes())
}

/// Writes a JSON document given as text, keeping its key order
/// (`JSON.stringify(value, null, 2)` + newline, as the JS flow wrote it).
fn put_ordered(path: &Path, text: &str) -> io::Result<()> {
    let value = JsValue::parse(text).map_err(io::Error::other)?;
    put(path, format!("{}\n", value.stringify_pretty()).as_bytes())
}

fn competing(copy: &str) -> String {
    format!(
        "---\nname: rdm-roadmap\ndescription: {copy} copy for the isolated coexistence test.\n---\n\nWhen explicitly selected, report {copy}_COPY_ONLY. Do not mutate anything.\n"
    )
}

/// Seeds the competing copies, the plugin manifest and the marketplace.
///
/// # Errors
///
/// The I/O error.
pub fn seed(l: &Layout) -> io::Result<()> {
    put(&l.user_skill(), competing("USER").as_bytes())?;
    put(&l.plugin_skill(), competing("PLUGIN").as_bytes())?;
    put_ordered(
        &l.plugin_manifest(),
        r#"{"name":"rdm-coexistence","version":"0.1.0","description":"Isolated coexistence fixture","skills":"./skills/"}"#,
    )?;
    put_ordered(
        &l.marketplace(),
        r#"{"name":"personal","interface":{"displayName":"Personal"},"plugins":[{"name":"rdm-coexistence","source":{"source":"local","path":"./plugins/rdm-coexistence"},"policy":{"installation":"AVAILABLE","authentication":"ON_INSTALL"},"category":"Productivity"}]}"#,
    )
}

/// Git variables that point a child at the caller's repository; removed so
/// the private installation's `git init` stands alone.
const GIT_CONTEXT_VARS: [&str; 7] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_COMMON_DIR",
    "GIT_PREFIX",
];

/// The isolated child environment.
#[derive(Debug, Clone)]
pub struct ChildEnv {
    set: Vec<(String, PathBuf)>,
    remove: Vec<std::ffi::OsString>,
}

impl ChildEnv {
    /// Redirects HOME/CODEX_HOME/XDG_CONFIG_HOME into `l`, nulls git's global
    /// and system config, and drops every `RDM_*` variable.
    pub fn for_layout(l: &Layout) -> Self {
        // Every RDM_* variable, plus the repository-locating git variables a
        // caller's own git context (for example a hook) would otherwise leak
        // into the private `git init`.
        let remove = std::env::vars_os()
            .map(|(k, _)| k)
            .filter(|k| {
                let k = k.to_string_lossy();
                k.starts_with("RDM_") || GIT_CONTEXT_VARS.contains(&k.as_ref())
            })
            .collect();
        Self {
            set: vec![
                ("HOME".to_owned(), l.home.clone()),
                ("CODEX_HOME".to_owned(), l.config.clone()),
                ("XDG_CONFIG_HOME".to_owned(), l.home.join(".config")),
                ("GIT_CONFIG_GLOBAL".to_owned(), PathBuf::from("/dev/null")),
                ("GIT_CONFIG_SYSTEM".to_owned(), PathBuf::from("/dev/null")),
            ],
            remove,
        }
    }

    fn apply(&self, mut spec: ProcessSpec) -> ProcessSpec {
        for k in &self.remove {
            spec = spec.env_remove(k);
        }
        for (k, v) in &self.set {
            spec = spec.env(k, v);
        }
        spec
    }
}

fn run_failure(what: &str, e: RunError) -> Failure {
    match e {
        RunError::TimedOut(d) => Failure::TimedOut(format!(
            "{what} did not finish within {}s and was killed",
            d.as_secs()
        )),
        RunError::Interrupted(sig) => Failure::Interrupted(sig),
        other => check(format!("{what} failed: {other}")),
    }
}

fn run_setup(
    env: &ChildEnv,
    cwd: &Path,
    program: &Path,
    args: &[&str],
    what: &str,
) -> Result<Vec<u8>, Failure> {
    let spec = env.apply(
        ProcessSpec::new(program)
            .args(args)
            .cwd(cwd)
            .timeout(SETUP_TIMEOUT),
    );
    run_bounded(&spec, Hooks::new())
        .map(|o| o.stdout)
        .map_err(|e| run_failure(what, e))
}

fn read(path: &Path) -> Result<Vec<u8>, Failure> {
    fs::read(path).map_err(|e| check(format!("cannot read {}: {e}", path.display())))
}

/// Runs the catalog protocol over `codex app-server --stdio` and returns
/// `result.data[0]` of the `skills/list` reply. The child is killed and reaped
/// on every path.
///
/// # Errors
///
/// An `error` message, an early exit, the deadline, or a signal.
pub fn discover(
    codex: &Path,
    env: &ChildEnv,
    source: &Path,
    timeout: Duration,
) -> Result<Value, Failure> {
    let spec = env.apply(
        ProcessSpec::new(codex)
            .args(["app-server", "--stdio"])
            .cwd(source)
            .timeout(timeout),
    );
    let mut session = Session::spawn(&spec)
        .map_err(|e| check(format!("could not start `codex app-server --stdio`: {e}")))?;
    let result = converse(&mut session, source);
    session.abort();
    result
}

fn converse(session: &mut Session, source: &Path) -> Result<Value, Failure> {
    let send = |session: &mut Session, v: Value| {
        session
            .send_line(&v.to_string())
            .map_err(|e| check(format!("Codex skill discovery could not write: {e}")))
    };
    send(
        session,
        json!({"id": 1, "method": "initialize", "params": {
            "clientInfo": {"name": "rdm-coexistence-test", "version": "1.0"}
        }}),
    )?;
    loop {
        let line = session.recv_line().map_err(|e| match e {
            SessionError::TimedOut { .. } => Failure::TimedOut(
                "Codex skill discovery timed out; the app-server was killed and reaped".to_owned(),
            ),
            SessionError::Interrupted(sig) => Failure::Interrupted(sig),
            SessionError::Exited { stderr } => check(format!(
                "Codex exited before discovery{}",
                if stderr.trim().is_empty() {
                    String::new()
                } else {
                    format!(": {}", stderr.trim())
                }
            )),
            other => check(format!("Codex skill discovery failed: {other}")),
        })?;
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if let Some(error) = message.get("error") {
            return Err(check(format!(
                "Codex skill discovery returned an error: {error}"
            )));
        }
        match message.get("id").and_then(Value::as_u64) {
            Some(1) => {
                send(session, json!({"method": "initialized", "params": {}}))?;
                send(
                    session,
                    json!({"id": 2, "method": "skills/list", "params": {
                        "cwds": [source], "forceReload": true
                    }}),
                )?;
            }
            Some(2) => {
                return message
                    .get("result")
                    .and_then(|r| r.get("data"))
                    .and_then(|d| d.get(0))
                    .cloned()
                    .ok_or_else(|| {
                        check(format!(
                            "skills/list reply carries no result.data[0]: {message}"
                        ))
                    });
            }
            _ => {}
        }
    }
}

/// Checks the catalog: no errors, all three copies enabled, the installed one
/// attributed to [`PLUGIN_ID`].
///
/// # Errors
///
/// The first failed check.
pub fn check_catalog(
    catalog: &Value,
    paths: [&Path; 3],
    installed_skill: &Path,
) -> Result<(), Failure> {
    if catalog.get("errors") != Some(&json!([])) {
        return Err(check(format!(
            "the skill catalog reported errors: {}",
            catalog.get("errors").cloned().unwrap_or(Value::Null)
        )));
    }
    let skills = catalog
        .get("skills")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let is =
        |s: &Value, p: &Path| s.get("path").and_then(Value::as_str) == Some(&*p.to_string_lossy());
    for p in paths {
        let enabled = skills
            .iter()
            .any(|s| is(s, p) && crate::measure::sidecar::truthy(s.get("enabled")));
        if !enabled {
            return Err(check(format!("Missing enabled skill: {}", p.display())));
        }
    }
    let attributed = skills.iter().any(|s| {
        is(s, installed_skill) && s.get("pluginId").and_then(Value::as_str) == Some(PLUGIN_ID)
    });
    if !attributed {
        return Err(check(format!(
            "the installed plugin skill {} is not attributed to {PLUGIN_ID}",
            installed_skill.display()
        )));
    }
    Ok(())
}

const PROMPT_TAIL: &str = ", not the user or plugin copy. This is an inspection-only smoke test: read the selected skill, identify its absolute path and the required plan-review gate, then stop. Do not run rdm, create plans, edit files, invoke agents, or use the network. Do not treat an alternative copy as selected merely because its name matches.";

/// `/needs-plan-review|independent.*review/i` (`.` stops at a newline).
fn plan_gate_ok(gate: &str) -> bool {
    gate.to_lowercase().split('\n').any(|line| {
        line.contains("needs-plan-review")
            || line
                .find("independent")
                .is_some_and(|at| line[at..].contains("review"))
    })
}

/// The live invocation: `codex exec` with the private login copy, checked
/// against the trace and the answer.
///
/// # Errors
///
/// A failed check, the exec deadline, or a signal (the login copy is removed
/// on every path).
pub fn live(o: &Options, l: &Layout, env: &ChildEnv, auth: &Path) -> Result<(), Failure> {
    let output = l.root.join("answer.json");
    let schema = l.root.join("schema.json");
    let events_path = l.root.join("events.jsonl");
    put_ordered(
        &schema,
        r#"{"type":"object","additionalProperties":false,"required":["selected_path","plan_gate"],"properties":{"selected_path":{"type":"string"},"plan_gate":{"type":"string"}}}"#,
    )
    .map_err(|e| check(format!("cannot write {}: {e}", schema.display())))?;
    let repository_skill = l.repository_skill();
    let prompt = format!(
        "Use $rdm-roadmap specifically from {}{PROMPT_TAIL}",
        repository_skill.display()
    );
    let source = l.source.to_string_lossy().into_owned();
    let spec = env.apply(
        ProcessSpec::new(&o.codex)
            .args([
                "exec",
                "--ephemeral",
                "--sandbox",
                "read-only",
                "--json",
                "--cd",
                &source,
                "--output-schema",
            ])
            .arg(&schema)
            .arg("--output-last-message")
            .arg(&output)
            .arg(&prompt)
            .cwd(&l.source)
            .timeout(o.exec_timeout),
    );
    let auth_copy = l.auth_copy();
    let hooks = Hooks::new()
        .prepare(|| {
            // Private before any byte of the login lands in it.
            put(&auth_copy, b"")?;
            put(&auth_copy, &fs::read(auth)?)
        })
        .cleanup(|| match fs::remove_file(&auth_copy) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            other => other,
        });
    let run = run_bounded(&spec, hooks).map_err(|e| run_failure("`codex exec`", e))?;
    put(&events_path, &run.stdout)
        .map_err(|e| check(format!("cannot write {}: {e}", events_path.display())))?;
    let events = String::from_utf8_lossy(&run.stdout);
    let wanted = repository_skill.to_string_lossy();
    let read_observed = events
        .trim()
        .split('\n')
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .any(|e| {
            let item = e.get("item");
            e.get("type").and_then(Value::as_str) == Some("item.completed")
                && item.and_then(|i| i.get("type")).and_then(Value::as_str)
                    == Some("command_execution")
                && item
                    .and_then(|i| i.get("exit_code"))
                    .and_then(Value::as_i64)
                    == Some(0)
                && item
                    .and_then(|i| i.get("command"))
                    .and_then(Value::as_str)
                    .is_some_and(|c| c.contains(&*wanted))
                && item
                    .and_then(|i| i.get("aggregated_output"))
                    .and_then(Value::as_str)
                    .is_some_and(|out| out.contains("needs-plan-review"))
        });
    if !read_observed {
        return Err(check(
            "No successful read of the selected repository skill in the CLI trace",
        ));
    }
    let answer_bytes = read(&output)?;
    let _ = fs::set_permissions(&output, fs::Permissions::from_mode(0o600));
    let answer: Value = serde_json::from_slice(&answer_bytes)
        .map_err(|e| check(format!("the answer {} is not JSON: {e}", output.display())))?;
    let selected = answer
        .get("selected_path")
        .and_then(Value::as_str)
        .unwrap_or("");
    if selected != wanted {
        return Err(check(format!(
            "Codex selected {selected:?}, not the repository skill {wanted}"
        )));
    }
    let gate = answer
        .get("plan_gate")
        .and_then(Value::as_str)
        .unwrap_or("");
    if !plan_gate_ok(gate) {
        return Err(check(format!(
            "the answer's plan gate {gate:?} names no plan-review gate"
        )));
    }
    let text = answer.to_string();
    if text.contains("USER_COPY_ONLY") || text.contains("PLUGIN_COPY_ONLY") {
        return Err(check(
            "the answer quotes a competing copy (USER_COPY_ONLY/PLUGIN_COPY_ONLY)",
        ));
    }
    Ok(())
}

/// Runs the whole flow in `l` (already created). The protected-byte guard runs
/// whether or not the checks before it passed.
///
/// # Errors
///
/// The failure to report; a protected-byte change supersedes any other.
pub fn run(o: &Options, l: &Layout) -> Result<Outcome, Failure> {
    let env = ChildEnv::for_layout(l);
    seed(l).map_err(|e| check(format!("cannot seed the fixture: {e}")))?;
    run_setup(
        &env,
        &l.source,
        Path::new("git"),
        &["init", "-q"],
        "`git init`",
    )?;
    let installed = run_setup(
        &env,
        &l.source,
        &o.codex,
        &["plugin", "add", PLUGIN_ID, "--json"],
        "`codex plugin add`",
    )?;
    let installed: Value = serde_json::from_slice(&installed)
        .map_err(|e| check(format!("`codex plugin add --json` printed no JSON: {e}")))?;
    let installed_path = installed
        .get("installedPath")
        .and_then(Value::as_str)
        .ok_or_else(|| check("`codex plugin add --json` reported no installedPath"))?;
    let installed_skill = Path::new(installed_path).join("skills/rdm-roadmap/SKILL.md");
    let protected = [
        l.user_skill(),
        l.plugin_skill(),
        installed_skill.clone(),
        l.marketplace(),
        l.plugin_manifest(),
    ];
    let before: Vec<Vec<u8>> = protected
        .iter()
        .map(|p| read(p))
        .collect::<Result<_, _>>()?;
    let source = l.source.to_string_lossy().into_owned();
    run_setup(
        &env,
        &l.source,
        &o.rdm,
        &[
            "agent-config",
            "codex",
            "--skills",
            "--project",
            "coexistence",
            "--out",
            &source,
        ],
        "`rdm agent-config codex --skills`",
    )?;
    let repository_skill = l.repository_skill();
    let repository_bytes = read(&repository_skill)?;

    let checked = (|| -> Result<bool, Failure> {
        let catalog = discover(&o.codex, &env, &l.source, o.discovery_timeout)?;
        check_catalog(
            &catalog,
            [&repository_skill, &l.user_skill(), &installed_skill],
            &installed_skill,
        )?;
        let rdm_skills: Vec<Value> = catalog
            .get("skills")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter(|s| {
                        s.get("name")
                            .and_then(Value::as_str)
                            .is_some_and(|n| n.contains("rdm-"))
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        put_json(&l.root.join("catalog.json"), &Value::Array(rdm_skills))
            .map_err(|e| check(format!("cannot write catalog.json: {e}")))?;
        match &o.copy_auth_from {
            Some(auth) => live(o, l, &env, auth).map(|()| true),
            None => Ok(false),
        }
    })();

    // The guard: runs on success and failure alike, and a changed protected
    // file is the failure reported.
    let _ = fs::remove_file(l.auth_copy());
    let mut changed: Vec<String> = Vec::new();
    for (p, b) in protected.iter().zip(&before) {
        if fs::read(p).ok().as_ref() != Some(b) {
            changed.push(format!("Changed competing copy: {}", p.display()));
        }
    }
    if fs::read(&repository_skill).ok() != Some(repository_bytes) {
        changed.push("Live test modified repository skill".to_owned());
    }
    if !changed.is_empty() {
        let mut msg = changed.join("; ");
        if let Err(e) = &checked {
            msg.push_str(&format!(" (after: {e})"));
        }
        return Err(check(msg));
    }
    let live = checked?;
    let version = run_setup(
        &env,
        &l.source,
        &o.codex,
        &["--version"],
        "`codex --version`",
    )?;
    let selected = repository_skill.strip_prefix(&l.root).map_or_else(
        |_| repository_skill.display().to_string(),
        |p| p.display().to_string(),
    );
    let result = obj([
        (
            "codexVersion",
            JsValue::from(String::from_utf8_lossy(&version).trim()),
        ),
        ("discoveredCopies", JsValue::Number(3.0)),
        ("liveInvocation", JsValue::Bool(live)),
        ("selectedPath", JsValue::String(selected)),
        ("competingCopiesUnchanged", JsValue::Bool(true)),
        ("authCopyRemoved", JsValue::Bool(o.copy_auth_from.is_some())),
    ]);
    put(
        &l.root.join("result.json"),
        format!("{}\n", result.stringify_pretty()).as_bytes(),
    )
    .map_err(|e| check(format!("cannot write result.json: {e}")))?;
    Ok(Outcome {
        live,
        result: result.to_json(),
    })
}

/// The final line on success.
pub fn pass_line(outcome: &Outcome, l: &Layout) -> String {
    format!(
        "PASS: three enabled copies discovered; {}; competing files unchanged. Evidence: {}",
        if outcome.live {
            "live repository invocation passed"
        } else {
            "live invocation NOT RUN (no --copy-auth-from login file supplied)"
        },
        l.root.display()
    )
}

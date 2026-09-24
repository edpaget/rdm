//! Nested `cargo nextest run` invocations.
//!
//! A [`Nested`] run spawns `$CARGO nextest run …` through
//! `rdm_devtools::process::run_bounded`: in a fresh process group, under an
//! inner deadline ([`INNER_DEADLINE`]) below the `suite-hygiene` profile's
//! slow-timeout backstop, with stdout and stderr in a log file. A nextest
//! timeout (SIGTERM) or the inner deadline tears the whole nested tree down.
//!
//! The environment is the invoking one — so `RUSTFLAGS`, `RUSTUP_TOOLCHAIN`,
//! `CARGO_TARGET_DIR` and the rest match and nothing rebuilds — minus every
//! `RDM_*` except `RDM_TEST_NODE`, every `NEXTEST_*`/`__NEXTEST_*`, the
//! repo-redirecting git variables
//! ([`crate::git_test_support::repo_redirect_removals`]) and the per-test cargo
//! variables. Then `CARGO_TERM_COLOR=never` and `TMPDIR=<scratch>` are set, and
//! the scenario's overrides are applied last. The test process's own
//! environment is never touched.
//!
//! The result is classified by nextest's exit status, never by parsing its
//! text: 0 and 100 (`TEST_RUN_FAILED`) are results the test judges
//! ([`NestedRun::code`]); anything else — 101 (build failed), 4 (no tests
//! run), 96 (setup error), 94 (invalid filterset), a signal, the deadline — is
//! [`Failure::Infra`]: the scenario was not run.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rdm_devtools::process::{Hooks, ProcessSpec, RunError, run_bounded};

use crate::git_test_support::repo_redirect_removals;
use crate::workflow_support::{Failure, infra, repo_root};

/// The inner deadline of one nested run: below the `suite-hygiene` profile's
/// slow-timeout backstop (`.config/nextest.toml`), so a hung nested suite
/// fails here, with its log, rather than as a bare nextest timeout.
pub const INNER_DEADLINE: Duration = Duration::from_secs(25 * 60);

/// nextest's `TEST_RUN_FAILED` exit status: the build succeeded, tests ran,
/// and at least one failed.
pub const TEST_RUN_FAILED: i32 = 100;

/// How many trailing log lines a failure message carries.
const TAIL_LINES: usize = 60;

/// The per-test cargo variables a test process inherits from nextest, none of
/// which may reach a nested cargo.
const PER_TEST_CARGO_VARS: &[&str] = &[
    "CARGO_MANIFEST_DIR",
    "CARGO_MANIFEST_PATH",
    "CARGO_CRATE_NAME",
    "CARGO_BIN_NAME",
    "CARGO_PRIMARY_PACKAGE",
    "CARGO_TARGET_TMPDIR",
    "CARGO_RUSTC_CURRENT_DIR",
];

/// The cargo that runs nested suites: `$CARGO` (set by cargo for its
/// children), else the one that compiled this test.
pub fn cargo() -> OsString {
    std::env::var_os("CARGO").unwrap_or_else(|| env!("CARGO").into())
}

/// The inherited variables a nested run must not see.
fn removals() -> Vec<OsString> {
    let mut keys: Vec<OsString> = std::env::vars_os()
        .map(|(k, _)| k)
        .filter(|k| {
            let k = k.to_string_lossy();
            (k.starts_with("RDM_") && k != "RDM_TEST_NODE")
                || k.starts_with("NEXTEST_")
                || k.starts_with("__NEXTEST_")
                || k.starts_with("CARGO_PKG_")
                || k.starts_with("CARGO_BIN_EXE_")
        })
        .collect();
    keys.extend(repo_redirect_removals());
    keys.extend(PER_TEST_CARGO_VARS.iter().map(OsString::from));
    keys
}

/// `RUSTUP_HOME` and `CARGO_HOME` at their real values, for a scenario that
/// overrides `HOME`: under the rustup shim `cargo` resolves its toolchain
/// through them, and they follow `HOME` unless set.
pub fn rust_home_pins() -> Vec<(OsString, OsString)> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    let pin = |var: &str, default: &str| {
        (
            OsString::from(var),
            std::env::var_os(var).unwrap_or_else(|| home.join(default).into()),
        )
    };
    vec![pin("RUSTUP_HOME", ".rustup"), pin("CARGO_HOME", ".cargo")]
}

/// One nested `cargo nextest run`.
pub struct Nested {
    cwd: PathBuf,
    scratch: PathBuf,
    args: Vec<OsString>,
    env: Vec<(OsString, OsString)>,
    remove: Vec<OsString>,
}

/// How a nested run ended when it ran: exit 0 or [`TEST_RUN_FAILED`].
#[derive(Debug)]
pub struct NestedRun {
    /// nextest's exit status (0 or [`TEST_RUN_FAILED`]).
    pub code: i32,
    /// The run's combined output.
    pub log: String,
}

impl NestedRun {
    /// The last lines of the log.
    pub fn tail(&self) -> String {
        tail(&self.log)
    }
}

fn tail(log: &str) -> String {
    let lines: Vec<&str> = log.lines().collect();
    lines[lines.len().saturating_sub(TAIL_LINES)..].join("\n")
}

impl Nested {
    /// A run in the checkout, with `scratch` (a directory the test owns) as
    /// its `TMPDIR` and log location.
    pub fn in_checkout(scratch: &Path) -> Self {
        Self::in_dir(&repo_root(), scratch)
    }

    /// A run with working directory `cwd`.
    pub fn in_dir(cwd: &Path, scratch: &Path) -> Self {
        Self {
            cwd: cwd.to_owned(),
            scratch: scratch.to_owned(),
            args: Vec::new(),
            env: Vec::new(),
            remove: Vec::new(),
        }
    }

    /// Appends `cargo nextest run` arguments.
    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    /// Sets one variable, applied after the removals.
    pub fn env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    /// Removes one more inherited variable.
    pub fn env_remove(mut self, key: impl Into<OsString>) -> Self {
        self.remove.push(key.into());
        self
    }

    /// The `TMPDIR` the nested run sees.
    pub fn tmpdir(&self) -> PathBuf {
        self.scratch.join("tmp")
    }

    /// Runs it.
    ///
    /// # Errors
    ///
    /// [`Failure::Infra`] when the run did not produce a test result: no
    /// `cargo-nextest`, a build failure, no tests selected, a setup error, a
    /// signal or the inner deadline. The message carries the log's tail.
    pub fn run(self) -> Result<NestedRun, Failure> {
        let tmpdir = self.tmpdir();
        std::fs::create_dir_all(&tmpdir).map_err(infra)?;
        let log_path = self.scratch.join("nested.log");

        let mut spec = ProcessSpec::new(cargo())
            .arg("nextest")
            .arg("run")
            .args(&self.args)
            .cwd(&self.cwd)
            .timeout(INNER_DEADLINE)
            .output_file(&log_path);
        for key in removals().into_iter().chain(self.remove) {
            spec = spec.env_remove(key);
        }
        spec = spec.env("CARGO_TERM_COLOR", "never").env("TMPDIR", &tmpdir);
        for (key, value) in self.env {
            spec = spec.env(key, value);
        }

        let started = Instant::now();
        let result = run_bounded(&spec, Hooks::new());
        let elapsed = started.elapsed();
        let log = std::fs::read_to_string(&log_path).unwrap_or_default();
        let code = match result {
            Ok(_) => 0,
            Err(RunError::NonZeroExit {
                code: Some(TEST_RUN_FAILED),
                ..
            }) => TEST_RUN_FAILED,
            Err(e) => return Err(not_run(&self.cwd, &self.args, &e, &log)),
        };
        eprintln!(
            "nested `cargo nextest run {}` in {}: exit {code} after {:.1}s",
            join(&self.args),
            self.cwd.display(),
            elapsed.as_secs_f64()
        );
        Ok(NestedRun { code, log })
    }
}

fn join(args: &[OsString]) -> String {
    args.iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

fn not_run(cwd: &Path, args: &[OsString], e: &RunError, log: &str) -> Failure {
    Failure::Infra(format!(
        "the nested `cargo nextest run {}` in {} produced no test result ({e}): the \
         scenario was not run. If the log says cargo has no `nextest` command, install \
         the pinned tools with `mise install` (it pins `cargo:cargo-nextest`).\n\
         --- last {TAIL_LINES} lines ---\n{}",
        join(args),
        cwd.display(),
        tail(log)
    ))
}

/// The `*__worktrees` entries directly in `dir`: a worktree fixture rooted
/// at its own `TempDir` leaves its sibling worktree directory here, where
/// `TempDir::drop` never reaches it.
pub fn worktree_leaks(dir: &Path) -> Result<Vec<PathBuf>, Failure> {
    let mut leaks: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(infra)?
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with("__worktrees"))
        .map(|e| e.path())
        .collect();
    leaks.sort();
    Ok(leaks)
}

/// A remedy line naming the leaked directories.
pub fn describe_leaks(leaks: &[PathBuf]) -> String {
    let shown: Vec<String> = leaks
        .iter()
        .take(5)
        .map(|p| format!("  {}", p.display()))
        .collect();
    format!(
        "{} leaked `*__worktrees` director(ies) in the nested run's TMPDIR:\n{}\n\
         A worktree fixture is rooted at its TempDir instead of a subdirectory of it, \
         so rdm_git::worktree::add's sibling worktree escapes TempDir::drop. Root the \
         repo one level down: `let root = dir.path().join(\"repo\");`",
        leaks.len(),
        shown.join("\n")
    )
}

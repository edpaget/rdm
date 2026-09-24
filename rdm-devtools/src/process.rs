//! Bounded, cleanup-guaranteeing child process runner.
//!
//! [`run_bounded`] runs one program under a hard wall-clock limit and an
//! output cap, in its own process group, and guarantees the following order on
//! every path it can observe (success, non-zero exit, spawn failure, prepare
//! failure, timeout, output overflow, SIGINT, SIGTERM):
//!
//! 1. SIGINT/SIGTERM interception is installed for the lifetime of the run.
//! 2. The caller's `prepare` hook runs (for example writing a private copy of
//!    a login file). A signal arriving during `prepare` stops the run before
//!    anything is spawned.
//! 3. The program is spawned in a fresh process group, stdin closed, stdout
//!    captured up to a cap and stderr drained and discarded.
//! 4. On any non-success path the whole process group is sent `SIGKILL`.
//! 5. The direct child is always reaped with `wait`.
//! 6. The caller's `cleanup` hook runs exactly once, whatever happened — a
//!    cleanup failure supersedes the run's own result.
//! 7. The signal interception is removed.
//!
//! `SIGKILL` of the runner itself, or a machine failure, cannot run cleanup;
//! that limitation is inherent and documented for callers.
//!
//! Environment changes are applied only to the child [`Command`]; this module
//! never mutates the calling process's environment.
//!
//! Signal interception uses `signal-hook`: once a run ends its actions are
//! unregistered, but the process-wide handler it installed stays in place, so
//! a later SIGINT/SIGTERM in the same process no longer terminates it by
//! default. The `rdm-smoke` entrypoint exits right after its single run, so
//! this does not affect it; long-lived library callers should be aware of it.

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io::{self, Read};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use rustix::process::{Pid, Signal, kill_process_group};
use signal_hook::SigId;
use signal_hook::consts::{SIGINT, SIGTERM};

/// Default wall-clock limit for a run: two minutes.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);

/// Default cap on captured stdout: 8 MiB. Exceeding it terminates the run.
pub const DEFAULT_STDOUT_CAP: usize = 8 * 1024 * 1024;

/// How long to wait, after the direct child is reaped, for its stdout/stderr
/// pipes to reach end-of-file. A descendant that escaped the process group
/// can hold a pipe open indefinitely; the runner must not hang on it.
const READER_GRACE: Duration = Duration::from_secs(2);

/// Interval between liveness polls of the child.
const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Failure-path teardown policy.
///
/// Only [`Teardown::Correct`] is a real behaviour. The other variants are
/// deliberately broken implementations of the teardown step, kept so tests can
/// prove that their lifecycle checks detect a leaked process or a skipped
/// cleanup through the real spawn/wait path. They are not part of the
/// supported API.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Teardown {
    /// Kill the whole process group and always run cleanup.
    #[default]
    Correct,
    /// Broken: kill only the direct child, leaking the rest of its group.
    SkipGroupKill,
    /// Broken: skip the cleanup hook when the run fails.
    SkipCleanupOnError,
}

/// How the child's environment is derived from the runner's environment.
#[derive(Debug, Clone, Default)]
enum EnvBase {
    /// Start from the runner's environment.
    #[default]
    Inherit,
    /// Start from an empty environment.
    Clear,
}

/// Description of one bounded run.
///
/// Built with [`ProcessSpec::new`] and the builder methods; every setting
/// applies only to the spawned child.
#[derive(Debug, Clone)]
pub struct ProcessSpec {
    program: OsString,
    args: Vec<OsString>,
    cwd: Option<PathBuf>,
    env_base: EnvBase,
    env_set: Vec<(OsString, OsString)>,
    env_remove: Vec<OsString>,
    timeout: Duration,
    stdout_cap: usize,
    teardown: Teardown,
}

impl ProcessSpec {
    /// Starts a spec for `program`, inheriting the runner's environment, with
    /// [`DEFAULT_TIMEOUT`] and [`DEFAULT_STDOUT_CAP`].
    pub fn new(program: impl AsRef<OsStr>) -> Self {
        Self {
            program: program.as_ref().to_owned(),
            args: Vec::new(),
            cwd: None,
            env_base: EnvBase::Inherit,
            env_set: Vec::new(),
            env_remove: Vec::new(),
            timeout: DEFAULT_TIMEOUT,
            stdout_cap: DEFAULT_STDOUT_CAP,
            teardown: Teardown::Correct,
        }
    }

    /// Appends one argument.
    pub fn arg(mut self, arg: impl AsRef<OsStr>) -> Self {
        self.args.push(arg.as_ref().to_owned());
        self
    }

    /// Appends several arguments.
    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.args
            .extend(args.into_iter().map(|a| a.as_ref().to_owned()));
        self
    }

    /// Sets the child's working directory.
    pub fn cwd(mut self, dir: impl AsRef<Path>) -> Self {
        self.cwd = Some(dir.as_ref().to_owned());
        self
    }

    /// Sets one environment variable for the child only.
    pub fn env(mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> Self {
        self.env_set
            .push((key.as_ref().to_owned(), value.as_ref().to_owned()));
        self
    }

    /// Removes one inherited environment variable from the child only.
    pub fn env_remove(mut self, key: impl AsRef<OsStr>) -> Self {
        self.env_remove.push(key.as_ref().to_owned());
        self
    }

    /// Starts the child from an empty environment; only variables set with
    /// [`ProcessSpec::env`] reach it.
    pub fn env_clear(mut self) -> Self {
        self.env_base = EnvBase::Clear;
        self
    }

    /// Sets the wall-clock limit for the run.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Sets the stdout cap in bytes. Output beyond it terminates the run.
    pub fn stdout_cap(mut self, bytes: usize) -> Self {
        self.stdout_cap = bytes;
        self
    }

    /// Selects a teardown policy. Test seam only; see [`Teardown`].
    #[doc(hidden)]
    pub fn teardown(mut self, teardown: Teardown) -> Self {
        self.teardown = teardown;
        self
    }

    fn command(&self) -> Command {
        let mut cmd = Command::new(&self.program);
        cmd.args(&self.args);
        if let Some(dir) = &self.cwd {
            cmd.current_dir(dir);
        }
        if matches!(self.env_base, EnvBase::Clear) {
            cmd.env_clear();
        }
        for key in &self.env_remove {
            cmd.env_remove(key);
        }
        for (key, value) in &self.env_set {
            cmd.env(key, value);
        }
        cmd.process_group(0)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd
    }
}

type PrepareHook<'a> = Box<dyn FnOnce() -> io::Result<()> + 'a>;
type SpawnHook<'a> = Box<dyn FnOnce(u32) + 'a>;

/// Caller hooks around a run. Every hook is optional.
#[derive(Default)]
pub struct Hooks<'a> {
    prepare: Option<PrepareHook<'a>>,
    cleanup: Option<PrepareHook<'a>>,
    on_spawn: Option<SpawnHook<'a>>,
}

impl<'a> Hooks<'a> {
    /// Hooks that do nothing.
    pub fn new() -> Self {
        Self::default()
    }

    /// Runs `f` after signal interception is installed and before spawning.
    /// A failure aborts the run (cleanup still runs).
    pub fn prepare(mut self, f: impl FnOnce() -> io::Result<()> + 'a) -> Self {
        self.prepare = Some(Box::new(f));
        self
    }

    /// Runs `f` exactly once after the child has been reaped, on every path.
    pub fn cleanup(mut self, f: impl FnOnce() -> io::Result<()> + 'a) -> Self {
        self.cleanup = Some(Box::new(f));
        self
    }

    /// Runs `f` with the child's pid right after a successful spawn (a
    /// readiness signal for callers and tests).
    pub fn on_spawn(mut self, f: impl FnOnce(u32) + 'a) -> Self {
        self.on_spawn = Some(Box::new(f));
        self
    }
}

/// Result of a successful run.
#[derive(Debug)]
pub struct RunOutput {
    /// Captured stdout (never more than the spec's cap).
    pub stdout: Vec<u8>,
    /// The child's exit status (always success).
    pub status: ExitStatus,
}

/// Why a bounded run failed.
///
/// Messages never include the child's output or environment, which may hold
/// secrets.
#[derive(Debug)]
pub enum RunError {
    /// Installing SIGINT/SIGTERM interception failed.
    Signals(io::Error),
    /// The `prepare` hook failed; nothing was spawned.
    Prepare(io::Error),
    /// The program could not be spawned.
    Spawn(io::Error),
    /// Polling or reaping the child failed.
    Wait(io::Error),
    /// The child exited unsuccessfully (`code` is `None` if it was killed by
    /// `signal`).
    NonZeroExit {
        /// Exit code, when the child exited normally.
        code: Option<i32>,
        /// Terminating signal, when the child was killed by one.
        signal: Option<i32>,
    },
    /// The run exceeded its wall-clock limit and was killed.
    TimedOut(Duration),
    /// The runner received the given signal (SIGINT or SIGTERM) and killed
    /// the run.
    Interrupted(i32),
    /// The child wrote more than the given number of bytes to stdout and was
    /// killed.
    OutputCap(usize),
    /// The `cleanup` hook failed. This supersedes the run's own outcome.
    Cleanup(io::Error),
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Signals(e) => write!(f, "could not install SIGINT/SIGTERM handling: {e}"),
            Self::Prepare(e) => write!(f, "preparing the run failed before spawning: {e}"),
            Self::Spawn(e) => write!(
                f,
                "could not start the program: {e}; check the path and that it is executable"
            ),
            Self::Wait(e) => write!(f, "waiting for the child failed: {e}"),
            Self::NonZeroExit {
                code: Some(code), ..
            } => write!(f, "the program exited with code {code}"),
            Self::NonZeroExit {
                signal: Some(sig), ..
            } => write!(f, "the program was killed by signal {sig}"),
            Self::NonZeroExit { .. } => write!(f, "the program exited unsuccessfully"),
            Self::TimedOut(d) => write!(
                f,
                "the program did not finish within {}s and was killed; raise the timeout if it is legitimately slow",
                d.as_secs_f64()
            ),
            Self::Interrupted(sig) => {
                write!(f, "interrupted by signal {sig}; the program was killed")
            }
            Self::OutputCap(cap) => write!(
                f,
                "the program wrote more than {cap} bytes to stdout and was killed"
            ),
            Self::Cleanup(e) => write!(
                f,
                "cleanup failed: {e}; remove any private copy the run created by hand"
            ),
        }
    }
}

impl std::error::Error for RunError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Signals(e) | Self::Prepare(e) | Self::Spawn(e) | Self::Wait(e) => Some(e),
            Self::Cleanup(e) => Some(e),
            _ => None,
        }
    }
}

/// Unregisters the run's signal actions when dropped.
struct SignalGuard(Vec<SigId>);

impl SignalGuard {
    fn install(flag: &Arc<AtomicUsize>) -> io::Result<Self> {
        let mut ids = Vec::new();
        for sig in [SIGINT, SIGTERM] {
            // A negative signal number never occurs for these constants.
            let value = usize::try_from(sig).unwrap_or(0);
            match signal_hook::flag::register_usize(sig, Arc::clone(flag), value) {
                Ok(id) => ids.push(id),
                Err(e) => {
                    for id in ids {
                        signal_hook::low_level::unregister(id);
                    }
                    return Err(e);
                }
            }
        }
        Ok(Self(ids))
    }
}

impl Drop for SignalGuard {
    fn drop(&mut self) {
        for id in self.0.drain(..) {
            signal_hook::low_level::unregister(id);
        }
    }
}

fn interrupted(flag: &AtomicUsize) -> Option<i32> {
    match flag.load(Ordering::SeqCst) {
        0 => None,
        sig => i32::try_from(sig).ok(),
    }
}

/// Runs `spec` with `hooks`, bounded by the spec's timeout and stdout cap.
///
/// See the [module documentation](self) for the ordering guarantees.
///
/// # Errors
///
/// - [`RunError::Signals`] if SIGINT/SIGTERM interception cannot be installed
///   (no hook runs).
/// - [`RunError::Prepare`] if the `prepare` hook fails.
/// - [`RunError::Interrupted`] if SIGINT or SIGTERM arrives during the run.
/// - [`RunError::Spawn`] if the program cannot be started.
/// - [`RunError::Wait`] if polling or reaping the child fails.
/// - [`RunError::TimedOut`] if the timeout elapses.
/// - [`RunError::OutputCap`] if stdout exceeds the cap.
/// - [`RunError::NonZeroExit`] if the child exits unsuccessfully.
/// - [`RunError::Cleanup`] if the `cleanup` hook fails, replacing any other
///   outcome.
pub fn run_bounded(spec: &ProcessSpec, hooks: Hooks<'_>) -> Result<RunOutput, RunError> {
    let flag = Arc::new(AtomicUsize::new(0));
    let guard = SignalGuard::install(&flag).map_err(RunError::Signals)?;
    let Hooks {
        prepare,
        cleanup,
        on_spawn,
    } = hooks;

    let result = run_inner(spec, prepare, on_spawn, &flag);

    let skip_cleanup = result.is_err() && spec.teardown == Teardown::SkipCleanupOnError;
    let result = match cleanup {
        Some(cleanup) if !skip_cleanup => match cleanup() {
            Ok(()) => result,
            Err(e) => Err(RunError::Cleanup(e)),
        },
        _ => result,
    };
    drop(guard);
    result
}

fn run_inner(
    spec: &ProcessSpec,
    prepare: Option<PrepareHook<'_>>,
    on_spawn: Option<SpawnHook<'_>>,
    flag: &AtomicUsize,
) -> Result<RunOutput, RunError> {
    if let Some(prepare) = prepare {
        prepare().map_err(RunError::Prepare)?;
    }
    if let Some(sig) = interrupted(flag) {
        return Err(RunError::Interrupted(sig));
    }

    let mut child = spec.command().spawn().map_err(RunError::Spawn)?;
    let pid = child.id();

    let over_cap = Arc::new(AtomicBool::new(false));
    let stdout_buf = Arc::new(Mutex::new(Vec::new()));
    let (done_tx, done_rx) = mpsc::channel::<()>();
    let mut readers = 0;
    if let Some(out) = child.stdout.take() {
        readers += 1;
        let tx = done_tx.clone();
        let buf = Arc::clone(&stdout_buf);
        let over = Arc::clone(&over_cap);
        let cap = spec.stdout_cap;
        thread::spawn(move || {
            read_capped(out, &buf, &over, cap);
            let _ = tx.send(());
        });
    }
    if let Some(mut err) = child.stderr.take() {
        readers += 1;
        let tx = done_tx.clone();
        thread::spawn(move || {
            let _ = io::copy(&mut err, &mut io::sink());
            let _ = tx.send(());
        });
    }
    drop(done_tx);

    if let Some(on_spawn) = on_spawn {
        on_spawn(pid);
    }

    let deadline = Instant::now() + spec.timeout;
    let outcome = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => {}
            Err(e) => break Err(RunError::Wait(e)),
        }
        if over_cap.load(Ordering::SeqCst) {
            break Err(RunError::OutputCap(spec.stdout_cap));
        }
        if let Some(sig) = interrupted(flag) {
            break Err(RunError::Interrupted(sig));
        }
        if Instant::now() >= deadline {
            break Err(RunError::TimedOut(spec.timeout));
        }
        thread::sleep(POLL_INTERVAL);
    };

    let outcome = match outcome {
        Ok(status) => Ok(status),
        Err(e) => {
            // The child is still unreaped here, so its pid names its own
            // process group and cannot have been recycled.
            let kill_err = terminate(&mut child, pid, spec.teardown);
            let reap_err = child.wait().err();
            match (kill_err, reap_err) {
                (Some(k), _) => Err(RunError::Wait(k)),
                (None, Some(r)) if !matches!(e, RunError::Wait(_)) => Err(RunError::Wait(r)),
                _ => Err(e),
            }
        }
    };

    // Bounded wait for the pipes to drain; a descendant that escaped the
    // group must not hang the runner.
    let grace_end = Instant::now() + READER_GRACE;
    for _ in 0..readers {
        let left = grace_end.saturating_duration_since(Instant::now());
        if done_rx.recv_timeout(left).is_err() {
            break;
        }
    }

    let status = outcome?;
    if over_cap.load(Ordering::SeqCst) {
        return Err(RunError::OutputCap(spec.stdout_cap));
    }
    if !status.success() {
        return Err(RunError::NonZeroExit {
            code: status.code(),
            signal: status.signal(),
        });
    }
    let stdout = match stdout_buf.lock() {
        Ok(mut buf) => std::mem::take(&mut *buf),
        Err(poisoned) => std::mem::take(&mut *poisoned.into_inner()),
    };
    Ok(RunOutput { stdout, status })
}

/// Kills the child's process group (or, under the broken
/// [`Teardown::SkipGroupKill`] policy, only the child). Returns an error only
/// for failures other than "no such process".
fn terminate(child: &mut Child, pid: u32, teardown: Teardown) -> Option<io::Error> {
    if teardown == Teardown::SkipGroupKill {
        return child.kill().err();
    }
    let Some(pgid) = i32::try_from(pid).ok().and_then(Pid::from_raw) else {
        return child.kill().err();
    };
    match kill_process_group(pgid, Signal::KILL) {
        Ok(()) => None,
        Err(e) if e == rustix::io::Errno::SRCH => None,
        Err(e) => Some(e.into()),
    }
}

fn read_capped(mut out: impl Read, buf: &Mutex<Vec<u8>>, over: &AtomicBool, cap: usize) {
    let mut chunk = [0u8; 64 * 1024];
    let mut total = 0usize;
    loop {
        match out.read(&mut chunk) {
            Ok(0) => return,
            Ok(n) => {
                total = total.saturating_add(n);
                if total > cap {
                    over.store(true, Ordering::SeqCst);
                    // Keep draining (discarding) so the writer never blocks
                    // before it is killed.
                    continue;
                }
                if let Ok(mut b) = buf.lock() {
                    b.extend_from_slice(&chunk[..n]);
                }
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => return,
        }
    }
}

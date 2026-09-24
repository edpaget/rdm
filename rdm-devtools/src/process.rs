//! Bounded, cleanup-guaranteeing child process runner.
//!
//! Two entry points share one spawn/signal/teardown path: [`run_bounded`] for
//! a single run whose stdout is collected, and [`Session`] for a line-framed
//! conversation over the child's stdin/stdout (see its own docs). The
//! guarantees below are stated for [`run_bounded`]; [`Session`] keeps the
//! same group-kill-then-reap order on every exit path.
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
//! 4. Once the child has been spawned, its whole process group is sent
//!    `SIGKILL` on every path — success and non-zero exit included — so no
//!    descendant left in the group outlives the run. The child's exit is only
//!    *observed* (`waitid` with `WNOWAIT`), not reaped, before this sweep, so
//!    its pid still names its own group and cannot have been recycled.
//! 5. The direct child is then always reaped with `wait`.
//! 6. The caller's `cleanup` hook runs exactly once, whatever happened — a
//!    cleanup failure supersedes the run's own result.
//! 7. The signal interception is removed.
//!
//! A panic in a caller hook is a catchable path too: a panic in `on_spawn`
//! still kills the group and reaps the child, and a panic in `prepare` or
//! `on_spawn` still runs `cleanup`, before the panic resumes unwinding.
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
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use rustix::process::{Pid, Signal, WaitId, WaitIdOptions, kill_process_group, waitid};
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

/// Teardown policy applied once the child has been spawned.
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
    output_file: Option<PathBuf>,
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
            output_file: None,
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

    /// Sends the child's stdout and stderr, interleaved, to the file at
    /// `path` (created or truncated) instead of capturing them, so the output
    /// survives a non-zero exit, a timeout or an interruption for the caller
    /// to read. [`RunOutput::stdout`] is then empty and the stdout cap does
    /// not apply. [`run_bounded`] only; a [`Session`] ignores it.
    ///
    /// The file is opened just before spawning; if it cannot be created (for
    /// example its directory does not exist) the run fails with
    /// [`RunError::OutputFile`] and nothing is spawned.
    pub fn output_file(mut self, path: impl AsRef<Path>) -> Self {
        self.output_file = Some(path.as_ref().to_owned());
        self
    }

    /// Selects a teardown policy. Test seam only; see [`Teardown`].
    #[doc(hidden)]
    pub fn teardown(mut self, teardown: Teardown) -> Self {
        self.teardown = teardown;
        self
    }

    /// The child [`Command`]: a fresh process group, the given stdin, and
    /// piped stdout/stderr. Shared by [`run_bounded`] and [`Session`].
    fn command(&self, stdin: Stdio) -> Command {
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
            .stdin(stdin)
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
    /// The [`ProcessSpec::output_file`] log at the given path could not be
    /// created or duplicated for the child's stderr; nothing was spawned.
    OutputFile(PathBuf, io::Error),
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
            Self::OutputFile(path, e) => write!(
                f,
                "could not open the output log {}: {e}; check that its directory exists and is writable",
                path.display()
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
            Self::OutputFile(_, e) | Self::Cleanup(e) => Some(e),
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
/// - [`RunError::OutputFile`] if the [`ProcessSpec::output_file`] log cannot
///   be created (nothing is spawned).
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

    // A panic in `prepare` or `on_spawn` must still run cleanup (the child,
    // if any, has already been torn down inside `run_inner`); the panic then
    // resumes once cleanup and signal unregistration are done.
    let result = match panic::catch_unwind(AssertUnwindSafe(|| {
        run_inner(spec, prepare, on_spawn, &flag)
    })) {
        Ok(result) => result,
        Err(payload) => {
            if let Some(cleanup) = cleanup {
                let _ = cleanup();
            }
            drop(guard);
            panic::resume_unwind(payload);
        }
    };

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

    let mut cmd = spec.command(Stdio::null());
    if let Some(path) = &spec.output_file {
        let output_error = |e| RunError::OutputFile(path.clone(), e);
        let file = std::fs::File::create(path).map_err(output_error)?;
        let err = file.try_clone().map_err(output_error)?;
        cmd.stdout(file).stderr(err);
    }
    let mut child = cmd.spawn().map_err(RunError::Spawn)?;
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

    if let Some(on_spawn) = on_spawn
        && let Err(payload) = panic::catch_unwind(AssertUnwindSafe(|| on_spawn(pid)))
    {
        // The hook panicked: tear the child down before unwinding further.
        // Errors are ignored because the panic is the outcome being reported.
        let _ = terminate(&mut child, pid, true, spec.teardown);
        let _ = child.wait();
        panic::resume_unwind(payload);
    }

    let deadline = Instant::now() + spec.timeout;
    let outcome = loop {
        match exited_unreaped(pid) {
            Ok(true) => break Ok(()),
            Ok(false) => {}
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

    // Sweep the group on every path, success included. The child is still
    // unreaped here (its exit, if any, was only observed with WNOWAIT), so
    // its pid names its own process group and cannot have been recycled.
    let kill_err = terminate(&mut child, pid, outcome.is_ok(), spec.teardown);
    let reaped = child.wait();
    let outcome = match (outcome, kill_err, reaped) {
        (_, Some(k), _) => Err(RunError::Wait(k)),
        (Err(e @ RunError::Wait(_)), None, _) => Err(e),
        (_, None, Err(r)) => Err(RunError::Wait(r)),
        (Err(e), None, Ok(_)) => Err(e),
        (Ok(()), None, Ok(status)) => Ok(status),
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

/// Whether the child `pid` has exited, observed without reaping it
/// (`waitid(P_PID, pid, WEXITED | WNOWAIT | WNOHANG)`): the zombie keeps its
/// pid, and therefore its process-group id, reserved until [`Child::wait`].
fn exited_unreaped(pid: u32) -> io::Result<bool> {
    let Some(pid) = i32::try_from(pid).ok().and_then(Pid::from_raw) else {
        return Err(io::Error::other("child pid out of range"));
    };
    let options = WaitIdOptions::EXITED | WaitIdOptions::NOWAIT | WaitIdOptions::NOHANG;
    loop {
        match waitid(WaitId::Pid(pid), options) {
            Ok(status) => return Ok(status.is_some()),
            Err(e) if e == rustix::io::Errno::INTR => continue,
            Err(e) => return Err(e.into()),
        }
    }
}

/// Kills the child's process group (or, under the broken
/// [`Teardown::SkipGroupKill`] policy, only the child). Returns an error only
/// for failures other than "no such process".
///
/// `child_exited` says the direct child has already exited (it is a zombie
/// awaiting [`Child::wait`]). Darwin reports `EPERM`, not `ESRCH`, for a
/// group whose only remaining member is that zombie, so in that case `EPERM`
/// means "nothing left to kill" and is not an error.
fn terminate(
    child: &mut Child,
    pid: u32,
    child_exited: bool,
    teardown: Teardown,
) -> Option<io::Error> {
    if teardown == Teardown::SkipGroupKill {
        return child.kill().err();
    }
    let Some(pgid) = i32::try_from(pid).ok().and_then(Pid::from_raw) else {
        return child.kill().err();
    };
    match kill_process_group(pgid, Signal::KILL) {
        Ok(()) => None,
        Err(e) if e == rustix::io::Errno::SRCH => None,
        Err(e) if child_exited && e == rustix::io::Errno::PERM => None,
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

// --- Interactive sessions -----------------------------------------------------

/// Default cap on one line read from a [`Session`]'s stdout: 16 MiB.
pub const DEFAULT_LINE_CAP: usize = 16 * 1024 * 1024;

/// How many trailing bytes of a [`Session`] child's stderr are kept for
/// diagnostics.
pub const STDERR_TAIL_BYTES: usize = 8 * 1024;

/// How long [`Session::shutdown`] waits for the child to exit on its own after
/// its stdin is closed, before the group sweep kills it.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(2);

/// Upper bound on one blocking wait inside [`Session::recv_line`], so signal
/// interception and the deadline are re-checked promptly.
const RECV_SLICE: Duration = Duration::from_millis(25);

/// One event from a session's stdout reader thread.
enum LineEvent {
    Line(String),
    TooLong,
    Eof,
    Failed(io::Error),
}

/// Why an interactive [`Session`] failed.
///
/// Every variant except [`SessionError::Signals`] and [`SessionError::Spawn`]
/// is produced *after* the session has already torn its child down: the
/// process group is killed, the child reaped, and `stderr` holds the last
/// [`STDERR_TAIL_BYTES`] bytes the child wrote to stderr.
#[derive(Debug)]
pub enum SessionError {
    /// Installing SIGINT/SIGTERM interception failed; nothing was spawned.
    Signals(io::Error),
    /// The program could not be spawned.
    Spawn {
        /// The program that failed to start.
        program: OsString,
        /// The underlying spawn error.
        source: io::Error,
    },
    /// The session was already torn down by an earlier failure or shutdown.
    Closed,
    /// The child closed its stdout (usually: it exited) before sending the
    /// line the caller was waiting for.
    Exited {
        /// The tail of the child's stderr.
        stderr: String,
    },
    /// The session's deadline passed while waiting for a line.
    TimedOut {
        /// The session's overall time limit.
        timeout: Duration,
        /// The tail of the child's stderr.
        stderr: String,
    },
    /// SIGINT or SIGTERM arrived while the session was running.
    Interrupted(i32),
    /// The child wrote a single line longer than the session's line cap.
    LineTooLong {
        /// The cap in bytes.
        cap: usize,
        /// The tail of the child's stderr.
        stderr: String,
    },
    /// Reading the child's stdout, polling, killing or reaping it failed.
    Io {
        /// The tail of the child's stderr.
        stderr: String,
        /// The underlying error.
        source: io::Error,
    },
    /// After [`Session::shutdown`] the child exited unsuccessfully.
    Status {
        /// Exit code, when the child exited normally.
        code: Option<i32>,
        /// Terminating signal, when it was killed by one.
        signal: Option<i32>,
        /// The tail of the child's stderr.
        stderr: String,
    },
}

impl SessionError {
    /// The tail of the child's stderr carried by this error, if any.
    pub fn stderr(&self) -> Option<&str> {
        match self {
            Self::Exited { stderr }
            | Self::TimedOut { stderr, .. }
            | Self::LineTooLong { stderr, .. }
            | Self::Io { stderr, .. }
            | Self::Status { stderr, .. } => Some(stderr),
            _ => None,
        }
    }
}

fn stderr_suffix(stderr: &str) -> String {
    let trimmed = stderr.trim_end();
    if trimmed.is_empty() {
        "; the child wrote nothing to stderr".to_owned()
    } else {
        format!("; child stderr (tail):\n{trimmed}")
    }
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Signals(e) => write!(f, "could not install SIGINT/SIGTERM handling: {e}"),
            Self::Spawn { program, source } => write!(
                f,
                "could not start {}: {source}; check the path and that it is executable",
                Path::new(program).display()
            ),
            Self::Closed => write!(f, "the session was already shut down"),
            Self::Exited { stderr } => write!(
                f,
                "the child closed its output before replying (it probably exited){}",
                stderr_suffix(stderr)
            ),
            Self::TimedOut { timeout, stderr } => write!(
                f,
                "no reply within the session's {}s limit; the child was killed{}",
                timeout.as_secs_f64(),
                stderr_suffix(stderr)
            ),
            Self::Interrupted(sig) => {
                write!(f, "interrupted by signal {sig}; the child was killed")
            }
            Self::LineTooLong { cap, stderr } => write!(
                f,
                "the child wrote a line longer than {cap} bytes and was killed{}",
                stderr_suffix(stderr)
            ),
            Self::Io { stderr, source } => write!(
                f,
                "talking to the child failed: {source}{}",
                stderr_suffix(stderr)
            ),
            Self::Status {
                code: Some(code),
                stderr,
                ..
            } => write!(
                f,
                "the child exited with code {code}{}",
                stderr_suffix(stderr)
            ),
            Self::Status {
                signal: Some(sig),
                stderr,
                ..
            } => write!(
                f,
                "the child was killed by signal {sig}{}",
                stderr_suffix(stderr)
            ),
            Self::Status { stderr, .. } => write!(
                f,
                "the child exited unsuccessfully{}",
                stderr_suffix(stderr)
            ),
        }
    }
}

impl std::error::Error for SessionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Signals(e) | Self::Spawn { source: e, .. } | Self::Io { source: e, .. } => {
                Some(e)
            }
            _ => None,
        }
    }
}

/// A bounded, line-framed conversation with one child process.
///
/// The interactive sibling of [`run_bounded`], built from the same pieces: the
/// child is spawned by the same [`ProcessSpec`] in a fresh process group, the
/// same SIGINT/SIGTERM interception is installed for the session's lifetime,
/// and teardown is the same group `SIGKILL` followed by a reap — on every
/// exit path, including [`Drop`] and every error this type returns. In
/// addition to [`run_bounded`]'s contract:
///
/// - stdin is piped; [`Session::send_line`] queues a line to a writer thread,
///   so a child that stops reading can never block the caller;
/// - stdout is split into lines, each capped at the session's line cap
///   ([`DEFAULT_LINE_CAP`] unless set with [`Session::spawn_with_line_cap`]);
/// - the last [`STDERR_TAIL_BYTES`] of stderr are retained and attached to
///   every error, so a failure names what the child said;
/// - the spec's timeout is one overall deadline for the whole session,
///   enforced on every [`Session::recv_line`].
///
/// A session never mutates the calling process's environment.
pub struct Session {
    child: Child,
    pid: u32,
    stdin: Option<mpsc::Sender<Vec<u8>>>,
    lines: mpsc::Receiver<LineEvent>,
    stderr_tail: Arc<Mutex<Vec<u8>>>,
    stderr_done: mpsc::Receiver<()>,
    deadline: Instant,
    timeout: Duration,
    line_cap: usize,
    flag: Arc<AtomicUsize>,
    guard: Option<SignalGuard>,
    torn_down: bool,
    teardown: Teardown,
}

impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session")
            .field("pid", &self.pid)
            .field("torn_down", &self.torn_down)
            .finish_non_exhaustive()
    }
}

impl Session {
    /// Spawns `spec` as an interactive session with [`DEFAULT_LINE_CAP`].
    ///
    /// The spec's timeout becomes the whole session's deadline; its stdout cap
    /// is not used (lines are capped individually instead).
    ///
    /// # Errors
    ///
    /// - [`SessionError::Signals`] if signal interception cannot be installed.
    /// - [`SessionError::Spawn`] if the program cannot be started.
    pub fn spawn(spec: &ProcessSpec) -> Result<Self, SessionError> {
        Self::spawn_with_line_cap(spec, DEFAULT_LINE_CAP)
    }

    /// Like [`Session::spawn`], with an explicit per-line cap in bytes.
    ///
    /// # Errors
    ///
    /// As for [`Session::spawn`].
    pub fn spawn_with_line_cap(spec: &ProcessSpec, line_cap: usize) -> Result<Self, SessionError> {
        let flag = Arc::new(AtomicUsize::new(0));
        let guard = SignalGuard::install(&flag).map_err(SessionError::Signals)?;
        let mut child =
            spec.command(Stdio::piped())
                .spawn()
                .map_err(|source| SessionError::Spawn {
                    program: spec.program.clone(),
                    source,
                })?;
        let pid = child.id();

        let (line_tx, lines) = mpsc::channel();
        if let Some(out) = child.stdout.take() {
            thread::spawn(move || read_lines(out, &line_tx, line_cap));
        }
        let stderr_tail = Arc::new(Mutex::new(Vec::new()));
        let (done_tx, stderr_done) = mpsc::channel();
        if let Some(err) = child.stderr.take() {
            let tail = Arc::clone(&stderr_tail);
            thread::spawn(move || {
                read_tail(err, &tail);
                let _ = done_tx.send(());
            });
        }
        let stdin = child.stdin.take().map(|mut input| {
            let (tx, rx) = mpsc::channel::<Vec<u8>>();
            thread::spawn(move || {
                use std::io::Write;
                for chunk in rx {
                    if input
                        .write_all(&chunk)
                        .and_then(|()| input.flush())
                        .is_err()
                    {
                        return;
                    }
                }
            });
            tx
        });

        Ok(Self {
            child,
            pid,
            stdin,
            lines,
            stderr_tail,
            stderr_done,
            deadline: Instant::now() + spec.timeout,
            timeout: spec.timeout,
            line_cap,
            flag,
            guard: Some(guard),
            torn_down: false,
            teardown: spec.teardown,
        })
    }

    /// The child's pid (also its process-group id).
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Whether the session has been torn down.
    pub fn is_closed(&self) -> bool {
        self.torn_down
    }

    /// Queues `line` (a trailing newline is added) for the child's stdin.
    ///
    /// Never blocks: a writer thread owns the pipe. A child that has exited
    /// is detected by the next [`Session::recv_line`].
    ///
    /// # Errors
    ///
    /// [`SessionError::Closed`] if the session was already torn down.
    pub fn send_line(&mut self, line: &str) -> Result<(), SessionError> {
        if self.torn_down {
            return Err(SessionError::Closed);
        }
        let mut bytes = Vec::with_capacity(line.len() + 1);
        bytes.extend_from_slice(line.as_bytes());
        bytes.push(b'\n');
        if let Some(tx) = &self.stdin {
            // A send error means the writer already hit a closed pipe; the
            // child's exit surfaces on the next receive.
            let _ = tx.send(bytes);
        }
        Ok(())
    }

    /// Waits for the next stdout line (without its newline), bounded by the
    /// session deadline.
    ///
    /// # Errors
    ///
    /// Every error tears the session down first (group killed, child reaped):
    /// [`SessionError::TimedOut`], [`SessionError::Interrupted`],
    /// [`SessionError::Exited`] (stdout closed), [`SessionError::LineTooLong`],
    /// [`SessionError::Io`], or [`SessionError::Closed`] if the session was
    /// already torn down.
    pub fn recv_line(&mut self) -> Result<String, SessionError> {
        if self.torn_down {
            return Err(SessionError::Closed);
        }
        loop {
            if let Some(sig) = interrupted(&self.flag) {
                self.teardown_now();
                return Err(SessionError::Interrupted(sig));
            }
            let now = Instant::now();
            if now >= self.deadline {
                let stderr = self.teardown_now();
                return Err(SessionError::TimedOut {
                    timeout: self.timeout,
                    stderr,
                });
            }
            let wait = RECV_SLICE.min(self.deadline - now);
            match self.lines.recv_timeout(wait) {
                Ok(LineEvent::Line(line)) => return Ok(line),
                Ok(LineEvent::TooLong) => {
                    let stderr = self.teardown_now();
                    return Err(SessionError::LineTooLong {
                        cap: self.line_cap,
                        stderr,
                    });
                }
                Ok(LineEvent::Failed(source)) => {
                    let stderr = self.teardown_now();
                    return Err(SessionError::Io { stderr, source });
                }
                Ok(LineEvent::Eof) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                    let stderr = self.teardown_now();
                    return Err(SessionError::Exited { stderr });
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
    }

    /// The tail of the child's stderr captured so far.
    pub fn stderr_tail(&self) -> String {
        tail_string(&self.stderr_tail)
    }

    /// Kills the group, reaps the child and returns the stderr tail. Used
    /// when a caller above the line protocol (for example a protocol parser)
    /// decides the conversation cannot continue. Idempotent.
    pub fn abort(&mut self) -> String {
        self.teardown_now()
    }

    /// Ends the session gracefully: closes the child's stdin, gives it a
    /// short grace period to exit on its own, then sweeps its process group
    /// and reaps it exactly as every other path does.
    ///
    /// # Errors
    ///
    /// - [`SessionError::Closed`] if the session was already torn down.
    /// - [`SessionError::Status`] if the child exited unsuccessfully (or had
    ///   to be killed because it did not exit within the grace period).
    /// - [`SessionError::Io`] if polling, killing or reaping failed.
    pub fn shutdown(mut self) -> Result<ExitStatus, SessionError> {
        if self.torn_down {
            return Err(SessionError::Closed);
        }
        self.stdin = None;
        let grace_end = Instant::now() + SHUTDOWN_GRACE;
        let mut exited = false;
        while Instant::now() < grace_end {
            match exited_unreaped(self.pid) {
                Ok(true) => {
                    exited = true;
                    break;
                }
                Ok(false) => thread::sleep(POLL_INTERVAL),
                Err(_) => break,
            }
        }
        let kill_err = terminate(&mut self.child, self.pid, exited, self.teardown);
        let reaped = self.child.wait();
        self.torn_down = true;
        let stderr = self.collect_stderr();
        self.guard = None;
        if let Some(source) = kill_err {
            return Err(SessionError::Io { stderr, source });
        }
        let status = reaped.map_err(|source| SessionError::Io {
            stderr: stderr.clone(),
            source,
        })?;
        if status.success() {
            Ok(status)
        } else {
            Err(SessionError::Status {
                code: status.code(),
                signal: status.signal(),
                stderr,
            })
        }
    }

    /// Kills the process group, reaps the child and returns the stderr tail.
    /// Idempotent.
    fn teardown_now(&mut self) -> String {
        if !self.torn_down {
            self.stdin = None;
            let exited = exited_unreaped(self.pid).unwrap_or(false);
            // Errors are ignored: this runs on paths that are already
            // reporting a failure, and the reap below is what matters.
            let _ = terminate(&mut self.child, self.pid, exited, self.teardown);
            let _ = self.child.wait();
            self.torn_down = true;
            self.guard = None;
        }
        self.collect_stderr()
    }

    /// Waits (bounded) for the stderr reader to reach end-of-file, then
    /// returns the tail.
    fn collect_stderr(&self) -> String {
        let _ = self.stderr_done.recv_timeout(READER_GRACE);
        tail_string(&self.stderr_tail)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if !self.torn_down {
            self.stdin = None;
            let exited = exited_unreaped(self.pid).unwrap_or(false);
            let _ = terminate(&mut self.child, self.pid, exited, self.teardown);
            let _ = self.child.wait();
            self.torn_down = true;
        }
    }
}

fn tail_string(tail: &Mutex<Vec<u8>>) -> String {
    let bytes = match tail.lock() {
        Ok(b) => b.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    };
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Splits `out` into lines, sending each (or a cap/EOF/error event) on `tx`.
fn read_lines(out: impl Read, tx: &mpsc::Sender<LineEvent>, cap: usize) {
    use std::io::BufRead;
    let mut reader = io::BufReader::new(out);
    let mut line: Vec<u8> = Vec::new();
    loop {
        let buf = match reader.fill_buf() {
            Ok(buf) => buf,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => {
                let _ = tx.send(LineEvent::Failed(e));
                return;
            }
        };
        if buf.is_empty() {
            if !line.is_empty() {
                let _ = tx.send(LineEvent::Line(String::from_utf8_lossy(&line).into_owned()));
            }
            let _ = tx.send(LineEvent::Eof);
            return;
        }
        let (take, complete) = match buf.iter().position(|&b| b == b'\n') {
            Some(i) => (i, true),
            None => (buf.len(), false),
        };
        line.extend_from_slice(&buf[..take]);
        let consumed = if complete { take + 1 } else { take };
        reader.consume(consumed);
        if line.len() > cap {
            let _ = tx.send(LineEvent::TooLong);
            return;
        }
        if complete {
            let text = String::from_utf8_lossy(&line).into_owned();
            line.clear();
            if tx.send(LineEvent::Line(text)).is_err() {
                return;
            }
        }
    }
}

/// Drains `err`, keeping only the last [`STDERR_TAIL_BYTES`] bytes.
fn read_tail(mut err: impl Read, tail: &Mutex<Vec<u8>>) {
    let mut chunk = [0u8; 8 * 1024];
    loop {
        match err.read(&mut chunk) {
            Ok(0) => return,
            Ok(n) => {
                if let Ok(mut t) = tail.lock() {
                    t.extend_from_slice(&chunk[..n]);
                    if t.len() > STDERR_TAIL_BYTES {
                        let excess = t.len() - STDERR_TAIL_BYTES;
                        t.drain(..excess);
                    }
                }
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => return,
        }
    }
}

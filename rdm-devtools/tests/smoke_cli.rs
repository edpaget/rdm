//! End-to-end tests of the `rdm-smoke` entrypoint, including SIGINT/SIGTERM
//! delivered to the real runner process.
//!
//! The crate refuses to compile on non-unix targets, so these signal tests
//! can never be silently configured away; they run on every supported
//! platform, including the Linux CI `cargo nextest run` step.

mod common;

use std::io::{BufRead, BufReader};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::ExitStatusExt;
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use common::{ReapGuard, check_teardown, fixture, read_pid, smoke, wait_for_file};
use rustix::process::{Pid, Signal, kill_process};
use tempfile::TempDir;

/// A non-secret source file and the private-copy destination for it.
struct Copy {
    src: PathBuf,
    dest: PathBuf,
}

impl Copy {
    fn new(dir: &TempDir) -> Self {
        let src = dir.path().join("not-a-credential");
        std::fs::write(&src, "marker-bytes").unwrap();
        Self {
            src,
            dest: dir.path().join("config").join("not-a-credential.copy"),
        }
    }

    fn arg(&self) -> String {
        format!("{}:{}", self.src.display(), self.dest.display())
    }
}

/// A running `rdm-smoke` process, killed and reaped on drop.
struct Running {
    child: Option<Child>,
    ready: mpsc::Receiver<u32>,
}

impl Running {
    fn start(args: &[&str]) -> Self {
        let mut child = Command::new(smoke())
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn rdm-smoke");
        let stderr = child.stderr.take().expect("stderr piped");
        let (tx, ready) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines() {
                let Ok(line) = line else { return };
                if let Some(pid) = line.strip_prefix("rdm-smoke: ready pid=")
                    && let Ok(pid) = pid.trim().parse()
                {
                    let _ = tx.send(pid);
                }
            }
        });
        Self {
            child: Some(child),
            ready,
        }
    }

    fn pid(&self) -> u32 {
        self.child.as_ref().expect("running").id()
    }

    fn wait_ready(&self) -> u32 {
        self.ready
            .recv_timeout(Duration::from_secs(10))
            .expect("rdm-smoke never reported readiness")
    }

    fn wait(&mut self, within: Duration) -> ExitStatus {
        let child = self.child.as_mut().expect("running");
        let end = Instant::now() + within;
        loop {
            if let Some(status) = child.try_wait().expect("try_wait") {
                self.child = None;
                return status;
            }
            assert!(Instant::now() < end, "rdm-smoke did not exit in time");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn run_to_end(args: &[&str]) -> ExitStatus {
    let mut running = Running::start(args);
    running.wait(Duration::from_secs(30))
}

#[test]
fn cli_success_writes_stdout_file_and_removes_private_copy() {
    let dir = TempDir::new().unwrap();
    let copy = Copy::new(&dir);
    let out = dir.path().join("events.jsonl");
    let dest = copy.dest.to_str().unwrap().to_owned();
    let status = run_to_end(&[
        "run",
        "--private-copy",
        &copy.arg(),
        "--stdout-file",
        out.to_str().unwrap(),
        "--",
        fixture(),
        "cat",
        &dest,
    ]);
    assert_eq!(status.code(), Some(0));
    // The child read the private copy's bytes, so it existed during the run.
    assert_eq!(std::fs::read(&out).unwrap(), b"marker-bytes");
    let mode = std::fs::metadata(&out).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    assert!(copy.src.exists(), "the source must never be touched");
    check_teardown(&copy.dest, &[]).unwrap();
}

#[test]
fn cli_nonzero_exit_propagates_and_removes_private_copy() {
    let dir = TempDir::new().unwrap();
    let copy = Copy::new(&dir);
    let status = run_to_end(&[
        "run",
        "--private-copy",
        &copy.arg(),
        "--",
        fixture(),
        "exit",
        "7",
    ]);
    assert_eq!(status.code(), Some(7));
    check_teardown(&copy.dest, &[]).unwrap();
}

#[test]
fn cli_timeout_removes_private_copy() {
    let dir = TempDir::new().unwrap();
    let copy = Copy::new(&dir);
    let mut running = Running::start(&[
        "run",
        "--timeout-secs",
        "1",
        "--private-copy",
        &copy.arg(),
        "--",
        fixture(),
        "sleep",
    ]);
    let mut guard = ReapGuard::default();
    guard.track(running.wait_ready());
    let status = running.wait(Duration::from_secs(20));
    assert_eq!(status.code(), Some(124));
    check_teardown(&copy.dest, &guard.0.clone()).unwrap();
    guard.clear();
}

fn signal_scenario(signal: Signal, expected_code: i32) {
    let dir = TempDir::new().unwrap();
    let copy = Copy::new(&dir);
    let pidfile = dir.path().join("grandchild.pid");
    let mut running = Running::start(&[
        "run",
        "--timeout-secs",
        "60",
        "--private-copy",
        &copy.arg(),
        "--",
        fixture(),
        "spawn-grandchild",
        pidfile.to_str().unwrap(),
    ]);
    let mut guard = ReapGuard::default();
    guard.track(running.wait_ready());
    assert!(
        wait_for_file(&pidfile, Duration::from_secs(10)),
        "fixture never wrote the grandchild pid"
    );
    guard.track(read_pid(&pidfile).unwrap());
    assert!(copy.dest.exists(), "the private copy was not prepared");

    let smoke_pid = i32::try_from(running.pid())
        .ok()
        .and_then(Pid::from_raw)
        .unwrap();
    // Signal the runner only; its child lives in its own process group.
    kill_process(smoke_pid, signal).expect("signal rdm-smoke");
    let status = running.wait(Duration::from_secs(20));

    assert_eq!(status.signal(), None, "rdm-smoke must exit, not die");
    assert_eq!(status.code(), Some(expected_code));
    check_teardown(&copy.dest, &guard.0.clone()).unwrap();
    guard.clear();
}

#[test]
fn cli_sigint_kills_child_group_and_removes_private_copy() {
    signal_scenario(Signal::INT, 130);
}

#[test]
fn cli_sigterm_kills_child_group_and_removes_private_copy() {
    signal_scenario(Signal::TERM, 143);
}

#[test]
fn cli_rejects_malformed_private_copy() {
    let status = run_to_end(&["run", "--private-copy", "no-separator", "--", fixture()]);
    assert_eq!(status.code(), Some(2));
}

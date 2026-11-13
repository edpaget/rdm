//! `process::run_forwarding` through real fixture processes. The signal is
//! fed through `Interrupts::manual()`, so no process-wide signal is ever
//! raised in the test process; the only signals sent go to the fixture's own
//! process group, by the function under test.

mod common;

use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use common::{ReapGuard, fixture, gone_within, read_pid, wait_for_file};
use rdm_devtools::process::{Interrupts, run_forwarding};
use signal_hook::consts::{SIGKILL, SIGTERM};

/// Polls until `path` holds `ready` or `within` elapses.
fn wait_for_ready(path: &Path, within: Duration) -> bool {
    let end = Instant::now() + within;
    while Instant::now() < end {
        if std::fs::read_to_string(path).is_ok_and(|t| t.contains("ready")) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

#[test]
fn forwarded_sigterm_stops_child_and_its_group() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let pidfile = tmp.path().join("grandchild.pid");
    let interrupts = Interrupts::manual();
    let mut guard = ReapGuard::default();
    let mut cmd = Command::new(fixture());
    cmd.arg("spawn-grandchild")
        .arg(&pidfile)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let result = std::thread::scope(|s| {
        let trigger = s.spawn(|| {
            // Bounded: trigger regardless, so the run below always ends.
            let ready = wait_for_file(&pidfile, Duration::from_secs(30));
            interrupts.trigger(SIGTERM);
            ready
        });
        let result = run_forwarding(&mut cmd, &interrupts, Duration::from_secs(10));
        (result, trigger.join().expect("trigger thread"))
    });
    let (result, ready) = result;
    let grandchild = read_pid(&pidfile);
    if let Ok(pid) = grandchild {
        guard.track(pid);
    }
    assert!(ready, "the fixture never wrote its grandchild pidfile");
    let forwarded = result.expect("run_forwarding");
    assert_eq!(forwarded.interrupted, Some(SIGTERM));
    assert!(!forwarded.killed, "the forwarded SIGTERM should end it");
    assert_eq!(forwarded.status.signal(), Some(SIGTERM));
    let grandchild = grandchild.expect("grandchild pid");
    assert!(
        gone_within(grandchild, Duration::from_secs(3)),
        "grandchild {grandchild} survived"
    );
    guard.clear();
}

#[test]
fn child_ignoring_sigterm_is_killed_after_grace() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let out = tmp.path().join("stdout");
    let interrupts = Interrupts::manual();
    let grace = Duration::from_millis(300);
    let mut cmd = Command::new(fixture());
    cmd.arg("ignore-sigterm-then-sleep")
        .stdin(Stdio::null())
        .stdout(std::fs::File::create(&out).expect("stdout file"))
        .stderr(Stdio::null());
    let (result, triggered) = std::thread::scope(|s| {
        let trigger = s.spawn(|| {
            let ready = wait_for_ready(&out, Duration::from_secs(30));
            let at = Instant::now();
            interrupts.trigger(SIGTERM);
            (ready, at)
        });
        let result = run_forwarding(&mut cmd, &interrupts, grace);
        let ended = Instant::now();
        (result.map(|f| (f, ended)), trigger.join().expect("trigger"))
    });
    let (ready, at) = triggered;
    assert!(ready, "the fixture never printed ready");
    let (forwarded, ended) = result.expect("run_forwarding");
    assert_eq!(forwarded.interrupted, Some(SIGTERM));
    assert!(forwarded.killed, "a SIGTERM-ignoring child must be killed");
    assert_eq!(forwarded.status.signal(), Some(SIGKILL));
    assert!(
        ended.duration_since(at) >= grace,
        "killed before the grace elapsed"
    );
}

#[test]
fn success_path_sweeps_the_group() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let pidfile = tmp.path().join("grandchild.pid");
    let interrupts = Interrupts::manual();
    let mut guard = ReapGuard::default();
    let mut cmd = Command::new(fixture());
    cmd.arg("spawn-grandchild-then-exit")
        .arg(&pidfile)
        .arg("0")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let forwarded = run_forwarding(&mut cmd, &interrupts, Duration::from_secs(10));
    let grandchild = read_pid(&pidfile);
    if let Ok(pid) = grandchild {
        guard.track(pid);
    }
    let forwarded = forwarded.expect("run_forwarding");
    assert_eq!(forwarded.interrupted, None);
    assert!(!forwarded.killed);
    assert!(forwarded.status.success());
    let grandchild = grandchild.expect("grandchild pid");
    assert!(
        gone_within(grandchild, Duration::from_secs(3)),
        "grandchild {grandchild} survived the sweep"
    );
    guard.clear();
}

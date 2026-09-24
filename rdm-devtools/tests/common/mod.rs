//! Shared lifecycle checkers and leak guards for the rdm-devtools tests.

#![allow(dead_code)]

use std::path::Path;
use std::time::{Duration, Instant};

use rustix::process::{Pid, Signal, kill_process, test_kill_process};

/// Path of the fixture executable, resolved by Cargo.
pub fn fixture() -> &'static str {
    env!("CARGO_BIN_EXE_rdm-devtools-fixture")
}

/// Path of the `rdm-smoke` entrypoint, resolved by Cargo.
pub fn smoke() -> &'static str {
    env!("CARGO_BIN_EXE_rdm-smoke")
}

fn pid(raw: u32) -> Option<Pid> {
    i32::try_from(raw).ok().and_then(Pid::from_raw)
}

/// Whether a process with this pid still exists and is not a zombie.
pub fn alive(raw: u32) -> bool {
    pid(raw).is_some_and(|p| test_kill_process(p).is_ok()) && !zombie(raw)
}

/// On Linux, an exited-but-unreaped process (for example an orphan whose
/// reaper is slow in a container) is reported via `/proc`; it holds no
/// resources and counts as terminated.
#[cfg(target_os = "linux")]
fn zombie(raw: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{raw}/stat"))
        .ok()
        .and_then(|stat| {
            stat.rsplit_once(')')
                .and_then(|(_, rest)| rest.trim_start().chars().next())
        })
        .is_some_and(|state| state == 'Z')
}

/// Elsewhere `kill(pid, 0)` is the only portable probe; orphans are reaped
/// promptly by launchd.
#[cfg(not(target_os = "linux"))]
fn zombie(_raw: u32) -> bool {
    false
}

/// Polls until `raw` is gone or `within` elapses; returns whether it is gone.
pub fn gone_within(raw: u32, within: Duration) -> bool {
    let end = Instant::now() + within;
    while Instant::now() < end {
        if !alive(raw) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    !alive(raw)
}

/// Polls until `path` exists or `within` elapses.
pub fn wait_for_file(path: &Path, within: Duration) -> bool {
    let end = Instant::now() + within;
    while Instant::now() < end {
        if path.exists() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    path.exists()
}

/// Reads a pid written by the fixture's `spawn-grandchild` mode.
pub fn read_pid(path: &Path) -> Result<u32, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("pidfile {} unreadable: {e}", path.display()))?;
    text.trim()
        .parse()
        .map_err(|e| format!("pidfile {} malformed: {e}", path.display()))
}

/// The lifecycle contract after a run returns: the cleanup marker is gone
/// and every listed process has terminated.
pub fn check_teardown(marker: &Path, pids: &[u32]) -> Result<(), String> {
    if marker.exists() {
        return Err(format!(
            "leaked marker {}: cleanup did not run",
            marker.display()
        ));
    }
    for &p in pids {
        if !gone_within(p, Duration::from_secs(3)) {
            return Err(format!("pid {p} survived teardown"));
        }
    }
    Ok(())
}

/// Kills every tracked pid on drop, so a failing assertion never leaks a
/// process. Pids confirmed gone should be released with [`ReapGuard::clear`].
#[derive(Default)]
pub struct ReapGuard(pub Vec<u32>);

impl ReapGuard {
    /// Tracks one more pid.
    pub fn track(&mut self, raw: u32) {
        self.0.push(raw);
    }

    /// Stops tracking all pids (they were confirmed gone).
    pub fn clear(&mut self) {
        self.0.clear();
    }

    /// Kills and forgets every tracked pid now.
    pub fn kill_all(&mut self) {
        for raw in self.0.drain(..) {
            if let Some(p) = pid(raw) {
                let _ = kill_process(p, Signal::KILL);
            }
        }
    }
}

impl Drop for ReapGuard {
    fn drop(&mut self) {
        self.kill_all();
    }
}

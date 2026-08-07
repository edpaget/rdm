//! Process-ancestry lookup backing the inherited-lease rung of session
//! identity.
//!
//! [`ProcessTable`] is the injection seam: the real backends
//! ([`SystemProcessTable`]) read the OS process table, while
//! [`MapProcessTable`] lets tests construct ancestries — including
//! otherwise-unconstructible ones such as a recycled pid whose start time no
//! longer matches a recorded lease.
//!
//! No backend uses `unsafe`, and every backend degrades to an *empty* table
//! rather than an error: a missing `/proc`, an absent or sandboxed `ps`, or an
//! unparsable line all yield no ancestry, which lands session resolution on
//! the always-resolving per-process rung.

use std::collections::BTreeMap;

/// A single process's identity: its pid, its parent's pid, and an
/// OS-reported start time.
///
/// `start_time` is an opaque, OS-formatted string (Linux jiffies-since-boot,
/// macOS `lstart`). It is only ever compared byte-for-byte, never parsed or
/// ordered, so clock skew and locale differences cannot cause a false match.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcInfo {
    /// The process id.
    pub pid: u32,
    /// The parent process id.
    pub ppid: u32,
    /// An opaque, OS-reported start time, compared only for byte equality.
    pub start_time: String,
}

/// A read-only view of the OS process table.
pub trait ProcessTable {
    /// Returns the entry for `pid`, or `None` when the pid is not live (or the
    /// table could not be read at all).
    fn get(&self, pid: u32) -> Option<ProcInfo>;

    /// Returns the pid of the calling process.
    fn self_pid(&self) -> u32;

    /// Returns every live pid the table knows about.
    ///
    /// Backends that cannot enumerate return an empty vector; callers must
    /// treat "empty" as "unknown", never as "nothing is alive".
    fn pids(&self) -> Vec<u32>;
}

/// An in-memory [`ProcessTable`] built from an explicit map.
///
/// This is the hermetic test double: it makes the pid-recycle case (a live pid
/// whose start time differs from a recorded lease) constructible without
/// waiting for the OS to actually recycle a pid.
#[derive(Clone, Debug)]
pub struct MapProcessTable {
    procs: BTreeMap<u32, ProcInfo>,
    self_pid: u32,
}

impl MapProcessTable {
    /// Creates an empty table whose calling process is `self_pid`.
    ///
    /// An empty table has no ancestry at all, which is the degradation case
    /// that lands resolution on the per-process rung.
    pub fn empty(self_pid: u32) -> Self {
        Self {
            procs: BTreeMap::new(),
            self_pid,
        }
    }

    /// Adds a process entry, returning `self` for chaining.
    #[must_use]
    pub fn with(mut self, pid: u32, ppid: u32, start_time: &str) -> Self {
        self.procs.insert(
            pid,
            ProcInfo {
                pid,
                ppid,
                start_time: start_time.to_string(),
            },
        );
        self
    }

    /// Builds a straight ancestry chain `self_pid -> parents[0] -> parents[1]
    /// -> …`, giving each entry the supplied start time.
    ///
    /// The final ancestor's parent is pid 1, which terminates the ascent.
    #[must_use]
    pub fn chain(self_pid: u32, parents: &[(u32, &str)]) -> Self {
        let mut table = Self::empty(self_pid);
        let mut child = self_pid;
        let mut child_parent = if parents.is_empty() { 1 } else { parents[0].0 };
        table = table.with(child, child_parent, "self-start");
        for (i, (pid, start)) in parents.iter().enumerate() {
            child = *pid;
            child_parent = parents.get(i + 1).map_or(1, |(p, _)| *p);
            table = table.with(child, child_parent, start);
        }
        table
    }
}

impl ProcessTable for MapProcessTable {
    fn get(&self, pid: u32) -> Option<ProcInfo> {
        self.procs.get(&pid).cloned()
    }

    fn self_pid(&self) -> u32 {
        self.self_pid
    }

    fn pids(&self) -> Vec<u32> {
        self.procs.keys().copied().collect()
    }
}

/// A [`ProcessTable`] backed by the real OS process table.
///
/// The snapshot is taken once, in [`SystemProcessTable::snapshot`], and is
/// immutable thereafter — callers memoize a single instance per process so the
/// (macOS) `ps` spawn or (Linux) `/proc` scan is paid at most once per `rdm`
/// invocation.
///
/// Backends by target:
///
/// - **Linux**: reads `/proc/<pid>/stat` for every numeric entry in `/proc`.
///   Fields are parsed *after the last* `)` so a process name containing
///   spaces or parentheses cannot shift the field indices.
/// - **Other unix**: one `ps -Ao pid=,ppid=,lstart=` spawn. `lstart` is an
///   absolute start time and therefore stable, unlike the relative `etime`.
/// - **Anything else**: an empty table.
#[derive(Clone, Debug)]
pub struct SystemProcessTable {
    procs: BTreeMap<u32, ProcInfo>,
    self_pid: u32,
}

impl SystemProcessTable {
    /// Takes a one-shot snapshot of the OS process table.
    ///
    /// Never fails: an unreadable or unavailable source yields an empty table.
    pub fn snapshot() -> Self {
        Self {
            procs: snapshot_procs(),
            self_pid: std::process::id(),
        }
    }

    /// Returns the number of processes in the snapshot.
    ///
    /// Zero means the table could not be read; treat it as "unknown", not as
    /// "nothing is running".
    pub fn len(&self) -> usize {
        self.procs.len()
    }

    /// Returns whether the snapshot is empty (the table could not be read).
    pub fn is_empty(&self) -> bool {
        self.procs.is_empty()
    }
}

impl ProcessTable for SystemProcessTable {
    fn get(&self, pid: u32) -> Option<ProcInfo> {
        self.procs.get(&pid).cloned()
    }

    fn self_pid(&self) -> u32 {
        self.self_pid
    }

    fn pids(&self) -> Vec<u32> {
        self.procs.keys().copied().collect()
    }
}

#[cfg(target_os = "linux")]
fn snapshot_procs() -> BTreeMap<u32, ProcInfo> {
    let mut out = BTreeMap::new();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return out;
    };
    for entry in entries.flatten() {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        let Ok(pid) = name.parse::<u32>() else {
            continue;
        };
        let Ok(contents) = std::fs::read_to_string(entry.path().join("stat")) else {
            continue;
        };
        if let Some(info) = parse_proc_stat(pid, &contents) {
            out.insert(pid, info);
        }
    }
    out
}

#[cfg(all(unix, not(target_os = "linux")))]
fn snapshot_procs() -> BTreeMap<u32, ProcInfo> {
    let mut out = BTreeMap::new();
    let Ok(output) = std::process::Command::new("ps")
        .args(["-Ao", "pid=,ppid=,lstart="])
        .output()
    else {
        return out;
    };
    if !output.status.success() {
        return out;
    }
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        if let Some(info) = parse_ps_line(line) {
            out.insert(info.pid, info);
        }
    }
    out
}

#[cfg(not(unix))]
fn snapshot_procs() -> BTreeMap<u32, ProcInfo> {
    BTreeMap::new()
}

/// Parses one `/proc/<pid>/stat` line into a [`ProcInfo`].
///
/// The process name (field 2) is wrapped in parentheses and may itself contain
/// spaces and parentheses, so everything before the **last** `)` is discarded
/// before splitting. The remainder starts at field 3, making the parent pid
/// (field 4) index 1 and the start time (field 22) index 19.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_proc_stat(pid: u32, contents: &str) -> Option<ProcInfo> {
    let rest = &contents[contents.rfind(')')? + 1..];
    let fields: Vec<&str> = rest.split_whitespace().collect();
    // fields[0] is field 3 (state); field N lives at index N - 3.
    let ppid = fields.get(1)?.parse::<u32>().ok()?;
    let start_time = (*fields.get(19)?).to_string();
    Some(ProcInfo {
        pid,
        ppid,
        start_time,
    })
}

/// Parses one `ps -Ao pid=,ppid=,lstart=` line into a [`ProcInfo`].
///
/// `lstart` renders as an absolute date containing spaces
/// (`Mon Aug  4 12:34:56 2026`), so only the first two whitespace-delimited
/// fields are split off; the remainder is kept verbatim as the start time.
#[cfg_attr(target_os = "linux", allow(dead_code))]
fn parse_ps_line(line: &str) -> Option<ProcInfo> {
    let line = line.trim();
    let (pid_str, rest) = line.split_once(char::is_whitespace)?;
    let rest = rest.trim_start();
    let (ppid_str, start) = rest.split_once(char::is_whitespace)?;
    let start_time = start.trim().to_string();
    if start_time.is_empty() {
        return None;
    }
    Some(ProcInfo {
        pid: pid_str.parse().ok()?,
        ppid: ppid_str.parse().ok()?,
        start_time,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proc_stat_parses_simple_line() {
        let line = "42 (bash) S 7 42 42 34816 42 4194304 1 0 0 0 1 2 0 0 20 0 1 0 90210 4096 1";
        let info = parse_proc_stat(42, line).unwrap();
        assert_eq!(info.pid, 42);
        assert_eq!(info.ppid, 7);
        assert_eq!(info.start_time, "90210");
    }

    #[test]
    fn proc_stat_parses_comm_with_spaces_and_parens() {
        // A comm containing both a space and a `)` would shift every field if
        // the split were done naively; parsing after the LAST `)` is what
        // keeps ppid and starttime correct.
        let line =
            "42 (sh with ) paren) S 7 42 42 34816 42 4194304 1 0 0 0 1 2 0 0 20 0 1 0 90210 4096 1";
        let info = parse_proc_stat(42, line).unwrap();
        assert_eq!(info.ppid, 7);
        assert_eq!(info.start_time, "90210");
    }

    #[test]
    fn proc_stat_rejects_truncated_line() {
        assert!(parse_proc_stat(42, "42 (bash) S 7").is_none());
        assert!(parse_proc_stat(42, "no parens here").is_none());
    }

    #[test]
    fn ps_line_parses_lstart_with_spaces() {
        let info = parse_ps_line("  4242  1337 Mon Aug  4 12:34:56 2026").unwrap();
        assert_eq!(info.pid, 4242);
        assert_eq!(info.ppid, 1337);
        assert_eq!(info.start_time, "Mon Aug  4 12:34:56 2026");
    }

    #[test]
    fn ps_line_rejects_garbage() {
        assert!(parse_ps_line("").is_none());
        assert!(parse_ps_line("nonsense").is_none());
        assert!(parse_ps_line("1 2").is_none());
        assert!(parse_ps_line("abc def Mon Aug 4 12:34:56 2026").is_none());
    }

    #[test]
    fn map_table_chain_builds_ancestry() {
        let table = MapProcessTable::chain(100, &[(200, "s200"), (300, "s300")]);
        assert_eq!(table.self_pid(), 100);
        assert_eq!(table.get(100).unwrap().ppid, 200);
        assert_eq!(table.get(200).unwrap().ppid, 300);
        assert_eq!(table.get(300).unwrap().ppid, 1);
        assert_eq!(table.get(300).unwrap().start_time, "s300");
        assert!(table.get(999).is_none());
    }

    #[test]
    fn empty_map_table_has_no_ancestry() {
        let table = MapProcessTable::empty(5);
        assert!(table.get(5).is_none());
        assert!(table.pids().is_empty());
    }

    #[test]
    fn system_snapshot_never_panics() {
        // The result is platform-dependent (an empty table is a legal
        // outcome), so assert only that taking it is infallible.
        let table = SystemProcessTable::snapshot();
        assert_eq!(table.self_pid(), std::process::id());
        assert_eq!(table.is_empty(), table.pids().is_empty());
    }
}

//! The inherited-lease rung: a small on-disk record, keyed by an ancestor's
//! pid, that lets every `rdm` process spawned under one long-lived shell
//! resolve the *same* changeset id.
//!
//! # The shipped stopping rule
//!
//! Phase 3 fixed the failure asymmetry and left the exact rule to phase 4:
//! stopping too **high** merges two concurrent sessions into one identity
//! (which is the very bug this roadmap exists to remove), while stopping too
//! **low** merely fragments one session's batch. The rule shipped here errs
//! low, in two halves that are deliberately asymmetric:
//!
//! - **Adoption** ascends up to [`MAX_ANCESTOR_DEPTH`] ancestors looking for an
//!   *existing* valid lease, taking the first (lowest) match.
//! - **Creation** happens only at depth 1 — the immediate parent — never
//!   higher.
//!
//! Because a lease is only ever created at the immediate parent, two
//! concurrent sessions can share an identity only if some ancestor they have
//! in common was itself the immediate parent of an earlier `rdm` invocation.
//! When the parent is an ephemeral per-invocation shell, no ancestor is ever
//! leased and each invocation fragments into its own changeset — the safe
//! direction.
//!
//! # Why creation still stops at depth 1 (phase 11)
//!
//! Phase 11 evaluated raising creation to the nearest *non-shell* ancestor, so
//! that an agent harness spawning a fresh wrapper shell per tool call would
//! mint one lease at the agent process instead of one per invocation. It was
//! **rejected on measurement**, and creation is unchanged. Two findings, both
//! recorded with their evidence in `docs/session-identity.md`:
//!
//! - The guard that would have made such an ascent safe — stopping at a
//!   session boundary — needs a per-process session id. macOS exposes none to
//!   an unprivileged reader (`ps -o sess=` reports `0` for every process, and
//!   there is no `sid` keyword), so the mechanism could only ever have been
//!   Linux-only.
//! - A captured real ancestry shows the ascent merging what it must keep
//!   apart: two `rdm`-using scripts backgrounded from one shell under an agent
//!   harness share an unbroken *shell* path up to a single non-shell agent
//!   process, which a shell-crossing ascent would select as one common anchor
//!   for both. That is the "stopping too high" direction phase 3 declared
//!   unacceptable, and no finite deny-list of anchor command names can
//!   enumerate every agent, editor, or runner that may sit there.
//!
//! What ships instead is honesty plus hygiene: [`create_at_parent`] sweeps
//! stale leases before minting so the fragmenting topology cannot also leak,
//! and [`continuity_advisory`](super::continuity_advisory) makes `rdm commit`
//! name the cause and the remedy — export `RDM_HARNESS_SESSION_ID` once per
//! session — rather than fragmenting silently.
//!
//! A lease records the ancestor's start time alongside its id, so a recycled
//! pid (same number, different process) is detected and the stale lease is
//! removed rather than adopted.

use std::collections::BTreeSet;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::process::{ProcInfo, ProcessTable};
use super::{SessionId, SessionPaths, hex16};

/// The maximum number of ancestors the lease ascent will inspect.
///
/// Bounded so the walk terminates on a cycle, a truncated ancestry, or a
/// pathologically deep process tree.
pub const MAX_ANCESTOR_DEPTH: usize = 8;

/// The number of lease files a single [`gc`] pass will consider.
pub const MAX_GC_ENTRIES: usize = 64;

/// A lease record: the changeset id an ancestor process owns, plus the
/// ancestor's start time so a recycled pid is detectable.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Lease {
    /// The changeset id every descendant of this ancestor adopts.
    pub id: String,
    /// The ancestor's OS-reported start time, compared byte-for-byte.
    pub start_time: String,
    /// When the lease was created, as an RFC 3339 UTC timestamp.
    pub created_utc: String,
}

/// Returns the on-disk path of the lease owned by `pid`.
///
/// The file is keyed by pid alone; the start time lives *inside* it, so a
/// mismatch is observable rather than silently invisible.
pub fn lease_path(paths: &SessionPaths, pid: u32) -> PathBuf {
    paths.leases_dir().join(format!("{pid}.lease"))
}

/// Reads and parses the lease at `pid`, returning `None` when the file is
/// absent, unreadable, or not valid JSON.
pub fn read_lease(paths: &SessionPaths, pid: u32) -> Option<Lease> {
    let raw = std::fs::read_to_string(lease_path(paths, pid)).ok()?;
    serde_json::from_str(&raw).ok()
}

/// Walks the bounded ancestor chain, nearest ancestor first.
fn ancestors(procs: &dyn ProcessTable) -> Vec<ProcInfo> {
    let mut out = Vec::new();
    let Some(me) = procs.get(procs.self_pid()) else {
        return out;
    };
    let mut cur = me.ppid;
    let mut seen: BTreeSet<u32> = BTreeSet::new();
    while out.len() < MAX_ANCESTOR_DEPTH && cur > 1 {
        if !seen.insert(cur) {
            break; // cycle guard
        }
        let Some(info) = procs.get(cur) else {
            break;
        };
        cur = info.ppid;
        out.push(info);
    }
    out
}

/// Adopts the nearest ancestor's still-valid lease, if any.
///
/// A lease whose recorded `start_time` does not byte-match the live process
/// table's value for that pid is stale (the pid was recycled): it is removed
/// and the ascent continues.
///
/// Returns `None` when no ancestor within [`MAX_ANCESTOR_DEPTH`] holds a valid
/// lease — including when the process table is empty.
pub fn adopt_inherited(paths: &SessionPaths, procs: &dyn ProcessTable) -> Option<SessionId> {
    for info in ancestors(procs) {
        let path = lease_path(paths, info.pid);
        if !path.exists() {
            continue;
        }
        if let Some(lease) = read_lease(paths, info.pid)
            && lease.start_time == info.start_time
            && let Some(id) = SessionId::new(&lease.id)
        {
            return Some(id);
        }
        // Present but unusable — a recycled pid, a corrupt file, or an id that
        // cannot be a file name. Drop it so it can never be adopted later,
        // then keep ascending.
        let _ = std::fs::remove_file(&path);
    }
    None
}

/// Derives the id a freshly created lease receives.
fn derive_lease_id(paths: &SessionPaths, info: &ProcInfo) -> SessionId {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let digest = hex16(&[
        &paths.base().to_string_lossy(),
        &info.pid.to_string(),
        &info.start_time,
        &nanos.to_string(),
    ]);
    SessionId::new(&format!("s-{digest}")).expect("derived lease id is always well-formed")
}

/// Writes `lease` for `pid`, atomically and exclusively.
///
/// Content is written to a temp file first and linked into place, so a
/// concurrent reader can never observe a half-written lease. When the link
/// fails because another process won the race, the winner's lease is read back
/// and returned — which is what makes concurrent creation *converge* on one id
/// instead of splitting the session.
fn create_exclusive(paths: &SessionPaths, pid: u32, lease: &Lease) -> Option<Lease> {
    let dir = paths.leases_dir();
    std::fs::create_dir_all(&dir).ok()?;
    let final_path = lease_path(paths, pid);
    let tmp = dir.join(format!(".{pid}.{}.tmp", std::process::id()));
    let encoded = serde_json::to_string(lease).ok()?;
    std::fs::write(&tmp, encoded).ok()?;
    let result = match std::fs::hard_link(&tmp, &final_path) {
        Ok(()) => Some(lease.clone()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => read_lease(paths, pid),
        Err(_) => None,
    };
    let _ = std::fs::remove_file(&tmp);
    result
}

/// Creates a lease at the **immediate parent** and returns its id.
///
/// This is the "stop low" half of the stopping rule: creation never ascends.
/// Returns `None` when there is no known parent (an empty or unreadable
/// process table) or when the state directory cannot be written.
///
/// # Lease hygiene
///
/// A bounded [`gc`] pass runs immediately before minting. This is what keeps
/// the *fragmenting* topology from also being a *leaking* one: under an agent
/// harness that spawns a fresh wrapper shell per tool call (phase 11), every
/// invocation reaches this function and mints a lease at a parent that is dead
/// moments later. Sweeping first means the lease directory stays bounded at
/// roughly one entry instead of growing without limit, so the fragmentation
/// rdm reports is never compounded by unbounded state.
///
/// The sweep is safe to run here because [`gc`] already skips entirely when
/// the process table cannot see the calling process — a table that reads as
/// empty means "unknown", never "nothing is alive" — and because it only ever
/// removes a lease whose pid is gone or whose recorded start time no longer
/// matches. It never touches a journal, so a killed session's work stays
/// recoverable via [`adopt_changeset`](super::journal::adopt_changeset).
pub fn create_at_parent(paths: &SessionPaths, procs: &dyn ProcessTable) -> Option<SessionId> {
    let parent = ancestors(procs).into_iter().next()?;
    // Sweep before minting, never after: the entry written just below names a
    // live pid by construction, and GC-ing after would pointlessly re-read it.
    gc(paths, procs);
    let lease = Lease {
        id: derive_lease_id(paths, &parent).to_string(),
        start_time: parent.start_time.clone(),
        created_utc: chrono::Utc::now().to_rfc3339(),
    };
    let landed = create_exclusive(paths, parent.pid, &lease)?;
    SessionId::new(&landed.id)
}

/// Re-points the caller's immediate-parent lease at `id`.
///
/// This is how an orphaned changeset is recovered: the next `rdm` invocation
/// from the same shell adopts `id` instead of the shell's previous changeset.
///
/// Returns `false` when there is no known parent to hold the lease.
pub fn repoint_parent_lease(
    paths: &SessionPaths,
    procs: &dyn ProcessTable,
    id: &SessionId,
) -> std::io::Result<bool> {
    let Some(parent) = ancestors(procs).into_iter().next() else {
        return Ok(false);
    };
    let lease = Lease {
        id: id.to_string(),
        start_time: parent.start_time.clone(),
        created_utc: chrono::Utc::now().to_rfc3339(),
    };
    std::fs::create_dir_all(paths.leases_dir())?;
    let encoded = serde_json::to_string(&lease)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(lease_path(paths, parent.pid), encoded)?;
    Ok(true)
}

/// Returns the set of ids held by leases whose owning process is still live.
///
/// Used to decide whether a changeset is orphaned. When the process table
/// cannot see the caller itself the table is untrustworthy, so *every*
/// recorded id is reported live — never claim a session is gone on the
/// strength of a table that reads as empty.
pub fn live_lease_ids(paths: &SessionPaths, procs: &dyn ProcessTable) -> BTreeSet<String> {
    let trustworthy = procs.get(procs.self_pid()).is_some();
    let mut out = BTreeSet::new();
    let Ok(entries) = std::fs::read_dir(paths.leases_dir()) else {
        return out;
    };
    for entry in entries.flatten() {
        let Some(pid) = lease_pid_from_name(&entry.file_name().to_string_lossy()) else {
            continue;
        };
        let Some(lease) = read_lease(paths, pid) else {
            continue;
        };
        if !trustworthy {
            out.insert(lease.id);
            continue;
        }
        match procs.get(pid) {
            Some(info) if info.start_time == lease.start_time => {
                out.insert(lease.id);
            }
            _ => {}
        }
    }
    out
}

/// Returns the set of changeset ids that have a lease file but whose owning
/// process is dead or has been recycled.
///
/// This is used to distinguish truly orphaned changesets (dead lease) from
/// unleased changesets (no lease file at all). Both may appear orphaned from
/// another session's perspective, but the distinction is important for
/// labeling in `rdm session list`.
pub fn dead_lease_ids(paths: &SessionPaths, procs: &dyn ProcessTable) -> BTreeSet<String> {
    let trustworthy = procs.get(procs.self_pid()).is_some();
    let mut out = BTreeSet::new();
    let Ok(entries) = std::fs::read_dir(paths.leases_dir()) else {
        return out;
    };
    for entry in entries.flatten() {
        let Some(pid) = lease_pid_from_name(&entry.file_name().to_string_lossy()) else {
            continue;
        };
        let Some(lease) = read_lease(paths, pid) else {
            continue;
        };
        if !trustworthy {
            // If we can't trust the process table, assume no leases are dead.
            continue;
        }
        match procs.get(pid) {
            Some(info) if info.start_time == lease.start_time => {
                // Process is alive and start time matches - not dead.
            }
            _ => {
                // Process is dead or pid was recycled.
                out.insert(lease.id);
            }
        }
    }
    out
}

/// Extracts the pid from a `<pid>.lease` file name.
fn lease_pid_from_name(name: &str) -> Option<u32> {
    name.strip_suffix(".lease")?.parse().ok()
}

/// Removes leases whose owning process is gone or whose pid was recycled.
///
/// Bounded at [`MAX_GC_ENTRIES`] files per call. Journals are never touched —
/// a killed session's work stays recoverable via
/// [`adopt_changeset`](super::journal::adopt_changeset).
///
/// GC is skipped entirely when the process table cannot even see the calling
/// process, since a table that reads as empty would otherwise sweep away every
/// live session's lease.
pub fn gc(paths: &SessionPaths, procs: &dyn ProcessTable) -> usize {
    if procs.get(procs.self_pid()).is_none() {
        return 0;
    }
    let Ok(entries) = std::fs::read_dir(paths.leases_dir()) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten().take(MAX_GC_ENTRIES) {
        let Some(pid) = lease_pid_from_name(&entry.file_name().to_string_lossy()) else {
            continue;
        };
        let stale = match (read_lease(paths, pid), procs.get(pid)) {
            (None, _) => true,
            (Some(lease), Some(info)) => lease.start_time != info.start_time,
            (Some(_), None) => true,
        };
        if stale && std::fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn paths(dir: &TempDir) -> SessionPaths {
        SessionPaths::new(dir.path().join("rdm"))
    }

    fn write_lease(p: &SessionPaths, pid: u32, id: &str, start_time: &str) {
        std::fs::create_dir_all(p.leases_dir()).unwrap();
        let lease = Lease {
            id: id.to_string(),
            start_time: start_time.to_string(),
            created_utc: "2026-08-07T00:00:00Z".to_string(),
        };
        std::fs::write(lease_path(p, pid), serde_json::to_string(&lease).unwrap()).unwrap();
    }

    #[test]
    fn adopts_immediate_parent_lease() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let table = MapProcessTableAlias::chain(10, &[(20, "s20"), (30, "s30")]);
        write_lease(&p, 20, "s-aaaabbbbccccdddd", "s20");
        let id = adopt_inherited(&p, &table).unwrap();
        assert_eq!(id.as_str(), "s-aaaabbbbccccdddd");
    }

    #[test]
    fn adopts_lowest_matching_ancestor_when_several_hold_leases() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let table = MapProcessTableAlias::chain(10, &[(20, "s20"), (30, "s30")]);
        write_lease(&p, 20, "s-low", "s20");
        write_lease(&p, 30, "s-high", "s30");
        assert_eq!(adopt_inherited(&p, &table).unwrap().as_str(), "s-low");
    }

    #[test]
    fn ascent_is_bounded_at_max_ancestor_depth() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        // Ten ancestors; only the tenth holds a lease, which is beyond the
        // bound and therefore must not be adopted.
        let parents: Vec<(u32, String)> = (1..=10)
            .map(|i| (100 + i as u32, format!("s{i}")))
            .collect();
        let refs: Vec<(u32, &str)> = parents.iter().map(|(p, s)| (*p, s.as_str())).collect();
        let table = MapProcessTableAlias::chain(10, &refs);
        write_lease(&p, 110, "s-too-high", "s10");
        assert!(adopt_inherited(&p, &table).is_none());
    }

    #[test]
    fn recycled_pid_with_different_start_time_is_not_adopted() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        // The lease says pid 4242 started at "111"; the live table says "999",
        // so the pid was recycled and the lease belongs to a dead process.
        let table = MapProcessTableAlias::chain(10, &[(4242, "999")]);
        write_lease(&p, 4242, "s-deadbeefdeadbeef", "111");

        assert!(adopt_inherited(&p, &table).is_none());
        assert!(
            !lease_path(&p, 4242).exists(),
            "stale lease must be removed, not left to be adopted later"
        );

        let fresh = create_at_parent(&p, &table).unwrap();
        assert_ne!(fresh.as_str(), "s-deadbeefdeadbeef");
        assert!(fresh.as_str().starts_with("s-"));
    }

    /// Two concurrent sessions as the OS actually presents them: one process
    /// table containing both ancestries (10 under 20, and 11 under 21), viewed
    /// from `self_pid`.
    ///
    /// Modelling them instead as two disjoint partial tables would be a fake
    /// no real backend produces — `/proc` and `ps -A` both enumerate every
    /// process — and it would make each session's create-path GC sweep the
    /// other's lease purely because its private world could not see that
    /// parent. That is an artifact of the double, not of the shipped rule.
    fn two_session_table(self_pid: u32) -> MapProcessTableAlias {
        MapProcessTableAlias::empty(self_pid)
            .with(10, 20, "self-start")
            .with(20, 1, "s20")
            .with(11, 21, "self-start")
            .with(21, 1, "s21")
    }

    #[test]
    fn two_distinct_ancestors_get_distinct_ids_same_ancestor_reuses_one() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let a = two_session_table(10);
        let b = two_session_table(11);

        let id_a = create_at_parent(&p, &a).unwrap();
        let id_b = create_at_parent(&p, &b).unwrap();
        assert_ne!(
            id_a.as_str(),
            id_b.as_str(),
            "distinct ancestors, distinct ids"
        );

        // A second process under the same ancestor adopts, never re-creates.
        let a2 = two_session_table(12).with(12, 20, "self-start");
        assert_eq!(adopt_inherited(&p, &a2).unwrap().as_str(), id_a.as_str());
        assert_eq!(adopt_inherited(&p, &b).unwrap().as_str(), id_b.as_str());
    }

    #[test]
    fn creating_a_lease_never_sweeps_a_live_concurrent_sessions_lease() {
        // The create-path GC is the phase-11 leak fix, and this is the bound
        // on it: minting for session B must not disturb session A's lease
        // while A's parent is still alive. If it did, every second concurrent
        // session would silently orphan the first one's changeset.
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let id_a = create_at_parent(&p, &two_session_table(10)).unwrap();
        create_at_parent(&p, &two_session_table(11)).unwrap();

        assert!(
            lease_path(&p, 20).exists(),
            "a live session's lease was swept"
        );
        assert_eq!(
            adopt_inherited(&p, &two_session_table(10))
                .unwrap()
                .as_str(),
            id_a.as_str()
        );
    }

    #[test]
    fn creating_a_lease_sweeps_the_dead_predecessor_it_replaces() {
        // The per-tool-call wrapper-shell topology: each invocation's parent
        // is gone by the time the next one runs. Without the create-path
        // sweep this directory grows by one dead entry per invocation.
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        write_lease(&p, 900, "s-dead-wrapper", "s900");
        write_lease(&p, 901, "s-dead-wrapper-2", "s901");

        // The live table knows pid 20 (this call's parent) but neither 900 nor
        // 901 — both wrappers have exited.
        create_at_parent(&p, &MapProcessTableAlias::chain(10, &[(20, "s20")])).unwrap();

        assert!(
            !lease_path(&p, 900).exists(),
            "dead predecessor was not swept"
        );
        assert!(
            !lease_path(&p, 901).exists(),
            "dead predecessor was not swept"
        );
        let live = std::fs::read_dir(p.leases_dir())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".lease"))
            .count();
        assert_eq!(live, 1, "exactly the freshly-minted lease should remain");
    }

    #[test]
    fn create_path_gc_is_skipped_when_the_table_cannot_see_the_caller() {
        // `gc`'s own guard, re-asserted at the new call site: a table that
        // reads as empty means "unknown", never "nothing is alive". Here the
        // caller (pid 10) is absent from the table, so nothing may be swept —
        // and with no ancestry there is nothing to mint either.
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        write_lease(&p, 20, "s-someone-elses", "s20");
        assert!(create_at_parent(&p, &MapProcessTableAlias::empty(10)).is_none());
        assert!(
            lease_path(&p, 20).exists(),
            "an unreadable table must never sweep a lease"
        );
    }

    #[test]
    fn creation_never_happens_above_the_immediate_parent() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let table = MapProcessTableAlias::chain(10, &[(20, "s20"), (30, "s30")]);
        create_at_parent(&p, &table).unwrap();
        assert!(lease_path(&p, 20).exists());
        assert!(
            !lease_path(&p, 30).exists(),
            "creating above the immediate parent would merge sibling sessions"
        );
    }

    #[test]
    fn corrupt_lease_is_removed_and_not_adopted() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        std::fs::create_dir_all(p.leases_dir()).unwrap();
        std::fs::write(lease_path(&p, 20), b"\x00\x01not json at all").unwrap();
        let table = MapProcessTableAlias::chain(10, &[(20, "s20")]);
        assert!(adopt_inherited(&p, &table).is_none());
        assert!(!lease_path(&p, 20).exists());
    }

    #[test]
    fn empty_process_table_yields_no_lease() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let table = MapProcessTableAlias::empty(10);
        assert!(adopt_inherited(&p, &table).is_none());
        assert!(create_at_parent(&p, &table).is_none());
    }

    #[test]
    fn unwritable_state_dir_yields_no_lease_and_no_panic() {
        let dir = TempDir::new().unwrap();
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, b"i am a file").unwrap();
        // Base sits *under* a regular file, so create_dir_all must fail.
        let p = SessionPaths::new(blocker.join("rdm"));
        let table = MapProcessTableAlias::chain(10, &[(20, "s20")]);
        assert!(create_at_parent(&p, &table).is_none());
        assert!(adopt_inherited(&p, &table).is_none());
    }

    #[test]
    fn gc_removes_dead_and_recycled_leases_only() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let table = MapProcessTableAlias::chain(10, &[(20, "s20")]).with(40, 1, "s40");
        write_lease(&p, 20, "s-live", "s20");
        write_lease(&p, 40, "s-recycled", "different");
        write_lease(&p, 55, "s-dead", "s55");

        assert_eq!(gc(&p, &table), 2);
        assert!(lease_path(&p, 20).exists());
        assert!(!lease_path(&p, 40).exists());
        assert!(!lease_path(&p, 55).exists());
    }

    #[test]
    fn gc_is_skipped_when_the_process_table_is_untrustworthy() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        write_lease(&p, 20, "s-live", "s20");
        assert_eq!(gc(&p, &MapProcessTableAlias::empty(10)), 0);
        assert!(
            lease_path(&p, 20).exists(),
            "an unreadable table must never be read as 'nothing is alive'"
        );
    }

    #[test]
    fn live_lease_ids_reports_only_matching_live_leases() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let table = MapProcessTableAlias::chain(10, &[(20, "s20")]).with(40, 1, "s40");
        write_lease(&p, 20, "s-live", "s20");
        write_lease(&p, 40, "s-recycled", "different");
        write_lease(&p, 55, "s-dead", "s55");

        let live = live_lease_ids(&p, &table);
        assert!(live.contains("s-live"));
        assert!(!live.contains("s-recycled"));
        assert!(!live.contains("s-dead"));
    }

    #[test]
    fn repoint_parent_lease_switches_the_adopted_id() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let table = MapProcessTableAlias::chain(10, &[(20, "s20")]);
        create_at_parent(&p, &table).unwrap();
        let target = SessionId::new("s-orphanedchangeset").unwrap();
        assert!(repoint_parent_lease(&p, &table, &target).unwrap());
        assert_eq!(
            adopt_inherited(&p, &table).unwrap().as_str(),
            target.as_str()
        );
    }

    // Local alias so the tests read against the trait-object call sites above.
    use super::super::process::MapProcessTable as MapProcessTableAlias;
}

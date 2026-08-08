//! The changeset journal: an append-only record of exactly which paths each
//! session flushed to disk.
//!
//! One file per changeset, at `<base>/changesets/<id>.jsonl`, one JSON line per
//! [`Store::commit`](crate::store::Store::commit) batch:
//!
//! ```text
//! {"paths":[{"path":"projects/demo/tasks/a.md","kind":"write"}]}
//! ```
//!
//! Two properties fall out of the layout rather than out of discipline:
//!
//! - **Disjointness.** Concurrent sessions have distinct ids and therefore
//!   distinct files, so one session physically cannot write into another's
//!   journal.
//! - **Lock-free appends.** Each batch is one `write_all` of one line to a file
//!   opened with `O_APPEND`, which POSIX does not interleave, so there is no
//!   lock and no read-modify-write race.
//!
//! Recording is best-effort at every call site: an unwritable state directory
//! must never fail a mutation. The cost of a lost record is bounded and
//! reported rather than silent — the paths simply become unattributed, and a
//! scoped `rdm commit` names them and points at its recovery routes instead
//! of sweeping them.
//!
//! Journals are never garbage-collected. A session killed mid-batch leaves an
//! *orphaned* changeset, which [`list_changesets`] flags and
//! [`adopt_changeset`] hands to a live session; only an explicit
//! [`discard_changeset`] destroys one.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::process::ProcessTable;
use super::{SessionId, SessionPaths, lease};
use crate::error::{Error, Result};

/// Whether a journaled path was written or deleted by its batch.
///
/// The distinction is load-bearing for the scoped commit that consumes this
/// journal: a delete that journaled as a write could not be reproduced.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JournalKind {
    /// The path was created or overwritten.
    Write,
    /// The path was removed.
    Delete,
}

impl JournalKind {
    /// Returns the lowercase wire spelling (`write` / `delete`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Write => "write",
            Self::Delete => "delete",
        }
    }
}

/// One journaled path plus what happened to it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalEntry {
    /// The store-relative path.
    pub path: String,
    /// Whether the path was written or deleted.
    pub kind: JournalKind,
}

/// A summary of one changeset on disk.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ChangesetSummary {
    /// The changeset (session) id.
    pub id: String,
    /// How many distinct paths the changeset has journaled.
    pub paths: usize,
    /// Whether no live session currently owns this changeset.
    pub orphaned: bool,
}

#[derive(Serialize, Deserialize)]
struct JournalLine {
    paths: Vec<JournalEntry>,
}

/// Returns the on-disk path of `id`'s journal.
pub fn changeset_path(paths: &SessionPaths, id: &SessionId) -> PathBuf {
    paths.changesets_dir().join(format!("{id}.jsonl"))
}

/// Appends one batch of journal entries to `id`'s journal.
///
/// An empty `entries` slice records nothing at all, so an empty flush can never
/// leave behind a journal that claims a batch happened.
///
/// # Errors
///
/// Returns [`Error::Io`] if the state directory cannot be created or the
/// append fails. Callers on the mutation path swallow this deliberately —
/// journaling is best-effort.
pub fn record(paths: &SessionPaths, id: &SessionId, entries: &[JournalEntry]) -> Result<()> {
    if entries.is_empty() {
        return Ok(());
    }
    let dir = paths.changesets_dir();
    std::fs::create_dir_all(&dir)?;
    let line = serde_json::to_string(&JournalLine {
        paths: entries.to_vec(),
    })
    .map_err(|e| Error::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, e)))?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(changeset_path(paths, id))?;
    // One write_all of one complete line: O_APPEND makes concurrent
    // single-line appends non-interleaving, so no lock is needed.
    file.write_all(format!("{line}\n").as_bytes())?;
    Ok(())
}

/// Reads `id`'s journal as a deduped, path-sorted union across every recorded
/// batch.
///
/// A path written and later deleted (or vice versa) reports its **last**
/// recorded kind. Unparsable lines are skipped rather than failing the read, so
/// a torn tail cannot make an otherwise-recoverable changeset unreadable.
///
/// # Errors
///
/// Returns [`Error::Io`] if the journal exists but cannot be read. A missing
/// journal is not an error — it reads as empty.
pub fn read_journal(paths: &SessionPaths, id: &SessionId) -> Result<Vec<JournalEntry>> {
    let path = changeset_path(paths, id);
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(Error::Io(e)),
    };
    let mut merged: BTreeMap<String, JournalKind> = BTreeMap::new();
    for line in raw.lines() {
        let Ok(parsed) = serde_json::from_str::<JournalLine>(line) else {
            continue;
        };
        for entry in parsed.paths {
            merged.insert(entry.path, entry.kind);
        }
    }
    Ok(merged
        .into_iter()
        .map(|(path, kind)| JournalEntry { path, kind })
        .collect())
}

/// Drops `landed` from `id`'s journal, rewriting it with whatever remains.
///
/// **This is correctness, not cleanup.** A journal that still claims a path
/// after that path has been committed lets this session's *next* commit
/// re-commit it — and by then another session may have edited it, so the
/// re-commit would sweep up work this session never did. Truncating on a
/// successful commit is what keeps attribution honest across a session's
/// second and subsequent commits.
///
/// Rewrites the file as a single line holding the surviving entries (or
/// removes it when nothing survives). Unlike [`record`], this is a
/// read-modify-write, which is safe because a changeset is by construction
/// owned by one session: the only appender is the same shell that is
/// committing.
///
/// # Errors
///
/// Returns [`Error::Io`] if the journal cannot be read back or rewritten.
/// Callers on the commit path swallow this: the commit itself has already
/// landed, and failing afterwards would be worse than a stale journal.
pub fn truncate(paths: &SessionPaths, id: &SessionId, landed: &[String]) -> Result<()> {
    let remaining: Vec<JournalEntry> = read_journal(paths, id)?
        .into_iter()
        .filter(|e| !landed.contains(&e.path))
        .collect();
    let path = changeset_path(paths, id);
    if remaining.is_empty() {
        return match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(Error::Io(e)),
        };
    }
    let line = serde_json::to_string(&JournalLine { paths: remaining })
        .map_err(|e| Error::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, e)))?;
    std::fs::write(&path, format!("{line}\n"))?;
    Ok(())
}

/// Lists every changeset on disk, flagging the ones no live session owns.
///
/// A changeset is orphaned when no live lease records its id and it is not the
/// caller's own id. Ids that never had a lease (explicit `RDM_SESSION`, a
/// harness variable, or a per-process fallback) therefore read as orphaned once
/// their session is gone — which is exactly the recoverable state
/// [`adopt_changeset`] exists to resolve.
///
/// # Errors
///
/// Returns [`Error::Io`] if the changesets directory exists but cannot be
/// listed. A missing directory reads as empty.
pub fn list_changesets(
    paths: &SessionPaths,
    procs: &dyn ProcessTable,
    current: Option<&SessionId>,
) -> Result<Vec<ChangesetSummary>> {
    let dir = paths.changesets_dir();
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(Error::Io(e)),
    };
    let live = lease::live_lease_ids(paths, procs);
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let Some(stem) = name.strip_suffix(".jsonl") else {
            continue;
        };
        let Some(id) = SessionId::new(stem) else {
            continue;
        };
        let owned =
            live.contains(id.as_str()) || current.is_some_and(|c| c.as_str() == id.as_str());
        out.push(ChangesetSummary {
            paths: read_journal(paths, &id)?.len(),
            orphaned: !owned,
            id: id.to_string(),
        });
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

/// Re-points the caller's session at an existing changeset.
///
/// This is orphan recovery: after adopting, the caller's shell resolves `id`
/// and its subsequent mutations append to that changeset's journal.
///
/// # Errors
///
/// Returns [`Error::Io`] if the lease cannot be written, or a
/// [`Error::InvalidPath`] describing the problem if the caller has no parent
/// process to hold the lease.
pub fn adopt_changeset(
    paths: &SessionPaths,
    procs: &dyn ProcessTable,
    id: &SessionId,
) -> Result<()> {
    if !lease::repoint_parent_lease(paths, procs, id)? {
        return Err(Error::InvalidPath(
            "cannot adopt a changeset: no parent process is available to hold the lease — \
             set RDM_SESSION to the changeset id instead"
                .to_string(),
        ));
    }
    Ok(())
}

/// Deletes a changeset's journal.
///
/// Returns `false` when there was nothing to delete.
///
/// # Errors
///
/// Returns [`Error::Io`] if the journal exists but cannot be removed.
pub fn discard_changeset(paths: &SessionPaths, id: &SessionId) -> Result<bool> {
    match std::fs::remove_file(changeset_path(paths, id)) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(Error::Io(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::super::process::MapProcessTable;
    use super::*;
    use tempfile::TempDir;

    fn paths(dir: &TempDir) -> SessionPaths {
        SessionPaths::new(dir.path().join("rdm"))
    }

    fn entry(path: &str, kind: JournalKind) -> JournalEntry {
        JournalEntry {
            path: path.to_string(),
            kind,
        }
    }

    #[test]
    fn record_then_read_returns_exactly_the_recorded_paths() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let id = SessionId::new("s-one").unwrap();
        record(
            &p,
            &id,
            &[
                entry("projects/demo/tasks/a.md", JournalKind::Write),
                entry("INDEX.md", JournalKind::Write),
            ],
        )
        .unwrap();
        let read = read_journal(&p, &id).unwrap();
        assert_eq!(
            read,
            vec![
                entry("INDEX.md", JournalKind::Write),
                entry("projects/demo/tasks/a.md", JournalKind::Write),
            ],
            "read is the deduped, path-sorted union"
        );
    }

    #[test]
    fn empty_batch_records_nothing() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let id = SessionId::new("s-empty").unwrap();
        record(&p, &id, &[]).unwrap();
        assert!(!changeset_path(&p, &id).exists());
        assert!(read_journal(&p, &id).unwrap().is_empty());
    }

    #[test]
    fn later_batch_supersedes_the_kind_of_the_same_path() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let id = SessionId::new("s-two").unwrap();
        record(&p, &id, &[entry("a.md", JournalKind::Write)]).unwrap();
        record(&p, &id, &[entry("a.md", JournalKind::Delete)]).unwrap();
        assert_eq!(
            read_journal(&p, &id).unwrap(),
            vec![entry("a.md", JournalKind::Delete)]
        );
    }

    #[test]
    fn two_changesets_never_see_each_others_paths() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let a = SessionId::new("s-aaa").unwrap();
        let b = SessionId::new("s-bbb").unwrap();
        record(&p, &a, &[entry("tasks/alpha.md", JournalKind::Write)]).unwrap();
        record(&p, &b, &[entry("tasks/beta.md", JournalKind::Write)]).unwrap();

        let a_paths: Vec<String> = read_journal(&p, &a)
            .unwrap()
            .into_iter()
            .map(|e| e.path)
            .collect();
        let b_paths: Vec<String> = read_journal(&p, &b)
            .unwrap()
            .into_iter()
            .map(|e| e.path)
            .collect();
        assert_eq!(a_paths, vec!["tasks/alpha.md".to_string()]);
        assert_eq!(b_paths, vec!["tasks/beta.md".to_string()]);
    }

    #[test]
    fn torn_line_is_skipped_rather_than_failing_the_read() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let id = SessionId::new("s-torn").unwrap();
        record(&p, &id, &[entry("a.md", JournalKind::Write)]).unwrap();
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(changeset_path(&p, &id))
            .unwrap();
        file.write_all(b"{\"paths\":[{\"pa\n").unwrap();
        assert_eq!(
            read_journal(&p, &id).unwrap(),
            vec![entry("a.md", JournalKind::Write)]
        );
    }

    #[test]
    fn truncate_drops_only_the_landed_paths() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let id = SessionId::new("s-trunc").unwrap();
        record(
            &p,
            &id,
            &[
                entry("a.md", JournalKind::Write),
                entry("b.md", JournalKind::Write),
            ],
        )
        .unwrap();

        truncate(&p, &id, &["a.md".to_string()]).unwrap();
        assert_eq!(
            read_journal(&p, &id).unwrap(),
            vec![entry("b.md", JournalKind::Write)],
            "only the landed path is dropped"
        );
    }

    #[test]
    fn truncating_every_path_removes_the_journal_entirely() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let id = SessionId::new("s-trunc-all").unwrap();
        record(&p, &id, &[entry("a.md", JournalKind::Write)]).unwrap();

        truncate(&p, &id, &["a.md".to_string()]).unwrap();
        assert!(!changeset_path(&p, &id).exists());
        // Idempotent: truncating an already-gone journal is not an error.
        truncate(&p, &id, &["a.md".to_string()]).unwrap();
    }

    #[test]
    fn a_truncated_changeset_cannot_re_claim_a_landed_path() {
        // The correctness property, stated directly: after truncation the
        // journal no longer claims the path, so a later commit built from it
        // cannot re-commit — and therefore cannot sweep another session's
        // subsequent edit to that same path.
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let id = SessionId::new("s-reclaim").unwrap();
        record(&p, &id, &[entry("shared.md", JournalKind::Write)]).unwrap();
        truncate(&p, &id, &["shared.md".to_string()]).unwrap();
        assert!(
            !read_journal(&p, &id)
                .unwrap()
                .iter()
                .any(|e| e.path == "shared.md"),
            "a landed path must not survive in the journal"
        );
    }

    #[test]
    fn missing_journal_reads_as_empty() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let id = SessionId::new("s-nothing").unwrap();
        assert!(read_journal(&p, &id).unwrap().is_empty());
        assert!(
            list_changesets(&p, &MapProcessTable::empty(1), None)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn list_flags_changesets_with_no_live_lease_as_orphaned() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let table = MapProcessTable::chain(10, &[(20, "s20")]);

        let live = lease::create_at_parent(&p, &table).unwrap();
        record(&p, &live, &[entry("a.md", JournalKind::Write)]).unwrap();
        let orphan = SessionId::new("s-orphan").unwrap();
        record(&p, &orphan, &[entry("b.md", JournalKind::Write)]).unwrap();

        let listed = list_changesets(&p, &table, None).unwrap();
        let by_id = |id: &str| listed.iter().find(|c| c.id == id).unwrap().clone();
        assert!(!by_id(live.as_str()).orphaned);
        assert!(by_id("s-orphan").orphaned);
        assert_eq!(by_id("s-orphan").paths, 1);
    }

    #[test]
    fn the_callers_own_changeset_is_never_orphaned() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let id = SessionId::new("explicit-one").unwrap();
        record(&p, &id, &[entry("a.md", JournalKind::Write)]).unwrap();
        let listed = list_changesets(&p, &MapProcessTable::empty(1), Some(&id)).unwrap();
        assert_eq!(listed.len(), 1);
        assert!(!listed[0].orphaned);
    }

    #[test]
    fn adopt_repoints_the_caller_and_discard_removes_the_journal() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let table = MapProcessTable::chain(10, &[(20, "s20")]);
        let orphan = SessionId::new("s-orphan").unwrap();
        record(&p, &orphan, &[entry("a.md", JournalKind::Write)]).unwrap();

        adopt_changeset(&p, &table, &orphan).unwrap();
        assert_eq!(
            lease::adopt_inherited(&p, &table).unwrap().as_str(),
            "s-orphan"
        );

        assert!(discard_changeset(&p, &orphan).unwrap());
        assert!(!discard_changeset(&p, &orphan).unwrap());
    }

    #[test]
    fn adopt_without_a_parent_process_reports_an_actionable_error() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let id = SessionId::new("s-orphan").unwrap();
        let err = adopt_changeset(&p, &MapProcessTable::empty(10), &id).unwrap_err();
        assert!(err.to_string().contains("RDM_SESSION"));
    }

    #[test]
    fn unwritable_state_dir_surfaces_as_an_error_callers_can_swallow() {
        let dir = TempDir::new().unwrap();
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, b"i am a file").unwrap();
        let p = SessionPaths::new(blocker.join("rdm"));
        let id = SessionId::new("s-one").unwrap();
        assert!(record(&p, &id, &[entry("a.md", JournalKind::Write)]).is_err());
    }
}

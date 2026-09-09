//! The changeset journal: an append-only record of exactly which paths each
//! session flushed to disk.
//!
//! One file per changeset, at `<base>/changesets/<id>.jsonl`, one JSON line per
//! [`Store::commit`](crate::store::Store::commit) batch:
//!
//! ```text
//! {"paths":[{"path":"projects/demo/tasks/a.md","kind":"write","digest":"<sha256-hex>"}]}
//! ```
//!
//! The `digest` is the content identity of the bytes that batch flushed. It is
//! optional — absent on a delete, and absent on lines written before the field
//! existed — so an older journal stays readable.
//!
//! A commit does not rewrite this file. It appends a *tombstone* line naming
//! the entries it proved are now reflected at HEAD:
//!
//! ```text
//! {"landed":[{"path":"projects/demo/tasks/a.md","kind":"write","digest":"<sha256-hex>"}]}
//! ```
//!
//! [`read_journal`] is the ordered fold that gives the file its meaning: a
//! `paths` line inserts its entries, and a `landed` tombstone removes a path
//! **only when the entry currently folded for it is byte-for-byte the entry
//! that landed**. A concurrent session that rewrote the same path in between
//! has a different `digest` (or a different `kind`), so its record survives
//! the tombstone instead of being swept by it.
//!
//! Two properties fall out of the layout rather than out of discipline:
//!
//! - **Append-only.** *Every* write to a journal — a batch record and a
//!   commit's truncation alike — is one `write_all` of one complete line to a
//!   file opened with `O_APPEND`, which POSIX does not interleave. So there is
//!   no lock, no read-modify-write window, and neither kind of write can
//!   destroy the other. Compaction ([`compact`]) is the sole exception: it is
//!   the one operation that replaces or unlinks the file, and appends do not
//!   trust it to be quiescent — they detect having written into a journal
//!   compaction swapped out and redo the write against the live one (see
//!   [`append_line`]).
//! - **Content-keyed truncation.** Because a tombstone identifies *what*
//!   landed rather than *where* it sat in the file, the fold is independent of
//!   byte offsets, so compaction can rewrite a journal without changing what a
//!   later tombstone means.
//!
//! What does **not** fall out of the layout is disjointness. Concurrent
//! sessions have distinct ids and therefore distinct files, so a *batch*
//! record only ever reaches its own session's journal — but
//! [`GitStore::commit_whole_tree`](../../../rdm_store_git/struct.GitStore.html)
//! deliberately tombstones **every** changeset on disk, so one session does
//! write into another's journal. `O_APPEND` is what keeps that safe.
//!
//! Recording is best-effort at every call site: an unwritable state directory
//! must never fail a mutation. The cost of a lost record is bounded and
//! reported rather than silent — the paths simply become unattributed, and a
//! scoped `rdm commit` names them and points at its recovery routes instead
//! of sweeping them.
//!
//! A session killed mid-batch leaves an *orphaned* changeset, which
//! [`list_changesets`] flags and [`adopt_changeset`] hands to a live session;
//! an explicit [`discard_changeset`] destroys one outright. Journals that fold
//! to nothing are swept by [`gc_changesets`] (behind `rdm session gc`), which
//! skips changesets a live lease names. That skip is a courtesy, not a proof:
//! rungs 1 and 3 resolve an id without ever creating a lease, so a changeset
//! being appended to right now can read as unleased. What actually makes the
//! sweep safe is [`append_line`]'s redo, not the lease check.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::process::ProcessTable;
use super::{SessionId, SessionPaths, lease};
use crate::error::{Error, Result};
use crate::lock::AdvisoryLock;

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
    /// The [`content_digest`](crate::store::content_digest) of the bytes this
    /// batch flushed to `path` — base-blob identity, so a scoped commit can
    /// tell "the content I wrote" from "whatever happens to be there now".
    ///
    /// `None` on a delete (there are no bytes), and `None` on journal lines
    /// written before this field existed. A missing digest must never make an
    /// otherwise-good line unparsable, so the field is `#[serde(default)]`
    /// and every consumer skips its check rather than failing when it is
    /// absent — an in-flight changeset from an older rdm keeps working.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
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
    /// The batch's entries.
    ///
    /// **Must stay required.** `paths` being mandatory is what lets
    /// [`read_journal`] tell a batch line from a [`TombstoneLine`] by trying
    /// one and then the other: give it `#[serde(default)]` and every
    /// `{"landed":[…]}` line would parse as an empty batch, silently turning
    /// truncation into a no-op.
    paths: Vec<JournalEntry>,
}

/// A commit's truncation, written as one appended line rather than a rewrite.
///
/// `landed` holds the journal entries — path, kind *and* digest — that a
/// commit proved are now reflected at HEAD. Carrying the whole entry rather
/// than a bare path is what makes truncation content-keyed: the fold removes
/// a path only when what is currently recorded for it is exactly what landed,
/// so a concurrent session's newer record of the same path is not swept.
#[derive(Serialize, Deserialize)]
struct TombstoneLine {
    /// The entries this commit landed.
    ///
    /// **Must stay required**, for the mirror of the reason above.
    landed: Vec<JournalEntry>,
}

/// The environment variable naming a harness barrier file for truncation.
///
/// The window between a commit reading a journal and truncating it is opened
/// and closed inside one `rdm` invocation, so a harness cannot interleave two
/// real processes at it by racing them. When this is set, [`truncate`] blocks
/// until the named file appears, letting a harness park a committing process
/// there and drive a second process's mutation to completion before releasing
/// it. It follows `RDM_HARNESS_FLUSH_BARRIER`'s contract exactly: inert when
/// unset or empty, and bounded when set, so an abandoned harness can delay a
/// real run but never wedge it.
const HARNESS_JOURNAL_BARRIER: &str = "RDM_HARNESS_JOURNAL_BARRIER";

/// How long the harness barrier waits before proceeding regardless.
const HARNESS_BARRIER_CEILING: Duration = Duration::from_secs(60);

/// How long compaction waits for the advisory lock before giving up.
///
/// Short on purpose: compaction is pure hygiene, so a contended lock should
/// cost the caller nothing. Unlike every other `AdvisoryLock` user in rdm,
/// losing this lock means *skip*, never *proceed unlocked* — see [`compact`].
const COMPACT_LOCK_WAIT: Duration = Duration::from_millis(200);

/// How stale a compaction lock must be before another process takes it over.
const COMPACT_LOCK_STALE_AFTER: Duration = Duration::from_secs(30);

/// How many times an append redoes itself after discovering it wrote into a
/// journal that is no longer the live one.
///
/// One redo covers a single racing [`compact`]; the bound is four so a burst
/// of them still converges. It is a *bound* rather than a condition, so the
/// loop terminates unconditionally — the final attempt is accepted without a
/// recheck, degrading to the same best-effort over-claim as every other
/// journaling failure.
const APPEND_ATTEMPTS: usize = 4;

/// The environment variable naming a harness barrier file for the append
/// itself.
///
/// When set, an append blocks between *opening* the journal and *writing* to
/// it — the window in which a concurrent [`compact`] can replace or unlink the
/// inode the open descriptor names. That window is far too narrow for a
/// harness to hit by timing alone, so driving it needs a seam, exactly as
/// [`HARNESS_JOURNAL_BARRIER`] does for truncation. Same contract: inert when
/// unset or empty, bounded by [`HARNESS_BARRIER_CEILING`] when set.
const HARNESS_APPEND_BARRIER: &str = "RDM_HARNESS_APPEND_BARRIER";

/// Blocks until the barrier file named by `var` appears, or the ceiling
/// elapses.
///
/// Inert unless `var` is set to a non-empty value. Shared by the two seams —
/// [`HARNESS_JOURNAL_BARRIER`] around truncation and
/// [`HARNESS_APPEND_BARRIER`] inside the append — so their contract is written
/// once.
fn harness_barrier(var: &str) {
    let Ok(marker) = std::env::var(var) else {
        return;
    };
    if marker.is_empty() {
        return;
    }
    let marker = PathBuf::from(marker);
    let deadline = std::time::Instant::now() + HARNESS_BARRIER_CEILING;
    while !marker.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Appends one complete line to the journal at `path`, redoing the write if it
/// landed in a file that is no longer the live journal.
///
/// The append itself is one `write_all` of one complete line to a descriptor
/// opened with `O_APPEND`, which POSIX does not interleave. That is what makes
/// two concurrent *appenders* safe from each other with no lock at all.
///
/// What it does not make them safe from is [`compact`] — the one operation in
/// this module that replaces or unlinks the journal. A process that opened its
/// descriptor before a compaction and wrote after it writes into an inode
/// nothing will ever read again, and its record is gone silently.
///
/// Liveness cannot rule that out. The two rungs this whole subsystem exists to
/// serve — an explicit `RDM_SESSION` and a harness-published id — create and
/// read *no lease at all* (see [`resolve_id`](super::resolve_session)), so
/// "no live lease names this changeset" is not evidence that nothing is
/// appending to it. Any sweep that treats it as evidence is asserting an
/// invariant the identity chain does not provide.
///
/// So the append detects the loss instead of assuming it away: after writing,
/// it compares the identity of the file it wrote to against the identity of
/// `path` now, and redoes the append against the live journal when they differ
/// — including when compaction removed the file outright, in which case the
/// redo recreates it. Redoing is safe to repeat, because the fold is keyed by
/// path and applied in file order: a duplicated line contributes exactly what
/// the original contributed.
///
/// # Errors
///
/// Returns [`Error::Io`] if the journal cannot be opened or written.
fn append_line(path: &std::path::Path, line: &str) -> Result<()> {
    for attempt in 1..=APPEND_ATTEMPTS {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        harness_barrier(HARNESS_APPEND_BARRIER);
        run_after_open_hook(path);
        file.write_all(line.as_bytes())?;
        if attempt == APPEND_ATTEMPTS || wrote_to_the_live_journal(&file, path) {
            return Ok(());
        }
    }
    Ok(())
}

/// Whether `file` is still the journal `path` names.
///
/// On Unix that is the `(dev, ino)` pair: both of [`compact`]'s exits —
/// `rename` over the journal and `remove_file` of it — leave an appender's
/// descriptor pointing at an inode the path no longer names. A `path` that
/// cannot be stat'd at all reads as *not* live, which is the correct answer
/// for the removal case and costs one extra attempt for anything else. A
/// descriptor that cannot be stat'd reads as live, so an unexpected failure
/// degrades to a single append rather than to a spin.
#[cfg(unix)]
fn wrote_to_the_live_journal(file: &std::fs::File, path: &std::path::Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let Ok(written) = file.metadata() else {
        return true;
    };
    match std::fs::metadata(path) {
        Ok(live) => live.dev() == written.dev() && live.ino() == written.ino(),
        Err(_) => false,
    }
}

/// Windows refuses to rename over or unlink a file another process holds open,
/// so compaction cannot swap the journal out from under an appender there and
/// there is nothing to detect.
#[cfg(not(unix))]
fn wrote_to_the_live_journal(_file: &std::fs::File, _path: &std::path::Path) -> bool {
    true
}

/// A test-only callback run between opening the journal and writing to it.
#[cfg(test)]
type AfterOpenHook = Box<dyn FnMut(&std::path::Path)>;

#[cfg(test)]
thread_local! {
    /// Test-only seam run between opening the journal for append and writing
    /// to it, so a unit test can drive the exact interleave a concurrent
    /// [`compact`] creates rather than racing threads for a window measured in
    /// microseconds.
    static AFTER_OPEN_HOOK: std::cell::RefCell<Option<AfterOpenHook>> =
        const { std::cell::RefCell::new(None) };
}

/// Runs the test-only after-open seam, if one is installed.
#[cfg(test)]
fn run_after_open_hook(path: &std::path::Path) {
    AFTER_OPEN_HOOK.with(|hook| {
        if let Some(f) = hook.borrow_mut().as_mut() {
            f(path);
        }
    });
}

/// The seam compiles away entirely outside tests.
#[cfg(not(test))]
#[inline]
fn run_after_open_hook(_path: &std::path::Path) {}

/// Folds raw journal text into the set of entries it currently claims.
///
/// The single definition of what a journal *means*, shared by [`read_journal`]
/// and [`compact`]. Lines are processed in file order:
///
/// - a `paths` line inserts each of its entries, last write winning for both
///   `kind` and `digest`;
/// - a `landed` tombstone removes a path **only if** the entry currently
///   folded for it equals the entry that landed. A record appended after the
///   tombstone therefore resurrects the path, and so does a record of
///   *different* content appended before it — both describe a write this
///   session has not yet landed.
///
/// Unparsable lines are skipped, so a torn tail cannot make an otherwise
/// recoverable changeset unreadable. Skipping a torn *tombstone* leaves its
/// paths journaled: an over-claim, never a loss.
fn fold_lines(raw: &str) -> BTreeMap<String, JournalEntry> {
    let mut merged: BTreeMap<String, JournalEntry> = BTreeMap::new();
    for line in raw.lines() {
        if let Ok(batch) = serde_json::from_str::<JournalLine>(line) {
            for entry in batch.paths {
                // Last write wins for the digest exactly as it does for the
                // kind: a path this session flushed twice is owned by its most
                // recent content, not its first.
                merged.insert(entry.path.clone(), entry);
            }
        } else if let Ok(tombstone) = serde_json::from_str::<TombstoneLine>(line) {
            for landed in &tombstone.landed {
                if merged.get(&landed.path) == Some(landed) {
                    merged.remove(&landed.path);
                }
            }
        }
    }
    merged
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
    // One write_all of one complete line: O_APPEND makes concurrent
    // single-line appends non-interleaving, so no lock is needed. The redo
    // inside `append_line` covers the one thing O_APPEND does not: a
    // concurrent `compact` swapping the inode out from under this descriptor.
    append_line(&changeset_path(paths, id), &format!("{line}\n"))
}

/// Reads what `id`'s journal currently claims: a deduped, path-sorted fold
/// over every recorded batch and every commit's tombstone.
///
/// A path written and later deleted (or vice versa) reports its **last**
/// recorded kind, and likewise its last recorded
/// [`digest`](JournalEntry::digest). A path a commit landed is removed —
/// unless it has since been recorded again with different content, in which
/// case the newer record survives, because that write has *not* landed.
/// Unparsable lines are skipped rather than failing the read, so a torn tail
/// cannot make an otherwise-recoverable changeset unreadable.
///
/// That collapse is load-bearing for the scoped commit's two content guards,
/// not just a deduplication convenience: a path deleted and then recreated
/// within one changeset reports `Write`, so it is routed to the commit's
/// *write* guard (digest comparison) and never reaches its *delete* guard
/// (working-tree presence check). The fold preserves that across a tombstone
/// too — a record appended after one carries its own `kind` and `digest`.
///
/// A journal written before tombstones existed folds exactly as it always
/// did: with no `landed` line to apply, this degenerates to last-write-wins.
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
    Ok(fold_lines(&raw).into_values().collect())
}

/// Records that `landed` is now reflected at HEAD, so `id`'s journal stops
/// claiming it.
///
/// **This is correctness, not cleanup.** A journal that still claims a path
/// after that path has been committed lets this session's *next* commit
/// re-commit it — and by then another session may have edited it, so the
/// re-commit would sweep up work this session never did. Truncating on a
/// successful commit is what keeps attribution honest across a session's
/// second and subsequent commits.
///
/// Written as one appended tombstone line, using the same
/// `O_APPEND` + single `write_all` as [`record`] — never as a rewrite. That
/// is the whole point: under one session id, a parallel subagent or MCP call
/// is a *different process* appending to this same file, so the previous
/// read-modify-write silently destroyed anything recorded between its read
/// and its write. Two single-line appends cannot destroy each other, so the
/// window is gone rather than narrowed.
///
/// Truncation is content-keyed. The tombstone carries whole entries, and
/// [`read_journal`]'s fold drops a path only when what it currently holds is
/// exactly what landed. So a concurrent session that rewrote one of these
/// paths in between — the regenerated `INDEX.md` files, in practice — keeps
/// its record and its next commit still lands it.
///
/// An empty `landed` slice writes nothing at all, mirroring [`record`], so a
/// fully no-op commit never grows the journal.
///
/// The file is not removed even when nothing survives; see [`compact`] for
/// why cleanup is deliberately deferred to a quiescent moment.
///
/// # Errors
///
/// Returns [`Error::Io`] if the state directory cannot be created or the
/// append fails. Callers on the commit path swallow this: the commit itself
/// has already landed, and failing afterwards would be worse than a stale
/// journal — the paths simply stay journaled, which over-claims rather than
/// loses.
pub fn truncate(paths: &SessionPaths, id: &SessionId, landed: &[JournalEntry]) -> Result<()> {
    harness_barrier(HARNESS_JOURNAL_BARRIER);
    if landed.is_empty() {
        return Ok(());
    }
    let dir = paths.changesets_dir();
    std::fs::create_dir_all(&dir)?;
    let line = serde_json::to_string(&TombstoneLine {
        landed: landed.to_vec(),
    })
    .map_err(|e| Error::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, e)))?;
    // One write_all of one complete line, exactly as `record` does it —
    // including its redo against a journal compaction replaced mid-append.
    append_line(&changeset_path(paths, id), &format!("{line}\n"))
}

/// Rewrites `id`'s journal as the entries it currently claims, or removes it
/// when it claims nothing. Returns whether the file is gone afterwards.
///
/// Pure hygiene, and the **only** operation in this module that is not an
/// append. It is deliberately not called from the commit path — a rewrite is
/// never worth doing on a hot path — but it is emphatically **not** justified
/// by the changeset being quiescent. It cannot be: [`gc_changesets`] can only
/// ask whether a live *lease* names the changeset, and rungs 1 and 3 of the
/// identity chain (an explicit `RDM_SESSION`, a harness-published id) resolve
/// without ever creating or reading one. Those are precisely the rungs under
/// which parallel subagents and MCP calls share a changeset, so an unleased
/// changeset may well have several processes appending to it.
///
/// Three guards make it safe anyway, and only the third is unconditional:
///
/// - The advisory lock at `<changesets>/<id>.lock`. Unlike every other
///   [`AdvisoryLock`] user in rdm, an *unheld* guard means **skip**, not
///   proceed unlocked: those callers are safe because a real correctness
///   mechanism sits underneath (HEAD compare-and-swap; the content-digest
///   precondition), and a journal rewrite has no such underlayer.
/// - A compare-and-swap on the file's byte length. A journal only ever grows,
///   so its length is a valid version token: if anything was appended between
///   the fold and the rewrite, the length differs and compaction skips.
/// - [`append_line`]'s redo, which is what closes the window the first two
///   only narrow. An appender that already held its `O_APPEND` descriptor when
///   the length check passed writes into the inode this call is about to
///   unlink or replace — the lock cannot see it and the length cannot reflect
///   it. The appender notices afterwards that the file it wrote to is no
///   longer the journal, and writes again against the one that is. So the
///   record is not lost, and this call's contract holds unconditionally in the
///   safe direction: compaction may fail to clean, never lose.
///
/// Losing the compare-and-swap repeatedly under sustained load leaves the
/// journal growing: one small line per flushed batch plus one per commit.
/// That is a size concern, not a correctness one.
pub fn compact(paths: &SessionPaths, id: &SessionId) -> bool {
    let lock = AdvisoryLock::acquire(
        paths.changesets_dir().join(format!("{id}.lock")),
        COMPACT_LOCK_WAIT,
        COMPACT_LOCK_STALE_AFTER,
    );
    if !lock.held() {
        return false;
    }
    let path = changeset_path(paths, id);
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return false;
    };
    let folded = fold_lines(&raw);
    // The compare-and-swap: anything appended since the read makes the length
    // differ, and losing means skip.
    let unchanged = std::fs::metadata(&path)
        .map(|md| md.len() == raw.len() as u64)
        .unwrap_or(false);
    if !unchanged {
        return false;
    }
    if folded.is_empty() {
        return std::fs::remove_file(&path).is_ok();
    }
    let entries: Vec<JournalEntry> = folded.into_values().collect();
    let Ok(line) = serde_json::to_string(&JournalLine { paths: entries }) else {
        return false;
    };
    let tmp = path.with_extension("jsonl.compacting");
    if std::fs::write(&tmp, format!("{line}\n")).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return false;
    }
    if std::fs::rename(&tmp, &path).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    false
}

/// Compacts every changeset no live session owns, returning how many journals
/// were removed outright.
///
/// The sweep behind `rdm session gc`, which keeps `rdm session list` from
/// accumulating changesets that have been fully committed and now claim
/// nothing, given that [`truncate`] no longer removes a journal inline.
///
/// The live-lease filter is a *cost* filter, not a safety proof, and the
/// distinction matters enough to state outright: [`lease::live_lease_ids`]
/// enumerates lease files, and rungs 1 and 3 (an explicit `RDM_SESSION`, a
/// harness-published id) never write one. A changeset several sibling
/// processes are actively appending to under one shared id therefore reads as
/// unleased here, and `current` excludes only the *invoking* process's own
/// resolved id — not a sibling elsewhere sharing it. Skipping leased
/// changesets simply avoids rewrites that would obviously lose their
/// compare-and-swap.
///
/// Safety comes from [`append_line`] instead: an appender whose write landed
/// in a journal [`compact`] replaced or removed detects it and redoes the
/// write against the live file. That holds for every rung, leased or not, and
/// for a `rdm session gc` run from a process that shares nothing with the
/// sessions it is sweeping.
pub fn gc_changesets(
    paths: &SessionPaths,
    procs: &dyn ProcessTable,
    current: Option<&SessionId>,
) -> usize {
    let live = lease::live_lease_ids(paths, procs);
    let mut removed = 0;
    for id in list_changeset_ids(paths).unwrap_or_default() {
        let owned =
            live.contains(id.as_str()) || current.is_some_and(|c| c.as_str() == id.as_str());
        if owned {
            continue;
        }
        if compact(paths, &id) {
            removed += 1;
        }
    }
    removed
}

/// Lists every changeset on disk, flagging the ones no live session owns.
///
/// A changeset is orphaned when no live lease records its id and it is not the
/// caller's own id. Ids that never had a lease (explicit `RDM_SESSION`, a
/// harness variable, or a per-process fallback) therefore read as orphaned once
/// their session is gone — which is exactly the recoverable state
/// [`adopt_changeset`] exists to resolve.
///
/// A changeset whose journal folds to nothing is **omitted**. It claims no
/// path, so there is nothing for [`adopt_changeset`] or `rdm commit
/// --changeset` to recover, and reporting it — especially as an orphan —
/// would be noise pointing at no work. This keeps the listing identical to
/// what it was when a fully-committed journal was deleted inline; the file
/// itself is swept later by [`gc_changesets`].
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
        let claimed = read_journal(paths, &id)?.len();
        if claimed == 0 {
            continue;
        }
        out.push(ChangesetSummary {
            paths: claimed,
            orphaned: !owned,
            id: id.to_string(),
        });
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

/// Lists every changeset id on disk, live or orphaned alike.
///
/// The bare-ids counterpart to [`list_changesets`], with no
/// [`ProcessTable`] dependency: a caller that only needs to enumerate ids —
/// for example, a whole-tree committer that must clear every journal once
/// everything lands, regardless of who (if anyone) still holds a lease on
/// it — has no reason to pay for liveness resolution it will not use.
///
/// # Errors
///
/// Returns [`Error::Io`] if the changesets directory exists but cannot be
/// listed. A missing directory reads as empty.
pub fn list_changeset_ids(paths: &SessionPaths) -> Result<Vec<SessionId>> {
    let dir = paths.changesets_dir();
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(Error::Io(e)),
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let Some(stem) = name.strip_suffix(".jsonl") else {
            continue;
        };
        let Some(id) = SessionId::new(stem) else {
            continue;
        };
        out.push(id);
    }
    out.sort();
    Ok(out)
}

/// Re-points the caller's session at an existing changeset.
///
/// This is orphan recovery: after adopting, the caller's shell resolves `id`
/// and its subsequent mutations append to that changeset's journal. Adoption
/// works by repointing the caller's *parent-lease* state (rung 2) — since
/// phase 10 reordered [`super::resolve_id`] to check a harness variable
/// (rung 3) before that lease, adoption is refused up front when a harness
/// variable is present, rather than silently repointing a lease the caller's
/// own next resolution will never reach.
///
/// # Errors
///
/// Returns a [`Error::InvalidPath`] describing the problem if: a harness
/// variable (e.g. `CLAUDE_CODE_SESSION_ID`) is set in `env`, so the caller
/// would keep resolving its harness id and never see the repointed lease; or
/// the caller has no parent process to hold the lease. Returns
/// [`Error::Io`] if the lease cannot be written.
pub fn adopt_changeset(
    paths: &SessionPaths,
    procs: &dyn ProcessTable,
    env: &dyn super::EnvSource,
    id: &SessionId,
) -> Result<()> {
    if let Some((var, _)) = super::active_harness_var(env) {
        return Err(Error::InvalidPath(format!(
            "cannot adopt a changeset: {var} is set, and a harness variable now \
             always wins over an inherited lease (rung 3 outranks rung 2) — the \
             repointed lease would never be consulted. Unset {var} for this shell, \
             or set RDM_SESSION={id} instead"
        )));
    }
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
    use super::super::MapEnv;
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
            digest: None,
        }
    }

    fn entry_with_digest(path: &str, kind: JournalKind, digest: &str) -> JournalEntry {
        JournalEntry {
            path: path.to_string(),
            kind,
            digest: Some(digest.to_string()),
        }
    }

    #[test]
    fn a_journal_line_written_before_digests_existed_still_parses() {
        // Back-compat is correctness, not politeness: a changeset in flight
        // when rdm upgrades must stay committable. An unparsable line is
        // silently skipped by `read_journal`, so a required field here would
        // make an older changeset quietly lose its paths.
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let id = SessionId::new("s-legacy").unwrap();
        std::fs::create_dir_all(p.changesets_dir()).unwrap();
        std::fs::write(
            changeset_path(&p, &id),
            "{\"paths\":[{\"path\":\"projects/demo/tasks/a.md\",\"kind\":\"write\"}]}\n",
        )
        .unwrap();

        let read = read_journal(&p, &id).unwrap();
        assert_eq!(
            read,
            vec![entry("projects/demo/tasks/a.md", JournalKind::Write)]
        );
        assert_eq!(read[0].digest, None, "a legacy line carries no digest");
    }

    #[test]
    fn a_digest_round_trips_and_the_last_one_recorded_wins() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let id = SessionId::new("s-digest").unwrap();
        let path = "projects/demo/tasks/a.md";

        record(
            &p,
            &id,
            &[entry_with_digest(path, JournalKind::Write, "aaa")],
        )
        .unwrap();
        assert_eq!(
            read_journal(&p, &id).unwrap()[0].digest.as_deref(),
            Some("aaa")
        );

        // A path this session flushed twice is owned by its most recent
        // content, exactly as it is owned by its most recent kind.
        record(
            &p,
            &id,
            &[entry_with_digest(path, JournalKind::Write, "bbb")],
        )
        .unwrap();
        let read = read_journal(&p, &id).unwrap();
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].digest.as_deref(), Some("bbb"));
    }

    #[test]
    fn a_delete_recorded_over_a_write_drops_the_digest() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let id = SessionId::new("s-del").unwrap();
        let path = "projects/demo/tasks/a.md";

        record(
            &p,
            &id,
            &[entry_with_digest(path, JournalKind::Write, "aaa")],
        )
        .unwrap();
        record(&p, &id, &[entry(path, JournalKind::Delete)]).unwrap();

        let read = read_journal(&p, &id).unwrap();
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].kind, JournalKind::Delete);
        assert_eq!(
            read[0].digest, None,
            "a delete has no bytes, so it must not inherit the write's digest"
        );
    }

    /// The routing fact the delete guard rests on.
    ///
    /// A session that deletes a path and then recreates it within one
    /// uncommitted changeset must be handled by the scoped commit's *write*
    /// guard, not its delete guard — and it is, because `read_journal`'s
    /// `BTreeMap` collapse keeps only the last recorded kind per path. The
    /// delete becomes unreachable, so `ChangesetScope::deletes` never names
    /// the path and the delete-side presence check is never consulted for it.
    #[test]
    fn a_write_recorded_over_a_delete_collapses_to_write() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let id = SessionId::new("s-recreate").unwrap();
        let path = "projects/demo/tasks/a.md";

        record(&p, &id, &[entry(path, JournalKind::Delete)]).unwrap();
        record(
            &p,
            &id,
            &[entry_with_digest(path, JournalKind::Write, "ccc")],
        )
        .unwrap();

        let read = read_journal(&p, &id).unwrap();
        assert_eq!(read.len(), 1, "one path, one entry: {read:?}");
        assert_eq!(
            read[0].kind,
            JournalKind::Write,
            "the recreate is the last recorded kind, so the delete is collapsed away"
        );
        assert_eq!(
            read[0].digest.as_deref(),
            Some("ccc"),
            "and it carries the recreate's digest, so the WRITE guard can check it"
        );
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

        truncate(&p, &id, &[entry("a.md", JournalKind::Write)]).unwrap();
        assert_eq!(
            read_journal(&p, &id).unwrap(),
            vec![entry("b.md", JournalKind::Write)],
            "only the landed path is dropped"
        );
    }

    #[test]
    fn truncating_every_path_leaves_a_journal_that_claims_nothing() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let id = SessionId::new("s-trunc-all").unwrap();
        record(&p, &id, &[entry("a.md", JournalKind::Write)]).unwrap();

        truncate(&p, &id, &[entry("a.md", JournalKind::Write)]).unwrap();
        assert!(
            read_journal(&p, &id).unwrap().is_empty(),
            "the journal must claim nothing once everything landed"
        );
        // The FILE survives, deliberately: removing it is a rewrite, and a
        // rewrite can destroy a concurrent O_APPEND record. Cleanup is
        // `compact`/`gc_changesets`, at a quiescent moment.
        assert!(changeset_path(&p, &id).exists());
        // Idempotent: tombstoning an already-landed path claims nothing new.
        truncate(&p, &id, &[entry("a.md", JournalKind::Write)]).unwrap();
        assert!(read_journal(&p, &id).unwrap().is_empty());
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
        truncate(&p, &id, &[entry("shared.md", JournalKind::Write)]).unwrap();
        assert!(
            !read_journal(&p, &id)
                .unwrap()
                .iter()
                .any(|e| e.path == "shared.md"),
            "a landed path must not survive in the journal"
        );
    }

    #[test]
    fn an_append_during_truncation_survives() {
        // The defect this phase exists to close. Under one session id, a
        // parallel subagent's `record` and this session's commit-time
        // `truncate` are two processes writing the same file. The old
        // read-modify-write destroyed anything appended in between; two
        // O_APPEND lines cannot destroy each other.
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let id = SessionId::new("s-race").unwrap();
        let landed = entry_with_digest("a.md", JournalKind::Write, "aaa");
        record(&p, &id, std::slice::from_ref(&landed)).unwrap();

        // The interleave: the other process appends between the committer
        // reading the journal (it read exactly `landed`) and truncating it.
        record(
            &p,
            &id,
            &[entry_with_digest("b.md", JournalKind::Write, "bbb")],
        )
        .unwrap();
        truncate(&p, &id, &[landed]).unwrap();

        assert_eq!(
            read_journal(&p, &id).unwrap(),
            vec![entry_with_digest("b.md", JournalKind::Write, "bbb")],
            "the concurrently appended entry must survive the truncation"
        );
    }

    #[test]
    fn a_concurrent_rewrite_of_a_landed_path_survives_its_tombstone() {
        // The reported symptom, reduced: both sessions regenerate INDEX.md,
        // so the racing append names a path that IS in `landed`. A
        // path-keyed tombstone would sweep it and leave the file dirty and
        // unattributed; a content-keyed one keeps it, because those bytes
        // are not the bytes that landed.
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let id = SessionId::new("s-index").unwrap();
        let landed = entry_with_digest("INDEX.md", JournalKind::Write, "old");
        record(&p, &id, std::slice::from_ref(&landed)).unwrap();
        record(
            &p,
            &id,
            &[entry_with_digest("INDEX.md", JournalKind::Write, "new")],
        )
        .unwrap();

        truncate(&p, &id, &[landed]).unwrap();

        assert_eq!(
            read_journal(&p, &id).unwrap(),
            vec![entry_with_digest("INDEX.md", JournalKind::Write, "new")],
            "a newer record of a landed path describes content that has NOT landed"
        );
    }

    #[test]
    fn an_append_after_a_tombstone_resurrects_its_path() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let id = SessionId::new("s-resurrect").unwrap();
        let landed = entry_with_digest("a.md", JournalKind::Write, "aaa");
        record(&p, &id, std::slice::from_ref(&landed)).unwrap();
        truncate(&p, &id, &[landed]).unwrap();
        assert!(read_journal(&p, &id).unwrap().is_empty());

        record(
            &p,
            &id,
            &[entry_with_digest("a.md", JournalKind::Write, "ccc")],
        )
        .unwrap();
        assert_eq!(
            read_journal(&p, &id).unwrap(),
            vec![entry_with_digest("a.md", JournalKind::Write, "ccc")],
            "a write after the commit is a new uncommitted write"
        );
    }

    #[test]
    fn a_delete_then_recreate_spanning_a_tombstone_still_folds_to_write() {
        // Phase 9's delete guard depends on this routing: a path that ends up
        // `Write` reaches the commit's digest guard, never its working-tree
        // presence check. A tombstone in the middle must not change that.
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let id = SessionId::new("s-route").unwrap();
        let landed = entry("gone.md", JournalKind::Delete);
        record(&p, &id, std::slice::from_ref(&landed)).unwrap();
        truncate(&p, &id, &[landed]).unwrap();
        record(
            &p,
            &id,
            &[entry_with_digest("gone.md", JournalKind::Write, "back")],
        )
        .unwrap();

        let read = read_journal(&p, &id).unwrap();
        assert_eq!(read.len(), 1);
        assert_eq!(
            read[0].kind,
            JournalKind::Write,
            "recreation routes to Write"
        );
        assert_eq!(
            read[0].digest.as_deref(),
            Some("back"),
            "with its NEW digest"
        );
    }

    #[test]
    fn truncate_with_no_landed_paths_writes_nothing() {
        // A fully no-op commit must not grow the journal, exactly as an empty
        // flush records nothing.
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let id = SessionId::new("s-noop").unwrap();
        truncate(&p, &id, &[]).unwrap();
        assert!(!changeset_path(&p, &id).exists(), "no journal was created");

        record(&p, &id, &[entry("a.md", JournalKind::Write)]).unwrap();
        let before = std::fs::read_to_string(changeset_path(&p, &id)).unwrap();
        truncate(&p, &id, &[]).unwrap();
        assert_eq!(
            std::fs::read_to_string(changeset_path(&p, &id)).unwrap(),
            before,
            "an empty truncation appends nothing"
        );
    }

    #[test]
    fn a_tombstone_is_never_parsed_as_an_empty_batch() {
        // `JournalLine::paths` must stay required. If it ever gained
        // `#[serde(default)]`, every tombstone would parse as an empty batch
        // first, and truncation would silently become a no-op.
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let id = SessionId::new("s-discriminate").unwrap();
        record(&p, &id, &[entry("a.md", JournalKind::Write)]).unwrap();
        truncate(&p, &id, &[entry("a.md", JournalKind::Write)]).unwrap();

        let raw = std::fs::read_to_string(changeset_path(&p, &id)).unwrap();
        let tombstone = raw.lines().last().unwrap();
        assert!(
            tombstone.contains("\"landed\""),
            "the last line is a tombstone"
        );
        assert!(
            serde_json::from_str::<JournalLine>(tombstone).is_err(),
            "a tombstone must not parse as a batch line"
        );
        assert!(read_journal(&p, &id).unwrap().is_empty());
    }

    #[test]
    fn a_reader_that_predates_tombstones_over_claims_rather_than_losing() {
        // The cross-version edge: an OLDER rdm skips a `{"landed":…}` line as
        // unparsable, so it keeps claiming paths that already landed. That is
        // an over-claim — the safe direction — never a loss. Simulated here by
        // folding only the batch lines, which is exactly what the old reader
        // did.
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let id = SessionId::new("s-oldreader").unwrap();
        record(&p, &id, &[entry("a.md", JournalKind::Write)]).unwrap();
        truncate(&p, &id, &[entry("a.md", JournalKind::Write)]).unwrap();

        let raw = std::fs::read_to_string(changeset_path(&p, &id)).unwrap();
        let old_reader: Vec<String> = raw
            .lines()
            .filter_map(|l| serde_json::from_str::<JournalLine>(l).ok())
            .flat_map(|b| b.paths)
            .map(|e| e.path)
            .collect();
        assert_eq!(
            old_reader,
            vec!["a.md".to_string()],
            "an older reader still claims the landed path — over-claim, not loss"
        );
    }

    #[test]
    fn compaction_collapses_a_journal_and_removes_an_empty_one() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let id = SessionId::new("s-compact").unwrap();
        record(&p, &id, &[entry("a.md", JournalKind::Write)]).unwrap();
        record(&p, &id, &[entry("b.md", JournalKind::Write)]).unwrap();
        truncate(&p, &id, &[entry("a.md", JournalKind::Write)]).unwrap();

        assert!(
            !compact(&p, &id),
            "a non-empty journal is rewritten, not removed"
        );
        let raw = std::fs::read_to_string(changeset_path(&p, &id)).unwrap();
        assert_eq!(raw.lines().count(), 1, "collapsed to one line");
        assert_eq!(
            read_journal(&p, &id).unwrap(),
            vec![entry("b.md", JournalKind::Write)],
            "compaction preserves exactly what the fold claimed"
        );

        truncate(&p, &id, &[entry("b.md", JournalKind::Write)]).unwrap();
        assert!(compact(&p, &id), "a journal claiming nothing is removed");
        assert!(!changeset_path(&p, &id).exists());
    }

    #[test]
    fn compaction_skips_when_the_lock_is_already_held() {
        // The one place that diverges from rdm's proceed-anyway lock
        // convention: with no correctness mechanism underneath a rewrite, an
        // unheld guard must mean skip.
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let id = SessionId::new("s-locked").unwrap();
        record(&p, &id, &[entry("a.md", JournalKind::Write)]).unwrap();
        truncate(&p, &id, &[entry("a.md", JournalKind::Write)]).unwrap();

        std::fs::create_dir_all(p.changesets_dir()).unwrap();
        let held = p.changesets_dir().join(format!("{id}.lock"));
        std::fs::write(&held, "").unwrap();

        assert!(!compact(&p, &id), "compaction skipped");
        assert!(
            changeset_path(&p, &id).exists(),
            "and left the journal untouched rather than rewriting it unlocked"
        );
    }

    #[test]
    fn a_changeset_that_claims_nothing_is_not_listed_and_is_swept_by_gc() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let id = SessionId::new("s-empty").unwrap();
        record(&p, &id, &[entry("a.md", JournalKind::Write)]).unwrap();
        truncate(&p, &id, &[entry("a.md", JournalKind::Write)]).unwrap();

        let table = MapProcessTable::empty(1);
        assert!(
            list_changesets(&p, &table, None).unwrap().is_empty(),
            "a changeset with nothing to recover is not reported"
        );
        assert!(changeset_path(&p, &id).exists(), "the file is still there");

        assert_eq!(gc_changesets(&p, &table, None), 1);
        assert!(!changeset_path(&p, &id).exists(), "gc swept it");
    }

    #[test]
    fn gc_leaves_a_changeset_that_still_claims_a_path() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let id = SessionId::new("s-live-claim").unwrap();
        record(&p, &id, &[entry("a.md", JournalKind::Write)]).unwrap();

        let table = MapProcessTable::empty(1);
        assert_eq!(gc_changesets(&p, &table, None), 0);
        assert_eq!(
            read_journal(&p, &id).unwrap(),
            vec![entry("a.md", JournalKind::Write)]
        );
    }

    /// Installs the after-open seam for the duration of the returned guard.
    ///
    /// The seam runs between opening the journal for append and writing to it,
    /// which is exactly where a concurrent `compact` does its damage. Driving
    /// it directly is what makes these tests deterministic rather than a race
    /// two threads may or may not lose.
    fn with_after_open_hook(f: impl FnMut(&std::path::Path) + 'static) -> HookGuard {
        AFTER_OPEN_HOOK.with(|hook| *hook.borrow_mut() = Some(Box::new(f)));
        HookGuard
    }

    struct HookGuard;

    impl Drop for HookGuard {
        fn drop(&mut self) {
            AFTER_OPEN_HOOK.with(|hook| *hook.borrow_mut() = None);
        }
    }

    /// The blocking finding this fix answers.
    ///
    /// `gc_changesets` can only ask whether a live *lease* names a changeset,
    /// and rungs 1 and 3 never create one — so a `rdm session gc` from an
    /// unrelated process can compact a changeset that sibling processes are
    /// still appending to under one shared `RDM_SESSION`. Here compaction
    /// removes the journal outright (its fold is empty) in the window between
    /// this appender opening its descriptor and writing to it. The record must
    /// still be readable afterwards.
    #[test]
    fn an_append_survives_a_compaction_that_removed_the_journal_under_it() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let id = SessionId::new("s-swept").unwrap();

        // A fully-committed journal: the fold is empty, so compaction removes
        // the file rather than rewriting it.
        record(&p, &id, &[entry("a.md", JournalKind::Write)]).unwrap();
        truncate(&p, &id, &[entry("a.md", JournalKind::Write)]).unwrap();

        let swept = std::cell::Cell::new(false);
        let table = MapProcessTable::empty(1);
        {
            let p2 = SessionPaths::new(p.base().to_path_buf());
            let _guard = with_after_open_hook(move |_| {
                if swept.replace(true) {
                    return;
                }
                // A DIFFERENT process's `rdm session gc`: it names no lease,
                // shares no session id, and finds nothing that says "someone is
                // appending here right now" — because nothing can say that.
                assert_eq!(
                    gc_changesets(&p2, &MapProcessTable::empty(2), None),
                    1,
                    "the sweep must actually have removed the journal, or this \
test is not exercising the race"
                );
            });
            record(&p, &id, &[entry("b.md", JournalKind::Write)]).unwrap();
        }

        assert_eq!(
            read_journal(&p, &id).unwrap(),
            vec![entry("b.md", JournalKind::Write)],
            "the record appended across the sweep must not be lost"
        );
        assert!(
            !list_changesets(&p, &table, None).unwrap().is_empty(),
            "and it must be visible as recoverable work, not silently dropped"
        );
    }

    /// The rewrite half of the same race: compaction replaces the journal with
    /// a compacted one, so the appender's descriptor names an unlinked inode.
    #[test]
    fn an_append_survives_a_compaction_that_replaced_the_journal_under_it() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let id = SessionId::new("s-rewritten").unwrap();

        record(&p, &id, &[entry("a.md", JournalKind::Write)]).unwrap();
        record(&p, &id, &[entry("b.md", JournalKind::Write)]).unwrap();
        truncate(&p, &id, &[entry("a.md", JournalKind::Write)]).unwrap();

        let compacted = std::cell::Cell::new(false);
        {
            let p2 = SessionPaths::new(p.base().to_path_buf());
            let id2 = id.clone();
            let _guard = with_after_open_hook(move |_| {
                if compacted.replace(true) {
                    return;
                }
                assert!(
                    !compact(&p2, &id2),
                    "a journal that still claims a path is rewritten, not removed"
                );
            });
            record(&p, &id, &[entry("c.md", JournalKind::Write)]).unwrap();
        }

        let read = read_journal(&p, &id).unwrap();
        assert!(
            read.contains(&entry("b.md", JournalKind::Write)),
            "compaction must preserve what the fold already claimed: {read:?}"
        );
        assert!(
            read.contains(&entry("c.md", JournalKind::Write)),
            "and the record appended across the rewrite must not be lost: {read:?}"
        );
    }

    /// Truncation appends through the same path, so its tombstone survives a
    /// compaction too — otherwise a commit could silently fail to stop the
    /// journal claiming what it landed.
    #[test]
    fn a_tombstone_survives_a_compaction_that_replaced_the_journal_under_it() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let id = SessionId::new("s-tombstone-race").unwrap();

        record(&p, &id, &[entry("a.md", JournalKind::Write)]).unwrap();
        record(&p, &id, &[entry("b.md", JournalKind::Write)]).unwrap();

        let compacted = std::cell::Cell::new(false);
        {
            let p2 = SessionPaths::new(p.base().to_path_buf());
            let id2 = id.clone();
            let _guard = with_after_open_hook(move |_| {
                if compacted.replace(true) {
                    return;
                }
                let _ = compact(&p2, &id2);
            });
            truncate(&p, &id, &[entry("a.md", JournalKind::Write)]).unwrap();
        }

        assert_eq!(
            read_journal(&p, &id).unwrap(),
            vec![entry("b.md", JournalKind::Write)],
            "the tombstone must still have dropped exactly what landed"
        );
    }

    /// The redo is bounded, not conditional: a journal that is replaced on
    /// *every* attempt still terminates, writing at most `APPEND_ATTEMPTS`
    /// lines and never spinning.
    #[test]
    fn a_journal_replaced_on_every_attempt_still_terminates() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let id = SessionId::new("s-hostile").unwrap();
        std::fs::create_dir_all(p.changesets_dir()).unwrap();

        let attempts = std::rc::Rc::new(std::cell::Cell::new(0usize));
        {
            let seen = std::rc::Rc::clone(&attempts);
            let _guard = with_after_open_hook(move |path| {
                seen.set(seen.get() + 1);
                // Unlink the journal on every single attempt, so the identity
                // check can never succeed.
                let _ = std::fs::remove_file(path);
            });
            record(&p, &id, &[entry("a.md", JournalKind::Write)]).unwrap();
        }

        assert_eq!(
            attempts.get(),
            APPEND_ATTEMPTS,
            "the redo is bounded by APPEND_ATTEMPTS and stops there"
        );
    }

    #[test]
    fn gc_never_touches_the_callers_own_changeset() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let id = SessionId::new("s-mine").unwrap();
        record(&p, &id, &[entry("a.md", JournalKind::Write)]).unwrap();
        truncate(&p, &id, &[entry("a.md", JournalKind::Write)]).unwrap();

        let table = MapProcessTable::empty(1);
        assert_eq!(gc_changesets(&p, &table, Some(&id)), 0);
        assert!(
            changeset_path(&p, &id).exists(),
            "the caller is a live appender by definition"
        );
    }

    #[test]
    fn the_harness_barrier_is_inert_when_unset_or_empty() {
        // Asserted as a property of `truncate` itself rather than of the env
        // var: if the seam ever stopped being inert, every real commit would
        // stall for the 60s ceiling.
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let id = SessionId::new("s-barrier").unwrap();
        record(&p, &id, &[entry("a.md", JournalKind::Write)]).unwrap();

        let started = std::time::Instant::now();
        truncate(&p, &id, &[entry("a.md", JournalKind::Write)]).unwrap();
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "an unset barrier must not park a real commit"
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
    fn list_changeset_ids_returns_every_id_regardless_of_liveness() {
        // No lease, no `ProcessTable`, no `current` — `list_changeset_ids`
        // has no liveness concept at all, unlike `list_changesets`.
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let a = SessionId::new("s-a").unwrap();
        let b = SessionId::new("s-b").unwrap();
        record(&p, &a, &[entry("a.md", JournalKind::Write)]).unwrap();
        record(&p, &b, &[entry("b.md", JournalKind::Write)]).unwrap();

        let ids = list_changeset_ids(&p).unwrap();
        assert_eq!(
            ids.iter().map(SessionId::as_str).collect::<Vec<_>>(),
            vec!["s-a", "s-b"],
            "both ids come back, sorted, with no lease ever set up"
        );
    }

    #[test]
    fn adopt_repoints_the_caller_and_discard_removes_the_journal() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let table = MapProcessTable::chain(10, &[(20, "s20")]);
        let orphan = SessionId::new("s-orphan").unwrap();
        record(&p, &orphan, &[entry("a.md", JournalKind::Write)]).unwrap();

        adopt_changeset(&p, &table, &MapEnv::new(), &orphan).unwrap();
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
        let err =
            adopt_changeset(&p, &MapProcessTable::empty(10), &MapEnv::new(), &id).unwrap_err();
        assert!(err.to_string().contains("RDM_SESSION"));
    }

    #[test]
    fn adopt_with_a_harness_var_set_refuses_up_front() {
        // Since phase 10, rung 3 (harness) is checked before rung 2 (lease),
        // so repointing the parent lease would have no effect for a caller
        // whose environment carries a harness variable — the caller's next
        // `resolve_id` call would keep resolving its harness id and never
        // reach the repointed lease. Adoption must refuse loudly instead of
        // reporting a false success.
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let table = MapProcessTable::chain(10, &[(20, "s20")]);
        let orphan = SessionId::new("s-orphan").unwrap();
        record(&p, &orphan, &[entry("a.md", JournalKind::Write)]).unwrap();

        let env = MapEnv::new().with("CLAUDE_CODE_SESSION_ID", "child-one");
        let err = adopt_changeset(&p, &table, &env, &orphan).unwrap_err();
        assert!(err.to_string().contains("CLAUDE_CODE_SESSION_ID"));
        assert!(err.to_string().contains("RDM_SESSION"));

        // Non-vacuousness: nothing was written — the parent lease still
        // reads back whatever it held before (nothing, here), never the
        // orphan's id, proving the refusal happens before the repoint.
        assert_ne!(
            lease::adopt_inherited(&p, &table).map(|id| id.as_str().to_string()),
            Some("s-orphan".to_string())
        );
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

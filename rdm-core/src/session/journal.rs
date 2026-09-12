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
//!   no read-modify-write window, and neither kind of write can destroy the
//!   other. Compaction ([`compact`]) is the sole exception: it is the one
//!   operation that replaces or unlinks the file, and it is excluded from
//!   every append by a lock the kernel enforces. Compaction holds
//!   `<changesets>/journal.lock` exclusively across its read, its fold and its
//!   `rename`; every append holds the same lock shared across its open and its
//!   write (the protocol is documented on [`compact`]). Two appends never wait
//!   on each other; a
//!   compaction waits for in-flight appends and skips if it cannot get in; an
//!   append waits out a compaction and then opens the file it left behind. A
//!   lock the kernel owns is released the instant its holder dies, so there
//!   is no staleness horizon and no takeover, and losing an entry to
//!   compaction is excluded rather than made unlikely.
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
//! an explicit [`discard_changeset`] retires everything one claims. Journals that fold
//! to nothing are swept by [`gc_changesets`] (behind `rdm session gc`), which
//! skips changesets a live lease names. That skip is a courtesy, not a proof:
//! rungs 1 and 3 resolve an id without ever creating a lease, so a changeset
//! being appended to right now can read as unleased. What actually makes the
//! sweep safe is the journal lock, not the lease check: an append in flight
//! holds it shared, so the sweep cannot rewrite underneath it.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

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

/// The liveness state of a changeset when viewed from another session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LivenessState {
    /// This is the caller's own changeset (unflagged).
    Current,
    /// A live lease backs this changeset (unflagged).
    Live,
    /// This changeset has no lease file, so liveness cannot be determined
    /// from process metadata (labeled "unleased").
    Unleased,
    /// This changeset has a lease file, but the owning process is dead or
    /// has been recycled (labeled "orphaned").
    Orphaned,
}

impl LivenessState {
    /// Returns the label shown in human output, or None if unflagged.
    pub fn label(self) -> Option<&'static str> {
        match self {
            Self::Current | Self::Live => None,
            Self::Unleased => Some("unleased"),
            Self::Orphaned => Some("orphaned"),
        }
    }
}

/// A summary of one changeset on disk.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ChangesetSummary {
    /// The changeset (session) id.
    pub id: String,
    /// How many distinct paths the changeset has journaled.
    pub paths: usize,
    /// The liveness state of this changeset.
    pub liveness: LivenessState,
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

/// How long compaction waits for the journal lock before giving up.
///
/// Short on purpose: compaction is pure hygiene, so a contended lock should
/// cost the caller nothing. Losing this lock means *skip*, never *proceed
/// unlocked* — see [`compact`].
const COMPACT_LOCK_WAIT: Duration = Duration::from_millis(200);

/// How long an append waits for the journal lock before giving up.
///
/// The lock is only ever held against an append by a [`compact`] inside its
/// critical section — one read, one fold, one temporary-file write and one
/// `rename` — so a real wait is measured in microseconds and this bound sits
/// four orders of magnitude above it. It exists for a holder that is alive but
/// not running: a process stopped under a debugger, or parked on a harness
/// barrier nothing releases. The kernel releases a *dead* holder's lock on its
/// own, so that case never reaches the bound at all. Past it the append fails
/// **without writing** (see [`append_line`]) — the reported degradation the
/// module contract allows, never the silent one.
#[cfg(not(test))]
const APPEND_LOCK_WAIT: Duration = Duration::from_secs(10);

/// The test-time value of [`APPEND_LOCK_WAIT`], short enough for a unit test
/// to drive the deadline without dominating the suite's wall clock.
#[cfg(test)]
const APPEND_LOCK_WAIT: Duration = Duration::from_secs(1);

/// How long a lock attempt sleeps between polls of a contended lock.
const LOCK_POLL: Duration = Duration::from_millis(10);

/// The environment variable naming a harness barrier file for the append
/// itself.
///
/// When set, an append blocks between *opening* the journal and *writing* to
/// it, holding the journal lock shared the whole time. That is the window in
/// which a concurrent [`compact`] would have to replace or unlink the inode
/// the open descriptor names, and it is far too narrow for a harness to hit by
/// timing alone, so driving it needs a seam, exactly as
/// [`HARNESS_JOURNAL_BARRIER`] does for truncation. Parking there is what lets
/// a harness prove that a real `rdm session gc` from another process is
/// excluded rather than raced. Same contract: inert when unset or empty,
/// bounded by [`HARNESS_BARRIER_CEILING`] when set.
const HARNESS_APPEND_BARRIER: &str = "RDM_HARNESS_APPEND_BARRIER";

/// The environment variable naming a harness barrier file for compaction.
///
/// When set, [`compact`] blocks after its checks have passed and before it
/// does anything irreversible, holding the journal lock exclusively the whole
/// time. That is the window in which an append made *without* the lock would
/// land in the inode compaction is about to replace and be renamed away, and
/// it opens and closes inside one `rdm session gc` invocation, so driving it
/// across two real processes needs a seam. Same contract as the other two:
/// inert when unset or empty, bounded by [`HARNESS_BARRIER_CEILING`] when set.
const HARNESS_COMPACT_BARRIER: &str = "RDM_HARNESS_COMPACT_BARRIER";

/// Blocks until the barrier file named by `var` appears, or the ceiling
/// elapses.
///
/// Inert unless `var` is set to a non-empty value. Shared by all three seams —
/// [`HARNESS_JOURNAL_BARRIER`] around truncation (and so around the discard
/// that now routes through it), [`HARNESS_APPEND_BARRIER`] inside the append,
/// and [`HARNESS_COMPACT_BARRIER`] inside compaction — so their contract is
/// written once.
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

/// Appends one complete line to the journal at `path`, holding the journal
/// lock shared so that no [`compact`] can replace or unlink the file while the
/// line is being written.
///
/// The append itself is one `write_all` of one complete line to a descriptor
/// opened with `O_APPEND`, which POSIX does not interleave. That is what makes
/// two concurrent *appenders* safe from each other — and it is why the lock is
/// taken *shared*: appends never wait on one another.
///
/// What `O_APPEND` does not make them safe from is [`compact`], the one
/// operation in this module that replaces or unlinks the journal. Liveness
/// cannot rule that out. The two rungs this whole subsystem exists to serve —
/// an explicit `RDM_SESSION` and a harness-published id — create and read *no
/// lease at all* (see [`resolve_id`](super::resolve_session)), so "no live
/// lease names this changeset" is not evidence that nothing is appending to
/// it, and `rdm session gc` from an unrelated shell will compact a journal
/// several processes are writing to. So the two are excluded from each other
/// by a lock instead. [`compact`] holds [`journal_lock_path`] exclusively
/// from before it reads the journal until after it has renamed over it, and
/// this function holds the same lock shared from before it opens the journal
/// until after its write has returned. Taking the lock *before* the open is
/// what closes the window: the inode this function opens is the live journal
/// and stays the live journal until the lock is released. A compaction that
/// arrives in between waits or skips; a compaction already inside its critical
/// section holds this append off until it has finished, after which the open
/// finds the file it left behind. There is no instant at which an append can
/// land in an inode compaction is about to discard.
///
/// The lock is one the kernel enforces (`flock` on Unix, `LockFileEx` on
/// Windows), which is what makes this a guarantee rather than a bound: it is
/// released the moment its holder exits for any reason, so there is no
/// staleness horizon, no age-based takeover, and no double hold — the failure
/// modes an age-bounded advisory file would reintroduce.
///
/// Every writer in this module follows the protocol, including
/// [`discard_changeset`], which destroys a changeset deliberately and used to
/// do it with a bare `remove_file`. It now retires what it read through
/// [`truncate`], so it appends like everything else, and sweeps the file with
/// [`compact`], so it locks like everything else.
///
/// # Errors
///
/// Returns [`Error::Io`] if the lock file or the journal cannot be opened or
/// written, or if the lock stayed held against this append for longer than
/// [`APPEND_LOCK_WAIT`]. In that last case **nothing was written**: an append
/// made without the lock could land in an inode compaction is about to
/// discard, which is the silent loss this module exists to exclude, whereas a
/// missing record leaves the path on disk as unattributed work that
/// `rdm commit` names. Every caller records best-effort, so the mutation
/// itself still succeeds.
fn append_line(path: &std::path::Path, line: &str) -> Result<()> {
    let _lock = match lock_journal(path, LockMode::Shared, APPEND_LOCK_WAIT)? {
        JournalLock::Held(file) => Some(file),
        // No locking on this filesystem means no compaction on it either —
        // `compact` skips unless it holds the lock — so there is nothing to
        // exclude and the append is safe to make bare.
        JournalLock::Unsupported => None,
        JournalLock::Contended => {
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                format!(
                    "the changeset journal lock at {} stayed held by another \
process for {}s; the record was not written",
                    journal_lock_path(path).display(),
                    APPEND_LOCK_WAIT.as_secs()
                ),
            )));
        }
    };
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    harness_barrier(HARNESS_APPEND_BARRIER);
    run_after_open_hook(path);
    file.write_all(line.as_bytes())?;
    Ok(())
}

/// The one file every journal writer locks: `journal.lock`, beside the
/// journals in the changesets directory.
///
/// One lock for the whole directory rather than one per changeset, and never
/// removed. A per-changeset lock file would have to be unlinked eventually to
/// keep the directory bounded, and unlinking a lock file is exactly what
/// breaks kernel-level exclusion: a process that opened the old inode and a
/// process that created the new one hold locks that do not conflict. A single
/// file nothing ever removes has no such seam, and it costs nothing in
/// contention — compaction is confined to `rdm session gc` and
/// `rdm session discard`, and appends take the lock shared.
fn journal_lock_path(journal: &std::path::Path) -> PathBuf {
    journal.with_file_name("journal.lock")
}

/// Which side of the journal lock a caller wants.
#[derive(Clone, Copy)]
enum LockMode {
    /// An append: any number may hold it at once, none while a compaction does.
    Shared,
    /// A compaction: held alone, or not at all.
    Exclusive,
}

/// The outcome of one bounded attempt to take the journal lock.
enum JournalLock {
    /// Held, for exactly as long as the file lives.
    Held(std::fs::File),
    /// Another holder kept it for the whole wait.
    Contended,
    /// The filesystem does not support locking at all.
    Unsupported,
}

/// Takes the journal lock beside `journal` in `mode`, polling a contended
/// lock for up to `wait`.
///
/// The lock is the kernel's (`File::try_lock` / `File::try_lock_shared`), so
/// a holder that exits — cleanly or not — releases it on the spot, and no
/// caller ever needs to guess whether a holder is still alive.
///
/// # Errors
///
/// Returns the I/O error if the lock file cannot be opened or created.
fn lock_journal(
    journal: &std::path::Path,
    mode: LockMode,
    wait: Duration,
) -> std::io::Result<JournalLock> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(journal_lock_path(journal))?;
    let deadline = std::time::Instant::now() + wait;
    loop {
        let attempt = match mode {
            LockMode::Shared => file.try_lock_shared(),
            LockMode::Exclusive => file.try_lock(),
        };
        match attempt {
            Ok(()) => return Ok(JournalLock::Held(file)),
            Err(std::fs::TryLockError::WouldBlock) => {
                if std::time::Instant::now() >= deadline {
                    return Ok(JournalLock::Contended);
                }
                std::thread::sleep(LOCK_POLL);
            }
            Err(std::fs::TryLockError::Error(e)) if e.kind() == std::io::ErrorKind::Interrupted => {
            }
            Err(std::fs::TryLockError::Error(_)) => return Ok(JournalLock::Unsupported),
        }
    }
}

/// A test-only callback run at one of this module's race seams.
#[cfg(test)]
type PathHook = Box<dyn FnMut(&std::path::Path)>;

/// The type one of the seam slots below has.
#[cfg(test)]
type HookSlot = std::cell::RefCell<Option<PathHook>>;

#[cfg(test)]
thread_local! {
    /// Test-only seam run between opening the journal for append and writing
    /// to it, so a unit test can drive the exact interleave a concurrent
    /// [`compact`] creates rather than racing threads for a window measured in
    /// microseconds.
    static AFTER_OPEN_HOOK: HookSlot = const { std::cell::RefCell::new(None) };

    /// Test-only seam run inside [`compact`] between reading-and-folding the
    /// journal and the compare-and-swap on its byte length — the window an
    /// appended line has to land in for that check to *fail*. Without it the
    /// mismatch branch is reachable only by timing luck, which is the one kind
    /// of coverage the rest of this module deliberately does without.
    static AFTER_FOLD_HOOK: HookSlot = const { std::cell::RefCell::new(None) };

    /// Test-only seam run inside [`compact`] immediately before its
    /// irreversible step — after its compare-and-swap, with the journal lock
    /// held — so a unit test can drive an append or a second compaction into
    /// exactly the window the lock exists to close.
    static BEFORE_REWRITE_HOOK: HookSlot = const { std::cell::RefCell::new(None) };
}

/// Runs whichever seam `slot` holds, if one is installed.
#[cfg(test)]
fn run_hook(slot: &'static std::thread::LocalKey<HookSlot>, path: &std::path::Path) {
    slot.with(|hook| {
        if let Some(f) = hook.borrow_mut().as_mut() {
            f(path);
        }
    });
}

/// Runs the test-only after-open seam, if one is installed.
#[cfg(test)]
fn run_after_open_hook(path: &std::path::Path) {
    run_hook(&AFTER_OPEN_HOOK, path);
}

/// Runs the test-only post-fold seam, if one is installed.
#[cfg(test)]
fn run_after_fold_hook(path: &std::path::Path) {
    run_hook(&AFTER_FOLD_HOOK, path);
}

/// Runs the test-only pre-rewrite seam, if one is installed.
#[cfg(test)]
fn run_before_rewrite_hook(path: &std::path::Path) {
    run_hook(&BEFORE_REWRITE_HOOK, path);
}

/// The seams compile away entirely outside tests.
#[cfg(not(test))]
#[inline]
fn run_after_open_hook(_path: &std::path::Path) {}

/// The seams compile away entirely outside tests.
#[cfg(not(test))]
#[inline]
fn run_after_fold_hook(_path: &std::path::Path) {}

/// The seams compile away entirely outside tests.
#[cfg(not(test))]
#[inline]
fn run_before_rewrite_hook(_path: &std::path::Path) {}

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
/// Blocks at `RDM_HARNESS_APPEND_BARRIER` when that variable names a file
/// (a test seam; inert otherwise).
///
/// # Errors
///
/// Returns [`Error::Io`] if the state directory cannot be created or the
/// append fails — including the one case a caller is least likely to expect:
/// the journal lock stayed held against this append for longer than
/// `APPEND_LOCK_WAIT` (10 s). Every append takes `<changesets>/journal.lock`
/// shared, and a running [`compact`] holds it exclusively for microseconds,
/// so that wait is only ever exhausted by a compaction that is alive but not
/// running (stopped under a debugger, parked on a harness barrier). When it
/// is, **nothing is written**: the error means the batch was deliberately not
/// recorded, so its paths surface as unattributed in the next `rdm commit`
/// rather than being written into a file about to be thrown away. Callers on
/// the mutation path swallow this deliberately — journaling is best-effort.
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
    // single-line appends non-interleaving. The shared journal lock inside
    // `append_line` covers the one thing O_APPEND does not: a concurrent
    // `compact` swapping the inode out from under this descriptor.
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
/// is the whole point: under one session id, a parallel subagent is a
/// *different process* appending to this same file, so the previous
/// read-modify-write silently destroyed anything recorded between its read
/// and its write. Two single-line appends cannot destroy each other, so the
/// window is gone rather than narrowed.
///
/// Truncation is content-keyed. The tombstone carries whole entries, and
/// [`read_journal`]'s fold drops a path only when what it currently holds is
/// exactly what landed. So a concurrent session that rewrote one of these
/// paths in between keeps its record and its next commit still lands it.
///
/// An empty `landed` slice writes nothing at all, mirroring [`record`], so a
/// fully no-op commit never grows the journal.
///
/// The file is not removed even when nothing survives; see [`compact`] for
/// why cleanup is deliberately deferred to a quiescent moment.
///
/// [`discard_changeset`] reuses this for the opposite reason — retiring what a
/// deliberately destroyed changeset claims rather than what a commit landed —
/// so that a discard, too, cannot take a sibling's concurrent append with it.
///
/// Blocks at `RDM_HARNESS_JOURNAL_BARRIER` when that variable names a file
/// (a test seam; inert otherwise).
///
/// # Errors
///
/// Returns [`Error::Io`] if the state directory cannot be created or the
/// append fails, including the bounded wait on the journal lock described
/// under [`record`] — a tombstone is an append like any other, and on that
/// wait expiring nothing is written. Callers on the commit path swallow this:
/// the commit itself has already landed, and failing afterwards would be
/// worse than a stale journal — the paths simply stay journaled, which
/// over-claims rather than loses.
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
    // under the same shared lock that holds compaction off mid-append.
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
/// which parallel subagents share a changeset, so an unleased changeset may
/// well have several processes appending to it.
///
/// Two guards make it safe anyway, and the first is the one that carries the
/// guarantee:
///
/// - **The journal lock, held exclusively.** Every append holds
///   `<changesets>/journal.lock` shared from before it opens the journal until its
///   write has returned; this call holds it exclusively from before it reads
///   the journal until after its `rename` or `remove_file`. So an append in
///   flight keeps this call out, and this call keeps every later append
///   waiting until the file it will find on open is the one that survives.
///   Unlike every [`AdvisoryLock`](crate::lock::AdvisoryLock) user in rdm, a
///   lock this call cannot take means **skip**, not proceed unlocked: those
///   callers are safe because a real correctness mechanism sits underneath
///   (HEAD compare-and-swap; the content-digest precondition), and a journal
///   rewrite has no such underlayer. A filesystem that cannot lock at all is
///   the same answer — no compaction ever runs there, which is what lets an
///   append proceed bare on it.
/// - A compare-and-swap on the file's byte length. A journal only ever grows,
///   so its length is a valid version token: if anything was appended between
///   the fold and the rewrite, the length differs and compaction skips. Under
///   the lock no rdm writer can append there, so against rdm's own writers
///   this never fires; it is kept because it is one `stat`, and it turns a
///   write made by something that did not take the lock — a foreign process
///   editing the file by hand — into a skip rather than a loss.
///
/// Losing the compare-and-swap repeatedly under sustained load leaves the
/// journal growing: one small line per flushed batch plus one per commit.
/// That is a size concern, not a correctness one.
///
/// Blocks at `RDM_HARNESS_COMPACT_BARRIER` when that variable names a file
/// (a test seam; inert otherwise).
pub fn compact(paths: &SessionPaths, id: &SessionId) -> bool {
    let path = changeset_path(paths, id);
    let _lock = match lock_journal(&path, LockMode::Exclusive, COMPACT_LOCK_WAIT) {
        Ok(JournalLock::Held(file)) => file,
        Ok(JournalLock::Contended | JournalLock::Unsupported) | Err(_) => return false,
    };
    // Holding the lock proves no compaction of any journal here is mid-rename,
    // so a temporary file left by one killed between its write and its
    // rename is an orphan; sweep it rather than let it accumulate forever.
    let tmp = path.with_extension("jsonl.compacting");
    let _ = std::fs::remove_file(&tmp);
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return false;
    };
    let folded = fold_lines(&raw);
    run_after_fold_hook(&path);
    // The compare-and-swap: anything appended since the read makes the length
    // differ, and losing means skip.
    let unchanged = std::fs::metadata(&path)
        .map(|md| md.len() == raw.len() as u64)
        .unwrap_or(false);
    if !unchanged {
        return false;
    }
    // The last instant at which this call has done nothing irreversible: every
    // check has passed and neither exit has been taken, and the lock is held.
    // Parking here is what lets a harness prove an append made meanwhile
    // waits rather than writes.
    harness_barrier(HARNESS_COMPACT_BARRIER);
    if folded.is_empty() {
        run_before_rewrite_hook(&path);
        return std::fs::remove_file(&path).is_ok();
    }
    let entries: Vec<JournalEntry> = folded.into_values().collect();
    let Ok(line) = serde_json::to_string(&JournalLine { paths: entries }) else {
        return false;
    };
    if std::fs::write(&tmp, format!("{line}\n")).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return false;
    }
    run_before_rewrite_hook(&path);
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
/// Safety comes from the journal lock instead (the protocol is documented on
/// [`compact`]): an append in flight holds it shared, so a sweep that arrives
/// while one is running cannot take the exclusive hold it needs and skips that
/// changeset; a sweep already inside its critical section holds the append
/// off until it has finished, after which the append opens the file the sweep
/// left behind. That holds for every rung, leased or not, and for a
/// `rdm session gc` run from a process that shares nothing with the sessions
/// it is sweeping — so a sweep can slow a concurrent appender down, never
/// cost it its record.
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

/// Lists every changeset on disk, reporting their liveness states.
///
/// A changeset's liveness is determined as follows:
/// - [`LivenessState::Current`] if it is the caller's own changeset.
/// - [`LivenessState::Live`] if a live lease backs it.
/// - [`LivenessState::Orphaned`] if it has a lease file but the owning process is dead.
/// - [`LivenessState::Unleased`] if it has no lease file (liveness unknown).
///
/// A changeset whose journal folds to nothing is **omitted**. It claims no
/// path, so there is nothing for [`adopt_changeset`] or `rdm commit
/// --changeset` to recover, and reporting it — especially with a liveness
/// label — would be noise pointing at no work. This keeps the listing identical to
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
    let dead = lease::dead_lease_ids(paths, procs);
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let Some(stem) = name.strip_suffix(".jsonl") else {
            continue;
        };
        let Some(id) = SessionId::new(stem) else {
            continue;
        };
        let claimed = read_journal(paths, &id)?.len();
        if claimed == 0 {
            continue;
        }
        let liveness = if current.is_some_and(|c| c.as_str() == id.as_str()) {
            LivenessState::Current
        } else if live.contains(id.as_str()) {
            LivenessState::Live
        } else if dead.contains(id.as_str()) {
            LivenessState::Orphaned
        } else {
            LivenessState::Unleased
        };
        out.push(ChangesetSummary {
            paths: claimed,
            liveness,
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
/// phase 10 reordered the identity chain ([`super::resolve_session`]) to check a harness variable
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

/// Retires everything `id`'s journal claims, so the changeset owns nothing
/// afterwards.
///
/// Returns `false` when it already claimed nothing — including when there is
/// no journal at all — so `rdm session discard` can still tell a real discard
/// from a no-op.
///
/// Written as a [`truncate`] over what the journal currently claims, **not**
/// as a `remove_file`. That distinction is the difference between destroying a
/// changeset and destroying whatever happens to be in the file when the
/// removal lands: under one shared session id a sibling process may be
/// appending *while* this runs, and an unlink takes its record with it. That
/// is precisely the loss the journal lock (see [`compact`]) exists to prevent,
/// arriving from the one writer that used to ignore the protocol outright — so
/// the fix is to stop ignoring it rather than to document the hole. Going
/// through [`truncate`] also makes the retirement content-keyed: a path
/// recorded again with different content between this read and the tombstone
/// keeps its record, and a record appended after the tombstone resurrects its
/// path.
///
/// The file itself is then swept by an ordinary [`compact`], which removes it
/// when the fold is empty and declines when it cannot take the compaction lock
/// or loses its compare-and-swap. So the quiescent case — every real
/// `rdm session discard --force` — still leaves no journal behind, and the
/// contended case degrades to a journal that outlives its claims until
/// `rdm session gc`, never to a lost append.
///
/// # Errors
///
/// Returns [`Error::Io`] if the journal exists but cannot be read, or if the
/// tombstone cannot be appended — including the bounded journal-lock wait
/// described under [`record`], in which case nothing was retired.
pub fn discard_changeset(paths: &SessionPaths, id: &SessionId) -> Result<bool> {
    let claimed = read_journal(paths, id)?;
    retire(paths, id, &claimed)?;
    Ok(!claimed.is_empty())
}

/// Retires exactly `claimed` from `id`'s journal — the entries a caller read
/// and has since acted on — and then sweeps the file.
///
/// The content-keyed half of [`discard_changeset`], split out for a caller
/// that restores paths from a read of its *own*: the git store's scoped
/// `rdm discard` reads the journal, restores those paths to HEAD, and only
/// then retires them. Retiring on a fresh read at that point would take a
/// sibling's record appended in between — a path the store never restored,
/// now claimed by nobody, left on disk as unattributed dirt. Passing what was
/// actually read keeps that record: [`truncate`]'s tombstone is content-keyed,
/// so a path recorded after the read, or re-recorded with different content,
/// keeps its claim and its next commit lands it.
///
/// An empty `claimed` retires nothing and still sweeps, so a discard of an
/// already-committed changeset leaves the directory as clean as it used to.
///
/// # Errors
///
/// Returns [`Error::Io`] if the tombstone cannot be appended — including the
/// bounded journal-lock wait described under [`record`], in which case
/// nothing was retired.
pub fn retire(paths: &SessionPaths, id: &SessionId, claimed: &[JournalEntry]) -> Result<()> {
    if !claimed.is_empty() {
        truncate(paths, id, claimed)?;
    }
    let _ = compact(paths, id);
    Ok(())
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
        // The reported symptom, reduced: two sessions write the same shared
        // path, so the racing append names a path that IS in `landed`. A
        // path-keyed tombstone would sweep it and leave the file dirty and
        // unattributed; a content-keyed one keeps it, because those bytes
        // are not the bytes that landed. (`INDEX.md` below is just a
        // conveniently-shared path — rdm gives it no special status.)
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
    fn compaction_skips_when_an_append_holds_the_lock() {
        // The one place that diverges from rdm's proceed-anyway lock
        // convention: with no correctness mechanism underneath a rewrite, a
        // lock this call cannot take must mean skip.
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let id = SessionId::new("s-locked").unwrap();
        record(&p, &id, &[entry("a.md", JournalKind::Write)]).unwrap();
        truncate(&p, &id, &[entry("a.md", JournalKind::Write)]).unwrap();

        // An append in flight: the shared side of the lock, held from a
        // descriptor of its own, exactly as a sibling process would hold it.
        let appender = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(journal_lock_path(&changeset_path(&p, &id)))
            .unwrap();
        appender.lock_shared().unwrap();

        assert!(!compact(&p, &id), "compaction skipped");
        assert!(
            changeset_path(&p, &id).exists(),
            "and left the journal untouched rather than rewriting it unlocked"
        );
    }

    /// The compare-and-swap guard, driven at its own seam.
    ///
    /// Under the journal lock no rdm writer can append between compaction's
    /// fold and its rewrite, so the only thing that can grow the file there
    /// is a writer that never took the lock — a foreign process editing the
    /// journal by hand. The length compare-and-swap is what notices, and it
    /// must turn that into a skip rather than a rewrite that drops the line.
    /// Every other race window in this module is driven deterministically
    /// through a seam rather than raced for, and this one is no different:
    /// without the seam the mismatch branch is only ever reached by timing
    /// luck, so a CAS computed backwards would pass the suite.
    #[test]
    fn compaction_skips_when_the_journal_grew_between_its_fold_and_its_rewrite() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let id = SessionId::new("s-cas").unwrap();
        record(&p, &id, &[entry("a.md", JournalKind::Write)]).unwrap();
        record(&p, &id, &[entry("b.md", JournalKind::Write)]).unwrap();
        let path = changeset_path(&p, &id);
        let before = std::fs::read_to_string(&path).unwrap();

        let raced = std::cell::Cell::new(false);
        {
            let _guard = with_after_fold_hook(move |journal| {
                if raced.replace(true) {
                    return;
                }
                // A writer outside the protocol: a bare O_APPEND write that
                // never asked for the lock this compaction is holding.
                let mut foreign = std::fs::OpenOptions::new()
                    .append(true)
                    .open(journal)
                    .unwrap();
                let line = serde_json::to_string(&JournalLine {
                    paths: vec![entry("c.md", JournalKind::Write)],
                })
                .unwrap();
                foreign.write_all(format!("{line}\n").as_bytes()).unwrap();
            });
            assert!(
                !compact(&p, &id),
                "a compaction whose snapshot went stale must not report a removal"
            );
        }

        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(
            raw.starts_with(&before),
            "the rewrite must have been abandoned, not applied to a stale fold"
        );
        let claimed = read_journal(&p, &id).unwrap();
        assert!(
            claimed.contains(&entry("c.md", JournalKind::Write)),
            "the line appended across the fold must survive: {claimed:?}"
        );
        assert_eq!(claimed.len(), 3, "and so must the two that preceded it");
    }

    /// The blocking finding this fix answers.
    ///
    /// Compaction's length compare-and-swap is checked once, before it builds
    /// its replacement file, and nothing re-checks the journal immediately
    /// before the `rename`. An append that lands in that window — after the
    /// CAS, before the rename — would be written into the inode compaction is
    /// about to discard, and would report success. So the window must not be
    /// *reachable*: an append that arrives while compaction holds the lock
    /// has to wait, and then write into the file the rename left behind.
    #[test]
    fn an_append_arriving_between_the_cas_and_the_rename_waits_and_lands() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let id = SessionId::new("s-cas-window").unwrap();
        record(&p, &id, &[entry("a.md", JournalKind::Write)]).unwrap();
        record(&p, &id, &[entry("b.md", JournalKind::Write)]).unwrap();
        let path = changeset_path(&p, &id);

        let (tx, rx) = std::sync::mpsc::channel();
        let raced = std::cell::Cell::new(false);
        {
            let p2 = SessionPaths::new(p.base().to_path_buf());
            let id2 = id.clone();
            let _guard = with_before_rewrite_hook(move |_| {
                if raced.replace(true) {
                    return;
                }
                // The appender: a sibling process arriving exactly in the
                // window, after the CAS passed and before the rename.
                let (p3, id3, tx) = (
                    SessionPaths::new(p2.base().to_path_buf()),
                    id2.clone(),
                    tx.clone(),
                );
                std::thread::spawn(move || {
                    let outcome = record(&p3, &id3, &[entry("c.md", JournalKind::Write)]);
                    tx.send(outcome).unwrap();
                });
                // Give it every chance to write blind. It must instead be
                // waiting on the lock this compaction holds.
                std::thread::sleep(Duration::from_millis(100));
                assert!(
                    matches!(rx.try_recv(), Err(std::sync::mpsc::TryRecvError::Empty)),
                    "the append completed while compaction held the lock — \
nothing excluded it from the CAS-to-rename window"
                );
            });
            assert!(
                !compact(&p, &id),
                "a non-empty journal is rewritten, not removed"
            );
        }

        // The append lands only once the lock is released, into the rewritten
        // journal. (The receiver lives in the hook above; the thread's own
        // result is observed through the journal.)
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let claimed = loop {
            let claimed = read_journal(&p, &id).unwrap();
            if claimed.contains(&entry("c.md", JournalKind::Write))
                || std::time::Instant::now() >= deadline
            {
                break claimed;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        assert!(
            claimed.contains(&entry("c.md", JournalKind::Write)),
            "the append made in the window must survive the rename: {claimed:?}"
        );
        assert!(
            claimed.contains(&entry("a.md", JournalKind::Write))
                && claimed.contains(&entry("b.md", JournalKind::Write)),
            "and compaction must have kept what it folded: {claimed:?}"
        );
        let raw = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            raw.lines().count(),
            2,
            "the compacted line, then the append written after it: {raw}"
        );
    }

    /// Two compactions never run at once: the second cannot take the lock the
    /// first holds, and skips.
    #[test]
    fn a_second_compaction_cannot_start_while_the_first_holds_the_lock() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let id = SessionId::new("s-two-sweeps").unwrap();
        record(&p, &id, &[entry("a.md", JournalKind::Write)]).unwrap();
        record(&p, &id, &[entry("b.md", JournalKind::Write)]).unwrap();
        let path = changeset_path(&p, &id);

        let inner = std::rc::Rc::new(std::cell::Cell::new(None));
        {
            let seen = std::rc::Rc::clone(&inner);
            let p2 = SessionPaths::new(p.base().to_path_buf());
            let id2 = id.clone();
            let _guard = with_before_rewrite_hook(move |_| {
                if seen.get().is_some() {
                    return;
                }
                // A second sweep, arriving while the first holds the lock.
                seen.set(Some(compact(&p2, &id2)));
            });
            assert!(!compact(&p, &id));
        }

        assert_eq!(
            inner.get(),
            Some(false),
            "the second compaction must have run and reported that it did nothing"
        );
        let raw = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            raw.lines().count(),
            1,
            "exactly one rewrite happened: {raw}"
        );
        assert!(
            !path.with_extension("jsonl.compacting").exists(),
            "and neither left a temporary file behind"
        );
    }

    /// The bound on waiting is a bound on *waiting*, never a licence to write
    /// blind: an append that cannot take the lock within `APPEND_LOCK_WAIT`
    /// reports failure and leaves the journal exactly as it found it.
    #[test]
    fn an_append_that_cannot_take_the_lock_in_time_fails_rather_than_writing_blind() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let id = SessionId::new("s-held-too-long").unwrap();
        record(&p, &id, &[entry("a.md", JournalKind::Write)]).unwrap();
        let path = changeset_path(&p, &id);
        let before = std::fs::read_to_string(&path).unwrap();

        // A compaction that is alive but not running — stopped under a
        // debugger, say — holding the exclusive side indefinitely.
        let stuck = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(journal_lock_path(&path))
            .unwrap();
        stuck.lock().unwrap();

        let started = std::time::Instant::now();
        let outcome = record(&p, &id, &[entry("b.md", JournalKind::Write)]);
        assert!(
            started.elapsed() >= APPEND_LOCK_WAIT,
            "the append must have waited the full bound before giving up"
        );
        assert!(outcome.is_err(), "and must report that it did not record");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            before,
            "with nothing written: a blind write into a doomed inode is the \
silent loss this module exists to exclude"
        );
        drop(stuck);

        record(&p, &id, &[entry("b.md", JournalKind::Write)]).unwrap();
        assert!(
            read_journal(&p, &id)
                .unwrap()
                .contains(&entry("b.md", JournalKind::Write)),
            "once the holder is gone the same append lands"
        );
    }

    /// The store's scoped discard restores paths from a read of its own and
    /// retires them afterwards. A sibling's record appended in between must
    /// not be retired with them: it names a path the store never restored.
    #[test]
    fn retire_takes_only_what_was_read_and_keeps_a_record_appended_since() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let id = SessionId::new("s-store-discard").unwrap();
        record(&p, &id, &[entry("a.md", JournalKind::Write)]).unwrap();

        // (1) the discarder's read, which is what it restores from.
        let read = read_journal(&p, &id).unwrap();
        assert_eq!(read, vec![entry("a.md", JournalKind::Write)]);

        // (2) a sibling under the same id, mid-discard.
        record(&p, &id, &[entry("b.md", JournalKind::Write)]).unwrap();

        // (3) the discarder retires exactly what it read.
        retire(&p, &id, &read).unwrap();

        assert_eq!(
            read_journal(&p, &id).unwrap(),
            vec![entry("b.md", JournalKind::Write)],
            "the record appended after the read must keep its claim"
        );
        assert!(
            changeset_path(&p, &id).exists(),
            "and the journal survives because it still claims something"
        );

        // With nothing appended in between, retiring what was read leaves a
        // journal that claims nothing, and the sweep removes it.
        let read = read_journal(&p, &id).unwrap();
        retire(&p, &id, &read).unwrap();
        assert!(read_journal(&p, &id).unwrap().is_empty());
        assert!(
            !changeset_path(&p, &id).exists(),
            "swept once it claims nothing"
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
        install_hook(&AFTER_OPEN_HOOK, f)
    }

    /// Installs a seam inside `compact` between its fold and its length
    /// compare-and-swap, so a test can drive the CAS-mismatch branch.
    fn with_after_fold_hook(f: impl FnMut(&std::path::Path) + 'static) -> HookGuard {
        install_hook(&AFTER_FOLD_HOOK, f)
    }

    /// Installs a seam inside `compact` immediately before its irreversible
    /// step — after its compare-and-swap, with the lock held — so a test can
    /// drive an append or a second compaction into exactly that window.
    fn with_before_rewrite_hook(f: impl FnMut(&std::path::Path) + 'static) -> HookGuard {
        install_hook(&BEFORE_REWRITE_HOOK, f)
    }

    fn install_hook(
        slot: &'static std::thread::LocalKey<HookSlot>,
        f: impl FnMut(&std::path::Path) + 'static,
    ) -> HookGuard {
        slot.with(|hook| *hook.borrow_mut() = Some(Box::new(f)));
        HookGuard { slot }
    }

    struct HookGuard {
        slot: &'static std::thread::LocalKey<HookSlot>,
    }

    impl Drop for HookGuard {
        fn drop(&mut self) {
            self.slot.with(|hook| *hook.borrow_mut() = None);
        }
    }

    /// The earlier blocking finding this module answers.
    ///
    /// `gc_changesets` can only ask whether a live *lease* names a changeset,
    /// and rungs 1 and 3 never create one — so a `rdm session gc` from an
    /// unrelated process will try to compact a changeset that sibling
    /// processes are still appending to under one shared `RDM_SESSION`. Here
    /// the sweep would remove the journal outright (its fold is empty) in the
    /// window between this appender opening its descriptor and writing to it.
    /// It must be excluded instead: the appender holds the lock shared across
    /// that window, so the sweep cannot take it, and the record lands.
    #[test]
    fn a_sweep_arriving_during_an_append_is_excluded_not_raced() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let id = SessionId::new("s-swept").unwrap();

        // A fully-committed journal: the fold is empty, so an unexcluded sweep
        // would remove the file rather than rewrite it.
        record(&p, &id, &[entry("a.md", JournalKind::Write)]).unwrap();
        truncate(&p, &id, &[entry("a.md", JournalKind::Write)]).unwrap();

        let swept = std::cell::Cell::new(None);
        let table = MapProcessTable::empty(1);
        {
            let p2 = SessionPaths::new(p.base().to_path_buf());
            let _guard = with_after_open_hook(move |_| {
                if swept.get().is_some() {
                    return;
                }
                // A DIFFERENT process's `rdm session gc`: it names no lease,
                // shares no session id, and finds nothing that says "someone is
                // appending here right now" — because nothing can say that.
                swept.set(Some(gc_changesets(&p2, &MapProcessTable::empty(2), None)));
                assert_eq!(
                    swept.get(),
                    Some(0),
                    "the sweep must have been held off by the appender's lock, \
not raced against it"
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

    /// The rewrite half of the same race: a compaction that would replace the
    /// journal under the appender's descriptor is held off instead.
    #[test]
    fn a_rewrite_arriving_during_an_append_is_excluded_not_raced() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let id = SessionId::new("s-rewritten").unwrap();

        record(&p, &id, &[entry("a.md", JournalKind::Write)]).unwrap();
        record(&p, &id, &[entry("b.md", JournalKind::Write)]).unwrap();
        truncate(&p, &id, &[entry("a.md", JournalKind::Write)]).unwrap();
        let path = changeset_path(&p, &id);
        let lines_before = std::fs::read_to_string(&path).unwrap().lines().count();

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
                    "a rewrite is never reported as a removal, excluded or not"
                );
            });
            record(&p, &id, &[entry("c.md", JournalKind::Write)]).unwrap();
        }

        let raw = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            raw.lines().count(),
            lines_before + 1,
            "the journal grew by the append and was not collapsed under it: {raw}"
        );
        let read = read_journal(&p, &id).unwrap();
        assert!(
            read.contains(&entry("b.md", JournalKind::Write)),
            "what the fold claimed is still claimed: {read:?}"
        );
        assert!(
            read.contains(&entry("c.md", JournalKind::Write)),
            "and the record appended across the attempted rewrite is there: {read:?}"
        );
    }

    /// Truncation appends through the same path, so its tombstone holds a
    /// compaction off too — otherwise a commit could silently fail to stop the
    /// journal claiming what it landed.
    #[test]
    fn a_tombstone_holds_a_compaction_off_while_it_is_written() {
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
        // Asserted as a property of the operations themselves rather than of
        // the env vars: if a seam ever stopped being inert, every real commit
        // — or every `rdm session gc` — would stall for the 60s ceiling.
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let id = SessionId::new("s-barrier").unwrap();
        record(&p, &id, &[entry("a.md", JournalKind::Write)]).unwrap();

        let started = std::time::Instant::now();
        // `record` covers HARNESS_APPEND_BARRIER, `truncate` covers
        // HARNESS_JOURNAL_BARRIER, `compact` covers HARNESS_COMPACT_BARRIER.
        truncate(&p, &id, &[entry("a.md", JournalKind::Write)]).unwrap();
        record(&p, &id, &[entry("b.md", JournalKind::Write)]).unwrap();
        compact(&p, &id);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "an unset barrier must not park a real commit or sweep"
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
    fn list_flags_unleased_changesets_as_unleased_not_orphaned() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let table = MapProcessTable::chain(10, &[(20, "s20")]);

        let live = lease::create_at_parent(&p, &table).unwrap();
        record(&p, &live, &[entry("a.md", JournalKind::Write)]).unwrap();
        let unleased = SessionId::new("s-unleased").unwrap();
        record(&p, &unleased, &[entry("b.md", JournalKind::Write)]).unwrap();

        let listed = list_changesets(&p, &table, None).unwrap();
        let by_id = |id: &str| listed.iter().find(|c| c.id == id).unwrap().clone();
        assert_eq!(by_id(live.as_str()).liveness, LivenessState::Live);
        assert_eq!(by_id("s-unleased").liveness, LivenessState::Unleased);
        assert_eq!(by_id("s-unleased").paths, 1);
    }

    #[test]
    fn the_callers_own_changeset_is_always_current() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let id = SessionId::new("explicit-one").unwrap();
        record(&p, &id, &[entry("a.md", JournalKind::Write)]).unwrap();
        let listed = list_changesets(&p, &MapProcessTable::empty(1), Some(&id)).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].liveness, LivenessState::Current);
    }

    #[test]
    fn list_flags_dead_lease_changesets_as_orphaned() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        // Create a lease for a process that is no longer alive.
        // Start with a process table containing pid 20, then switch to one without it
        // to simulate the process being gone.
        let _table_with_process = MapProcessTable::chain(10, &[(20, "s20-alive")]);

        let dead_id = SessionId::new("s-dead-proc").unwrap();
        // Manually create a lease file for a dead process
        let lease_path = p.leases_dir().join("999.lease");
        std::fs::create_dir_all(p.leases_dir()).ok();
        let lease = super::lease::Lease {
            id: dead_id.as_str().to_string(),
            start_time: "0x0102030405060708".to_string(),
            created_utc: "2026-09-09T00:00:00Z".to_string(),
        };
        let mut f = std::fs::File::create(&lease_path).unwrap();
        use std::io::Write as _;
        f.write_all(serde_json::to_string(&lease).unwrap().as_bytes())
            .unwrap();
        drop(f);

        // Record a changeset with this dead-lease id
        record(&p, &dead_id, &[entry("a.md", JournalKind::Write)]).unwrap();

        // Now list from a process table that does NOT include pid 999 - the lease is dead
        let table_no_process = MapProcessTable::chain(10, &[(20, "s20-other")]);
        let listed = list_changesets(&p, &table_no_process, None).unwrap();
        let by_id = |id: &str| listed.iter().find(|c| c.id == id).unwrap().clone();
        assert_eq!(by_id("s-dead-proc").liveness, LivenessState::Orphaned);
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

    /// A discard is a deliberate destruction, but only of what it read.
    ///
    /// This is the same interleave `an_append_during_truncation_survives`
    /// pins for the commit path, arriving from the other writer: under one
    /// shared session id a sibling process appends while the discard runs.
    /// A bare `remove_file` — what this used to be — took that record with
    /// it, which is the module's one unsafe direction.
    #[test]
    fn a_record_appended_during_a_discard_survives_it() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let id = SessionId::new("s-discard-race").unwrap();
        record(
            &p,
            &id,
            &[entry_with_digest("a.md", JournalKind::Write, "aaa")],
        )
        .unwrap();

        // The sibling's line lands between the discard reading the journal and
        // writing its tombstone. Appended raw rather than through `record`,
        // because re-entering the seam would double-borrow it.
        let sibling = raw_batch(&[entry_with_digest("b.md", JournalKind::Write, "bbb")]);
        let fired = std::rc::Rc::new(std::cell::Cell::new(false));
        {
            let once = std::rc::Rc::clone(&fired);
            let _guard = with_after_open_hook(move |path| {
                if once.replace(true) {
                    return;
                }
                append_raw(path, &sibling);
            });
            assert!(
                discard_changeset(&p, &id).unwrap(),
                "the changeset claimed a path, so this is a real discard"
            );
        }
        assert!(
            fired.get(),
            "the interleave never ran — this test is vacuous"
        );

        assert_eq!(
            read_journal(&p, &id).unwrap(),
            vec![entry_with_digest("b.md", JournalKind::Write, "bbb")],
            "the discard must retire what it read and nothing else"
        );
    }

    /// Content-keyed, exactly as a commit's tombstone is: a path re-recorded
    /// with different content during the discard keeps its record, because
    /// those bytes are not the bytes the discard decided to retire.
    #[test]
    fn a_discard_retires_only_the_content_it_read() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let id = SessionId::new("s-discard-rewrite").unwrap();
        record(
            &p,
            &id,
            &[entry_with_digest("INDEX.md", JournalKind::Write, "old")],
        )
        .unwrap();

        let sibling = raw_batch(&[entry_with_digest("INDEX.md", JournalKind::Write, "new")]);
        let fired = std::rc::Rc::new(std::cell::Cell::new(false));
        {
            let once = std::rc::Rc::clone(&fired);
            let _guard = with_after_open_hook(move |path| {
                if once.replace(true) {
                    return;
                }
                append_raw(path, &sibling);
            });
            assert!(discard_changeset(&p, &id).unwrap());
        }

        assert_eq!(
            read_journal(&p, &id).unwrap(),
            vec![entry_with_digest("INDEX.md", JournalKind::Write, "new")],
            "the rewritten copy is not what the discard read, so it survives"
        );
    }

    /// The observable `rdm session discard --force` promises: with nothing
    /// else appending, the journal is gone when it returns, not merely
    /// emptied. That is `compact`'s sweep, taken inline because a discard is
    /// never on a hot path.
    #[test]
    fn a_quiescent_discard_leaves_no_journal_behind() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let id = SessionId::new("s-discard-quiet").unwrap();
        record(&p, &id, &[entry("a.md", JournalKind::Write)]).unwrap();

        assert!(discard_changeset(&p, &id).unwrap());
        assert!(
            !changeset_path(&p, &id).exists(),
            "an uncontended discard sweeps the file, as it always did"
        );
    }

    /// Appends one already-formatted line the way a sibling process would,
    /// bypassing `record` so the after-open seam is not re-entered.
    fn append_raw(path: &std::path::Path, line: &str) {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        file.write_all(line.as_bytes()).unwrap();
    }

    /// Renders `entries` as the batch line `record` would have written.
    fn raw_batch(entries: &[JournalEntry]) -> String {
        format!(
            "{}\n",
            serde_json::to_string(&JournalLine {
                paths: entries.to_vec()
            })
            .unwrap()
        )
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

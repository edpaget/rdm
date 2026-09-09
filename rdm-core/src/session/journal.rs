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
//!   compaction swapped out and redo the write against the live one, and an
//!   append that keeps losing takes compaction's own lock so no compaction can
//!   run at all (see [`append_line`]). Losing an entry to compaction is
//!   therefore excluded, not merely made unlikely.
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
//! sweep safe is [`append_line`]'s redo-then-escalate, not the lease check.

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

/// The number of write attempts one append may make in each of its two
/// phases.
///
/// The arithmetic is worth stating explicitly, because the loop bound reads
/// off by one otherwise. [`append_line`] runs `for _ in 1..APPEND_ATTEMPTS`
/// **lock-free**, which is `APPEND_ATTEMPTS - 1` attempts; then it takes
/// [`compact`]'s lock and runs the same bound again, verifying each write; and
/// if even those all lose it makes one last unverified write rather than
/// spinning. At the value below that is 3 lock-free writes, 3 verified locked
/// writes, and 1 final write — `2 * APPEND_ATTEMPTS - 1` = 7 in total, which
/// is what `a_journal_replaced_on_every_attempt_still_terminates` counts.
///
/// Each attempt is one `O_APPEND` write plus the identity recheck that says
/// whether it landed in the journal `path` still names. One redo covers a
/// single racing compaction and three covers a burst, but the bound is not
/// what makes the append safe: exhausting the lock-free ones escalates to the
/// lock-guarded phase in [`append_line`] rather than accepting a write that
/// may already be doomed. Raising or lowering it trades lock-free attempts
/// against escalations and changes no guarantee.
const APPEND_ATTEMPTS: usize = 4;

/// How long the final, lock-guarded append waits for the compaction lock.
///
/// Deliberately longer than [`COMPACT_LOCK_STALE_AFTER`], which is what makes
/// a single bounded [`AdvisoryLock::acquire`] call a *guarantee* rather than
/// an attempt: it returns held either because the current holder released, or
/// because the lock file aged past staleness and was taken over. So the
/// escalation cannot come back empty-handed while the lock directory is
/// usable, and it still cannot block a real run indefinitely.
///
/// It is only ever reached after all `APPEND_ATTEMPTS - 1` lock-free writes
/// have lost to a concurrent compaction, so paying for it is already evidence
/// of sustained contention rather than of the ordinary case.
const APPEND_LOCK_WAIT: Duration = Duration::from_secs(35);

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

/// The environment variable naming a harness barrier file for compaction.
///
/// When set, [`compact`] blocks after its checks have passed and before it
/// does anything irreversible — the window in which its advisory lock can age
/// past [`COMPACT_LOCK_STALE_AFTER`] and be taken over by an escalating
/// [`append_line`] while it is still running. That window opens and closes
/// inside one `rdm session gc` invocation and is bounded by wall-clock time no
/// harness should wait on, so driving it across two real processes needs a
/// seam. Same contract as the other two: inert when unset or empty, bounded by
/// [`HARNESS_BARRIER_CEILING`] when set.
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

/// Appends one complete line to the journal at `path`, redoing the write if it
/// landed in a file that is no longer the live journal, and taking
/// [`compact`]'s own lock rather than giving up if redoing keeps losing.
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
/// So the append detects the loss instead of assuming it away, in two stages:
///
/// 1. **Detect and redo, lock-free.** After writing, it compares the identity
///    of the file it wrote to against the identity of `path` now, and redoes
///    the append against the live journal when they differ — including when
///    compaction removed the file outright, in which case the redo recreates
///    it. Redoing is safe to repeat, because the fold is keyed by path and
///    applied in file order: a duplicated line contributes exactly what the
///    original contributed. This covers the ordinary case at no cost.
/// 2. **Escalate and exclude.** After the `APPEND_ATTEMPTS - 1` lock-free
///    attempts have all lost, it stops racing and takes [`compact`]'s own
///    advisory lock before appending once more — the last of the
///    [`APPEND_ATTEMPTS`] writes an append may make. Compaction *skips entirely* unless it holds that
///    lock, so a write made while holding it cannot be replaced or unlinked
///    underneath. This is the step that makes "an appended entry is not lost
///    to compaction" an invariant rather than a bound: a fixed cap that simply
///    accepted its last write would accept one already landing in a doomed
///    inode, which is a silent loss and not the over-claim the rest of this
///    module degrades to.
///
/// The escalation terminates: [`AdvisoryLock::acquire`] is deadline-bounded,
/// and [`APPEND_LOCK_WAIT`] exceeds [`COMPACT_LOCK_STALE_AFTER`], so it comes
/// back held unless the lock directory itself is unusable.
///
/// Taking the lock is not on its own enough, because
/// [`AdvisoryLock::acquire`] takes a lock over on age alone: the escalation
/// can be handed a lock whose previous holder is a [`compact`] still running
/// past [`COMPACT_LOCK_STALE_AFTER`] and already past its own checks. Two
/// things close that. This function keeps *verifying* under the lock — the
/// locked phase runs the same identity recheck and redo as the lock-free one,
/// so a write the dispossessed compaction replaced is written again — and
/// [`compact`] asks [`AdvisoryLock::still_held`] immediately before its
/// `rename`/`remove_file` and abandons the rewrite when it has been
/// dispossessed. So the two parties detect the double-hold from both sides
/// instead of one of them silently overwriting the other.
///
/// Every writer in this module follows the protocol, including
/// [`discard_changeset`], which destroys a changeset deliberately and used to
/// do it with a bare `remove_file` — an unlink that took a sibling's
/// concurrent append with it exactly as an unguarded compaction would, since a
/// lock buys nothing against a party that never asks for it. It now retires
/// what it read through [`truncate`], so it appends like everything else. The
/// residual that remains is genuinely outside this module: out-of-band
/// tampering with the journal file by something that is not rdm.
///
/// # Errors
///
/// Returns [`Error::Io`] if the journal cannot be opened or written.
fn append_line(path: &std::path::Path, line: &str) -> Result<()> {
    // Phase one: `APPEND_ATTEMPTS - 1` lock-free attempts.
    for _ in 1..APPEND_ATTEMPTS {
        if append_once(path, line)? {
            return Ok(());
        }
    }
    // Phase two: stop racing compaction and exclude it. Holding this lock is
    // what keeps a protocol-following compaction from starting at all, so it
    // is taken for the writes themselves and released immediately after.
    let _lock = AdvisoryLock::acquire(
        compaction_lock_path(path),
        APPEND_LOCK_WAIT,
        COMPACT_LOCK_STALE_AFTER,
    );
    for _ in 1..APPEND_ATTEMPTS {
        // Verified, exactly as the lock-free phase is. Holding the lock is not
        // proof the write landed: `AdvisoryLock::acquire` takes a lock over on
        // age alone, so this lock may have come from a compaction still
        // running past its own checks. That compaction abandons its rewrite
        // once it sees it was dispossessed, but it may already have replaced
        // the journal before this write reached it — in which case the remedy
        // is the same as everywhere else in this module: write again against
        // whatever `path` names now.
        if append_once(path, line)? {
            return Ok(());
        }
    }
    // Every verified attempt lost. Stop spinning and write once more: the
    // bound is what keeps this function terminating, and an over-claimed path
    // is the module's safe direction.
    append_once(path, line)?;
    Ok(())
}

/// Appends `line` once, reporting whether it landed in the live journal.
///
/// `false` means the descriptor written to is no longer what `path` names — a
/// [`compact`] replaced or unlinked it in between — so the caller must write
/// again.
///
/// # Errors
///
/// Returns [`Error::Io`] if the journal cannot be opened or written.
fn append_once(path: &std::path::Path, line: &str) -> Result<bool> {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    harness_barrier(HARNESS_APPEND_BARRIER);
    run_after_open_hook(path);
    file.write_all(line.as_bytes())?;
    Ok(wrote_to_the_live_journal(&file, path))
}

/// Returns the advisory lock guarding compaction of the journal at `path`.
///
/// Single-sourced so [`compact`], which is the only operation that may replace
/// or unlink a journal, and [`append_line`]'s escalation, which exists to
/// exclude it, cannot drift onto two different lock files and quietly stop
/// excluding each other.
fn compaction_lock_path(journal: &std::path::Path) -> PathBuf {
    journal.with_extension("lock")
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
    /// irreversible step, so a unit test can drive a staleness takeover of the
    /// compaction lock at the one instant where it would otherwise cost an
    /// appender its record.
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
/// [`discard_changeset`] reuses this for the opposite reason — retiring what a
/// deliberately destroyed changeset claims rather than what a commit landed —
/// so that a discard, too, cannot take a sibling's concurrent append with it.
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
/// Four guards make it safe anyway, and only the last two are unconditional:
///
/// - The advisory lock at [`compaction_lock_path`]. Unlike every other
///   [`AdvisoryLock`] user in rdm, an *unheld* guard means **skip**, not
///   proceed unlocked: those callers are safe because a real correctness
///   mechanism sits underneath (HEAD compare-and-swap; the content-digest
///   precondition), and a journal rewrite has no such underlayer. That skip is
///   also what lets [`append_line`] use the *same* lock in the other
///   direction: an append that keeps losing takes it, and this call then
///   declines to run at all.
/// - A compare-and-swap on the file's byte length. A journal only ever grows,
///   so its length is a valid version token: if anything was appended between
///   the fold and the rewrite, the length differs and compaction skips.
/// - [`append_line`]'s redo-then-escalate, which is what closes the window the
///   first two only narrow. An appender that already held its `O_APPEND`
///   descriptor when the length check passed writes into the inode this call
///   is about to unlink or replace — the lock cannot see it and the length
///   cannot reflect it. The appender notices afterwards that the file it wrote
///   to is no longer the journal and writes again against the one that is,
///   and if that keeps losing it takes this lock so no compaction can run at
///   all. So the record is not lost, and this call's contract holds
///   unconditionally in the safe direction: compaction may fail to clean,
///   never lose.
/// - An [`AdvisoryLock::still_held`] check immediately before the
///   irreversible step. The guard above depends on holding the lock actually
///   excluding this call, and [`AdvisoryLock::acquire`] takes a lock over on
///   age alone — so a compaction that legitimately outran
///   [`COMPACT_LOCK_STALE_AFTER`] (slow disk, a descheduled process) can be
///   dispossessed *while still running*, and would otherwise `rename` over
///   the very append the successor made under the lock it now holds. Asking
///   whether the lock file still carries this call's own token turns that
///   silent double-hold into a detected one: a dispossessed compaction
///   abandons its rewrite, discards its temporary file, and reports that it
///   cleaned nothing. Skipping is always safe; rewriting under a lock this
///   call no longer holds is not.
///
/// Losing the compare-and-swap repeatedly under sustained load leaves the
/// journal growing: one small line per flushed batch plus one per commit.
/// That is a size concern, not a correctness one.
pub fn compact(paths: &SessionPaths, id: &SessionId) -> bool {
    let path = changeset_path(paths, id);
    let lock = AdvisoryLock::acquire(
        compaction_lock_path(&path),
        COMPACT_LOCK_WAIT,
        COMPACT_LOCK_STALE_AFTER,
    );
    if !lock.held() {
        return false;
    }
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
    // check has passed and neither exit has been taken. Parking here is what
    // lets a harness drive a real staleness takeover of the lock below.
    harness_barrier(HARNESS_COMPACT_BARRIER);
    if folded.is_empty() {
        // The unlink is irreversible, so re-establish ownership first.
        run_before_rewrite_hook(&path);
        if !lock.still_held() {
            return false;
        }
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
    // Last chance to notice a staleness takeover: the temporary file costs
    // nothing to throw away, the `rename` past this point cannot be undone.
    run_before_rewrite_hook(&path);
    if !lock.still_held() {
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
/// write against the live file, and an appender that keeps losing that race
/// takes compaction's own lock, which makes this sweep skip rather than run.
/// That holds for every rung, leased or not, and for a `rdm session gc` run
/// from a process that shares nothing with the sessions it is sweeping — so a
/// sweep can slow a concurrent appender down, never cost it its record.
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
/// is precisely the loss [`append_line`]'s protocol exists to prevent,
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
/// tombstone cannot be appended.
pub fn discard_changeset(paths: &SessionPaths, id: &SessionId) -> Result<bool> {
    let claimed = read_journal(paths, id)?;
    if claimed.is_empty() {
        // Nothing to retire. Sweep the file anyway when one is lying around
        // claiming nothing, so a discard of an already-committed changeset
        // still leaves the directory as clean as it used to.
        let _ = compact(paths, id);
        return Ok(false);
    }
    truncate(paths, id, &claimed)?;
    let _ = compact(paths, id);
    Ok(true)
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

    /// The compare-and-swap guard, driven at its own seam.
    ///
    /// Compaction folds a snapshot of the journal and then rewrites the file
    /// from that snapshot; anything appended in between is in the file but not
    /// in the snapshot, so rewriting would drop it. The length compare-and-swap
    /// is what notices. Every other race window in this module is driven
    /// deterministically through a seam rather than raced for, and this one is
    /// no different: without the seam the mismatch branch is only ever reached
    /// by timing luck, so a CAS computed backwards would pass the suite.
    #[test]
    fn compaction_skips_when_a_line_is_appended_between_its_fold_and_its_rewrite() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let id = SessionId::new("s-cas").unwrap();
        record(&p, &id, &[entry("a.md", JournalKind::Write)]).unwrap();
        record(&p, &id, &[entry("b.md", JournalKind::Write)]).unwrap();
        let path = changeset_path(&p, &id);
        let before = std::fs::read_to_string(&path).unwrap();

        let raced = std::cell::Cell::new(false);
        {
            let p2 = SessionPaths::new(p.base().to_path_buf());
            let id2 = id.clone();
            let _guard = with_after_fold_hook(move |_| {
                if raced.replace(true) {
                    return;
                }
                // A sibling process appending under the same shared id, after
                // this compaction read the journal and before it rewrites it.
                record(&p2, &id2, &[entry("c.md", JournalKind::Write)]).unwrap();
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
            "the record appended across the fold must survive: {claimed:?}"
        );
        assert_eq!(claimed.len(), 3, "and so must the two that preceded it");
    }

    /// The blocking finding this fix answers, in its second form.
    ///
    /// `AdvisoryLock::acquire` takes a lock over on age alone, so a compaction
    /// whose critical section legitimately outruns `COMPACT_LOCK_STALE_AFTER`
    /// — a slow disk, a descheduled process — can be dispossessed while still
    /// running. An appender that escalated then holds the lock, appends under
    /// it, and would have its record renamed away by the compaction that no
    /// longer holds anything. Compaction must notice before the rename.
    #[test]
    fn a_dispossessed_compaction_abandons_its_rewrite() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let id = SessionId::new("s-taken-over").unwrap();
        record(&p, &id, &[entry("a.md", JournalKind::Write)]).unwrap();
        record(&p, &id, &[entry("b.md", JournalKind::Write)]).unwrap();
        let path = changeset_path(&p, &id);
        // Two lines that fold to one, so a rewrite is unmistakable in the raw
        // bytes even though it would leave the folded reading identical.
        let before = std::fs::read_to_string(&path).unwrap();
        assert_eq!(before.lines().count(), 2, "the fold must be collapsible");

        let taken = std::cell::Cell::new(false);
        {
            let _guard = with_before_rewrite_hook(move |journal| {
                if taken.replace(true) {
                    return;
                }
                // The escalating appender takes the lock over mid-compaction.
                take_over_the_compaction_lock(journal);
            });
            assert!(
                !compact(&p, &id),
                "a dispossessed compaction must report that it cleaned nothing"
            );
        }

        assert!(
            path.exists(),
            "and must not have renamed over the journal the successor is appending to"
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            before,
            "the journal must be byte-for-byte what it was: a rewrite under a \
lock this compaction no longer holds is exactly the loss being prevented"
        );
        assert!(
            !path.with_extension("jsonl.compacting").exists(),
            "and must have discarded its temporary file rather than leaving litter"
        );
        // Release the lock the seam took over, so the temp dir cleans up.
        let _ = std::fs::remove_file(compaction_lock_path(&path));
    }

    /// The same dispossession on compaction's other exit: an empty fold, where
    /// the irreversible step is `remove_file` rather than `rename`.
    #[test]
    fn a_dispossessed_compaction_does_not_remove_the_journal() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let id = SessionId::new("s-taken-over-empty").unwrap();
        record(&p, &id, &[entry("a.md", JournalKind::Write)]).unwrap();
        truncate(&p, &id, &[entry("a.md", JournalKind::Write)]).unwrap();
        let path = changeset_path(&p, &id);
        assert!(read_journal(&p, &id).unwrap().is_empty(), "fold is empty");

        let taken = std::cell::Cell::new(false);
        {
            let _guard = with_before_rewrite_hook(move |journal| {
                if taken.replace(true) {
                    return;
                }
                take_over_the_compaction_lock(journal);
            });
            assert!(
                !compact(&p, &id),
                "a dispossessed sweep must report that it removed nothing"
            );
        }

        assert!(
            path.exists(),
            "the journal an escalated appender is writing to must survive the sweep"
        );
        let _ = std::fs::remove_file(compaction_lock_path(&path));
    }

    /// The end-to-end shape of the finding: an appender escalates into a
    /// compaction's lock because that compaction is still running, appends,
    /// and the record survives the compaction finishing afterwards.
    #[test]
    fn an_append_that_escalated_into_a_slow_compactions_lock_survives_it() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let id = SessionId::new("s-slow-compact").unwrap();
        record(&p, &id, &[entry("a.md", JournalKind::Write)]).unwrap();
        record(&p, &id, &[entry("a.md", JournalKind::Write)]).unwrap();
        let path = changeset_path(&p, &id);

        let raced = std::cell::Cell::new(false);
        {
            let p2 = SessionPaths::new(p.base().to_path_buf());
            let id2 = id.clone();
            let _guard = with_before_rewrite_hook(move |journal| {
                if raced.replace(true) {
                    return;
                }
                // The appender: it lost every lock-free attempt, escalated,
                // found this compaction's lock stale, took it over, and wrote.
                take_over_the_compaction_lock(journal);
                record(&p2, &id2, &[entry("b.md", JournalKind::Write)]).unwrap();
            });
            assert!(!compact(&p, &id));
        }

        let claimed = read_journal(&p, &id).unwrap();
        assert!(
            claimed.contains(&entry("b.md", JournalKind::Write)),
            "the escalated append must not be renamed away: {claimed:?}"
        );
        assert!(path.exists());
        let _ = std::fs::remove_file(compaction_lock_path(&path));
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
    /// step, so a test can drive a staleness takeover of the compaction lock.
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

    /// Impersonates the staleness takeover an escalating `append_line` does
    /// when it finds a compaction's lock older than `COMPACT_LOCK_STALE_AFTER`
    /// — byte for byte what `AdvisoryLock::acquire` does in that case: remove
    /// the incumbent's file and create its own, carrying its own token.
    ///
    /// Written as raw filesystem calls rather than by aging a real lock and
    /// acquiring it, so the tests below hold no successor guard alive and
    /// depend on no clock.
    fn take_over_the_compaction_lock(journal: &std::path::Path) {
        let lock_path = compaction_lock_path(journal);
        assert!(
            lock_path.exists(),
            "the compaction under test is not holding its lock, so there is \
nothing to take over and this test would prove nothing"
        );
        std::fs::remove_file(&lock_path).unwrap();
        std::fs::write(&lock_path, b"rdm-lock successor 0 0\n").unwrap();
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

    /// The blocking finding the *escalation* answers.
    ///
    /// A bound alone is not the guarantee AC2 states: an append whose every
    /// lock-free attempt loses to a compaction would, with a fixed cap that
    /// simply accepted its last write, put that write into an inode nothing
    /// will read again — a silent loss, not the over-claim the rest of this
    /// module degrades to. Here a real `gc_changesets` sweep runs on *every*
    /// attempt, so no lock-free attempt can ever win. The record must still be
    /// readable, because the escalation takes the lock that sweep needs and
    /// the sweep therefore skips.
    #[test]
    fn a_record_survives_a_compaction_that_wins_every_lock_free_attempt() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let id = SessionId::new("s-storm").unwrap();

        // Two claims, so the fold is non-empty and compaction takes its
        // `rename` exit — the sharper case, since the replaced journal still
        // looks healthy while the appender's bytes sit in an unlinked inode.
        record(&p, &id, &[entry("a.md", JournalKind::Write)]).unwrap();
        record(&p, &id, &[entry("b.md", JournalKind::Write)]).unwrap();

        let sweeps = std::rc::Rc::new(std::cell::Cell::new(0usize));
        {
            let seen = std::rc::Rc::clone(&sweeps);
            let p2 = SessionPaths::new(p.base().to_path_buf());
            let _guard = with_after_open_hook(move |_| {
                seen.set(seen.get() + 1);
                // A DIFFERENT process's `rdm session gc`, on every attempt.
                // It respects the compaction lock, which is the whole point:
                // once the appender escalates, this stops being able to run.
                gc_changesets(&p2, &MapProcessTable::empty(2), None);
            });
            record(&p, &id, &[entry("c.md", JournalKind::Write)]).unwrap();
        }

        assert!(
            sweeps.get() > 1,
            "the sweep must have won at least one lock-free attempt, or this \
test is not exercising the escalation (attempts: {})",
            sweeps.get()
        );
        let read = read_journal(&p, &id).unwrap();
        assert!(
            read.contains(&entry("c.md", JournalKind::Write)),
            "the record must survive a compaction that wins every lock-free \
attempt, not merely one that loses eventually: {read:?}"
        );
    }

    /// The escalation is what carries the guarantee, so removing it must break
    /// the test above — otherwise that test would pass on a fixed cap that
    /// accepts its last write, which is the exact defect being fixed.
    ///
    /// Driven here rather than by mutating the source: a hostile writer that
    /// ignores the compaction lock entirely reproduces precisely what an
    /// unescalated append would suffer, since the lock it takes buys nothing
    /// against a party that never asks for it.
    #[test]
    fn a_writer_that_ignores_the_compaction_lock_defeats_the_escalation() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let id = SessionId::new("s-lawless").unwrap();
        std::fs::create_dir_all(p.changesets_dir()).unwrap();
        record(&p, &id, &[entry("a.md", JournalKind::Write)]).unwrap();

        {
            let _guard = with_after_open_hook(move |path| {
                // No lock taken, so the escalation cannot exclude it.
                let _ = std::fs::remove_file(path);
            });
            record(&p, &id, &[entry("b.md", JournalKind::Write)]).unwrap();
        }

        assert!(
            !read_journal(&p, &id)
                .unwrap()
                .contains(&entry("b.md", JournalKind::Write)),
            "if this now survives, the test above no longer proves the \
escalation is doing the work"
        );
    }

    /// Both halves are bounded, not conditional: a journal replaced on *every*
    /// attempt still terminates, writing at most `2 * APPEND_ATTEMPTS - 1`
    /// lines (the lock-free tries, the verified locked tries, and the one
    /// final write) and never spinning.
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
            2 * APPEND_ATTEMPTS - 1,
            "both phases are bounded: APPEND_ATTEMPTS - 1 lock-free writes, \
the same again verified under compaction's lock, and one final write"
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

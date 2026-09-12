//! Filesystem-backed [`Store`] implementation with in-memory staging.
//!
//! Writes are buffered in memory until [`Store::commit`] flushes them to disk.
//! [`Store::discard`] drops the buffer without touching the filesystem.
//!
//! # The flush precondition
//!
//! The filesystem is *shared committed state*: two `rdm` processes see the
//! same bytes, and the staging overlay above them does not span a process. So
//! two sessions can each read an item, each edit it, and each flush — and the
//! second flush silently destroys the first session's edit. `--tags` replacing
//! the whole list on update makes that concrete: a reserved tag disappears
//! with no error anywhere.
//!
//! [`FsStore`] closes that window with optimistic concurrency keyed on
//! content. The first time a process touches a path — reading it, probing it,
//! deleting it, or blindly staging a write over it — it records a
//! [`Baseline`]: what was there. At flush time, before anything is written,
//! every staged path is re-observed and compared. A mismatch means this flush
//! was derived from content someone else has since changed, and the whole
//! flush is refused with [`Error::StaleWrite`] and nothing written.
//!
//! A baseline lives for exactly one read-modify-write cycle: it is recorded
//! on first touch and dropped by the flush (or the [`Store::discard`]) that
//! ends the cycle, so the next touch observes the world afresh. That is what
//! makes a session's own sequential writes safe — no session id is consulted
//! anywhere, and a store that outlives one flush (a long-lived host process
//! can hold one across many calls) keeps re-observing rather than staying
//! pinned to what it last wrote.

#![warn(missing_docs)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use rdm_core::error::{Error, Result};
use rdm_core::lock::AdvisoryLock;
use rdm_core::paths::describe_path;
use rdm_core::session::journal::JournalKind;
use rdm_core::store::{
    Baseline, DirEntry, DirEntryKind, RelPath, StagedEntry, StagedOverlay, Store, VersionedStore,
    content_digest,
};

/// How long a flush waits for the advisory lock before proceeding unlocked.
///
/// Sized like the scoped-commit lock and for the same reason: the `Done:` hook
/// path reaches this code under `hook_timeout_secs` (default 30s), and a lock
/// must never be what makes it miss its deadline. The lock is an optimization
/// — the content check is the correctness mechanism — so giving up is safe.
const FLUSH_LOCK_WAIT: Duration = Duration::from_secs(5);

/// How stale a flush lock must be before another process takes it over.
const FLUSH_LOCK_STALE_AFTER: Duration = Duration::from_secs(30);

/// The environment variable naming a harness barrier file.
///
/// A read → write window inside one `rdm` invocation is sub-millisecond, so a
/// harness cannot interleave two real processes at it by racing them. When
/// this is set, a flush blocks until the named file appears, letting a harness
/// park one process mid-flush and drive another to completion. Documented
/// alongside `RDM_HARNESS_SESSION_ID`, whose precedent it follows: inert when
/// unset, and bounded when set, so it can never wedge a real run.
const HARNESS_FLUSH_BARRIER: &str = "RDM_HARNESS_FLUSH_BARRIER";

/// How long the harness barrier waits before proceeding regardless.
const HARNESS_BARRIER_CEILING: Duration = Duration::from_secs(60);

/// A [`Store`] backed by the local filesystem with in-memory staging.
///
/// Writes and deletes are buffered in memory. Reads see staged changes first
/// (read-your-own-writes). Call [`Store::commit`] to flush staged changes to
/// disk, or [`Store::discard`] to drop them.
///
/// Commit verifies every staged path against the [`Baseline`] this process
/// first observed (see the module docs), then uses write-to-temp + rename for
/// best-effort atomicity on each file.
#[derive(Debug)]
pub struct FsStore {
    root: PathBuf,
    staged: StagedOverlay,
    /// What this process first saw at each touched path.
    ///
    /// A `Mutex` rather than a `RefCell` because `FsStore` must stay
    /// `Send + Sync` (it is stored behind `Box<dyn VersionedStore + Send +
    /// Sync>`), and interior mutability is required because baselines are
    /// recorded from `&self` methods (`read`, `exists`).
    baselines: Mutex<BTreeMap<String, Baseline>>,
}

impl Clone for FsStore {
    /// Deep-copies the baselines rather than sharing them.
    ///
    /// Hand-written because `Mutex` is not `Clone`. Sharing would be wrong
    /// regardless: cloning an `FsStore` already yields independent staging, so
    /// the two clones are logically separate stores and must observe the world
    /// separately too.
    fn clone(&self) -> Self {
        Self {
            root: self.root.clone(),
            staged: self.staged.clone(),
            baselines: Mutex::new(self.snapshot_baselines()),
        }
    }
}

impl FsStore {
    /// Creates a new `FsStore` rooted at the given path.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            staged: StagedOverlay::new(),
            baselines: Mutex::new(BTreeMap::new()),
        }
    }

    /// Returns the root path of this store.
    pub fn root(&self) -> &std::path::Path {
        &self.root
    }

    /// Snapshots the pending (staged, not yet flushed) paths, what will happen
    /// to each, and — for a write — the content identity about to be flushed.
    ///
    /// This is an inherent method, deliberately not part of the [`Store`]
    /// trait: it exists so a wrapping backend can journal exactly what a
    /// [`Store::commit`] flushed, and it must be called *before* `commit`,
    /// which drains the staging overlay. It reads nothing and changes nothing.
    ///
    /// The digest travels with the path so a later scoped commit can tell
    /// "the bytes this changeset wrote" from "whatever is on disk now". It is
    /// `None` for a delete, which has no bytes.
    ///
    /// Unparsable keys are skipped — which cannot happen in practice, since
    /// every key entered the overlay through a validated [`RelPath`].
    pub fn staged_paths(&self) -> Vec<(RelPath, JournalKind, Option<String>)> {
        self.staged
            .iter()
            .filter_map(|(key, entry)| {
                let path = RelPath::new(key).ok()?;
                let (kind, digest) = match entry {
                    StagedEntry::Write(content) => {
                        (JournalKind::Write, Some(content_digest(content)))
                    }
                    StagedEntry::Delete => (JournalKind::Delete, None),
                };
                Some((path, kind, digest))
            })
            .collect()
    }

    /// Resolves a `RelPath` to an absolute filesystem path.
    fn resolve(&self, path: &RelPath) -> PathBuf {
        if path.as_str().is_empty() {
            self.root.clone()
        } else {
            self.root.join(path.as_str())
        }
    }

    /// Returns a copy of the recorded baselines.
    ///
    /// A poisoned lock reads as empty, which degrades the check to a no-op
    /// rather than making every mutation fail: the guarantee is best-effort
    /// protection, never a new way to brick the store.
    fn snapshot_baselines(&self) -> BTreeMap<String, Baseline> {
        self.baselines.lock().map(|b| b.clone()).unwrap_or_default()
    }

    /// Records `observed` as `key`'s baseline **if this is the first touch of
    /// the current cycle**.
    ///
    /// First touch wins. A later read served from the staging overlay must
    /// never re-seed the baseline, or the flush would compare this process's
    /// own staged content against itself and never detect anything. "First"
    /// is scoped to the cycle, not to the store's lifetime: `commit` and
    /// `discard` both clear the map, so the next touch observes disk again.
    fn note_baseline(&self, key: &str, observed: impl FnOnce() -> Baseline) {
        let Ok(mut baselines) = self.baselines.lock() else {
            return;
        };
        if !baselines.contains_key(key) {
            baselines.insert(key.to_string(), observed());
        }
    }

    /// Observes `key`'s current on-disk state as a [`Baseline`].
    fn observe(&self, path: &RelPath) -> Baseline {
        Self::observe_at(&self.resolve(path))
    }

    /// Observes an absolute filesystem path as a [`Baseline`].
    fn observe_at(full: &std::path::Path) -> Baseline {
        match fs::read_to_string(full) {
            Ok(content) => Baseline::Present(content_digest(&content)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Baseline::Absent,
            // Unreadable, non-UTF8, a directory: fail open. An unreadable
            // neighbour must never brick an unrelated mutation.
            Err(_) => Baseline::Unknown,
        }
    }

    /// Blocks until the harness barrier file appears, or the ceiling elapses.
    ///
    /// Inert unless [`HARNESS_FLUSH_BARRIER`] is set. Bounded unconditionally:
    /// a harness that dies without releasing the barrier delays this flush by
    /// at most [`HARNESS_BARRIER_CEILING`], it does not wedge it.
    fn harness_barrier() {
        let Ok(marker) = std::env::var(HARNESS_FLUSH_BARRIER) else {
            return;
        };
        if marker.is_empty() {
            return;
        }
        let marker = std::path::PathBuf::from(marker);
        let deadline = std::time::Instant::now() + HARNESS_BARRIER_CEILING;
        while !marker.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Verifies every staged path still matches the baseline this process
    /// first observed.
    ///
    /// Runs before any disk mutation, so a rejection leaves the tree
    /// untouched and the staging overlay intact.
    ///
    /// # Errors
    ///
    /// Returns [`Error::StaleWrite`] naming the first drifted path.
    fn verify_baselines(&self) -> Result<()> {
        let baselines = self.snapshot_baselines();
        for key in self.staged.keys() {
            // No path class is exempt. `INDEX.md` used to be, on the grounds
            // that every mutation regenerated it from disk so two sessions
            // legitimately rewrote it; mutations write no index any more, and
            // nothing reconciles one at commit time to paper over a clobber,
            // so it needs this check like any other path. A blind write is
            // covered too — `Store::write` records the on-disk bytes as its
            // baseline — which is what makes an uncontested `rdm index` land
            // while a contested one is refused.
            let Some(baseline) = baselines.get(key) else {
                continue;
            };
            if matches!(baseline, Baseline::Unknown) {
                continue;
            }
            let Ok(rel) = RelPath::new(key) else {
                continue;
            };
            let current = self.observe(&rel);
            // An unreadable file *now* is also fail-open: the write is about
            // to replace it anyway, and erroring would be a new way to wedge.
            if matches!(current, Baseline::Unknown) {
                continue;
            }
            if &current != baseline {
                return Err(Error::StaleWrite {
                    item: describe_path(key),
                    path: key.clone(),
                });
            }
        }
        Ok(())
    }
}

impl Store for FsStore {
    fn read(&self, path: &RelPath) -> Result<String> {
        let full = self.resolve(path);
        self.staged.read(path.as_str(), || {
            // Only the committed fall-through observes the world; a read
            // served from the overlay saw this process's own staged content
            // and must not be mistaken for an observation of disk.
            match fs::read_to_string(&full) {
                Ok(content) => {
                    self.note_baseline(path.as_str(), || {
                        Baseline::Present(content_digest(&content))
                    });
                    Ok(content)
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    self.note_baseline(path.as_str(), || Baseline::Absent);
                    Err(Error::Io(e))
                }
                Err(e) => {
                    // Unreadable for some other reason: fail open rather than
                    // guessing, so the flush check skips this path.
                    self.note_baseline(path.as_str(), || Baseline::Unknown);
                    Err(Error::Io(e))
                }
            }
        })
    }

    fn exists(&self, path: &RelPath) -> bool {
        let full = self.resolve(path);
        self.staged.exists(path.as_str(), || {
            // A create only ever probes; recording the probe is what lets a
            // create racing another create on the same path be rejected.
            self.note_baseline(path.as_str(), || Self::observe_at(&full));
            full.exists()
        })
    }

    fn list(&self, path: &RelPath) -> Result<Vec<DirEntry>> {
        let prefix = if path.as_str().is_empty() {
            String::new()
        } else {
            format!("{}/", path.as_str())
        };

        // Start with filesystem entries
        let dir = self.resolve(path);
        let mut entries_map: BTreeMap<String, DirEntryKind> = BTreeMap::new();

        if dir.exists() {
            for entry in fs::read_dir(&dir)? {
                let entry = entry?;
                let Ok(name) = entry.file_name().into_string() else {
                    continue;
                };
                let kind = if entry.file_type()?.is_dir() {
                    DirEntryKind::Dir
                } else {
                    DirEntryKind::File
                };
                entries_map.insert(name, kind);
            }
        }

        // Collect disk file keys under this prefix for deletion checks
        let mut disk_file_keys: BTreeSet<String> = BTreeSet::new();
        if dir.exists() {
            Self::collect_disk_keys(&dir, &prefix, &mut disk_file_keys);
        }

        // Apply staged changes
        for (key, entry) in self.staged.iter() {
            let suffix = if prefix.is_empty() {
                key.as_str()
            } else if let Some(s) = key.strip_prefix(&prefix) {
                s
            } else {
                continue;
            };

            if suffix.is_empty() {
                continue;
            }

            // Get the direct child name
            let (child_name, is_nested) = match suffix.split_once('/') {
                Some((first, _)) => (first, true),
                None => (suffix, false),
            };

            match entry {
                StagedEntry::Write(_) => {
                    if is_nested {
                        entries_map
                            .entry(child_name.to_string())
                            .or_insert(DirEntryKind::Dir);
                    } else {
                        entries_map.insert(child_name.to_string(), DirEntryKind::File);
                    }
                }
                StagedEntry::Delete => {
                    if !is_nested {
                        // Direct child file is staged for deletion — remove it
                        entries_map.remove(child_name);
                    }
                    // For nested deletes, the parent dir might still have other entries,
                    // so we don't remove it here
                }
            }
        }

        // Remove directories that have become empty due to staged deletes
        // A directory entry should be removed if all its disk files are staged for delete
        // and no staged writes exist under it
        let dir_names: Vec<String> = entries_map
            .iter()
            .filter(|(_, kind)| **kind == DirEntryKind::Dir)
            .map(|(name, _)| name.clone())
            .collect();

        for dir_name in dir_names {
            let dir_prefix = if prefix.is_empty() {
                format!("{dir_name}/")
            } else {
                format!("{prefix}{dir_name}/")
            };

            // Check if any effective files exist under this directory
            let has_disk_files = disk_file_keys.iter().any(|k| {
                k.starts_with(&dir_prefix)
                    && !matches!(self.staged.get(k), Some(StagedEntry::Delete))
            });

            let has_staged_writes = self
                .staged
                .iter()
                .any(|(k, e)| k.starts_with(&dir_prefix) && matches!(e, StagedEntry::Write(_)));

            if !has_disk_files && !has_staged_writes {
                // Only remove if the directory came from disk and is now empty
                // Don't remove if we never checked disk (dir didn't exist)
                if dir.exists() {
                    let child_dir = dir.join(&dir_name);
                    if child_dir.exists() {
                        entries_map.remove(&dir_name);
                    }
                }
            }
        }

        Ok(entries_map
            .into_iter()
            .map(|(name, kind)| DirEntry { name, kind })
            .collect())
    }

    fn write(&mut self, path: &RelPath, content: String) -> Result<()> {
        // A blind write — one with no prior read or probe — still has a
        // baseline: whatever is on disk right now. Without it, a caller that
        // constructs content from nothing could clobber a concurrent write.
        self.note_baseline(path.as_str(), || self.observe(path));
        self.staged.write(path.as_str(), content);
        Ok(())
    }

    fn delete(&mut self, path: &RelPath) -> Result<()> {
        let full = self.resolve(path);
        self.note_baseline(path.as_str(), || Self::observe_at(&full));
        self.staged.delete(path.as_str(), || full.exists())
    }

    fn commit(&mut self) -> Result<()> {
        // Test seam, inert unless the harness variable is set.
        Self::harness_barrier();

        // Best-effort and age-bounded: the content check below is what makes
        // this correct; the lock only narrows the residual window between the
        // check and the rename. Skipped when there is no `.git` to put it
        // under, since that is the one directory the tree walkers ignore.
        let git_dir = self.root.join(".git");
        let _lock = git_dir.is_dir().then(|| {
            AdvisoryLock::acquire(
                git_dir.join("rdm").join("flush.lock"),
                FLUSH_LOCK_WAIT,
                FLUSH_LOCK_STALE_AFTER,
            )
        });

        // Verify BEFORE draining: a rejection must leave the overlay intact
        // and zero files touched, so the caller can retry or discard.
        self.verify_baselines()?;

        let staged = self.staged.drain();
        for (key, entry) in staged {
            let rel = if key.is_empty() {
                RelPath::root()
            } else {
                RelPath::new(&key)?
            };
            let full = self.resolve(&rel);

            match entry {
                StagedEntry::Write(content) => {
                    if let Some(parent) = full.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    // Write to temp file then rename for best-effort atomicity
                    let parent = full.parent().unwrap_or(&self.root);
                    let mut tmp = tempfile::NamedTempFile::new_in(parent).map_err(|e| {
                        Error::Io(std::io::Error::new(
                            e.kind(),
                            format!("failed to create temp file: {e}"),
                        ))
                    })?;
                    tmp.write_all(content.as_bytes())?;
                    tmp.persist(&full).map_err(|e| {
                        Error::Io(std::io::Error::other(format!(
                            "failed to persist temp file: {e}"
                        )))
                    })?;
                }
                StagedEntry::Delete => {
                    if full.exists() {
                        fs::remove_file(&full)?;
                    }
                }
            }
        }

        // A flush ends the cycle those baselines described, so drop all of
        // them — the same thing `discard` does, for the same reason. The next
        // touch of any path re-observes disk, which is what makes a session's
        // own sequential writes unable to trip the check (no session id is
        // consulted anywhere) *and* what keeps a store that outlives one
        // flush honest. Re-seeding only the flushed subset would instead pin
        // every previously written path to what this store last wrote: after
        // another session edited such a path, every later write to it — read
        // from disk, correctly derived, and retried exactly as the error
        // message advises — would be refused forever, because a plain read
        // never re-seeds an already-recorded baseline. That wedges any
        // long-lived store — a host process holding one across many calls.
        if let Ok(mut baselines) = self.baselines.lock() {
            baselines.clear();
        }
        Ok(())
    }

    fn discard(&mut self) {
        self.staged.discard();
        // Baselines describe reads that fed the discarded writes. Keeping
        // them would make a later flush compare against an observation this
        // store has disowned.
        if let Ok(mut baselines) = self.baselines.lock() {
            baselines.clear();
        }
    }

    fn modified(&self, path: &RelPath) -> Result<Option<std::time::SystemTime>> {
        match fs::metadata(self.resolve(path)) {
            Ok(meta) => Ok(meta.modified().ok()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(Error::Io(e)),
        }
    }
}

/// `FsStore` has no notion of committed history, so both methods report
/// [`Error::HistoryUnavailable`]. It implements [`VersionedStore`] only so
/// that callers generic over revision-aware reads (e.g. the server's
/// `?at=<sha>` path) type-check against a filesystem backend; such reads
/// surface as a "history unavailable" error rather than real history.
///
/// rdm-server prefers a real, git-backed `rdm_store_git::GitStore` by
/// default (see its `default_store_factory`), so this impl is now reached
/// only as the fallback: when the plan root is not (yet) a git repository,
/// or when the server is built without its `git` cargo feature.
impl VersionedStore for FsStore {
    fn head_sha(&self) -> Result<String> {
        Err(Error::HistoryUnavailable)
    }

    fn fetch_body_at(&self, _path: &RelPath, _sha: &str) -> Result<String> {
        Err(Error::HistoryUnavailable)
    }
}

impl FsStore {
    /// Recursively collects all file keys under a directory for deletion tracking.
    fn collect_disk_keys(dir: &std::path::Path, prefix: &str, keys: &mut BTreeSet<String>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries {
            let Ok(entry) = entry else { continue };
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            let key = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}{name}")
            };
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                Self::collect_disk_keys(&entry.path(), &format!("{key}/"), keys);
            } else {
                keys.insert(key);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup() -> (TempDir, FsStore) {
        let dir = TempDir::new().unwrap();
        let store = FsStore::new(dir.path());
        (dir, store)
    }

    /// Helper: write a file directly to disk (bypassing staging).
    fn write_disk(dir: &TempDir, path: &str, content: &str) {
        let full = dir.path().join(path);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(full, content).unwrap();
    }

    // --- Original tests (adapted for staging semantics) ---

    #[test]
    fn write_and_read_round_trip() {
        let (_dir, mut store) = setup();
        let path = RelPath::new("hello.md").unwrap();
        store.write(&path, "world".to_string()).unwrap();
        // Read-your-own-writes: visible before commit
        assert_eq!(store.read(&path).unwrap(), "world");
    }

    #[test]
    fn write_does_not_touch_disk_before_commit() {
        let (dir, mut store) = setup();
        let path = RelPath::new("staged.md").unwrap();
        store.write(&path, "content".to_string()).unwrap();
        // File should NOT exist on disk yet
        assert!(!dir.path().join("staged.md").exists());
    }

    #[test]
    fn commit_flushes_to_disk() {
        let (dir, mut store) = setup();
        let path = RelPath::new("hello.md").unwrap();
        store.write(&path, "world".to_string()).unwrap();
        store.commit().unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join("hello.md")).unwrap(),
            "world"
        );
    }

    #[test]
    fn commit_creates_parent_dirs() {
        let (dir, mut store) = setup();
        let path = RelPath::new("a/b/c.md").unwrap();
        store.write(&path, "deep".to_string()).unwrap();
        store.commit().unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join("a/b/c.md")).unwrap(),
            "deep"
        );
    }

    #[test]
    fn exists_reflects_staged_writes() {
        let (_dir, mut store) = setup();
        let path = RelPath::new("test.md").unwrap();
        assert!(!store.exists(&path));
        store.write(&path, "x".to_string()).unwrap();
        assert!(store.exists(&path));
    }

    #[test]
    fn exists_reflects_staged_deletes() {
        let (dir, mut store) = setup();
        let path = RelPath::new("test.md").unwrap();
        write_disk(&dir, "test.md", "content");
        assert!(store.exists(&path));
        store.delete(&path).unwrap();
        assert!(!store.exists(&path));
    }

    #[test]
    fn list_returns_sorted_entries() {
        let (dir, store) = setup();
        write_disk(&dir, "z.md", "z");
        write_disk(&dir, "a.md", "a");
        write_disk(&dir, "sub/nested.md", "n");

        let entries = store.list(&RelPath::root()).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].name, "a.md");
        assert_eq!(entries[0].kind, DirEntryKind::File);
        assert_eq!(entries[1].name, "sub");
        assert_eq!(entries[1].kind, DirEntryKind::Dir);
        assert_eq!(entries[2].name, "z.md");
        assert_eq!(entries[2].kind, DirEntryKind::File);
    }

    #[test]
    fn list_nonexistent_dir_returns_empty() {
        let (_dir, store) = setup();
        let entries = store.list(&RelPath::new("nope").unwrap()).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn delete_nonexistent_returns_error() {
        let (_dir, mut store) = setup();
        assert!(store.delete(&RelPath::new("nope.md").unwrap()).is_err());
    }

    // --- New staging semantics tests ---

    #[test]
    fn read_falls_through_to_disk() {
        let (dir, store) = setup();
        write_disk(&dir, "on-disk.md", "disk content");
        assert_eq!(
            store.read(&RelPath::new("on-disk.md").unwrap()).unwrap(),
            "disk content"
        );
    }

    #[test]
    fn staged_write_shadows_disk() {
        let (dir, mut store) = setup();
        write_disk(&dir, "f.md", "old");
        store
            .write(&RelPath::new("f.md").unwrap(), "new".to_string())
            .unwrap();
        assert_eq!(store.read(&RelPath::new("f.md").unwrap()).unwrap(), "new");
    }

    #[test]
    fn staged_delete_hides_disk_file() {
        let (dir, mut store) = setup();
        write_disk(&dir, "f.md", "content");
        store.delete(&RelPath::new("f.md").unwrap()).unwrap();
        assert!(store.read(&RelPath::new("f.md").unwrap()).is_err());
        assert!(!store.exists(&RelPath::new("f.md").unwrap()));
    }

    #[test]
    fn commit_deletes_from_disk() {
        let (dir, mut store) = setup();
        write_disk(&dir, "doomed.md", "bye");
        store.delete(&RelPath::new("doomed.md").unwrap()).unwrap();
        store.commit().unwrap();
        assert!(!dir.path().join("doomed.md").exists());
    }

    #[test]
    fn discard_drops_staged_changes() {
        let (dir, mut store) = setup();
        write_disk(&dir, "a.md", "original");
        store
            .write(&RelPath::new("a.md").unwrap(), "modified".to_string())
            .unwrap();
        store
            .write(&RelPath::new("new.md").unwrap(), "new".to_string())
            .unwrap();
        store.discard();

        // Original file still readable from disk
        assert_eq!(
            store.read(&RelPath::new("a.md").unwrap()).unwrap(),
            "original"
        );
        // Staged new file gone
        assert!(!store.exists(&RelPath::new("new.md").unwrap()));
    }

    #[test]
    fn list_merges_staged_writes() {
        let (dir, mut store) = setup();
        write_disk(&dir, "a.md", "a");
        store
            .write(&RelPath::new("c.md").unwrap(), "c".to_string())
            .unwrap();

        let entries = store.list(&RelPath::root()).unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["a.md", "c.md"]);
    }

    #[test]
    fn list_excludes_staged_deletes() {
        let (dir, mut store) = setup();
        write_disk(&dir, "a.md", "a");
        write_disk(&dir, "b.md", "b");
        store.delete(&RelPath::new("a.md").unwrap()).unwrap();

        let entries = store.list(&RelPath::root()).unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["b.md"]);
    }

    #[test]
    fn list_staged_creates_virtual_directory() {
        let (_dir, mut store) = setup();
        store
            .write(
                &RelPath::new("projects/rdm/tasks/foo.md").unwrap(),
                "x".to_string(),
            )
            .unwrap();
        let entries = store.list(&RelPath::root()).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "projects");
        assert_eq!(entries[0].kind, DirEntryKind::Dir);
    }

    #[test]
    fn commit_is_idempotent_when_empty() {
        let (_dir, mut store) = setup();
        store.commit().unwrap();
        store.commit().unwrap();
    }

    #[test]
    fn delete_staged_write() {
        let (_dir, mut store) = setup();
        let path = RelPath::new("ephemeral.md").unwrap();
        store.write(&path, "temp".to_string()).unwrap();
        assert!(store.exists(&path));
        store.delete(&path).unwrap();
        assert!(!store.exists(&path));
    }

    #[test]
    fn commit_uses_atomic_write() {
        // Verify that commit writes through temp file (file should appear atomically)
        let (dir, mut store) = setup();
        let path = RelPath::new("atomic.md").unwrap();
        store.write(&path, "atomic content".to_string()).unwrap();
        store.commit().unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join("atomic.md")).unwrap(),
            "atomic content"
        );
    }

    #[test]
    fn multiple_writes_to_same_path_last_wins() {
        let (dir, mut store) = setup();
        let path = RelPath::new("f.md").unwrap();
        store.write(&path, "first".to_string()).unwrap();
        store.write(&path, "second".to_string()).unwrap();
        assert_eq!(store.read(&path).unwrap(), "second");
        store.commit().unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join("f.md")).unwrap(),
            "second"
        );
    }

    // --- Flush precondition (optimistic concurrency on content) ---

    /// Simulates the other session: a write straight to disk, bypassing this
    /// store entirely — which is exactly what another `rdm` process is, from
    /// this process's point of view.
    fn other_session_writes(dir: &TempDir, path: &str, content: &str) {
        write_disk(dir, path, content);
    }

    fn stale_write_item(err: Error) -> String {
        match err {
            Error::StaleWrite { item, .. } => item,
            other => panic!("expected StaleWrite, got {other:?}"),
        }
    }

    #[test]
    fn a_write_derived_from_a_stale_read_is_refused() {
        let (dir, mut store) = setup();
        let path = RelPath::new("projects/demo/tasks/fix-bug.md").unwrap();
        write_disk(&dir, path.as_str(), "tags: [alpha]");

        // This session reads, then edits — the read-modify-write shape.
        assert_eq!(store.read(&path).unwrap(), "tags: [alpha]");
        store
            .write(&path, "tags: [alpha, from-a]".to_string())
            .unwrap();

        // Another session lands its own edit in between.
        other_session_writes(&dir, path.as_str(), "tags: [alpha, from-b]");

        let err = store.commit().unwrap_err();
        assert_eq!(stale_write_item(err), "task/fix-bug");
        assert_eq!(
            fs::read_to_string(dir.path().join(path.as_str())).unwrap(),
            "tags: [alpha, from-b]",
            "the other session's content must survive untouched"
        );
    }

    #[test]
    fn a_refused_flush_writes_nothing_at_all() {
        // All-or-nothing: a partial flush would leave the repo in a state
        // neither session intended, and the caller could not tell which half
        // landed.
        let (dir, mut store) = setup();
        let conflicted = RelPath::new("projects/demo/tasks/a.md").unwrap();
        let innocent = RelPath::new("projects/demo/tasks/b.md").unwrap();
        write_disk(&dir, conflicted.as_str(), "one");

        store.read(&conflicted).unwrap();
        store.write(&conflicted, "mine".to_string()).unwrap();
        store.write(&innocent, "unrelated".to_string()).unwrap();

        other_session_writes(&dir, conflicted.as_str(), "theirs");

        assert!(store.commit().is_err());
        assert!(
            !dir.path().join(innocent.as_str()).exists(),
            "an unrelated staged path must not land when the flush is refused"
        );
        assert_eq!(
            fs::read_to_string(dir.path().join(conflicted.as_str())).unwrap(),
            "theirs"
        );
        // And the staging overlay survives, so the caller can retry.
        assert_eq!(store.read(&innocent).unwrap(), "unrelated");
    }

    #[test]
    fn a_sessions_own_sequential_flushes_never_trip_the_check() {
        // The AC3 shape in miniature: a clean flush ends the cycle its
        // baselines described, so the next cycle observes disk afresh rather
        // than comparing against a stale read.
        let (dir, mut store) = setup();
        let path = RelPath::new("projects/demo/tasks/a.md").unwrap();

        store.write(&path, "one".to_string()).unwrap();
        store.commit().unwrap();

        let read = store.read(&path).unwrap();
        store.write(&path, format!("{read}-two")).unwrap();
        store.commit().unwrap();

        store.write(&path, "three".to_string()).unwrap();
        store.commit().unwrap();

        assert_eq!(
            fs::read_to_string(dir.path().join(path.as_str())).unwrap(),
            "three"
        );
    }

    #[test]
    fn a_long_lived_store_re_observes_after_each_flush() {
        // A store that outlives one flush — a long-lived host process
        // holding exactly one across many calls — must not stay pinned to
        // what it wrote last time. Otherwise the first
        // external edit to a path this store once wrote would refuse every
        // later write to it forever, including the retry the error message
        // itself recommends.
        let (dir, mut store) = setup();
        let path = RelPath::new("projects/demo/tasks/a.md").unwrap();

        // Cycle 1: this store writes the path and flushes.
        store.write(&path, "mine-one".to_string()).unwrap();
        store.commit().unwrap();

        // Someone else edits it between cycles.
        other_session_writes(&dir, path.as_str(), "theirs");

        // Cycle 2: a fresh read observes their content, and a write derived
        // from that read is legitimate — nothing changed between the read and
        // this flush.
        let current = store.read(&path).unwrap();
        assert_eq!(current, "theirs");
        store.write(&path, format!("{current}+mine")).unwrap();
        store.commit().unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join(path.as_str())).unwrap(),
            "theirs+mine"
        );

        // And the recovery path stays open: a third cycle after another
        // external edit works the same way, rather than wedging permanently.
        other_session_writes(&dir, path.as_str(), "theirs-again");
        let current = store.read(&path).unwrap();
        store.write(&path, format!("{current}+mine")).unwrap();
        store.commit().unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join(path.as_str())).unwrap(),
            "theirs-again+mine"
        );
    }

    #[test]
    fn a_stale_write_is_still_refused_on_a_reused_store() {
        // The flush-scoped baseline must not become a way to lose the
        // protection: within one cycle on a store that has already flushed
        // once, a genuine drift is still caught.
        let (dir, mut store) = setup();
        let path = RelPath::new("projects/demo/tasks/a.md").unwrap();

        store.write(&path, "mine-one".to_string()).unwrap();
        store.commit().unwrap();

        // Cycle 2: read, then someone else lands an edit before this flush.
        let current = store.read(&path).unwrap();
        store.write(&path, format!("{current}+mine")).unwrap();
        other_session_writes(&dir, path.as_str(), "theirs");

        let err = store.commit().unwrap_err();
        assert_eq!(stale_write_item(err), "task/a");
        assert_eq!(
            fs::read_to_string(dir.path().join(path.as_str())).unwrap(),
            "theirs"
        );
    }

    #[test]
    fn a_create_racing_another_create_on_the_same_path_is_refused() {
        // A create only ever probes for existence; recording that probe is
        // what makes the second creator lose rather than silently clobber.
        let (dir, mut store) = setup();
        let path = RelPath::new("projects/demo/tasks/new.md").unwrap();

        assert!(!store.exists(&path));
        store.write(&path, "mine".to_string()).unwrap();

        other_session_writes(&dir, path.as_str(), "theirs");

        let err = store.commit().unwrap_err();
        assert_eq!(stale_write_item(err), "task/new");
    }

    #[test]
    fn a_delete_racing_an_edit_is_refused() {
        let (dir, mut store) = setup();
        let path = RelPath::new("projects/demo/tasks/doomed.md").unwrap();
        write_disk(&dir, path.as_str(), "original");

        store.delete(&path).unwrap();
        other_session_writes(&dir, path.as_str(), "edited by someone else");

        let err = store.commit().unwrap_err();
        assert_eq!(stale_write_item(err), "task/doomed");
        assert!(
            dir.path().join(path.as_str()).exists(),
            "the other session's edit must not be destroyed"
        );
    }

    #[test]
    fn a_byte_identical_concurrent_write_is_not_a_conflict() {
        // There is no lost update when both sessions produced the same bytes.
        let (dir, mut store) = setup();
        let path = RelPath::new("projects/demo/tasks/a.md").unwrap();
        write_disk(&dir, path.as_str(), "same");

        store.read(&path).unwrap();
        store.write(&path, "mine".to_string()).unwrap();
        other_session_writes(&dir, path.as_str(), "same");

        store.commit().unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join(path.as_str())).unwrap(),
            "mine"
        );
    }

    #[test]
    fn an_index_write_is_checked_like_any_other_path() {
        // rdm has no generated-path class: an `INDEX.md` is an ordinary file.
        // A read-then-write of one is refused after a concurrent overwrite,
        // exactly like any authored document.
        let (dir, mut store) = setup();
        let root_index = RelPath::new("INDEX.md").unwrap();
        write_disk(&dir, root_index.as_str(), "old root");

        store.read(&root_index).unwrap();
        store.write(&root_index, "new root".to_string()).unwrap();
        other_session_writes(&dir, root_index.as_str(), "their root");

        let err = store.commit().unwrap_err();
        assert!(
            matches!(&err, Error::StaleWrite { path, .. } if path == "INDEX.md"),
            "a read-then-write of INDEX.md must be refused like any other \
             overwritten path, got {err:?}"
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("INDEX.md")).unwrap(),
            "their root",
            "the refusal must leave the other session's content in place"
        );

        // The other direction, and the shape the retired `rdm index` used to
        // take: a blind write of a path never read, with nothing overwriting
        // it in between, so the write lands. This is the case removing the
        // exemption must not brick. Note a blind write is NOT baseline-free —
        // `write` records whatever is on disk at write time (see `Store::write`
        // above) — so an index-shaped write is guarded from that instant,
        // exactly like any other blind write.
        let (dir2, mut store2) = setup();
        let project_index = RelPath::new("projects/demo/INDEX.md").unwrap();
        write_disk(&dir2, project_index.as_str(), "old project");
        store2
            .write(&project_index, "new project".to_string())
            .unwrap();
        store2.commit().unwrap();
        assert_eq!(
            fs::read_to_string(dir2.path().join(project_index.as_str())).unwrap(),
            "new project",
            "an uncontested index regeneration must still land"
        );
    }

    #[test]
    fn an_unreadable_target_fails_open_rather_than_bricking_the_mutation() {
        // A path whose baseline could not be observed is skipped: the check
        // exists to prevent lost updates, never to become a new way to wedge
        // an unrelated write.
        let (dir, mut store) = setup();
        let path = RelPath::new("projects/demo/tasks/weird.md").unwrap();
        // A directory where a file is expected: `read_to_string` fails with
        // something other than NotFound, so the baseline is `Unknown`.
        fs::create_dir_all(dir.path().join(path.as_str())).unwrap();

        assert!(store.read(&path).is_err());
        store.write(&path, "content".to_string()).unwrap();

        // The flush is attempted rather than refused. Whether the rename
        // succeeds over a directory is the filesystem's business; what
        // matters here is that the *precondition* did not reject it.
        let flushed = store.commit();
        assert!(
            !matches!(flushed, Err(Error::StaleWrite { .. })),
            "an Unknown baseline must be skipped, not treated as drift"
        );
    }

    #[test]
    fn a_staged_read_does_not_overwrite_the_first_touch_baseline() {
        // First touch wins. If a later read served from the overlay re-seeded
        // the baseline, the flush would compare this process's own staged
        // content against itself and never detect anything.
        let (dir, mut store) = setup();
        let path = RelPath::new("projects/demo/tasks/a.md").unwrap();
        write_disk(&dir, path.as_str(), "original");

        store.read(&path).unwrap();
        store.write(&path, "mine".to_string()).unwrap();
        assert_eq!(store.read(&path).unwrap(), "mine"); // served from overlay

        other_session_writes(&dir, path.as_str(), "theirs");

        assert!(
            store.commit().is_err(),
            "the baseline must still be the original disk content"
        );
    }

    #[test]
    fn discard_clears_baselines_along_with_staging() {
        // Baselines describe reads that fed the discarded writes; keeping
        // them would make a later flush compare against an observation this
        // store has disowned.
        let (dir, mut store) = setup();
        let path = RelPath::new("projects/demo/tasks/a.md").unwrap();
        write_disk(&dir, path.as_str(), "original");

        store.read(&path).unwrap();
        store.write(&path, "abandoned".to_string()).unwrap();
        store.discard();

        other_session_writes(&dir, path.as_str(), "theirs");

        // A fresh read-modify-write after the discard is legitimate.
        let current = store.read(&path).unwrap();
        assert_eq!(current, "theirs");
        store.write(&path, format!("{current}+mine")).unwrap();
        store.commit().unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join(path.as_str())).unwrap(),
            "theirs+mine"
        );
    }

    #[test]
    fn a_cloned_store_gets_its_own_baselines() {
        let (dir, mut store) = setup();
        let path = RelPath::new("projects/demo/tasks/a.md").unwrap();
        write_disk(&dir, path.as_str(), "original");
        store.read(&path).unwrap();

        let mut clone = store.clone();
        // The clone inherits the observation but not the identity: writing
        // through the clone must not disturb the original's baselines.
        clone.write(&path, "via clone".to_string()).unwrap();
        clone.commit().unwrap();

        // The original still believes it saw "original", so its own write is
        // correctly refused as stale.
        store.write(&path, "via original".to_string()).unwrap();
        assert!(store.commit().is_err());
    }

    #[test]
    fn staged_paths_carries_the_digest_of_the_content_about_to_flush() {
        let (_dir, mut store) = setup();
        let write = RelPath::new("a.md").unwrap();
        store.write(&write, "hello".to_string()).unwrap();

        let staged = store.staged_paths();
        assert_eq!(staged.len(), 1);
        assert_eq!(staged[0].0, write);
        assert_eq!(staged[0].1, JournalKind::Write);
        assert_eq!(
            staged[0].2.as_deref(),
            Some(content_digest("hello").as_str())
        );
    }

    #[test]
    fn staged_paths_reports_no_digest_for_a_delete() {
        let (dir, mut store) = setup();
        write_disk(&dir, "gone.md", "x");
        store.delete(&RelPath::new("gone.md").unwrap()).unwrap();

        let staged = store.staged_paths();
        assert_eq!(staged.len(), 1);
        assert_eq!(staged[0].1, JournalKind::Delete);
        assert_eq!(staged[0].2, None, "a delete has no bytes to identify");
    }

    #[test]
    fn write_after_delete_resurrects_file() {
        let (dir, mut store) = setup();
        write_disk(&dir, "f.md", "original");
        store.delete(&RelPath::new("f.md").unwrap()).unwrap();
        store
            .write(&RelPath::new("f.md").unwrap(), "resurrected".to_string())
            .unwrap();
        assert_eq!(
            store.read(&RelPath::new("f.md").unwrap()).unwrap(),
            "resurrected"
        );
    }
}

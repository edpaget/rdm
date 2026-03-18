//! Filesystem-backed [`Store`] implementation with in-memory staging.
//!
//! Writes are buffered in memory until [`Store::commit`] flushes them to disk.
//! [`Store::discard`] drops the buffer without touching the filesystem.

#![warn(missing_docs)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::PathBuf;

use rdm_core::error::{Error, Result};
use rdm_core::store::{DirEntry, DirEntryKind, RelPath, Store};

/// A staged entry: either a pending write or a pending delete.
#[derive(Clone, Debug)]
enum StagedEntry {
    /// Content to be written on commit.
    Write(String),
    /// Marker for a pending deletion.
    Delete,
}

/// A [`Store`] backed by the local filesystem with in-memory staging.
///
/// Writes and deletes are buffered in memory. Reads see staged changes first
/// (read-your-own-writes). Call [`Store::commit`] to flush staged changes to
/// disk, or [`Store::discard`] to drop them.
///
/// Commit uses write-to-temp + rename for best-effort atomicity on each file.
#[derive(Clone, Debug)]
pub struct FsStore {
    root: PathBuf,
    staged: BTreeMap<String, StagedEntry>,
}

impl FsStore {
    /// Creates a new `FsStore` rooted at the given path.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            staged: BTreeMap::new(),
        }
    }

    /// Returns the root path of this store.
    pub fn root(&self) -> &std::path::Path {
        &self.root
    }

    /// Resolves a `RelPath` to an absolute filesystem path.
    fn resolve(&self, path: &RelPath) -> PathBuf {
        if path.as_str().is_empty() {
            self.root.clone()
        } else {
            self.root.join(path.as_str())
        }
    }

    /// Checks whether a file exists on disk (ignoring staged state).
    fn exists_on_disk(&self, path: &RelPath) -> bool {
        self.resolve(path).exists()
    }
}

impl Store for FsStore {
    fn read(&self, path: &RelPath) -> Result<String> {
        let key = path.as_str();
        // Check staged first (read-your-own-writes)
        if let Some(entry) = self.staged.get(key) {
            return match entry {
                StagedEntry::Write(content) => Ok(content.clone()),
                StagedEntry::Delete => Err(Error::Io(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("file not found: {key}"),
                ))),
            };
        }
        Ok(fs::read_to_string(self.resolve(path))?)
    }

    fn exists(&self, path: &RelPath) -> bool {
        let key = path.as_str();
        if let Some(entry) = self.staged.get(key) {
            return matches!(entry, StagedEntry::Write(_));
        }
        self.exists_on_disk(path)
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
        for (key, entry) in &self.staged {
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
        self.staged
            .insert(path.as_str().to_string(), StagedEntry::Write(content));
        Ok(())
    }

    fn delete(&mut self, path: &RelPath) -> Result<()> {
        let key = path.as_str();
        if !self.exists(path) {
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("file not found: {key}"),
            )));
        }
        self.staged.insert(key.to_string(), StagedEntry::Delete);
        Ok(())
    }

    fn commit(&mut self) -> Result<()> {
        let staged = std::mem::take(&mut self.staged);
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
        Ok(())
    }

    fn discard(&mut self) {
        self.staged.clear();
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

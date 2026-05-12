//! In-memory store implementation for testing.
//!
//! # Synthetic revision SHAs
//!
//! [`MemoryStore`] supports the revision-scoped reads on [`Store`]
//! ([`Store::head_sha`] and [`Store::fetch_body_at`]) using synthetic SHAs of
//! the form `mem-N`, where `N` increments on each [`Store::commit`] call.
//! The initial state — before any commit — is `mem-0`; the first commit
//! advances HEAD to `mem-1`, and so on. These tokens are deliberately
//! non-hex so any caller that mistakes them for real git SHAs will fail
//! loudly rather than silently match an unrelated git object.
//!
//! Tests that need to seed arbitrary historical state can call
//! [`MemoryStore::seed_snapshot`].

use std::collections::{BTreeMap, BTreeSet};

use crate::error::{Error, Result};

use super::{DirEntry, DirEntryKind, RelPath, Store};

/// A staged entry: either a pending write or a pending delete.
#[derive(Clone, Debug)]
enum StagedEntry {
    Write(String),
    Delete,
}

/// An in-memory [`Store`] backed by `BTreeMap`s.
///
/// Useful for testing without touching the filesystem.
///
/// [`Store::head_sha`] returns a synthetic `mem-N` token (not a real git
/// SHA) that advances on each [`Store::commit`] call. The initial state is
/// `mem-0`; the first committed mutation advances HEAD to `mem-1`, and so
/// on. [`MemoryStore::seed_snapshot`] lets tests seed arbitrary historical
/// snapshots.
#[derive(Clone, Debug)]
pub struct MemoryStore {
    committed: BTreeMap<String, String>,
    staged: BTreeMap<String, StagedEntry>,
    snapshots: BTreeMap<String, BTreeMap<String, String>>,
    head_counter: u64,
}

impl MemoryStore {
    /// Creates a new, empty `MemoryStore`.
    pub fn new() -> Self {
        let committed: BTreeMap<String, String> = BTreeMap::new();
        let mut snapshots = BTreeMap::new();
        snapshots.insert("mem-0".to_string(), committed.clone());
        Self {
            committed,
            staged: BTreeMap::new(),
            snapshots,
            head_counter: 0,
        }
    }

    /// Creates a `MemoryStore` pre-populated with the given committed files.
    ///
    /// The initial revision (`mem-0`) reflects these contents.
    pub fn with_contents(files: Vec<(&str, &str)>) -> Self {
        let committed: BTreeMap<String, String> = files
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let mut snapshots = BTreeMap::new();
        snapshots.insert("mem-0".to_string(), committed.clone());
        Self {
            committed,
            staged: BTreeMap::new(),
            snapshots,
            head_counter: 0,
        }
    }

    /// Inserts a historical snapshot under an arbitrary SHA.
    ///
    /// Intended for tests that need to assert behavior against a specific
    /// revision identifier without driving through the commit path. The SHA
    /// does not need to match the `mem-N` convention.
    pub fn seed_snapshot(&mut self, sha: impl Into<String>, files: Vec<(&str, &str)>) {
        let map: BTreeMap<String, String> = files
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        self.snapshots.insert(sha.into(), map);
    }

    fn current_sha(&self) -> String {
        format!("mem-{}", self.head_counter)
    }
}

impl Default for MemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

impl Store for MemoryStore {
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
        self.committed.get(key).cloned().ok_or_else(|| {
            Error::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("file not found: {key}"),
            ))
        })
    }

    fn exists(&self, path: &RelPath) -> bool {
        let key = path.as_str();
        if let Some(entry) = self.staged.get(key) {
            return matches!(entry, StagedEntry::Write(_));
        }
        self.committed.contains_key(key)
    }

    fn list(&self, path: &RelPath) -> Result<Vec<DirEntry>> {
        let prefix = if path.as_str().is_empty() {
            String::new()
        } else {
            format!("{}/", path.as_str())
        };

        // Collect all effective keys (committed minus staged deletes plus staged writes)
        let mut effective_keys: BTreeSet<&str> = BTreeSet::new();
        for key in self.committed.keys() {
            effective_keys.insert(key.as_str());
        }
        for (key, entry) in &self.staged {
            match entry {
                StagedEntry::Write(_) => {
                    effective_keys.insert(key.as_str());
                }
                StagedEntry::Delete => {
                    effective_keys.remove(key.as_str());
                }
            }
        }

        let mut entries: BTreeMap<String, DirEntryKind> = BTreeMap::new();

        for key in effective_keys {
            let suffix = if prefix.is_empty() {
                key
            } else if let Some(s) = key.strip_prefix(&prefix) {
                s
            } else {
                continue;
            };

            if suffix.is_empty() {
                continue;
            }

            // Direct child: take the first component
            let name = match suffix.split_once('/') {
                Some((first, _)) => {
                    entries
                        .entry(first.to_string())
                        .or_insert(DirEntryKind::Dir);
                    continue;
                }
                None => suffix,
            };

            entries
                .entry(name.to_string())
                .or_insert(DirEntryKind::File);
        }

        Ok(entries
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
        // Check if file exists (in staged or committed)
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
        if staged.is_empty() {
            return Ok(());
        }
        for (key, entry) in staged {
            match entry {
                StagedEntry::Write(content) => {
                    self.committed.insert(key, content);
                }
                StagedEntry::Delete => {
                    self.committed.remove(&key);
                }
            }
        }
        self.head_counter += 1;
        self.snapshots
            .insert(self.current_sha(), self.committed.clone());
        Ok(())
    }

    fn discard(&mut self) {
        self.staged.clear();
    }

    fn head_sha(&self) -> Result<String> {
        Ok(self.current_sha())
    }

    fn fetch_body_at(&self, path: &RelPath, sha: &str) -> Result<String> {
        let snapshot = self
            .snapshots
            .get(sha)
            .ok_or_else(|| Error::RevisionUnknown {
                sha: sha.to_string(),
            })?;
        snapshot
            .get(path.as_str())
            .cloned()
            .ok_or_else(|| Error::BodyAtRevisionMissing {
                path: path.as_str().to_string(),
                sha: sha.to_string(),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_committed_file() {
        let store = MemoryStore::with_contents(vec![("foo.md", "hello")]);
        assert_eq!(
            store.read(&RelPath::new("foo.md").unwrap()).unwrap(),
            "hello"
        );
    }

    #[test]
    fn read_nonexistent_file_returns_error() {
        let store = MemoryStore::new();
        assert!(store.read(&RelPath::new("nope.md").unwrap()).is_err());
    }

    #[test]
    fn read_your_own_writes() {
        let mut store = MemoryStore::with_contents(vec![("f.md", "old")]);
        store
            .write(&RelPath::new("f.md").unwrap(), "new".to_string())
            .unwrap();
        assert_eq!(store.read(&RelPath::new("f.md").unwrap()).unwrap(), "new");
    }

    #[test]
    fn read_staged_delete_returns_error() {
        let mut store = MemoryStore::with_contents(vec![("f.md", "content")]);
        store.delete(&RelPath::new("f.md").unwrap()).unwrap();
        assert!(store.read(&RelPath::new("f.md").unwrap()).is_err());
    }

    #[test]
    fn exists_reflects_staged_state() {
        let mut store = MemoryStore::with_contents(vec![("a.md", "x")]);
        assert!(store.exists(&RelPath::new("a.md").unwrap()));
        store.delete(&RelPath::new("a.md").unwrap()).unwrap();
        assert!(!store.exists(&RelPath::new("a.md").unwrap()));

        store
            .write(&RelPath::new("b.md").unwrap(), "y".to_string())
            .unwrap();
        assert!(store.exists(&RelPath::new("b.md").unwrap()));
    }

    #[test]
    fn list_root_returns_files_and_dirs() {
        let store = MemoryStore::with_contents(vec![
            ("README.md", "hi"),
            ("docs/guide.md", "guide"),
            ("docs/faq.md", "faq"),
            ("src/main.rs", "fn main"),
        ]);
        let entries = store.list(&RelPath::root()).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(
            entries[0],
            DirEntry {
                name: "README.md".to_string(),
                kind: DirEntryKind::File
            }
        );
        assert_eq!(
            entries[1],
            DirEntry {
                name: "docs".to_string(),
                kind: DirEntryKind::Dir
            }
        );
        assert_eq!(
            entries[2],
            DirEntry {
                name: "src".to_string(),
                kind: DirEntryKind::Dir
            }
        );
    }

    #[test]
    fn list_subdirectory() {
        let store = MemoryStore::with_contents(vec![
            ("docs/guide.md", "guide"),
            ("docs/faq.md", "faq"),
            ("docs/advanced/deep.md", "deep"),
        ]);
        let entries = store.list(&RelPath::new("docs").unwrap()).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].name, "advanced");
        assert_eq!(entries[0].kind, DirEntryKind::Dir);
        assert_eq!(entries[1].name, "faq.md");
        assert_eq!(entries[2].name, "guide.md");
    }

    #[test]
    fn list_includes_staged_writes_excludes_staged_deletes() {
        let mut store = MemoryStore::with_contents(vec![("a.md", "a"), ("b.md", "b")]);
        store.delete(&RelPath::new("a.md").unwrap()).unwrap();
        store
            .write(&RelPath::new("c.md").unwrap(), "c".to_string())
            .unwrap();

        let entries = store.list(&RelPath::root()).unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["b.md", "c.md"]);
    }

    #[test]
    fn commit_merges_staged_into_committed() {
        let mut store = MemoryStore::with_contents(vec![("a.md", "old")]);
        store
            .write(&RelPath::new("a.md").unwrap(), "new".to_string())
            .unwrap();
        store
            .write(&RelPath::new("b.md").unwrap(), "added".to_string())
            .unwrap();
        store.commit().unwrap();

        // Staged should be empty now; reads should still work from committed
        assert_eq!(store.read(&RelPath::new("a.md").unwrap()).unwrap(), "new");
        assert_eq!(store.read(&RelPath::new("b.md").unwrap()).unwrap(), "added");
    }

    #[test]
    fn commit_applies_deletes() {
        let mut store = MemoryStore::with_contents(vec![("a.md", "content")]);
        store.delete(&RelPath::new("a.md").unwrap()).unwrap();
        store.commit().unwrap();
        assert!(!store.exists(&RelPath::new("a.md").unwrap()));
    }

    #[test]
    fn discard_clears_staged_changes() {
        let mut store = MemoryStore::with_contents(vec![("a.md", "original")]);
        store
            .write(&RelPath::new("a.md").unwrap(), "modified".to_string())
            .unwrap();
        store.discard();
        assert_eq!(
            store.read(&RelPath::new("a.md").unwrap()).unwrap(),
            "original"
        );
    }

    #[test]
    fn delete_nonexistent_returns_error() {
        let mut store = MemoryStore::new();
        assert!(store.delete(&RelPath::new("nope.md").unwrap()).is_err());
    }

    #[test]
    fn list_empty_directory_returns_empty() {
        let store = MemoryStore::new();
        let entries = store.list(&RelPath::root()).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn list_virtual_dir_from_staged_write() {
        let mut store = MemoryStore::new();
        store
            .write(
                &RelPath::new("projects/rdm/tasks/foo.md").unwrap(),
                "x".to_string(),
            )
            .unwrap();
        let entries = store.list(&RelPath::root()).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0],
            DirEntry {
                name: "projects".to_string(),
                kind: DirEntryKind::Dir
            }
        );
    }

    #[test]
    fn with_contents_convenience() {
        let store = MemoryStore::with_contents(vec![("a.md", "content a"), ("b.md", "content b")]);
        assert_eq!(
            store.read(&RelPath::new("a.md").unwrap()).unwrap(),
            "content a"
        );
        assert_eq!(
            store.read(&RelPath::new("b.md").unwrap()).unwrap(),
            "content b"
        );
    }

    #[test]
    fn head_sha_starts_stable_and_advances_on_commit() {
        let mut store = MemoryStore::new();
        assert_eq!(store.head_sha().unwrap(), "mem-0");
        // Staged writes do not advance HEAD.
        store
            .write(&RelPath::new("a.md").unwrap(), "a".to_string())
            .unwrap();
        assert_eq!(store.head_sha().unwrap(), "mem-0");
        // A commit advances HEAD by one.
        store.commit().unwrap();
        assert_eq!(store.head_sha().unwrap(), "mem-1");
        // An empty commit is a no-op.
        store.commit().unwrap();
        assert_eq!(store.head_sha().unwrap(), "mem-1");
        // Subsequent commits keep advancing.
        store
            .write(&RelPath::new("b.md").unwrap(), "b".to_string())
            .unwrap();
        store.commit().unwrap();
        assert_eq!(store.head_sha().unwrap(), "mem-2");
    }

    #[test]
    fn fetch_body_at_returns_committed_body_at_current_sha() {
        let store = MemoryStore::with_contents(vec![("a.md", "alpha")]);
        let sha = store.head_sha().unwrap();
        let body = store
            .fetch_body_at(&RelPath::new("a.md").unwrap(), &sha)
            .unwrap();
        assert_eq!(body, "alpha");
    }

    #[test]
    fn fetch_body_at_returns_historical_body_after_overwrite() {
        let mut store = MemoryStore::with_contents(vec![("a.md", "v1")]);
        let v1_sha = store.head_sha().unwrap();
        store
            .write(&RelPath::new("a.md").unwrap(), "v2".to_string())
            .unwrap();
        store.commit().unwrap();
        let v2_sha = store.head_sha().unwrap();
        assert_ne!(v1_sha, v2_sha);

        let old = store
            .fetch_body_at(&RelPath::new("a.md").unwrap(), &v1_sha)
            .unwrap();
        let new = store
            .fetch_body_at(&RelPath::new("a.md").unwrap(), &v2_sha)
            .unwrap();
        assert_eq!(old, "v1");
        assert_eq!(new, "v2");
    }

    #[test]
    fn fetch_body_at_returns_body_at_revision_missing_when_deleted_at_sha() {
        let mut store = MemoryStore::with_contents(vec![("a.md", "alpha")]);
        store.delete(&RelPath::new("a.md").unwrap()).unwrap();
        store.commit().unwrap();
        let after_delete = store.head_sha().unwrap();

        let err = store
            .fetch_body_at(&RelPath::new("a.md").unwrap(), &after_delete)
            .unwrap_err();
        match err {
            Error::BodyAtRevisionMissing { path, sha } => {
                assert_eq!(path, "a.md");
                assert_eq!(sha, after_delete);
            }
            other => panic!("expected BodyAtRevisionMissing, got {other:?}"),
        }
    }

    #[test]
    fn fetch_body_at_returns_revision_unknown_for_bogus_sha() {
        let store = MemoryStore::with_contents(vec![("a.md", "alpha")]);
        let err = store
            .fetch_body_at(&RelPath::new("a.md").unwrap(), "mem-does-not-exist")
            .unwrap_err();
        match err {
            Error::RevisionUnknown { sha } => assert_eq!(sha, "mem-does-not-exist"),
            other => panic!("expected RevisionUnknown, got {other:?}"),
        }
    }

    #[test]
    fn seed_snapshot_makes_arbitrary_sha_readable() {
        let mut store = MemoryStore::new();
        store.seed_snapshot("deadbeef", vec![("legacy.md", "from the past")]);
        let body = store
            .fetch_body_at(&RelPath::new("legacy.md").unwrap(), "deadbeef")
            .unwrap();
        assert_eq!(body, "from the past");
    }
}

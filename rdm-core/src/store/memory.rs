//! In-memory store implementation for testing.

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
#[derive(Clone, Debug)]
pub struct MemoryStore {
    committed: BTreeMap<String, String>,
    staged: BTreeMap<String, StagedEntry>,
}

impl MemoryStore {
    /// Creates a new, empty `MemoryStore`.
    pub fn new() -> Self {
        Self {
            committed: BTreeMap::new(),
            staged: BTreeMap::new(),
        }
    }

    /// Creates a `MemoryStore` pre-populated with the given committed files.
    pub fn with_contents(files: Vec<(&str, &str)>) -> Self {
        let committed = files
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Self {
            committed,
            staged: BTreeMap::new(),
        }
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
        Ok(())
    }

    fn discard(&mut self) {
        self.staged.clear();
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
}

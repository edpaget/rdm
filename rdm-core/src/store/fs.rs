//! Filesystem-backed store implementation.

use std::fs;
use std::path::PathBuf;

use crate::error::Result;

use super::{DirEntry, DirEntryKind, RelPath, Store};

/// A [`Store`] backed by the local filesystem.
///
/// Writes and deletes are immediate — `commit()` and `discard()` are no-ops.
/// This preserves the current behavior of direct filesystem I/O.
#[derive(Clone, Debug)]
pub struct FsStore {
    root: PathBuf,
}

impl FsStore {
    /// Creates a new `FsStore` rooted at the given path.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
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
}

impl Store for FsStore {
    fn read(&self, path: &RelPath) -> Result<String> {
        Ok(fs::read_to_string(self.resolve(path))?)
    }

    fn exists(&self, path: &RelPath) -> bool {
        self.resolve(path).exists()
    }

    fn list(&self, path: &RelPath) -> Result<Vec<DirEntry>> {
        let dir = self.resolve(path);
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut entries = Vec::new();
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
            entries.push(DirEntry { name, kind });
        }
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(entries)
    }

    fn write(&mut self, path: &RelPath, content: String) -> Result<()> {
        let full = self.resolve(path);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(full, content)?;
        Ok(())
    }

    fn delete(&mut self, path: &RelPath) -> Result<()> {
        fs::remove_file(self.resolve(path))?;
        Ok(())
    }

    fn commit(&mut self) -> Result<()> {
        // Immediate I/O — nothing to commit.
        Ok(())
    }

    fn discard(&mut self) {
        // Immediate I/O — nothing to discard.
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

    #[test]
    fn write_and_read_round_trip() {
        let (_dir, mut store) = setup();
        let path = RelPath::new("hello.md").unwrap();
        store.write(&path, "world".to_string()).unwrap();
        assert_eq!(store.read(&path).unwrap(), "world");
    }

    #[test]
    fn write_creates_parent_dirs() {
        let (_dir, mut store) = setup();
        let path = RelPath::new("a/b/c.md").unwrap();
        store.write(&path, "deep".to_string()).unwrap();
        assert_eq!(store.read(&path).unwrap(), "deep");
    }

    #[test]
    fn exists_reflects_writes_and_deletes() {
        let (_dir, mut store) = setup();
        let path = RelPath::new("test.md").unwrap();
        assert!(!store.exists(&path));
        store.write(&path, "x".to_string()).unwrap();
        assert!(store.exists(&path));
        store.delete(&path).unwrap();
        assert!(!store.exists(&path));
    }

    #[test]
    fn list_returns_sorted_entries() {
        let (_dir, mut store) = setup();
        store
            .write(&RelPath::new("z.md").unwrap(), "z".to_string())
            .unwrap();
        store
            .write(&RelPath::new("a.md").unwrap(), "a".to_string())
            .unwrap();
        store
            .write(&RelPath::new("sub/nested.md").unwrap(), "n".to_string())
            .unwrap();

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
}

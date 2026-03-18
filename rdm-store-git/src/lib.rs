//! Git-backed [`Store`] implementation with automatic commits via gitoxide.
//!
//! [`GitStore`] wraps [`FsStore`] and creates a git commit on every
//! [`Store::commit`] call. Reads, writes, and deletes are delegated to the
//! inner `FsStore`, with `GitStore` tracking which paths were touched so it
//! can generate meaningful commit messages.

#![warn(missing_docs)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use gix::object::tree::EntryKind;
use gix::objs::tree::EntryMode;
use rdm_core::error::{Error, Result};
use rdm_core::store::{DirEntry, RelPath, Store};
use rdm_store_fs::FsStore;

/// The kind of change tracked for commit message generation.
#[derive(Clone, Debug, PartialEq, Eq)]
enum ChangeKind {
    /// A file was written (created or updated).
    Write,
    /// A file was deleted.
    Delete,
}

/// A [`Store`] backed by git, wrapping [`FsStore`] for filesystem operations.
///
/// Every call to [`Store::commit`] flushes staged changes to disk via the inner
/// `FsStore`, then creates a git commit with an auto-generated message
/// summarizing which files were touched.
pub struct GitStore {
    inner: FsStore,
    repo: gix::Repository,
    touched: BTreeMap<String, ChangeKind>,
}

impl GitStore {
    /// Opens a `GitStore` for an existing git repository.
    ///
    /// # Errors
    ///
    /// Returns `Error::Git` if the path is not inside a git repository.
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        let repo = gix::open(&root).map_err(|e| Error::Git(e.to_string()))?;
        Ok(Self {
            inner: FsStore::new(&root),
            repo,
            touched: BTreeMap::new(),
        })
    }

    /// Initializes a new git repository and opens a `GitStore` for it.
    ///
    /// If the directory is already a git repository, opens it instead.
    ///
    /// # Errors
    ///
    /// Returns `Error::Git` if both initialization and opening fail.
    pub fn init(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        let repo = match gix::init(&root) {
            Ok(repo) => repo,
            Err(_) => gix::open(&root).map_err(|e| Error::Git(e.to_string()))?,
        };
        Ok(Self {
            inner: FsStore::new(&root),
            repo,
            touched: BTreeMap::new(),
        })
    }

    /// Returns the root path of this store.
    pub fn root(&self) -> &Path {
        self.inner.root()
    }

    /// Generates a commit message from the set of touched paths.
    fn commit_message(touched: &BTreeMap<String, ChangeKind>) -> String {
        let writes: Vec<&String> = touched
            .iter()
            .filter(|(_, k)| **k == ChangeKind::Write)
            .map(|(p, _)| p)
            .collect();
        let deletes: Vec<&String> = touched
            .iter()
            .filter(|(_, k)| **k == ChangeKind::Delete)
            .map(|(p, _)| p)
            .collect();

        match (writes.len(), deletes.len()) {
            (1, 0) => format!("rdm: update {}", writes[0]),
            (0, 1) => format!("rdm: delete {}", deletes[0]),
            _ => {
                let total = touched.len();
                let mut msg = format!("rdm: update {total} files\n");
                for path in touched.keys() {
                    msg.push_str(&format!("\n- {path}"));
                }
                msg
            }
        }
    }

    /// Recursively builds a git tree object from a directory on disk.
    ///
    /// Skips the `.git` directory. Writes blob objects for files and
    /// recursively creates subtree objects for directories.
    fn build_tree_from_dir(&self, dir: &Path) -> Result<gix::ObjectId> {
        let mut entries: Vec<gix::objs::tree::Entry> = Vec::new();

        let read_dir = std::fs::read_dir(dir)
            .map_err(|e| Error::Git(format!("failed to read directory {}: {e}", dir.display())))?;

        for entry in read_dir {
            let entry = entry.map_err(|e| Error::Git(e.to_string()))?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| Error::Git("non-UTF-8 filename".to_string()))?;

            if name == ".git" {
                continue;
            }

            let ft = entry
                .file_type()
                .map_err(|e| Error::Git(format!("failed to get file type for {name}: {e}")))?;

            if ft.is_dir() {
                let subtree_id = self.build_tree_from_dir(&entry.path())?;
                entries.push(gix::objs::tree::Entry {
                    mode: EntryMode::from(EntryKind::Tree),
                    filename: name.into(),
                    oid: subtree_id,
                });
            } else {
                let content = std::fs::read(entry.path()).map_err(|e| {
                    Error::Git(format!("failed to read {}: {e}", entry.path().display()))
                })?;
                let blob_id = self
                    .repo
                    .write_blob(&content)
                    .map_err(|e| Error::Git(format!("failed to write blob for {name}: {e}")))?
                    .detach();
                entries.push(gix::objs::tree::Entry {
                    mode: EntryMode::from(EntryKind::Blob),
                    filename: name.into(),
                    oid: blob_id,
                });
            }
        }

        // Git requires tree entries to be sorted
        entries.sort_by(|a, b| a.filename.cmp(&b.filename));

        let tree = gix::objs::Tree { entries };
        let tree_id = self
            .repo
            .write_object(&tree)
            .map_err(|e| Error::Git(format!("failed to write tree: {e}")))?
            .detach();

        Ok(tree_id)
    }
}

impl Store for GitStore {
    fn read(&self, path: &RelPath) -> Result<String> {
        self.inner.read(path)
    }

    fn exists(&self, path: &RelPath) -> bool {
        self.inner.exists(path)
    }

    fn list(&self, path: &RelPath) -> Result<Vec<DirEntry>> {
        self.inner.list(path)
    }

    fn write(&mut self, path: &RelPath, content: String) -> Result<()> {
        self.touched
            .insert(path.as_str().to_string(), ChangeKind::Write);
        self.inner.write(path, content)
    }

    fn delete(&mut self, path: &RelPath) -> Result<()> {
        self.touched
            .insert(path.as_str().to_string(), ChangeKind::Delete);
        self.inner.delete(path)
    }

    fn commit(&mut self) -> Result<()> {
        if self.touched.is_empty() {
            return Ok(());
        }

        let touched = std::mem::take(&mut self.touched);
        let message = Self::commit_message(&touched);

        // Flush files to disk
        self.inner.commit()?;

        // Build tree from working directory
        let root = self.inner.root().to_owned();
        let tree_id = self.build_tree_from_dir(&root)?;

        // Get author/committer identity from git config, falling back to rdm defaults
        let default_sig = || gix::actor::Signature {
            name: "rdm".into(),
            email: "rdm@localhost".into(),
            time: gix::date::Time::now_local_or_utc(),
        };
        let sig = match self.repo.committer() {
            Some(Ok(s)) => s.to_owned().unwrap_or_else(|_| default_sig()),
            _ => default_sig(),
        };
        let mut time_buf = gix::date::parse::TimeBuf::default();
        let sig_ref = sig.to_ref(&mut time_buf);

        // Find parent commit (HEAD), if any
        let parents: Vec<gix::ObjectId> = self
            .repo
            .head()
            .ok()
            .and_then(|mut h| h.peel_to_commit().ok())
            .map(|c| c.id().detach())
            .into_iter()
            .collect();

        // Create commit
        self.repo
            .commit_as(sig_ref, sig_ref, "HEAD", &message, tree_id, parents)
            .map_err(|e| Error::Git(format!("failed to create commit: {e}")))?;

        Ok(())
    }

    fn discard(&mut self) {
        self.touched.clear();
        self.inner.discard();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn init_creates_git_repo() {
        let dir = TempDir::new().unwrap();
        let _store = GitStore::init(dir.path()).unwrap();
        assert!(dir.path().join(".git").exists());
    }

    #[test]
    fn new_opens_existing_repo() {
        let dir = TempDir::new().unwrap();
        gix::init(dir.path()).unwrap();
        let store = GitStore::new(dir.path());
        assert!(store.is_ok());
    }

    #[test]
    fn new_fails_on_non_repo() {
        let dir = TempDir::new().unwrap();
        let result = GitStore::new(dir.path());
        assert!(result.is_err());
    }

    #[test]
    fn write_and_commit_creates_git_commit() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();
        let path = RelPath::new("hello.md").unwrap();
        store.write(&path, "world".to_string()).unwrap();
        store.commit().unwrap();

        let repo = gix::open(dir.path()).unwrap();
        let mut head = repo.head().unwrap();
        let commit = head.peel_to_commit().unwrap();
        assert!(commit.message_raw_sloppy().starts_with(b"rdm:"));
    }

    #[test]
    fn commit_message_single_file() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();
        let path = RelPath::new("test.md").unwrap();
        store.write(&path, "content".to_string()).unwrap();
        store.commit().unwrap();

        let repo = gix::open(dir.path()).unwrap();
        let mut head = repo.head().unwrap();
        let commit = head.peel_to_commit().unwrap();
        let msg = String::from_utf8_lossy(commit.message_raw_sloppy());
        assert_eq!(msg, "rdm: update test.md");
    }

    #[test]
    fn commit_message_multiple_files() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();
        store
            .write(&RelPath::new("a.md").unwrap(), "a".to_string())
            .unwrap();
        store
            .write(&RelPath::new("b.md").unwrap(), "b".to_string())
            .unwrap();
        store.commit().unwrap();

        let repo = gix::open(dir.path()).unwrap();
        let mut head = repo.head().unwrap();
        let commit = head.peel_to_commit().unwrap();
        let msg = String::from_utf8_lossy(commit.message_raw_sloppy());
        assert!(msg.starts_with("rdm: update 2 files"));
        assert!(msg.contains("- a.md"));
        assert!(msg.contains("- b.md"));
    }

    #[test]
    fn delete_is_committed() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();

        // Write and commit a file first
        let path = RelPath::new("doomed.md").unwrap();
        store.write(&path, "bye".to_string()).unwrap();
        store.commit().unwrap();
        assert!(dir.path().join("doomed.md").exists());

        // Delete and commit
        store.delete(&path).unwrap();
        store.commit().unwrap();
        assert!(!dir.path().join("doomed.md").exists());

        // Verify the latest commit message mentions delete
        let repo = gix::open(dir.path()).unwrap();
        let mut head = repo.head().unwrap();
        let commit = head.peel_to_commit().unwrap();
        let msg = String::from_utf8_lossy(commit.message_raw_sloppy());
        assert!(msg.contains("delete"));
    }

    #[test]
    fn discard_does_not_create_commit() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();

        // Create an initial commit so HEAD exists
        let path = RelPath::new("init.md").unwrap();
        store.write(&path, "init".to_string()).unwrap();
        store.commit().unwrap();

        let repo = gix::open(dir.path()).unwrap();
        let head_before = repo.head().unwrap().peel_to_commit().unwrap().id().detach();

        // Write then discard
        store
            .write(&RelPath::new("nope.md").unwrap(), "nope".to_string())
            .unwrap();
        store.discard();

        let repo = gix::open(dir.path()).unwrap();
        let head_after = repo.head().unwrap().peel_to_commit().unwrap().id().detach();
        assert_eq!(head_before, head_after);
        assert!(!dir.path().join("nope.md").exists());
    }

    #[test]
    fn empty_commit_is_noop() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();

        // Create an initial commit
        let path = RelPath::new("init.md").unwrap();
        store.write(&path, "init".to_string()).unwrap();
        store.commit().unwrap();

        let repo = gix::open(dir.path()).unwrap();
        let head_before = repo.head().unwrap().peel_to_commit().unwrap().id().detach();

        // Empty commit
        store.commit().unwrap();

        let repo = gix::open(dir.path()).unwrap();
        let head_after = repo.head().unwrap().peel_to_commit().unwrap().id().detach();
        assert_eq!(head_before, head_after);
    }

    #[test]
    fn read_your_own_writes() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();
        let path = RelPath::new("staged.md").unwrap();
        store.write(&path, "staged content".to_string()).unwrap();
        assert_eq!(store.read(&path).unwrap(), "staged content");
    }
}

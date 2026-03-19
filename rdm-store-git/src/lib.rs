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

/// The kind of file change detected by [`GitStore::git_status`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileChange {
    /// A new file not present in HEAD.
    Added,
    /// An existing file whose content differs from HEAD.
    Modified,
    /// A file present in HEAD but missing from the working directory.
    Deleted,
}

/// A single file's status as reported by [`GitStore::git_status`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileStatus {
    /// The relative path of the file within the repository.
    pub path: String,
    /// The kind of change detected.
    pub change: FileChange,
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
    staging_mode: bool,
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
            staging_mode: false,
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
            staging_mode: false,
        })
    }

    /// Enables or disables staging mode.
    ///
    /// When staging mode is enabled, [`Store::commit`] flushes files to disk
    /// but skips the git commit. Use [`git_commit`](Self::git_commit) to
    /// explicitly create a git commit later.
    pub fn with_staging_mode(mut self, staging: bool) -> Self {
        self.staging_mode = staging;
        self
    }

    /// Returns `true` if staging mode is enabled.
    pub fn staging_mode(&self) -> bool {
        self.staging_mode
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

    /// Builds a tree and creates a git commit with the given message.
    fn create_git_commit(&self, message: &str) -> Result<()> {
        let root = self.inner.root().to_owned();
        let tree_id = self.build_tree_from_dir(&root)?;

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

        let parents: Vec<gix::ObjectId> = self
            .repo
            .head()
            .ok()
            .and_then(|mut h| h.peel_to_commit().ok())
            .map(|c| c.id().detach())
            .into_iter()
            .collect();

        self.repo
            .commit_as(sig_ref, sig_ref, "HEAD", message, tree_id, parents)
            .map_err(|e| Error::Git(format!("failed to create commit: {e}")))?;

        Ok(())
    }

    /// Creates an explicit git commit with the given message.
    ///
    /// This is intended for use in staging mode, where [`Store::commit`]
    /// flushes to disk but skips the git commit. Calling this method creates
    /// a commit from the current working directory state.
    ///
    /// Returns `Ok(())` if the working directory matches HEAD (no-op).
    ///
    /// # Errors
    ///
    /// Returns `Error::Git` if the commit cannot be created.
    pub fn git_commit(&self, message: &str) -> Result<()> {
        let status = self.git_status()?;
        if status.is_empty() {
            return Ok(());
        }
        self.create_git_commit(message)
    }

    /// Compares the working directory to HEAD and returns a list of changes.
    ///
    /// Walks the working directory tree and the HEAD tree, reporting files
    /// that are added, modified, or deleted.
    ///
    /// # Errors
    ///
    /// Returns `Error::Git` if the repository state cannot be read.
    pub fn git_status(&self) -> Result<Vec<FileStatus>> {
        let head_files = self.collect_head_tree()?;
        let work_files = self.collect_working_tree(self.inner.root(), "")?;

        let mut statuses = Vec::new();

        // Check working tree against HEAD
        for (path, work_blob) in &work_files {
            match head_files.get(path) {
                None => statuses.push(FileStatus {
                    path: path.clone(),
                    change: FileChange::Added,
                }),
                Some(head_blob) => {
                    if work_blob != head_blob {
                        statuses.push(FileStatus {
                            path: path.clone(),
                            change: FileChange::Modified,
                        });
                    }
                }
            }
        }

        // Check for deleted files (in HEAD but not in working tree)
        for path in head_files.keys() {
            if !work_files.contains_key(path) {
                statuses.push(FileStatus {
                    path: path.clone(),
                    change: FileChange::Deleted,
                });
            }
        }

        statuses.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(statuses)
    }

    /// Restores the working directory to match HEAD.
    ///
    /// Overwrites modified files, deletes added files, and restores deleted
    /// files. This is a destructive operation.
    ///
    /// # Errors
    ///
    /// Returns `Error::Git` if the HEAD tree cannot be read or files cannot
    /// be written.
    pub fn git_discard(&self) -> Result<()> {
        let status = self.git_status()?;
        if status.is_empty() {
            return Ok(());
        }

        let head_files = self.collect_head_blobs()?;
        let root = self.inner.root();

        for fs in &status {
            let file_path = root.join(&fs.path);
            match fs.change {
                FileChange::Added => {
                    std::fs::remove_file(&file_path)
                        .map_err(|e| Error::Git(format!("failed to remove {}: {e}", fs.path)))?;
                    // Clean up empty parent directories
                    if let Some(parent) = file_path.parent() {
                        let _ = Self::remove_empty_parents(parent, root);
                    }
                }
                FileChange::Modified | FileChange::Deleted => {
                    if let Some(content) = head_files.get(&fs.path) {
                        if let Some(parent) = file_path.parent() {
                            std::fs::create_dir_all(parent).map_err(|e| {
                                Error::Git(format!(
                                    "failed to create directory {}: {e}",
                                    parent.display()
                                ))
                            })?;
                        }
                        std::fs::write(&file_path, content)
                            .map_err(|e| Error::Git(format!("failed to write {}: {e}", fs.path)))?;
                    }
                }
            }
        }

        Ok(())
    }

    /// Removes empty parent directories up to (but not including) `root`.
    fn remove_empty_parents(dir: &Path, root: &Path) -> std::io::Result<()> {
        let mut current = dir;
        while current != root {
            match std::fs::remove_dir(current) {
                Ok(()) => {}
                Err(_) => break, // Not empty or other error
            }
            match current.parent() {
                Some(p) => current = p,
                None => break,
            }
        }
        Ok(())
    }

    /// Collects all files from the HEAD tree as `path -> blob_oid`.
    fn collect_head_tree(&self) -> Result<BTreeMap<String, gix::ObjectId>> {
        let mut files = BTreeMap::new();
        let head = match self
            .repo
            .head()
            .ok()
            .and_then(|mut h| h.peel_to_commit().ok())
        {
            Some(commit) => commit,
            None => return Ok(files), // No commits yet
        };
        let tree = head
            .tree()
            .map_err(|e| Error::Git(format!("failed to get HEAD tree: {e}")))?;
        self.walk_tree(&tree, "", &mut files)?;
        Ok(files)
    }

    /// Collects all file contents from the HEAD tree as `path -> bytes`.
    fn collect_head_blobs(&self) -> Result<BTreeMap<String, Vec<u8>>> {
        let mut files = BTreeMap::new();
        let head = match self
            .repo
            .head()
            .ok()
            .and_then(|mut h| h.peel_to_commit().ok())
        {
            Some(commit) => commit,
            None => return Ok(files),
        };
        let tree = head
            .tree()
            .map_err(|e| Error::Git(format!("failed to get HEAD tree: {e}")))?;
        self.walk_tree_blobs(&tree, "", &mut files)?;
        Ok(files)
    }

    /// Recursively walks a git tree, collecting `path -> blob_oid`.
    fn walk_tree(
        &self,
        tree: &gix::Tree<'_>,
        prefix: &str,
        files: &mut BTreeMap<String, gix::ObjectId>,
    ) -> Result<()> {
        for entry in tree.iter() {
            let entry = entry.map_err(|e| Error::Git(format!("tree entry error: {e}")))?;
            let name = std::str::from_utf8(entry.filename())
                .map_err(|_| Error::Git("non-UTF-8 filename in tree".to_string()))?;
            let path = if prefix.is_empty() {
                name.to_string()
            } else {
                format!("{prefix}/{name}")
            };
            let mode = entry.mode();
            if mode.is_tree() {
                let subtree_obj = self
                    .repo
                    .find_object(entry.oid())
                    .map_err(|e| Error::Git(format!("failed to find object: {e}")))?;
                let subtree = subtree_obj
                    .try_into_tree()
                    .map_err(|e| Error::Git(format!("failed to convert to tree: {e}")))?;
                self.walk_tree(&subtree, &path, files)?;
            } else if mode.is_blob() {
                files.insert(path, entry.oid().to_owned());
            }
        }
        Ok(())
    }

    /// Recursively walks a git tree, collecting `path -> blob content`.
    fn walk_tree_blobs(
        &self,
        tree: &gix::Tree<'_>,
        prefix: &str,
        files: &mut BTreeMap<String, Vec<u8>>,
    ) -> Result<()> {
        for entry in tree.iter() {
            let entry = entry.map_err(|e| Error::Git(format!("tree entry error: {e}")))?;
            let name = std::str::from_utf8(entry.filename())
                .map_err(|_| Error::Git("non-UTF-8 filename in tree".to_string()))?;
            let path = if prefix.is_empty() {
                name.to_string()
            } else {
                format!("{prefix}/{name}")
            };
            let mode = entry.mode();
            if mode.is_tree() {
                let subtree_obj = self
                    .repo
                    .find_object(entry.oid())
                    .map_err(|e| Error::Git(format!("failed to find object: {e}")))?;
                let subtree = subtree_obj
                    .try_into_tree()
                    .map_err(|e| Error::Git(format!("failed to convert to tree: {e}")))?;
                self.walk_tree_blobs(&subtree, &path, files)?;
            } else if mode.is_blob() {
                let blob = self
                    .repo
                    .find_object(entry.oid())
                    .map_err(|e| Error::Git(format!("failed to find blob: {e}")))?;
                files.insert(path, blob.data.to_vec());
            }
        }
        Ok(())
    }

    /// Collects all files from the working directory as `path -> blob_oid`.
    fn collect_working_tree(
        &self,
        dir: &Path,
        prefix: &str,
    ) -> Result<BTreeMap<String, gix::ObjectId>> {
        let mut files = BTreeMap::new();
        let read_dir = match std::fs::read_dir(dir) {
            Ok(rd) => rd,
            Err(_) => return Ok(files),
        };
        for entry in read_dir {
            let entry = entry.map_err(|e| Error::Git(e.to_string()))?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| Error::Git("non-UTF-8 filename".to_string()))?;
            if name == ".git" {
                continue;
            }
            let path = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            let ft = entry
                .file_type()
                .map_err(|e| Error::Git(format!("failed to get file type: {e}")))?;
            if ft.is_dir() {
                let sub = self.collect_working_tree(&entry.path(), &path)?;
                files.extend(sub);
            } else {
                let content = std::fs::read(entry.path())
                    .map_err(|e| Error::Git(format!("failed to read {path}: {e}")))?;
                let blob_id = self
                    .repo
                    .write_blob(&content)
                    .map_err(|e| Error::Git(format!("failed to write blob: {e}")))?
                    .detach();
                files.insert(path, blob_id);
            }
        }
        Ok(files)
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

        // Flush files to disk
        self.inner.commit()?;

        // In staging mode, skip the git commit
        if self.staging_mode {
            return Ok(());
        }

        let message = Self::commit_message(&touched);
        self.create_git_commit(&message)
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

    #[test]
    fn staging_mode_flushes_to_disk_without_git_commit() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap().with_staging_mode(true);

        // Create an initial commit so HEAD exists
        // (need to temporarily disable staging to get initial commit)
        store.staging_mode = false;
        let init_path = RelPath::new("init.md").unwrap();
        store.write(&init_path, "init".to_string()).unwrap();
        store.commit().unwrap();
        store.staging_mode = true;

        let repo = gix::open(dir.path()).unwrap();
        let head_before = repo.head().unwrap().peel_to_commit().unwrap().id().detach();

        // Write and commit in staging mode
        let path = RelPath::new("staged.md").unwrap();
        store.write(&path, "staged content".to_string()).unwrap();
        store.commit().unwrap();

        // File should exist on disk
        assert!(dir.path().join("staged.md").exists());
        let content = std::fs::read_to_string(dir.path().join("staged.md")).unwrap();
        assert_eq!(content, "staged content");

        // But no new git commit should have been created
        let repo = gix::open(dir.path()).unwrap();
        let head_after = repo.head().unwrap().peel_to_commit().unwrap().id().detach();
        assert_eq!(head_before, head_after);
    }

    #[test]
    fn git_commit_creates_commit_with_custom_message() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap().with_staging_mode(true);

        // Create initial commit without staging mode
        store.staging_mode = false;
        store
            .write(&RelPath::new("init.md").unwrap(), "init".to_string())
            .unwrap();
        store.commit().unwrap();
        store.staging_mode = true;

        // Stage a file
        store
            .write(&RelPath::new("new.md").unwrap(), "new content".to_string())
            .unwrap();
        store.commit().unwrap();

        // Now explicitly git commit
        store.git_commit("my custom message").unwrap();

        let repo = gix::open(dir.path()).unwrap();
        let mut head = repo.head().unwrap();
        let commit = head.peel_to_commit().unwrap();
        let msg = String::from_utf8_lossy(commit.message_raw_sloppy());
        assert_eq!(msg, "my custom message");
    }

    #[test]
    fn git_status_detects_added_modified_deleted() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();

        // Create initial state with two files
        store
            .write(&RelPath::new("keep.md").unwrap(), "original".to_string())
            .unwrap();
        store
            .write(&RelPath::new("doomed.md").unwrap(), "delete me".to_string())
            .unwrap();
        store.commit().unwrap();

        // Now make changes directly on disk (simulating staging mode)
        std::fs::write(dir.path().join("keep.md"), "modified").unwrap();
        std::fs::write(dir.path().join("added.md"), "new file").unwrap();
        std::fs::remove_file(dir.path().join("doomed.md")).unwrap();

        let status = store.git_status().unwrap();
        assert_eq!(status.len(), 3);

        let added = status.iter().find(|s| s.path == "added.md").unwrap();
        assert_eq!(added.change, FileChange::Added);

        let modified = status.iter().find(|s| s.path == "keep.md").unwrap();
        assert_eq!(modified.change, FileChange::Modified);

        let deleted = status.iter().find(|s| s.path == "doomed.md").unwrap();
        assert_eq!(deleted.change, FileChange::Deleted);
    }

    #[test]
    fn git_discard_restores_head_state() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();

        // Create initial state
        store
            .write(&RelPath::new("keep.md").unwrap(), "original".to_string())
            .unwrap();
        store
            .write(&RelPath::new("doomed.md").unwrap(), "keep me".to_string())
            .unwrap();
        store.commit().unwrap();

        // Make changes on disk
        std::fs::write(dir.path().join("keep.md"), "modified").unwrap();
        std::fs::write(dir.path().join("added.md"), "new file").unwrap();
        std::fs::remove_file(dir.path().join("doomed.md")).unwrap();

        // Discard
        store.git_discard().unwrap();

        // Verify restored state
        assert_eq!(
            std::fs::read_to_string(dir.path().join("keep.md")).unwrap(),
            "original"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("doomed.md")).unwrap(),
            "keep me"
        );
        assert!(!dir.path().join("added.md").exists());

        // Status should be clean
        let status = store.git_status().unwrap();
        assert!(status.is_empty());
    }

    #[test]
    fn git_commit_noop_when_clean() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();

        store
            .write(&RelPath::new("init.md").unwrap(), "init".to_string())
            .unwrap();
        store.commit().unwrap();

        let repo = gix::open(dir.path()).unwrap();
        let head_before = repo.head().unwrap().peel_to_commit().unwrap().id().detach();

        // Git commit when clean should be a no-op
        store.git_commit("should not appear").unwrap();

        let repo = gix::open(dir.path()).unwrap();
        let head_after = repo.head().unwrap().peel_to_commit().unwrap().id().detach();
        assert_eq!(head_before, head_after);
    }
}

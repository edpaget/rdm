//! Git-backed [`Store`] implementation with automatic commits via gitoxide.
//!
//! [`GitStore`] wraps [`FsStore`] and creates a git commit on every
//! [`Store::commit`] call. Reads, writes, and deletes are delegated to the
//! inner `FsStore`, with `GitStore` tracking which paths were touched so it
//! can generate meaningful commit messages.
//!
//! All git logic — low-level plumbing and high-level porcelain — lives on the
//! [`GitRepo`] collaborator (see the [`repo`], [`commit`], [`remote`], and
//! [`merge`] modules). `GitStore` is a thin adapter composing an `FsStore`
//! with a `GitRepo`, exposing the git capability via [`GitStore::git`] and
//! [`GitStore::git_mut`].

#![warn(missing_docs)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rdm_core::conflict::ConflictItem;
use rdm_core::error::{Error, Result};
use rdm_core::store::{DirEntry, RelPath, Store, VersionedStore};
use rdm_store_fs::FsStore;

pub mod error;
pub mod worktree;

mod commit;
mod merge;
mod process;
mod remote;
mod repo;

pub use repo::{
    GitRepo, commit_messages_since_at, current_branch_at, discover_git_dir, discover_hooks_dir,
    head_commit_info_at, is_ancestor_of_head_at,
};

/// The kind of change tracked for commit message generation.
#[derive(Clone, Debug, PartialEq, Eq)]
enum ChangeKind {
    /// A file was written (created or updated).
    Write,
    /// A file was deleted.
    Delete,
}

/// The kind of file change detected by [`GitRepo::git_status`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileChange {
    /// A new file not present in HEAD.
    Added,
    /// An existing file whose content differs from HEAD.
    Modified,
    /// A file present in HEAD but missing from the working directory.
    Deleted,
}

/// A single file's status as reported by [`GitRepo::git_status`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileStatus {
    /// The relative path of the file within the repository.
    pub path: String,
    /// The kind of change detected.
    pub change: FileChange,
}

/// Information about the HEAD commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadCommitInfo {
    /// The full commit SHA.
    pub sha: String,
    /// The raw commit message.
    pub message: String,
}

/// Information about a configured git remote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteInfo {
    /// The remote's name (e.g., `"origin"`).
    pub name: String,
    /// The remote's fetch URL.
    pub url: String,
}

/// Result of a successful `git push` operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushResult {
    /// The remote that was pushed to.
    pub remote: String,
    /// The branch that was pushed.
    pub branch: String,
    /// Number of commits pushed.
    pub commits_pushed: usize,
}

/// Result of a successful `git pull` (fetch + fast-forward merge) operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullResult {
    /// The remote that was pulled from.
    pub remote: String,
    /// The branch that was pulled.
    pub branch: String,
    /// Number of commits merged.
    pub commits_merged: usize,
    /// Whether any file content changed.
    pub changed: bool,
}

/// Sync status between the local branch and a remote tracking branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncStatus {
    /// The remote name (e.g., `"origin"`).
    pub remote: String,
    /// The local branch name (e.g., `"main"`).
    pub branch: String,
    /// Number of commits ahead of the remote tracking branch.
    pub ahead: usize,
    /// Number of commits behind the remote tracking branch.
    pub behind: usize,
}

/// Result of a merge conflict during pull.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeConflictResult {
    /// The remote that was pulled from.
    pub remote: String,
    /// The branch that was merged.
    pub branch: String,
    /// Files with merge conflicts, classified by rdm item type.
    pub conflicted_files: Vec<ConflictItem>,
}

/// Outcome of a `git_pull` operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PullOutcome {
    /// The pull succeeded (fast-forward or clean merge).
    Success(PullResult),
    /// The merge produced conflicts that need manual resolution.
    Conflict(MergeConflictResult),
}

/// Result of resolving a single conflict file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveResult {
    /// The file that was resolved.
    pub path: String,
    /// Number of unmerged files remaining.
    pub remaining: usize,
    /// Whether the merge was auto-completed (all conflicts resolved).
    pub merge_completed: bool,
}

/// A [`Store`] backed by git, wrapping [`FsStore`] for filesystem operations.
///
/// Every call to [`Store::commit`] flushes staged changes to disk via the inner
/// `FsStore`, then creates a git commit with an auto-generated message
/// summarizing which files were touched. All git logic is delegated to the
/// composed [`GitRepo`], reachable via [`git`](Self::git)/[`git_mut`](Self::git_mut).
pub struct GitStore {
    inner: FsStore,
    git: GitRepo,
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
        let repo = gix::open(&root)
            .map_err(|e| Error::Git(e.to_string()))?
            .into_sync();
        Ok(Self {
            inner: FsStore::new(&root),
            git: GitRepo::new(root, repo),
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

        // Ensure the repo has a local user identity so that CLI git operations
        // (e.g. `git merge`) work even without a global gitconfig.
        // Only sets if not already configured.
        if repo.committer().is_none() {
            let config_path = root.join(".git").join("config");
            if let Ok(contents) = std::fs::read_to_string(&config_path)
                && !contents.contains("[user]")
            {
                let mut file = std::fs::OpenOptions::new()
                    .append(true)
                    .open(&config_path)
                    .map_err(|e| Error::Git(format!("failed to write git config: {e}")))?;
                use std::io::Write;
                writeln!(file, "\n[user]\n\tname = rdm\n\temail = rdm@localhost")
                    .map_err(|e| Error::Git(format!("failed to write git config: {e}")))?;
            }
        }

        Ok(Self {
            inner: FsStore::new(&root),
            git: GitRepo::new(root, repo.into_sync()),
            touched: BTreeMap::new(),
            staging_mode: false,
        })
    }

    /// Clones a remote git repository and opens a `GitStore` for it.
    ///
    /// This is the remote counterpart to [`GitStore::init`]. It shells out to
    /// `git clone` to fetch the remote repository into `root`. When `branch`
    /// is `Some`, `--branch <name>` is passed to `git clone` so the specified
    /// branch is checked out.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Git`] if:
    /// - The target directory exists and is not empty
    /// - `git clone` fails (bad URL, missing branch, network error, etc.)
    /// - The cloned repository cannot be opened
    ///
    /// Returns [`Error::Io`] if parent directory creation or directory reading
    /// fails.
    pub fn clone_remote(url: &str, root: impl Into<PathBuf>, branch: Option<&str>) -> Result<Self> {
        let root = root.into();
        // Reject non-empty target
        if root.exists() {
            let has_entries = std::fs::read_dir(&root)?.next().is_some();
            if has_entries {
                return Err(Error::Git(format!(
                    "target directory is not empty: {}",
                    root.display()
                )));
            }
        }
        // Ensure parent exists
        if let Some(parent) = root.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Clone
        let root_str = root.display().to_string();
        let mut args: Vec<&str> = vec!["clone"];
        if let Some(b) = branch {
            args.push("--branch");
            args.push(b);
        }
        args.push(url);
        args.push(&root_str);
        let output = repo::run_git(&args)?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::Git(format!("git clone failed: {}", stderr.trim())));
        }
        // Open the cloned repo
        let repo = gix::open(&root)
            .map_err(|e| Error::Git(format!("failed to open cloned repo: {e}")))?
            .into_sync();
        Ok(Self {
            inner: FsStore::new(&root),
            git: GitRepo::new(root, repo),
            touched: BTreeMap::new(),
            staging_mode: false,
        })
    }

    /// Enables or disables staging mode.
    ///
    /// When staging mode is enabled, [`Store::commit`] flushes files to disk
    /// but skips the git commit. Use [`GitRepo::git_commit`] to
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
        self.git.root()
    }

    /// Returns the path to the `.git` directory (or the git dir for worktrees).
    pub fn git_dir(&self) -> &Path {
        self.git.git_dir()
    }

    /// Returns the git capability collaborator for read-only git operations.
    pub fn git(&self) -> &GitRepo {
        &self.git
    }

    /// Returns the git capability collaborator for mutating git operations.
    pub fn git_mut(&mut self) -> &mut GitRepo {
        &mut self.git
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
        self.git.create_git_commit(&message)
    }

    fn discard(&mut self) {
        self.touched.clear();
        self.inner.discard();
    }
}

impl VersionedStore for GitStore {
    fn head_sha(&self) -> Result<String> {
        match self.git.head_commit_info()? {
            Some(info) => Ok(info.sha),
            None => Err(Error::HistoryUnavailable),
        }
    }

    fn fetch_body_at(&self, path: &RelPath, sha: &str) -> Result<String> {
        self.git.fetch_body_at(path, sha)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // GitStore must be Send + Sync so it can be wrapped in Mutex for the
    // async MCP server. These assertions catch regressions at the store
    // crate level rather than downstream in rdm-mcp.
    #[test]
    fn gitstore_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<GitStore>();
    }

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
        store.git().git_commit("my custom message").unwrap();

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

        let status = store.git().git_status().unwrap();
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
        store.git().git_discard().unwrap();

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
        let status = store.git().git_status().unwrap();
        assert!(status.is_empty());
    }

    #[test]
    fn git_remote_list_empty() {
        let dir = TempDir::new().unwrap();
        let store = GitStore::init(dir.path()).unwrap();
        let remotes = store.git().git_remote_list().unwrap();
        assert!(remotes.is_empty());
    }

    #[test]
    fn git_remote_add_and_list() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();
        store
            .git_mut()
            .git_remote_add("origin", "https://example.com/repo.git")
            .unwrap();

        let remotes = store.git().git_remote_list().unwrap();
        assert_eq!(remotes.len(), 1);
        assert_eq!(remotes[0].name, "origin");
        assert_eq!(remotes[0].url, "https://example.com/repo.git");
    }

    #[test]
    fn git_remote_add_duplicate_fails() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();
        store
            .git_mut()
            .git_remote_add("origin", "https://example.com/repo.git")
            .unwrap();

        let result = store
            .git_mut()
            .git_remote_add("origin", "https://other.com/repo.git");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("already exists"),
            "expected DuplicateRemote error, got: {err}"
        );
    }

    #[test]
    fn git_remote_remove_and_list() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();
        store
            .git_mut()
            .git_remote_add("origin", "https://example.com/repo.git")
            .unwrap();
        store.git_mut().git_remote_remove("origin").unwrap();

        let remotes = store.git().git_remote_list().unwrap();
        assert!(remotes.is_empty());
    }

    #[test]
    fn git_remote_remove_nonexistent_fails() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();

        let result = store.git_mut().git_remote_remove("nope");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("not found"),
            "expected RemoteNotFound error, got: {err}"
        );
    }

    #[test]
    fn git_remote_list_multiple_sorted() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();
        store
            .git_mut()
            .git_remote_add("upstream", "https://upstream.com/repo.git")
            .unwrap();
        store
            .git_mut()
            .git_remote_add("origin", "https://origin.com/repo.git")
            .unwrap();

        let remotes = store.git().git_remote_list().unwrap();
        assert_eq!(remotes.len(), 2);
        assert_eq!(remotes[0].name, "origin");
        assert_eq!(remotes[1].name, "upstream");
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
        store.git().git_commit("should not appear").unwrap();

        let repo = gix::open(dir.path()).unwrap();
        let head_after = repo.head().unwrap().peel_to_commit().unwrap().id().detach();
        assert_eq!(head_before, head_after);
    }

    /// Returns a git Command with GIT_DIR/GIT_WORK_TREE/GIT_INDEX_FILE cleared.
    /// Sets author/committer identity so commits work on CI without global gitconfig.
    fn git_cmd() -> std::process::Command {
        let mut cmd = std::process::Command::new("git");
        cmd.env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .env("GIT_AUTHOR_NAME", "test")
            .env("GIT_AUTHOR_EMAIL", "test@test.com")
            .env("GIT_COMMITTER_NAME", "test")
            .env("GIT_COMMITTER_EMAIL", "test@test.com");
        cmd
    }

    /// Creates a bare repo clone of the given store's repo for use as a remote.
    /// Returns the bare repo path and adds it as a remote to the store.
    fn setup_bare_remote(store: &mut GitStore, remote_name: &str) -> TempDir {
        let bare_dir = TempDir::new().unwrap();
        // Clone the repo as bare using git CLI
        git_cmd()
            .args(["clone", "--bare"])
            .arg(store.root())
            .arg(bare_dir.path())
            .output()
            .unwrap();
        // Add as remote
        store
            .git_mut()
            .git_remote_add(remote_name, bare_dir.path().to_str().unwrap())
            .unwrap();
        bare_dir
    }

    #[test]
    fn git_fetch_updates_remote_refs() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();
        store
            .write(&RelPath::new("init.md").unwrap(), "init".to_string())
            .unwrap();
        store.commit().unwrap();

        let bare_dir = setup_bare_remote(&mut store, "origin");

        // Push a new commit to the bare repo from a separate clone
        let clone_dir = TempDir::new().unwrap();
        git_cmd()
            .args(["clone"])
            .arg(bare_dir.path())
            .arg(clone_dir.path())
            .output()
            .unwrap();
        std::fs::write(clone_dir.path().join("extra.md"), "new content").unwrap();
        git_cmd()
            .args(["add", "."])
            .current_dir(clone_dir.path())
            .output()
            .unwrap();
        git_cmd()
            .args(["commit", "-m", "add extra"])
            .current_dir(clone_dir.path())
            .output()
            .unwrap();
        git_cmd()
            .args(["push"])
            .current_dir(clone_dir.path())
            .output()
            .unwrap();

        // Before fetch, verify fetch works and creates refs
        store.git_mut().git_fetch("origin").unwrap();

        // After fetch, HEAD branch tracking ref should exist
        let branch = store.git().current_branch_name().unwrap().unwrap();
        let tracking_ref = format!("refs/remotes/origin/{branch}");
        let check = git_cmd()
            .args(["rev-parse", "--verify", "--quiet", &tracking_ref])
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(
            check.status.success(),
            "expected tracking ref {tracking_ref} after fetch"
        );
    }

    #[test]
    fn git_fetch_remote_not_found() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();
        store
            .write(&RelPath::new("init.md").unwrap(), "init".to_string())
            .unwrap();
        store.commit().unwrap();

        let result = store.git_mut().git_fetch("nonexistent");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("not found"),
            "expected RemoteNotFound error, got: {err}"
        );
    }

    #[test]
    fn git_fetch_unreachable_remote() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();
        store
            .write(&RelPath::new("init.md").unwrap(), "init".to_string())
            .unwrap();
        store.commit().unwrap();
        store
            .git_mut()
            .git_remote_add("bad", "/nonexistent/path/to/repo.git")
            .unwrap();

        let result = store.git_mut().git_fetch("bad");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("git error"),
            "expected Git error, got: {err}"
        );
    }

    #[test]
    fn sync_status_up_to_date() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();
        store
            .write(&RelPath::new("init.md").unwrap(), "init".to_string())
            .unwrap();
        store.commit().unwrap();

        let bare_dir = setup_bare_remote(&mut store, "origin");
        store.git_mut().git_fetch("origin").unwrap();

        let status = store.git().git_sync_status("origin").unwrap();
        assert!(status.is_some(), "expected sync status, got None");
        let status = status.unwrap();
        assert_eq!(status.ahead, 0);
        assert_eq!(status.behind, 0);
        assert_eq!(status.remote, "origin");
        let _ = bare_dir; // keep alive
    }

    #[test]
    fn sync_status_ahead() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();
        store
            .write(&RelPath::new("init.md").unwrap(), "init".to_string())
            .unwrap();
        store.commit().unwrap();

        let bare_dir = setup_bare_remote(&mut store, "origin");
        store.git_mut().git_fetch("origin").unwrap();

        // Make two local commits
        store
            .write(&RelPath::new("local1.md").unwrap(), "local1".to_string())
            .unwrap();
        store.commit().unwrap();
        store
            .write(&RelPath::new("local2.md").unwrap(), "local2".to_string())
            .unwrap();
        store.commit().unwrap();

        let status = store.git().git_sync_status("origin").unwrap().unwrap();
        assert_eq!(status.ahead, 2);
        assert_eq!(status.behind, 0);
        let _ = bare_dir;
    }

    #[test]
    fn sync_status_behind() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();
        store
            .write(&RelPath::new("init.md").unwrap(), "init".to_string())
            .unwrap();
        store.commit().unwrap();

        let bare_dir = setup_bare_remote(&mut store, "origin");

        // Push new commits to bare from a separate clone
        let clone_dir = TempDir::new().unwrap();
        git_cmd()
            .args(["clone"])
            .arg(bare_dir.path())
            .arg(clone_dir.path())
            .output()
            .unwrap();
        std::fs::write(clone_dir.path().join("remote1.md"), "remote1").unwrap();
        git_cmd()
            .args(["add", "."])
            .current_dir(clone_dir.path())
            .output()
            .unwrap();
        git_cmd()
            .args(["commit", "-m", "remote commit 1"])
            .current_dir(clone_dir.path())
            .output()
            .unwrap();
        std::fs::write(clone_dir.path().join("remote2.md"), "remote2").unwrap();
        git_cmd()
            .args(["add", "."])
            .current_dir(clone_dir.path())
            .output()
            .unwrap();
        git_cmd()
            .args(["commit", "-m", "remote commit 2"])
            .current_dir(clone_dir.path())
            .output()
            .unwrap();
        git_cmd()
            .args(["push"])
            .current_dir(clone_dir.path())
            .output()
            .unwrap();

        // Fetch to update tracking refs
        store.git_mut().git_fetch("origin").unwrap();

        let status = store.git().git_sync_status("origin").unwrap().unwrap();
        assert_eq!(status.ahead, 0);
        assert_eq!(status.behind, 2);
    }

    #[test]
    fn sync_status_diverged() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();
        store
            .write(&RelPath::new("init.md").unwrap(), "init".to_string())
            .unwrap();
        store.commit().unwrap();

        let bare_dir = setup_bare_remote(&mut store, "origin");
        store.git_mut().git_fetch("origin").unwrap();

        // Make local commit
        store
            .write(&RelPath::new("local.md").unwrap(), "local".to_string())
            .unwrap();
        store.commit().unwrap();

        // Push a different commit to bare from a clone
        let clone_dir = TempDir::new().unwrap();
        git_cmd()
            .args(["clone"])
            .arg(bare_dir.path())
            .arg(clone_dir.path())
            .output()
            .unwrap();
        std::fs::write(clone_dir.path().join("remote.md"), "remote").unwrap();
        git_cmd()
            .args(["add", "."])
            .current_dir(clone_dir.path())
            .output()
            .unwrap();
        git_cmd()
            .args(["commit", "-m", "remote commit"])
            .current_dir(clone_dir.path())
            .output()
            .unwrap();
        git_cmd()
            .args(["push"])
            .current_dir(clone_dir.path())
            .output()
            .unwrap();

        // Fetch to update tracking refs
        store.git_mut().git_fetch("origin").unwrap();

        let status = store.git().git_sync_status("origin").unwrap().unwrap();
        assert!(status.ahead > 0, "expected ahead > 0, got {}", status.ahead);
        assert!(
            status.behind > 0,
            "expected behind > 0, got {}",
            status.behind
        );
    }

    #[test]
    fn sync_status_no_tracking_ref() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();
        store
            .write(&RelPath::new("init.md").unwrap(), "init".to_string())
            .unwrap();
        store.commit().unwrap();

        // Add remote but don't fetch
        store
            .git_mut()
            .git_remote_add("origin", "https://example.com/repo.git")
            .unwrap();

        let status = store.git().git_sync_status("origin").unwrap();
        assert!(status.is_none(), "expected None without tracking ref");
    }

    #[test]
    fn sync_status_detached_head() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();
        store
            .write(&RelPath::new("init.md").unwrap(), "init".to_string())
            .unwrap();
        store.commit().unwrap();

        store
            .git_mut()
            .git_remote_add("origin", "https://example.com/repo.git")
            .unwrap();

        // Detach HEAD using git CLI
        let head_output = git_cmd()
            .args(["rev-parse", "HEAD"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        let head_oid = String::from_utf8_lossy(&head_output.stdout)
            .trim()
            .to_string();
        git_cmd()
            .args(["checkout", &head_oid])
            .current_dir(dir.path())
            .stderr(std::process::Stdio::null())
            .output()
            .unwrap();

        // Reopen the store to pick up detached state
        let store = GitStore::new(dir.path()).unwrap();
        let status = store.git().git_sync_status("origin").unwrap();
        assert!(status.is_none(), "expected None for detached HEAD");
    }

    #[test]
    fn git_push_clean() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();
        store
            .write(&RelPath::new("init.md").unwrap(), "init".to_string())
            .unwrap();
        store.commit().unwrap();

        let bare_dir = setup_bare_remote(&mut store, "origin");
        store.git_mut().git_fetch("origin").unwrap();

        // Make two local commits
        store
            .write(&RelPath::new("a.md").unwrap(), "a".to_string())
            .unwrap();
        store.commit().unwrap();
        store
            .write(&RelPath::new("b.md").unwrap(), "b".to_string())
            .unwrap();
        store.commit().unwrap();

        let result = store.git_mut().git_push("origin", false).unwrap();
        assert_eq!(result.remote, "origin");
        assert_eq!(result.commits_pushed, 2);

        // After push, sync status should be up to date
        let status = store.git().git_sync_status("origin").unwrap().unwrap();
        assert_eq!(status.ahead, 0);
        assert_eq!(status.behind, 0);

        let _ = bare_dir;
    }

    #[test]
    fn git_push_rejected_behind() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();
        store
            .write(&RelPath::new("init.md").unwrap(), "init".to_string())
            .unwrap();
        store.commit().unwrap();

        let bare_dir = setup_bare_remote(&mut store, "origin");
        store.git_mut().git_fetch("origin").unwrap();

        // Push a commit to bare from a separate clone
        let clone_dir = TempDir::new().unwrap();
        git_cmd()
            .args(["clone"])
            .arg(bare_dir.path())
            .arg(clone_dir.path())
            .output()
            .unwrap();
        std::fs::write(clone_dir.path().join("remote.md"), "remote").unwrap();
        git_cmd()
            .args(["add", "."])
            .current_dir(clone_dir.path())
            .output()
            .unwrap();
        git_cmd()
            .args(["commit", "-m", "remote commit"])
            .current_dir(clone_dir.path())
            .output()
            .unwrap();
        git_cmd()
            .args(["push"])
            .current_dir(clone_dir.path())
            .output()
            .unwrap();

        // Make a local commit
        store
            .write(&RelPath::new("local.md").unwrap(), "local".to_string())
            .unwrap();
        store.commit().unwrap();

        // Push should fail — diverged histories
        let result = store.git_mut().git_push("origin", false);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("push rejected")
                || err.to_string().contains("non-fast-forward"),
            "expected push rejection, got: {err}"
        );

        let _ = bare_dir;
    }

    #[test]
    fn git_push_force() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();
        store
            .write(&RelPath::new("init.md").unwrap(), "init".to_string())
            .unwrap();
        store.commit().unwrap();

        let bare_dir = setup_bare_remote(&mut store, "origin");
        store.git_mut().git_fetch("origin").unwrap();

        // Push a commit to bare from a separate clone
        let clone_dir = TempDir::new().unwrap();
        git_cmd()
            .args(["clone"])
            .arg(bare_dir.path())
            .arg(clone_dir.path())
            .output()
            .unwrap();
        std::fs::write(clone_dir.path().join("remote.md"), "remote").unwrap();
        git_cmd()
            .args(["add", "."])
            .current_dir(clone_dir.path())
            .output()
            .unwrap();
        git_cmd()
            .args(["commit", "-m", "remote commit"])
            .current_dir(clone_dir.path())
            .output()
            .unwrap();
        git_cmd()
            .args(["push"])
            .current_dir(clone_dir.path())
            .output()
            .unwrap();

        // Make a local commit
        store
            .write(&RelPath::new("local.md").unwrap(), "local".to_string())
            .unwrap();
        store.commit().unwrap();

        // Force push should succeed
        let result = store.git_mut().git_push("origin", true).unwrap();
        assert_eq!(result.remote, "origin");

        let _ = bare_dir;
    }

    #[test]
    fn git_pull_clean() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();
        store
            .write(&RelPath::new("init.md").unwrap(), "init".to_string())
            .unwrap();
        store.commit().unwrap();

        let bare_dir = setup_bare_remote(&mut store, "origin");

        // Push new commits to bare from a separate clone
        let clone_dir = TempDir::new().unwrap();
        git_cmd()
            .args(["clone"])
            .arg(bare_dir.path())
            .arg(clone_dir.path())
            .output()
            .unwrap();
        std::fs::write(clone_dir.path().join("pulled.md"), "pulled content").unwrap();
        git_cmd()
            .args(["add", "."])
            .current_dir(clone_dir.path())
            .output()
            .unwrap();
        git_cmd()
            .args(["commit", "-m", "add pulled file"])
            .current_dir(clone_dir.path())
            .output()
            .unwrap();
        git_cmd()
            .args(["push"])
            .current_dir(clone_dir.path())
            .output()
            .unwrap();

        let outcome = store.git_mut().git_pull("origin").unwrap();
        match outcome {
            PullOutcome::Success(result) => {
                assert_eq!(result.remote, "origin");
                assert_eq!(result.commits_merged, 1);
                assert!(result.changed);
            }
            PullOutcome::Conflict(_) => panic!("expected success, got conflict"),
        }

        // File should now exist locally
        assert!(dir.path().join("pulled.md").exists());

        let _ = bare_dir;
    }

    #[test]
    fn git_pull_already_up_to_date() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();
        store
            .write(&RelPath::new("init.md").unwrap(), "init".to_string())
            .unwrap();
        store.commit().unwrap();

        let bare_dir = setup_bare_remote(&mut store, "origin");

        let outcome = store.git_mut().git_pull("origin").unwrap();
        match outcome {
            PullOutcome::Success(result) => {
                assert_eq!(result.commits_merged, 0);
                assert!(!result.changed);
            }
            PullOutcome::Conflict(_) => panic!("expected success, got conflict"),
        }

        let _ = bare_dir;
    }

    #[test]
    fn pull_diverged_non_conflicting_merges_cleanly() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();
        store
            .write(&RelPath::new("init.md").unwrap(), "init".to_string())
            .unwrap();
        store.commit().unwrap();

        let bare_dir = setup_bare_remote(&mut store, "origin");
        store.git_mut().git_fetch("origin").unwrap();

        // Make a local commit (different file from remote)
        store
            .write(&RelPath::new("local.md").unwrap(), "local".to_string())
            .unwrap();
        store.commit().unwrap();

        // Push a different file to bare from a clone
        let clone_dir = TempDir::new().unwrap();
        git_cmd()
            .args(["clone"])
            .arg(bare_dir.path())
            .arg(clone_dir.path())
            .output()
            .unwrap();
        std::fs::write(clone_dir.path().join("remote.md"), "remote").unwrap();
        git_cmd()
            .args(["add", "."])
            .current_dir(clone_dir.path())
            .output()
            .unwrap();
        git_cmd()
            .args(["commit", "-m", "remote commit"])
            .current_dir(clone_dir.path())
            .output()
            .unwrap();
        git_cmd()
            .args(["push"])
            .current_dir(clone_dir.path())
            .output()
            .unwrap();

        // Pull should succeed with a clean merge (different files)
        let outcome = store.git_mut().git_pull("origin").unwrap();
        match outcome {
            PullOutcome::Success(result) => {
                assert!(result.changed);
                assert!(result.commits_merged > 0);
            }
            PullOutcome::Conflict(_) => panic!("expected clean merge, got conflict"),
        }

        // Both files should exist
        assert!(dir.path().join("local.md").exists());
        assert!(dir.path().join("remote.md").exists());

        let _ = bare_dir;
    }

    #[test]
    fn pull_diverged_conflicting_detects_conflicts() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();
        store
            .write(&RelPath::new("shared.md").unwrap(), "original".to_string())
            .unwrap();
        store.commit().unwrap();

        let bare_dir = setup_bare_remote(&mut store, "origin");
        store.git_mut().git_fetch("origin").unwrap();

        // Make a local change to shared.md
        store
            .write(
                &RelPath::new("shared.md").unwrap(),
                "local change".to_string(),
            )
            .unwrap();
        store.commit().unwrap();

        // Push a conflicting change to shared.md from a clone
        let clone_dir = TempDir::new().unwrap();
        git_cmd()
            .args(["clone"])
            .arg(bare_dir.path())
            .arg(clone_dir.path())
            .output()
            .unwrap();
        std::fs::write(clone_dir.path().join("shared.md"), "remote change").unwrap();
        git_cmd()
            .args(["add", "."])
            .current_dir(clone_dir.path())
            .output()
            .unwrap();
        git_cmd()
            .args(["commit", "-m", "conflicting remote commit"])
            .current_dir(clone_dir.path())
            .output()
            .unwrap();
        git_cmd()
            .args(["push"])
            .current_dir(clone_dir.path())
            .output()
            .unwrap();

        // Pull should detect conflict
        let outcome = store.git_mut().git_pull("origin").unwrap();
        match outcome {
            PullOutcome::Conflict(conflict) => {
                assert_eq!(conflict.remote, "origin");
                assert!(!conflict.conflicted_files.is_empty());
                assert!(
                    conflict
                        .conflicted_files
                        .iter()
                        .any(|f| f.path == "shared.md")
                );
            }
            PullOutcome::Success(_) => panic!("expected conflict, got success"),
        }

        // Merge should be in progress
        assert!(store.git().git_is_merge_in_progress().unwrap());

        let _ = bare_dir;
    }

    #[test]
    fn resolve_conflict_completes_merge() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();
        store
            .write(&RelPath::new("shared.md").unwrap(), "original".to_string())
            .unwrap();
        store.commit().unwrap();

        let bare_dir = setup_bare_remote(&mut store, "origin");
        store.git_mut().git_fetch("origin").unwrap();

        // Local change
        store
            .write(
                &RelPath::new("shared.md").unwrap(),
                "local change".to_string(),
            )
            .unwrap();
        store.commit().unwrap();

        // Remote conflicting change
        let clone_dir = TempDir::new().unwrap();
        git_cmd()
            .args(["clone"])
            .arg(bare_dir.path())
            .arg(clone_dir.path())
            .output()
            .unwrap();
        std::fs::write(clone_dir.path().join("shared.md"), "remote change").unwrap();
        git_cmd()
            .args(["add", "."])
            .current_dir(clone_dir.path())
            .output()
            .unwrap();
        git_cmd()
            .args(["commit", "-m", "conflicting commit"])
            .current_dir(clone_dir.path())
            .output()
            .unwrap();
        git_cmd()
            .args(["push"])
            .current_dir(clone_dir.path())
            .output()
            .unwrap();

        // Pull to get conflict
        let outcome = store.git_mut().git_pull("origin").unwrap();
        assert!(matches!(outcome, PullOutcome::Conflict(_)));

        // Resolve the conflict by writing resolved content
        std::fs::write(dir.path().join("shared.md"), "resolved content").unwrap();

        let result = store.git_mut().git_resolve_conflict("shared.md").unwrap();
        assert_eq!(result.path, "shared.md");
        assert_eq!(result.remaining, 0);
        assert!(result.merge_completed);

        // Merge should no longer be in progress
        assert!(!store.git().git_is_merge_in_progress().unwrap());

        let _ = bare_dir;
    }

    #[test]
    fn resolve_when_no_merge_errors() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();
        store
            .write(&RelPath::new("init.md").unwrap(), "init".to_string())
            .unwrap();
        store.commit().unwrap();

        let result = store.git_mut().git_resolve_conflict("init.md");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("no merge in progress"),
            "expected NoMergeInProgress, got: {err}"
        );
    }

    #[test]
    fn commit_messages_since_returns_multiple_commits() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();

        // Create initial commit
        store
            .write(&RelPath::new("init.md").unwrap(), "init".to_string())
            .unwrap();
        store.commit().unwrap();

        // Tag the initial commit as our anchor
        git_cmd()
            .args(["tag", "anchor"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        // Create three more commits
        store
            .write(&RelPath::new("a.md").unwrap(), "a".to_string())
            .unwrap();
        store.commit().unwrap();
        store
            .write(&RelPath::new("b.md").unwrap(), "b".to_string())
            .unwrap();
        store.commit().unwrap();
        store
            .write(&RelPath::new("c.md").unwrap(), "c".to_string())
            .unwrap();
        store.commit().unwrap();

        let commits = store.git().commit_messages_since(Some("anchor")).unwrap();
        assert_eq!(
            commits.len(),
            3,
            "expected 3 commits, got {}",
            commits.len()
        );

        // Commits should be newest-first
        assert!(commits[0].message.contains("c.md"));
        assert!(commits[1].message.contains("b.md"));
        assert!(commits[2].message.contains("a.md"));

        // Each should have a valid SHA
        for c in &commits {
            assert_eq!(c.sha.len(), 40, "expected 40-char SHA, got {}", c.sha);
        }
    }

    #[test]
    fn commit_messages_since_empty_range() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();
        store
            .write(&RelPath::new("init.md").unwrap(), "init".to_string())
            .unwrap();
        store.commit().unwrap();

        // HEAD..HEAD should be empty
        let commits = store.git().commit_messages_since(Some("HEAD")).unwrap();
        assert!(commits.is_empty());
    }

    #[test]
    fn commit_messages_since_invalid_ref_returns_empty() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();
        store
            .write(&RelPath::new("init.md").unwrap(), "init".to_string())
            .unwrap();
        store.commit().unwrap();

        let commits = store
            .git()
            .commit_messages_since(Some("nonexistent-ref-abc123"))
            .unwrap();
        assert!(commits.is_empty());
    }

    #[test]
    fn is_ancestor_of_head_distinguishes_branches() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();
        // Base commit on the default branch.
        store
            .write(&RelPath::new("base.md").unwrap(), "base".to_string())
            .unwrap();
        store.commit().unwrap();
        let base_sha = store.git().head_commit_info().unwrap().unwrap().sha;

        // Branch A: a commit reachable from the branch-A tip.
        git_cmd()
            .args(["checkout", "-b", "branch-a"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        store
            .write(&RelPath::new("a.md").unwrap(), "a".to_string())
            .unwrap();
        store.commit().unwrap();
        let a_sha = store.git().head_commit_info().unwrap().unwrap().sha;

        // Branch B (off base): its tip is NOT reachable from branch A's HEAD.
        git_cmd()
            .args(["checkout", "-b", "branch-b", &base_sha])
            .current_dir(dir.path())
            .stderr(std::process::Stdio::null())
            .output()
            .unwrap();
        store
            .write(&RelPath::new("b.md").unwrap(), "b".to_string())
            .unwrap();
        store.commit().unwrap();
        let b_sha = store.git().head_commit_info().unwrap().unwrap().sha;

        // Back on branch A.
        git_cmd()
            .args(["checkout", "branch-a"])
            .current_dir(dir.path())
            .stderr(std::process::Stdio::null())
            .output()
            .unwrap();

        // From branch A's HEAD: base and A's own tip are ancestors; B is not.
        assert!(is_ancestor_of_head_at(dir.path(), &base_sha).unwrap());
        assert!(is_ancestor_of_head_at(dir.path(), &a_sha).unwrap());
        assert!(!is_ancestor_of_head_at(dir.path(), &b_sha).unwrap());
    }

    #[test]
    fn is_ancestor_of_head_errors_on_unknown_sha() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();
        store
            .write(&RelPath::new("init.md").unwrap(), "init".to_string())
            .unwrap();
        store.commit().unwrap();

        let result = is_ancestor_of_head_at(dir.path(), "0000000000000000000000000000000000000000");
        assert!(result.is_err());
    }

    #[test]
    fn list_unmerged_empty_when_clean() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();
        store
            .write(&RelPath::new("init.md").unwrap(), "init".to_string())
            .unwrap();
        store.commit().unwrap();

        let unmerged = store.git().git_list_unmerged().unwrap();
        assert!(unmerged.is_empty());
    }

    #[test]
    fn discover_git_dir_finds_repo() {
        let dir = TempDir::new().unwrap();
        gix::init(dir.path()).unwrap();
        let git_dir = discover_git_dir(dir.path()).unwrap();
        assert!(git_dir.ends_with(".git"));
    }

    #[test]
    fn discover_git_dir_from_subdir() {
        let dir = TempDir::new().unwrap();
        gix::init(dir.path()).unwrap();
        let sub = dir.path().join("a/b");
        std::fs::create_dir_all(&sub).unwrap();
        let git_dir = discover_git_dir(&sub).unwrap();
        assert!(git_dir.ends_with(".git"));
    }

    #[test]
    fn discover_git_dir_errors_for_non_repo() {
        let dir = TempDir::new().unwrap();
        assert!(discover_git_dir(dir.path()).is_err());
    }

    #[test]
    fn head_commit_info_at_returns_none_for_empty_repo() {
        let dir = TempDir::new().unwrap();
        gix::init(dir.path()).unwrap();
        let info = head_commit_info_at(dir.path()).unwrap();
        assert!(info.is_none());
    }

    #[test]
    fn head_commit_info_at_reads_commit() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();
        store
            .write(&RelPath::new("file.md").unwrap(), "content".to_string())
            .unwrap();
        store.commit().unwrap();

        let info = head_commit_info_at(dir.path()).unwrap().unwrap();
        assert!(!info.sha.is_empty());
        assert!(!info.message.is_empty());
    }

    #[test]
    fn commit_messages_since_at_returns_commits() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();
        store
            .write(&RelPath::new("init.md").unwrap(), "init".to_string())
            .unwrap();
        store.commit().unwrap();

        git_cmd()
            .args(["tag", "anchor"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        store
            .write(&RelPath::new("a.md").unwrap(), "a".to_string())
            .unwrap();
        store.commit().unwrap();
        store
            .write(&RelPath::new("b.md").unwrap(), "b".to_string())
            .unwrap();
        store.commit().unwrap();

        let commits = commit_messages_since_at(dir.path(), Some("anchor")).unwrap();
        assert_eq!(commits.len(), 2);
        assert!(commits[0].message.contains("b.md"));
        assert!(commits[1].message.contains("a.md"));
    }

    #[test]
    fn commit_messages_since_at_empty_range() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();
        store
            .write(&RelPath::new("init.md").unwrap(), "init".to_string())
            .unwrap();
        store.commit().unwrap();

        let commits = commit_messages_since_at(dir.path(), Some("HEAD")).unwrap();
        assert!(commits.is_empty());
    }

    // -- clone_remote tests --

    /// Helper: creates a bare clone of a plan-repo-like git repo.
    fn make_bare_plan_repo() -> (TempDir, TempDir) {
        let source = TempDir::new().unwrap();
        let mut store = GitStore::init(source.path()).unwrap();
        // Write rdm.toml and INDEX.md to simulate a plan repo
        store
            .write(
                &RelPath::new("rdm.toml").unwrap(),
                "default_project = \"demo\"\n".to_string(),
            )
            .unwrap();
        store
            .write(&RelPath::new("INDEX.md").unwrap(), "# Index\n".to_string())
            .unwrap();
        store.commit().unwrap();

        let bare = TempDir::new().unwrap();
        std::process::Command::new("git")
            .args(["clone", "--bare"])
            .arg(source.path())
            .arg(bare.path())
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .output()
            .unwrap();

        (source, bare)
    }

    #[test]
    fn clone_remote_creates_working_store() {
        let (_source, bare) = make_bare_plan_repo();
        let target = TempDir::new().unwrap();
        let target_path = target.path().join("cloned");

        let store =
            GitStore::clone_remote(bare.path().to_str().unwrap(), &target_path, None).unwrap();

        assert!(target_path.join(".git").exists());
        assert!(target_path.join("rdm.toml").exists());
        assert!(target_path.join("INDEX.md").exists());

        // Should have "origin" remote
        let remotes = store.git().git_remote_list().unwrap();
        assert!(remotes.iter().any(|r| r.name == "origin"));
    }

    #[test]
    fn clone_remote_fails_nonempty_dir() {
        let (_source, bare) = make_bare_plan_repo();
        let target = TempDir::new().unwrap();
        // Pre-populate target
        std::fs::write(target.path().join("blocker.txt"), "hi").unwrap();

        let result = GitStore::clone_remote(bare.path().to_str().unwrap(), target.path(), None);
        match result {
            Err(e) => assert!(e.to_string().contains("not empty"), "got: {e}"),
            Ok(_) => panic!("expected error for non-empty dir"),
        }
    }

    #[test]
    fn commit_syncs_git_index_so_status_is_clean() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();
        let path = RelPath::new("hello.md").unwrap();
        store.write(&path, "world".to_string()).unwrap();
        store.commit().unwrap();

        // After rdm commits, `git status` should report a clean tree.
        let output = git_cmd()
            .args(["status", "--porcelain"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        let status = String::from_utf8_lossy(&output.stdout);
        assert!(
            status.trim().is_empty(),
            "expected clean git status after commit, got: {status}"
        );
    }

    #[test]
    fn tree_sorting_dirs_sort_with_trailing_slash() {
        // Git sorts tree entries so that directories compare as if their name
        // ends with '/'.  This means a dir named "foo" sorts AFTER "foo.md"
        // because "foo/" > "foo.md" (byte '/' 0x2F < '.' 0x2E is false, but
        // actually '/' > '.' so "foo/" > "foo.").  The key case is names where
        // a plain comparison differs from the trailing-slash comparison.
        //
        // Concretely: "ab" (dir) vs "ab.c" (file).
        //   plain:  "ab" < "ab.c"   (shorter string)
        //   git:    "ab/" > "ab.c"  ('/' = 0x2F > '.' = 0x2E)
        //
        // If we get this wrong, `git fsck` rejects with "treeNotSorted".
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();

        // Create a directory "ab" with a file inside, and a file "ab.c"
        std::fs::create_dir_all(dir.path().join("ab")).unwrap();
        std::fs::write(dir.path().join("ab").join("x.md"), "inner").unwrap();
        std::fs::write(dir.path().join("ab.c"), "blob").unwrap();

        let path = RelPath::new("ab.c").unwrap();
        store.write(&path, "blob".to_string()).unwrap();
        let inner = RelPath::new("ab/x.md").unwrap();
        store.write(&inner, "inner".to_string()).unwrap();
        store.commit().unwrap();

        // Verify git fsck passes (would fail with "treeNotSorted" before fix)
        let output = git_cmd()
            .args(["fsck", "--strict"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git fsck failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn clone_remote_fails_bad_url() {
        let target = TempDir::new().unwrap();
        let target_path = target.path().join("cloned");

        let result = GitStore::clone_remote("file:///nonexistent/repo.git", &target_path, None);
        match result {
            Err(e) => assert!(e.to_string().contains("git clone failed"), "got: {e}"),
            Ok(_) => panic!("expected error for bad URL"),
        }
    }

    #[test]
    fn head_sha_returns_current_commit() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();
        let path = RelPath::new("a.md").unwrap();
        store.write(&path, "first".to_string()).unwrap();
        store.commit().unwrap();

        let sha = store.head_sha().unwrap();
        let info = store.git().head_commit_info().unwrap().unwrap();
        assert_eq!(sha, info.sha);
        assert_eq!(sha.len(), 40);
    }

    #[test]
    fn head_sha_returns_history_unavailable_on_unborn_head() {
        let dir = TempDir::new().unwrap();
        let store = GitStore::init(dir.path()).unwrap();
        match store.head_sha() {
            Err(Error::HistoryUnavailable) => {}
            other => panic!("expected HistoryUnavailable, got {other:?}"),
        }
    }

    #[test]
    fn fetch_body_at_returns_body_at_previous_commit() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();
        let path = RelPath::new("a.md").unwrap();
        store.write(&path, "v1".to_string()).unwrap();
        store.commit().unwrap();
        let v1_sha = store.head_sha().unwrap();

        store.write(&path, "v2".to_string()).unwrap();
        store.commit().unwrap();
        let v2_sha = store.head_sha().unwrap();
        assert_ne!(v1_sha, v2_sha);

        let old = store.fetch_body_at(&path, &v1_sha).unwrap();
        let new = store.fetch_body_at(&path, &v2_sha).unwrap();
        assert_eq!(old, "v1");
        assert_eq!(new, "v2");
    }

    #[test]
    fn fetch_body_at_returns_body_at_revision_missing_when_path_absent_at_sha() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();
        // Initial commit: only seed.md exists.
        store
            .write(&RelPath::new("seed.md").unwrap(), "seed".to_string())
            .unwrap();
        store.commit().unwrap();
        let early_sha = store.head_sha().unwrap();

        // Later commit: introduce later.md.
        store
            .write(&RelPath::new("later.md").unwrap(), "later".to_string())
            .unwrap();
        store.commit().unwrap();

        let err = store
            .fetch_body_at(&RelPath::new("later.md").unwrap(), &early_sha)
            .unwrap_err();
        match err {
            Error::BodyAtRevisionMissing { path, sha } => {
                assert_eq!(path, "later.md");
                assert_eq!(sha, early_sha);
            }
            other => panic!("expected BodyAtRevisionMissing, got {other:?}"),
        }
    }

    #[test]
    fn fetch_body_at_returns_revision_unknown_for_bogus_sha() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();
        let path = RelPath::new("a.md").unwrap();
        store.write(&path, "content".to_string()).unwrap();
        store.commit().unwrap();

        let err = store
            .fetch_body_at(&path, "0000000000000000000000000000000000000000")
            .unwrap_err();
        match err {
            Error::RevisionUnknown { sha } => {
                assert_eq!(sha, "0000000000000000000000000000000000000000");
            }
            other => panic!("expected RevisionUnknown, got {other:?}"),
        }
    }
}

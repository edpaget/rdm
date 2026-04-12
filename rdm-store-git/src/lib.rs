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
use rdm_core::conflict::{self, ConflictItem};
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
/// summarizing which files were touched.
pub struct GitStore {
    inner: FsStore,
    repo: gix::ThreadSafeRepository,
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
            repo: repo.into_sync(),
            touched: BTreeMap::new(),
            staging_mode: false,
        })
    }

    /// Clones a remote git repository and opens a `GitStore` for it.
    ///
    /// This is the remote counterpart to [`GitStore::init`]. It shells out to
    /// `git clone` to fetch the remote repository into `root`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Git`] if:
    /// - The target directory exists and is not empty
    /// - `git clone` fails (bad URL, network error, etc.)
    /// - The cloned repository cannot be opened
    ///
    /// Returns [`Error::Io`] if parent directory creation or directory reading
    /// fails.
    pub fn clone_remote(url: &str, root: impl Into<PathBuf>) -> Result<Self> {
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
        let output = run_git(&["clone", url, &root.display().to_string()])?;
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

    /// Returns the path to the `.git` directory (or the git dir for worktrees).
    pub fn git_dir(&self) -> &Path {
        self.repo.git_dir()
    }

    /// Information about the HEAD commit: SHA and full message.
    ///
    /// Returns `Ok(None)` if the repository has no commits (unborn HEAD).
    ///
    /// # Errors
    ///
    /// Returns `Error::Git` if the repository state cannot be read.
    pub fn head_commit_info(&self) -> Result<Option<HeadCommitInfo>> {
        let repo = self.repo.to_thread_local();
        let commit = match repo.head().ok().and_then(|mut h| h.peel_to_commit().ok()) {
            Some(c) => c,
            None => return Ok(None),
        };
        let sha = commit.id().to_string();
        let message = commit.message_raw_sloppy().to_string();
        Ok(Some(HeadCommitInfo { sha, message }))
    }

    /// Returns commit info for all commits in a range.
    ///
    /// When `since_ref` is `None`, uses `HEAD@{1}` (the reflog entry before the
    /// current HEAD) as the exclusion anchor — this covers the commits introduced
    /// by the most recent merge or pull.
    ///
    /// When `since_ref` is `Some(ref_str)`, uses that ref as the exclusion
    /// anchor — useful for backfilling or scanning a specific range.
    ///
    /// Returns commits newest-first. Returns an empty vec if the range is empty
    /// or the anchor ref is invalid.
    ///
    /// # Errors
    ///
    /// Returns `Error::Git` if the git command cannot be executed.
    pub fn commit_messages_since(&self, since_ref: Option<&str>) -> Result<Vec<HeadCommitInfo>> {
        let anchor = since_ref.unwrap_or("HEAD@{1}");
        let output = self.run_git(&["log", "--format=%H%n%B%n<END>", "HEAD", "--not", anchor])?;
        if !output.status.success() {
            // Anchor ref may not exist (e.g. shallow clone, no reflog).
            // Return empty rather than failing.
            return Ok(Vec::new());
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut commits = Vec::new();
        for block in stdout.split("<END>") {
            let block = block.trim();
            if block.is_empty() {
                continue;
            }
            // First line is the SHA, rest is the message.
            if let Some((sha, message)) = block.split_once('\n') {
                commits.push(HeadCommitInfo {
                    sha: sha.trim().to_string(),
                    message: message.trim().to_string(),
                });
            }
        }
        Ok(commits)
    }

    /// Returns the name of the remote's default branch.
    ///
    /// Tries `git symbolic-ref refs/remotes/origin/HEAD`, strips the prefix,
    /// and falls back to `"main"` if that fails.
    pub fn default_branch_name(&self) -> Result<String> {
        let output = self.run_git(&["symbolic-ref", "refs/remotes/origin/HEAD"]);
        if let Ok(ref o) = output
            && o.status.success()
        {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if let Some(branch) = s.strip_prefix("refs/remotes/origin/") {
                return Ok(branch.to_string());
            }
        }
        Ok("main".to_string())
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
    fn build_tree_from_dir(&self, repo: &gix::Repository, dir: &Path) -> Result<gix::ObjectId> {
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
                let subtree_id = self.build_tree_from_dir(repo, &entry.path())?;
                entries.push(gix::objs::tree::Entry {
                    mode: EntryMode::from(EntryKind::Tree),
                    filename: name.into(),
                    oid: subtree_id,
                });
            } else {
                let content = std::fs::read(entry.path()).map_err(|e| {
                    Error::Git(format!("failed to read {}: {e}", entry.path().display()))
                })?;
                let blob_id = repo
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

        // Git requires tree entries sorted by name, with directories compared
        // as if their name has a trailing '/'.  A plain byte comparison gets
        // this wrong (e.g. "foo" dir vs "foo.md" blob) and produces trees that
        // `git fsck` rejects with "treeNotSorted".
        entries.sort_by(|a, b| {
            let a_name = &*a.filename;
            let b_name = &*b.filename;
            let a_is_tree = a.mode == EntryMode::from(EntryKind::Tree);
            let b_is_tree = b.mode == EntryMode::from(EntryKind::Tree);
            let a_key: Vec<u8> = if a_is_tree {
                a_name.iter().chain(b"/").copied().collect()
            } else {
                a_name.to_vec()
            };
            let b_key: Vec<u8> = if b_is_tree {
                b_name.iter().chain(b"/").copied().collect()
            } else {
                b_name.to_vec()
            };
            a_key.cmp(&b_key)
        });

        let tree = gix::objs::Tree { entries };
        let tree_id = repo
            .write_object(&tree)
            .map_err(|e| Error::Git(format!("failed to write tree: {e}")))?
            .detach();

        Ok(tree_id)
    }

    /// Builds a tree and creates a git commit with the given message.
    fn create_git_commit(&self, message: &str) -> Result<()> {
        let repo = self.repo.to_thread_local();
        let root = self.inner.root().to_owned();
        let tree_id = self.build_tree_from_dir(&repo, &root)?;

        let default_sig = || gix::actor::Signature {
            name: "rdm".into(),
            email: "rdm@localhost".into(),
            time: gix::date::Time::now_local_or_utc(),
        };
        let sig = match repo.committer() {
            Some(Ok(s)) => s.to_owned().unwrap_or_else(|_| default_sig()),
            _ => default_sig(),
        };
        let mut time_buf = gix::date::parse::TimeBuf::default();
        let sig_ref = sig.to_ref(&mut time_buf);

        let parents: Vec<gix::ObjectId> = repo
            .head()
            .ok()
            .and_then(|mut h| h.peel_to_commit().ok())
            .map(|c| c.id().detach())
            .into_iter()
            .collect();

        repo.commit_as(sig_ref, sig_ref, "HEAD", message, tree_id, parents)
            .map_err(|e| Error::Git(format!("failed to create commit: {e}")))?;

        // We built the tree directly, bypassing the git index.  Sync the index
        // to HEAD so that `git status` doesn't report every touched file as
        // modified.
        self.sync_index_to_head()?;

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
        let repo = self.repo.to_thread_local();
        let head_files = self.collect_head_tree(&repo)?;
        let work_files = self.collect_working_tree(&repo, self.inner.root(), "")?;

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

        let repo = self.repo.to_thread_local();
        let head_files = self.collect_head_blobs(&repo)?;
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

    /// Lists all configured git remotes with their fetch URLs.
    ///
    /// Returns remotes sorted alphabetically by name.
    ///
    /// # Errors
    ///
    /// Returns `Error::Git` if the repository configuration cannot be read.
    pub fn git_remote_list(&self) -> Result<Vec<RemoteInfo>> {
        let config_path = self.repo.git_dir().join("config");
        let content = std::fs::read_to_string(&config_path)
            .map_err(|e| Error::Git(format!("failed to read git config: {e}")))?;

        let mut remotes = Vec::new();
        let mut current_remote: Option<String> = None;
        let mut current_url: Option<String> = None;

        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('[') {
                if let Some(name) = current_remote.take() {
                    remotes.push(RemoteInfo {
                        name,
                        url: current_url.take().unwrap_or_default(),
                    });
                }
                if let Some(rest) = trimmed.strip_prefix("[remote \"")
                    && let Some(name) = rest.strip_suffix("\"]")
                {
                    current_remote = Some(name.to_string());
                    current_url = None;
                }
            } else if current_remote.is_some()
                && let Some(url_val) = trimmed.strip_prefix("url = ")
            {
                current_url = Some(url_val.to_string());
            }
        }
        if let Some(name) = current_remote.take() {
            remotes.push(RemoteInfo {
                name,
                url: current_url.take().unwrap_or_default(),
            });
        }

        remotes.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(remotes)
    }

    /// Adds a new git remote with the given name and URL.
    ///
    /// Configures the standard fetch refspec
    /// `+refs/heads/*:refs/remotes/<name>/*`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::DuplicateRemote`] if a remote with the given name
    /// already exists. Returns `Error::Git` if the configuration cannot be
    /// written.
    pub fn git_remote_add(&mut self, name: &str, url: &str) -> Result<()> {
        let existing = self.git_remote_list()?;
        if existing.iter().any(|r| r.name == name) {
            return Err(Error::DuplicateRemote(name.to_string()));
        }

        let config_path = self.repo.git_dir().join("config");
        let mut content = std::fs::read_to_string(&config_path)
            .map_err(|e| Error::Git(format!("failed to read git config: {e}")))?;
        content.push_str(&format!(
            "[remote \"{}\"]\n\turl = {}\n\tfetch = +refs/heads/*:refs/remotes/{}/*\n",
            name, url, name
        ));
        std::fs::write(&config_path, &content)
            .map_err(|e| Error::Git(format!("failed to write git config: {e}")))?;

        // Reopen to refresh cached config
        self.repo = gix::open(self.inner.root())
            .map_err(|e| Error::Git(e.to_string()))?
            .into_sync();

        Ok(())
    }

    /// Removes a git remote by name.
    ///
    /// # Errors
    ///
    /// Returns [`Error::RemoteNotFound`] if no remote with the given name
    /// exists. Returns `Error::Git` if the configuration cannot be written.
    pub fn git_remote_remove(&mut self, name: &str) -> Result<()> {
        let existing = self.git_remote_list()?;
        if !existing.iter().any(|r| r.name == name) {
            return Err(Error::RemoteNotFound(name.to_string()));
        }

        let config_path = self.repo.git_dir().join("config");
        let content = std::fs::read_to_string(&config_path)
            .map_err(|e| Error::Git(format!("failed to read git config: {e}")))?;

        let section_header = format!("[remote \"{name}\"]");
        let mut output = String::new();
        let mut in_target_section = false;

        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('[') {
                in_target_section = trimmed == section_header;
            }
            if !in_target_section {
                output.push_str(line);
                output.push('\n');
            }
        }

        std::fs::write(&config_path, &output)
            .map_err(|e| Error::Git(format!("failed to write git config: {e}")))?;

        // Reopen to refresh cached config
        self.repo = gix::open(self.inner.root())
            .map_err(|e| Error::Git(e.to_string()))?
            .into_sync();

        Ok(())
    }

    /// Fetches from a named git remote using the `git` CLI.
    ///
    /// Verifies the remote exists first, then shells out to `git fetch`.
    /// After a successful fetch, the repository is reopened to refresh refs.
    ///
    /// # Errors
    ///
    /// Returns [`Error::RemoteNotFound`] if no remote with the given name exists.
    /// Returns `Error::Git` if `git` is not found or the fetch fails.
    pub fn git_fetch(&mut self, remote_name: &str) -> Result<()> {
        let existing = self.git_remote_list()?;
        if !existing.iter().any(|r| r.name == remote_name) {
            return Err(Error::RemoteNotFound(remote_name.to_string()));
        }

        let output = self.run_git(&["fetch", remote_name])?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::Git(format!(
                "git fetch {remote_name} failed: {stderr}"
            )));
        }

        // Reopen repo to refresh refs
        self.repo = gix::open(self.inner.root())
            .map_err(|e| Error::Git(e.to_string()))?
            .into_sync();

        Ok(())
    }

    /// Pushes the current branch to a named git remote.
    ///
    /// Verifies the remote exists, determines the current branch, then shells
    /// out to `git push`. If `force` is true, `--force` is added.
    ///
    /// # Errors
    ///
    /// Returns [`Error::RemoteNotFound`] if no remote with the given name exists.
    /// Returns [`Error::PushRejected`] if the push is rejected (non-fast-forward).
    /// Returns `Error::Git` if HEAD is detached, `git` is not found, or the
    /// push fails for another reason.
    pub fn git_push(&mut self, remote_name: &str, force: bool) -> Result<PushResult> {
        let existing = self.git_remote_list()?;
        if !existing.iter().any(|r| r.name == remote_name) {
            return Err(Error::RemoteNotFound(remote_name.to_string()));
        }

        let branch = self
            .current_branch_name()?
            .ok_or_else(|| Error::Git("cannot push: HEAD is detached".to_string()))?;

        // Get pre-push sync status to count commits
        let pre_status = self.git_sync_status(remote_name)?;
        let ahead_count = pre_status.as_ref().map_or(0, |s| s.ahead);

        let mut args = vec!["push", remote_name, &branch];
        if force {
            args.push("--force");
        }

        let args_refs: Vec<&str> = args.iter().map(|s| s.as_ref()).collect();
        let output = self.run_git(&args_refs)?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("non-fast-forward")
                || stderr.contains("rejected")
                || stderr.contains("fetch first")
            {
                return Err(Error::PushRejected(format!(
                    "remote has commits you don't have locally ({remote_name}/{branch})"
                )));
            }
            return Err(Error::Git(format!(
                "git push {remote_name} failed: {stderr}"
            )));
        }

        // Determine how many commits were pushed: use ahead_count if we had
        // tracking refs, otherwise check git's stderr for "Everything up-to-date"
        let stderr = String::from_utf8_lossy(&output.stderr);
        let commits_pushed = if ahead_count > 0 {
            ahead_count
        } else if stderr.contains("Everything up-to-date") {
            0
        } else {
            // First push or no tracking ref — something was pushed but we
            // can't count precisely without parsing, so report at least 1
            1
        };

        // Reopen repo to refresh refs
        self.repo = gix::open(self.inner.root())
            .map_err(|e| Error::Git(e.to_string()))?
            .into_sync();

        Ok(PushResult {
            remote: remote_name.to_string(),
            branch,
            commits_pushed,
        })
    }

    /// Pulls from a named git remote (fetch + fast-forward merge).
    ///
    /// Fetches from the remote, checks sync status, and if behind,
    /// performs a `git merge --ff-only` to incorporate remote changes.
    ///
    /// # Errors
    ///
    /// Returns [`Error::RemoteNotFound`] if no remote with the given name exists.
    /// Returns `Error::Git` if HEAD is detached, `git` is not found, or the
    /// merge fails for a non-conflict reason.
    pub fn git_pull(&mut self, remote_name: &str) -> Result<PullOutcome> {
        let existing = self.git_remote_list()?;
        if !existing.iter().any(|r| r.name == remote_name) {
            return Err(Error::RemoteNotFound(remote_name.to_string()));
        }

        let branch = self
            .current_branch_name()?
            .ok_or_else(|| Error::Git("cannot pull: HEAD is detached".to_string()))?;

        // Fetch first
        self.git_fetch(remote_name)?;

        // Check sync status
        let status = self.git_sync_status(remote_name)?;
        let (ahead, behind) = match &status {
            Some(s) => (s.ahead, s.behind),
            None => {
                return Ok(PullOutcome::Success(PullResult {
                    remote: remote_name.to_string(),
                    branch,
                    commits_merged: 0,
                    changed: false,
                }));
            }
        };

        if behind == 0 {
            return Ok(PullOutcome::Success(PullResult {
                remote: remote_name.to_string(),
                branch,
                commits_merged: 0,
                changed: false,
            }));
        }

        let tracking_ref = format!("{remote_name}/{branch}");

        if ahead > 0 {
            // Diverged — attempt a real merge
            // Check working tree is clean first
            let statuses = self.git_status()?;
            if !statuses.is_empty() {
                return Err(Error::Git(
                    "cannot pull with uncommitted changes — commit or discard first".to_string(),
                ));
            }

            // Sync the git index with HEAD (GitStore commits bypass the index)
            self.sync_index_to_head()?;

            let output = self.run_git(&["merge", &tracking_ref])?;

            if !output.status.success() {
                // Check if this is a merge conflict
                let unmerged = self.git_list_unmerged()?;
                if !unmerged.is_empty() {
                    let conflicted_files = unmerged
                        .iter()
                        .map(|p| conflict::classify_path(p))
                        .collect();
                    return Ok(PullOutcome::Conflict(MergeConflictResult {
                        remote: remote_name.to_string(),
                        branch,
                        conflicted_files,
                    }));
                }
                // Not a conflict — some other merge failure
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(Error::Git(format!("git merge failed: {stderr}")));
            }

            // Clean merge succeeded
            self.repo = gix::open(self.inner.root())
                .map_err(|e| Error::Git(e.to_string()))?
                .into_sync();
            return Ok(PullOutcome::Success(PullResult {
                remote: remote_name.to_string(),
                branch,
                commits_merged: behind,
                changed: true,
            }));
        }

        // Sync the git index with HEAD before fast-forward
        self.sync_index_to_head()?;

        // Fast-forward merge (behind only)
        let output = self.run_git(&["merge", "--ff-only", &tracking_ref])?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::Git(format!("git merge --ff-only failed: {stderr}")));
        }

        // Reopen repo to refresh state
        self.repo = gix::open(self.inner.root())
            .map_err(|e| Error::Git(e.to_string()))?
            .into_sync();

        Ok(PullOutcome::Success(PullResult {
            remote: remote_name.to_string(),
            branch,
            commits_merged: behind,
            changed: true,
        }))
    }

    /// Lists files with unresolved merge conflicts.
    ///
    /// Returns an empty list if no merge is in progress or all conflicts
    /// have been resolved.
    ///
    /// # Errors
    ///
    /// Returns `Error::Git` if `git` is not found or the command fails.
    pub fn git_list_unmerged(&self) -> Result<Vec<String>> {
        let output = self.run_git(&["diff", "--name-only", "--diff-filter=U"])?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::Git(format!(
                "git diff --diff-filter=U failed: {stderr}"
            )));
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(stdout
            .lines()
            .filter(|l| !l.is_empty())
            .map(|l| l.to_string())
            .collect())
    }

    /// Returns `true` if a merge is currently in progress.
    ///
    /// # Errors
    ///
    /// Returns `Error::Git` if the repository state cannot be determined.
    pub fn git_is_merge_in_progress(&self) -> Result<bool> {
        let merge_head = self.repo.git_dir().join("MERGE_HEAD");
        Ok(merge_head.exists())
    }

    /// Aborts an in-progress merge.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NoMergeInProgress`] if no merge is active.
    /// Returns `Error::Git` if `git merge --abort` fails.
    pub fn git_merge_abort(&mut self) -> Result<()> {
        if !self.git_is_merge_in_progress()? {
            return Err(Error::NoMergeInProgress);
        }
        let output = self.run_git(&["merge", "--abort"])?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::Git(format!("git merge --abort failed: {stderr}")));
        }
        self.repo = gix::open(self.inner.root())
            .map_err(|e| Error::Git(e.to_string()))?
            .into_sync();
        Ok(())
    }

    /// Marks a conflicted file as resolved and optionally completes the merge.
    ///
    /// If this was the last unmerged file, the merge is automatically
    /// completed with `git commit --no-edit`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NoMergeInProgress`] if no merge is active.
    /// Returns [`Error::NotConflicted`] if the file is not in the unmerged list.
    /// Returns `Error::Git` if `git add` or `git commit` fails.
    pub fn git_resolve_conflict(&mut self, path: &str) -> Result<ResolveResult> {
        if !self.git_is_merge_in_progress()? {
            return Err(Error::NoMergeInProgress);
        }

        let unmerged = self.git_list_unmerged()?;
        if !unmerged.iter().any(|p| p == path) {
            return Err(Error::NotConflicted(path.to_string()));
        }

        // Stage the resolved file
        let output = self.run_git(&["add", path])?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::Git(format!("git add failed: {stderr}")));
        }

        // Check remaining unmerged files
        let remaining = self.git_list_unmerged()?;
        let remaining_count = remaining.len();

        let mut merge_completed = false;
        if remaining_count == 0 {
            // All conflicts resolved — complete the merge
            let output = self.run_git(&["commit", "--no-edit"])?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(Error::Git(format!("git commit --no-edit failed: {stderr}")));
            }
            self.repo = gix::open(self.inner.root())
                .map_err(|e| Error::Git(e.to_string()))?
                .into_sync();
            merge_completed = true;
        }

        Ok(ResolveResult {
            path: path.to_string(),
            remaining: remaining_count,
            merge_completed,
        })
    }

    /// Syncs the git index with HEAD.
    ///
    /// `GitStore` creates commits by building tree objects directly, bypassing
    /// the git index. This means the index can become stale. Before operations
    /// that consult the index (like `git merge`), we reset it to match HEAD.
    fn sync_index_to_head(&self) -> Result<()> {
        let output = self.run_git(&["reset"])?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::Git(format!("git reset failed: {stderr}")));
        }
        Ok(())
    }

    /// Runs a git command in the store's working directory.
    fn run_git(&self, args: &[&str]) -> Result<std::process::Output> {
        match std::process::Command::new("git")
            .args(args)
            .current_dir(self.inner.root())
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .output()
        {
            Ok(o) => Ok(o),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(Error::Git(
                "git is not installed — install git to use remote features".to_string(),
            )),
            Err(e) => Err(Error::Git(format!("failed to run git: {e}"))),
        }
    }

    /// Returns the current branch name, or `None` if HEAD is detached or unborn.
    pub fn current_branch_name(&self) -> Result<Option<String>> {
        let output = self.run_git(&["symbolic-ref", "--quiet", "HEAD"])?;
        if !output.status.success() {
            return Ok(None); // detached or unborn
        }
        let full_ref = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(full_ref.strip_prefix("refs/heads/").map(|s| s.to_string()))
    }

    /// Computes the ahead/behind status between the local branch and a remote
    /// tracking branch.
    ///
    /// Returns `Ok(None)` if HEAD is detached, unborn, or no tracking ref
    /// exists for the remote (e.g., before the first fetch).
    ///
    /// # Errors
    ///
    /// Returns [`Error::RemoteNotFound`] if no remote with the given name exists.
    /// Returns `Error::Git` if the repository state cannot be read.
    pub fn git_sync_status(&self, remote_name: &str) -> Result<Option<SyncStatus>> {
        let existing = self.git_remote_list()?;
        if !existing.iter().any(|r| r.name == remote_name) {
            return Err(Error::RemoteNotFound(remote_name.to_string()));
        }

        // Get local branch name
        let branch = match self.current_branch_name()? {
            Some(b) => b,
            None => return Ok(None),
        };

        // Check tracking ref exists
        let tracking_ref = format!("refs/remotes/{remote_name}/{branch}");
        let output = self.run_git(&["rev-parse", "--verify", "--quiet", &tracking_ref])?;
        if !output.status.success() {
            return Ok(None); // no tracking ref
        }

        // Compute ahead/behind in one shot
        let range = format!("HEAD...{tracking_ref}");
        let output = self.run_git(&["rev-list", "--left-right", "--count", &range])?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::Git(format!(
                "failed to compute ahead/behind: {stderr}"
            )));
        }

        let counts = String::from_utf8_lossy(&output.stdout);
        let parts: Vec<&str> = counts.trim().split('\t').collect();
        let (ahead, behind) = if parts.len() == 2 {
            (
                parts[0].parse::<usize>().unwrap_or(0),
                parts[1].parse::<usize>().unwrap_or(0),
            )
        } else {
            (0, 0)
        };

        Ok(Some(SyncStatus {
            remote: remote_name.to_string(),
            branch,
            ahead,
            behind,
        }))
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
    fn collect_head_tree(&self, repo: &gix::Repository) -> Result<BTreeMap<String, gix::ObjectId>> {
        let mut files = BTreeMap::new();
        let head = match repo.head().ok().and_then(|mut h| h.peel_to_commit().ok()) {
            Some(commit) => commit,
            None => return Ok(files), // No commits yet
        };
        let tree = head
            .tree()
            .map_err(|e| Error::Git(format!("failed to get HEAD tree: {e}")))?;
        self.walk_tree(repo, &tree, "", &mut files)?;
        Ok(files)
    }

    /// Collects all file contents from the HEAD tree as `path -> bytes`.
    fn collect_head_blobs(&self, repo: &gix::Repository) -> Result<BTreeMap<String, Vec<u8>>> {
        let mut files = BTreeMap::new();
        let head = match repo.head().ok().and_then(|mut h| h.peel_to_commit().ok()) {
            Some(commit) => commit,
            None => return Ok(files),
        };
        let tree = head
            .tree()
            .map_err(|e| Error::Git(format!("failed to get HEAD tree: {e}")))?;
        self.walk_tree_blobs(repo, &tree, "", &mut files)?;
        Ok(files)
    }

    /// Recursively walks a git tree, collecting `path -> blob_oid`.
    fn walk_tree(
        &self,
        repo: &gix::Repository,
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
                let subtree_obj = repo
                    .find_object(entry.oid())
                    .map_err(|e| Error::Git(format!("failed to find object: {e}")))?;
                let subtree = subtree_obj
                    .try_into_tree()
                    .map_err(|e| Error::Git(format!("failed to convert to tree: {e}")))?;
                self.walk_tree(repo, &subtree, &path, files)?;
            } else if mode.is_blob() {
                files.insert(path, entry.oid().to_owned());
            }
        }
        Ok(())
    }

    /// Recursively walks a git tree, collecting `path -> blob content`.
    fn walk_tree_blobs(
        &self,
        repo: &gix::Repository,
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
                let subtree_obj = repo
                    .find_object(entry.oid())
                    .map_err(|e| Error::Git(format!("failed to find object: {e}")))?;
                let subtree = subtree_obj
                    .try_into_tree()
                    .map_err(|e| Error::Git(format!("failed to convert to tree: {e}")))?;
                self.walk_tree_blobs(repo, &subtree, &path, files)?;
            } else if mode.is_blob() {
                let blob = repo
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
        repo: &gix::Repository,
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
                let sub = self.collect_working_tree(repo, &entry.path(), &path)?;
                files.extend(sub);
            } else {
                let content = std::fs::read(entry.path())
                    .map_err(|e| Error::Git(format!("failed to read {path}: {e}")))?;
                let blob_id = repo
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

/// Discover the git directory for the repository containing `path`.
///
/// Uses `gix::discover` to walk up from `path` until a `.git` directory is
/// found.
///
/// # Errors
///
/// Returns `Error::Git` if no git repository is found at or above `path`.
pub fn discover_git_dir(path: &Path) -> Result<PathBuf> {
    let repo = gix::discover(path).map_err(|e| Error::Git(e.to_string()))?;
    Ok(repo.git_dir().to_owned())
}

/// Discover the effective hooks directory for the repository containing `path`.
///
/// Checks `core.hooksPath` in the merged git config first. If set, returns
/// that path (resolved against the working tree root when relative). Falls
/// back to `<git_dir>/hooks` when unset.
///
/// # Errors
///
/// Returns `Error::Git` if no git repository is found at or above `path`.
pub fn discover_hooks_dir(path: &Path) -> Result<PathBuf> {
    let repo = gix::discover(path).map_err(|e| Error::Git(e.to_string()))?;
    let config = repo.config_snapshot();
    if let Some(hooks_path) = config.string("core.hooksPath") {
        let p = PathBuf::from(hooks_path.to_string());
        if p.is_absolute() {
            return Ok(p);
        }
        // Relative paths are resolved against the working tree root.
        if let Some(work_dir) = repo.workdir() {
            return Ok(work_dir.join(p));
        }
    }
    Ok(repo.git_dir().join("hooks"))
}

/// Read HEAD commit info from the repository containing `path`.
///
/// Uses `gix::discover` to find the repo, then reads the HEAD commit.
/// Returns `Ok(None)` if the repository has no commits (unborn HEAD).
///
/// # Errors
///
/// Returns `Error::Git` if no git repository is found at or above `path`.
pub fn head_commit_info_at(path: &Path) -> Result<Option<HeadCommitInfo>> {
    let repo = gix::discover(path).map_err(|e| Error::Git(e.to_string()))?;
    let commit = match repo.head().ok().and_then(|mut h| h.peel_to_commit().ok()) {
        Some(c) => c,
        None => return Ok(None),
    };
    let sha = commit.id().to_string();
    let message = commit.message_raw_sloppy().to_string();
    Ok(Some(HeadCommitInfo { sha, message }))
}

/// Return commit messages from the repository at `path` in the range
/// `since_ref..HEAD`.
///
/// When `since_ref` is `None`, uses `HEAD@{1}` (the reflog anchor from
/// before the most recent merge). Commits are returned newest-first.
///
/// Returns an empty `Vec` if the anchor ref does not exist (e.g. shallow
/// clone or missing reflog entry) rather than failing.
///
/// # Errors
///
/// Returns `Error::Git` if `path` is not inside a git repository or git is
/// not installed.
pub fn commit_messages_since_at(
    path: &Path,
    since_ref: Option<&str>,
) -> Result<Vec<HeadCommitInfo>> {
    let anchor = since_ref.unwrap_or("HEAD@{1}");
    let output = run_git_at(
        path,
        &["log", "--format=%H%n%B%n<END>", "HEAD", "--not", anchor],
    )?;
    if !output.status.success() {
        return Ok(Vec::new());
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut commits = Vec::new();
    for block in stdout.split("<END>") {
        let block = block.trim();
        if block.is_empty() {
            continue;
        }
        if let Some((sha, message)) = block.split_once('\n') {
            commits.push(HeadCommitInfo {
                sha: sha.trim().to_string(),
                message: message.trim().to_string(),
            });
        }
    }
    Ok(commits)
}

/// Returns the current branch name for the repository containing `path`,
/// or `None` if HEAD is detached or unborn.
///
/// # Errors
///
/// Returns `Error::Git` if `path` is not inside a git repository or git is
/// not installed.
pub fn current_branch_at(path: &Path) -> Result<Option<String>> {
    let output = run_git_at(path, &["symbolic-ref", "--quiet", "HEAD"])?;
    if !output.status.success() {
        return Ok(None);
    }
    let full_ref = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(full_ref.strip_prefix("refs/heads/").map(|s| s.to_string()))
}

/// Run a git command without a working directory (e.g. for `git clone`).
fn run_git(args: &[&str]) -> Result<std::process::Output> {
    match std::process::Command::new("git")
        .args(args)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
    {
        Ok(o) => Ok(o),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(Error::Git(
            "git is not installed — install git to use remote features".to_string(),
        )),
        Err(e) => Err(Error::Git(format!("failed to run git: {e}"))),
    }
}

/// Run a git command in the working directory of the repository containing
/// `path`.
fn run_git_at(path: &Path, args: &[&str]) -> Result<std::process::Output> {
    match std::process::Command::new("git")
        .args(args)
        .current_dir(path)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
    {
        Ok(o) => Ok(o),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(Error::Git(
            "git is not installed — install git to use remote features".to_string(),
        )),
        Err(e) => Err(Error::Git(format!("failed to run git: {e}"))),
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
    fn git_remote_list_empty() {
        let dir = TempDir::new().unwrap();
        let store = GitStore::init(dir.path()).unwrap();
        let remotes = store.git_remote_list().unwrap();
        assert!(remotes.is_empty());
    }

    #[test]
    fn git_remote_add_and_list() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();
        store
            .git_remote_add("origin", "https://example.com/repo.git")
            .unwrap();

        let remotes = store.git_remote_list().unwrap();
        assert_eq!(remotes.len(), 1);
        assert_eq!(remotes[0].name, "origin");
        assert_eq!(remotes[0].url, "https://example.com/repo.git");
    }

    #[test]
    fn git_remote_add_duplicate_fails() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();
        store
            .git_remote_add("origin", "https://example.com/repo.git")
            .unwrap();

        let result = store.git_remote_add("origin", "https://other.com/repo.git");
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
            .git_remote_add("origin", "https://example.com/repo.git")
            .unwrap();
        store.git_remote_remove("origin").unwrap();

        let remotes = store.git_remote_list().unwrap();
        assert!(remotes.is_empty());
    }

    #[test]
    fn git_remote_remove_nonexistent_fails() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();

        let result = store.git_remote_remove("nope");
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
            .git_remote_add("upstream", "https://upstream.com/repo.git")
            .unwrap();
        store
            .git_remote_add("origin", "https://origin.com/repo.git")
            .unwrap();

        let remotes = store.git_remote_list().unwrap();
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
        store.git_commit("should not appear").unwrap();

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
        store.git_fetch("origin").unwrap();

        // After fetch, HEAD branch tracking ref should exist
        let branch = store.current_branch_name().unwrap().unwrap();
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

        let result = store.git_fetch("nonexistent");
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
            .git_remote_add("bad", "/nonexistent/path/to/repo.git")
            .unwrap();

        let result = store.git_fetch("bad");
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
        store.git_fetch("origin").unwrap();

        let status = store.git_sync_status("origin").unwrap();
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
        store.git_fetch("origin").unwrap();

        // Make two local commits
        store
            .write(&RelPath::new("local1.md").unwrap(), "local1".to_string())
            .unwrap();
        store.commit().unwrap();
        store
            .write(&RelPath::new("local2.md").unwrap(), "local2".to_string())
            .unwrap();
        store.commit().unwrap();

        let status = store.git_sync_status("origin").unwrap().unwrap();
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
        store.git_fetch("origin").unwrap();

        let status = store.git_sync_status("origin").unwrap().unwrap();
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
        store.git_fetch("origin").unwrap();

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
        store.git_fetch("origin").unwrap();

        let status = store.git_sync_status("origin").unwrap().unwrap();
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
            .git_remote_add("origin", "https://example.com/repo.git")
            .unwrap();

        let status = store.git_sync_status("origin").unwrap();
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
        let status = store.git_sync_status("origin").unwrap();
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
        store.git_fetch("origin").unwrap();

        // Make two local commits
        store
            .write(&RelPath::new("a.md").unwrap(), "a".to_string())
            .unwrap();
        store.commit().unwrap();
        store
            .write(&RelPath::new("b.md").unwrap(), "b".to_string())
            .unwrap();
        store.commit().unwrap();

        let result = store.git_push("origin", false).unwrap();
        assert_eq!(result.remote, "origin");
        assert_eq!(result.commits_pushed, 2);

        // After push, sync status should be up to date
        let status = store.git_sync_status("origin").unwrap().unwrap();
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
        store.git_fetch("origin").unwrap();

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
        let result = store.git_push("origin", false);
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
        store.git_fetch("origin").unwrap();

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
        let result = store.git_push("origin", true).unwrap();
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

        let outcome = store.git_pull("origin").unwrap();
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

        let outcome = store.git_pull("origin").unwrap();
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
        store.git_fetch("origin").unwrap();

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
        let outcome = store.git_pull("origin").unwrap();
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
        store.git_fetch("origin").unwrap();

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
        let outcome = store.git_pull("origin").unwrap();
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
        assert!(store.git_is_merge_in_progress().unwrap());

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
        store.git_fetch("origin").unwrap();

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
        let outcome = store.git_pull("origin").unwrap();
        assert!(matches!(outcome, PullOutcome::Conflict(_)));

        // Resolve the conflict by writing resolved content
        std::fs::write(dir.path().join("shared.md"), "resolved content").unwrap();

        let result = store.git_resolve_conflict("shared.md").unwrap();
        assert_eq!(result.path, "shared.md");
        assert_eq!(result.remaining, 0);
        assert!(result.merge_completed);

        // Merge should no longer be in progress
        assert!(!store.git_is_merge_in_progress().unwrap());

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

        let result = store.git_resolve_conflict("init.md");
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

        let commits = store.commit_messages_since(Some("anchor")).unwrap();
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
        let commits = store.commit_messages_since(Some("HEAD")).unwrap();
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
            .commit_messages_since(Some("nonexistent-ref-abc123"))
            .unwrap();
        assert!(commits.is_empty());
    }

    #[test]
    fn list_unmerged_empty_when_clean() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();
        store
            .write(&RelPath::new("init.md").unwrap(), "init".to_string())
            .unwrap();
        store.commit().unwrap();

        let unmerged = store.git_list_unmerged().unwrap();
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

        let store = GitStore::clone_remote(bare.path().to_str().unwrap(), &target_path).unwrap();

        assert!(target_path.join(".git").exists());
        assert!(target_path.join("rdm.toml").exists());
        assert!(target_path.join("INDEX.md").exists());

        // Should have "origin" remote
        let remotes = store.git_remote_list().unwrap();
        assert!(remotes.iter().any(|r| r.name == "origin"));
    }

    #[test]
    fn clone_remote_fails_nonempty_dir() {
        let (_source, bare) = make_bare_plan_repo();
        let target = TempDir::new().unwrap();
        // Pre-populate target
        std::fs::write(target.path().join("blocker.txt"), "hi").unwrap();

        let result = GitStore::clone_remote(bare.path().to_str().unwrap(), target.path());
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

        let result = GitStore::clone_remote("file:///nonexistent/repo.git", &target_path);
        match result {
            Err(e) => assert!(e.to_string().contains("git clone failed"), "got: {e}"),
            Ok(_) => panic!("expected error for bad URL"),
        }
    }
}

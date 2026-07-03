//! [`GitRepo`] local commit, status, discard, and tree-walking machinery.
//!
//! These methods build commits by writing tree objects directly (bypassing the
//! git index) and compare the working tree against HEAD for status/discard.

use std::collections::BTreeMap;
use std::path::Path;

use gix::object::tree::EntryKind;
use gix::objs::tree::EntryMode;
use rdm_core::error::{Error, Result};
use rdm_core::store::RelPath;

use crate::repo::GitRepo;
use crate::{FileChange, FileStatus, HeadCommitInfo};

impl GitRepo {
    /// Information about the HEAD commit: SHA and full message.
    ///
    /// Returns `Ok(None)` if the repository has no commits (unborn HEAD).
    ///
    /// # Errors
    ///
    /// Returns `Error::Git` if the repository state cannot be read.
    pub fn head_commit_info(&self) -> Result<Option<HeadCommitInfo>> {
        rdm_git::head_commit_info_at(&self.root)
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
        rdm_git::commit_messages_since_at(&self.root, since_ref)
    }

    /// Returns the name of the remote's default branch.
    ///
    /// Tries `git symbolic-ref refs/remotes/origin/HEAD`, strips the prefix,
    /// and falls back to `"main"` if that fails.
    ///
    /// # Errors
    ///
    /// Does not currently return an error: any failure of the underlying
    /// `git symbolic-ref` query falls back to `"main"`. The `Result` is
    /// retained for signature parity with the other commit-info queries.
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
    ///
    /// This method **never invokes git hooks**. It builds the tree object
    /// directly and writes the commit via gix's low-level `commit_as` (a
    /// gitoxide object-database write plus a `gix_ref` transaction) rather
    /// than shelling out to the `git` CLI — it bypasses the git porcelain
    /// entirely, including the porcelain's hook-invocation step. Concretely:
    /// an ordinary `rdm phase update`/`rdm task update` auto-commit on the
    /// plan repo can *never* re-trigger that same plan repo's own
    /// `post-commit` hook, even when `rdm hook install` has been run against
    /// it (a documented, supported configuration). This was investigated and
    /// ruled out as the mechanism behind an observed post-commit hang — see
    /// `run_post_commit_hook`'s doc comment in `rdm-cli` for the actual
    /// re-entrancy path (a *real* subprocess `git commit`/`git merge`,
    /// e.g. via [`GitRepo::git_resolve_conflict`]) and the guard that handles
    /// it.
    pub(crate) fn create_git_commit(&self, message: &str) -> Result<()> {
        let repo = self.repo.to_thread_local();
        let root = self.root.clone();
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
    /// This is the low-level commit primitive. Prefer the blessed
    /// caller-facing API [`GitStore::commit_now`], which delegates here, when
    /// a commit must land regardless of staging mode (e.g. hook handlers
    /// applying `Done:` directives or `rdm bootstrap --init`). Unlike
    /// [`Store::commit`], which honors staging mode and skips the git commit
    /// while staging is enabled, this always commits from the current working
    /// directory state.
    ///
    /// Returns `Ok(())` if the working directory matches HEAD (no-op).
    ///
    /// # Errors
    ///
    /// Returns `Error::Git` if the commit cannot be created.
    ///
    /// [`Store::commit`]: rdm_core::store::Store::commit
    /// [`GitStore::commit_now`]: crate::GitStore::commit_now
    pub fn git_commit(&self, message: &str) -> Result<()> {
        let status = self.git_status()?;
        if status.is_empty() {
            return Ok(());
        }
        self.create_git_commit(message)
    }

    /// Generates a default commit message summarizing a set of file statuses.
    ///
    /// Mirrors the CLI's `rdm commit` default-message behavior (and the
    /// message [`GitStore::commit`](crate::GitStore) generates from touched
    /// paths): a single changed file yields `"rdm: <verb> <path>"`; multiple
    /// files yield a `"rdm: update <n> files"` summary line followed by a
    /// blank-line-separated `<verb> <path>` bullet per file. `<verb>` is
    /// `add`, `update`, or `delete` depending on [`FileChange`].
    ///
    /// Used by [`GitStore::commit_now`](crate::GitStore::commit_now) callers
    /// (the CLI's `rdm commit` and the MCP `rdm_commit` tool) to generate a
    /// message when none is supplied explicitly.
    ///
    /// # Examples
    ///
    /// ```
    /// use rdm_store_git::{FileChange, FileStatus, GitRepo};
    ///
    /// let statuses = vec![FileStatus {
    ///     path: "a.md".to_string(),
    ///     change: FileChange::Added,
    /// }];
    /// assert_eq!(GitRepo::default_commit_message(&statuses), "rdm: add a.md");
    /// ```
    pub fn default_commit_message(statuses: &[FileStatus]) -> String {
        let summary: Vec<String> = statuses
            .iter()
            .map(|s| {
                let kind = match s.change {
                    FileChange::Added => "add",
                    FileChange::Modified => "update",
                    FileChange::Deleted => "delete",
                };
                format!("{kind} {}", s.path)
            })
            .collect();
        if summary.len() == 1 {
            format!("rdm: {}", summary[0])
        } else {
            let mut msg = format!("rdm: update {} files", statuses.len());
            for s in &summary {
                msg.push_str(&format!("\n\n- {s}"));
            }
            msg
        }
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
        let work_files = self.collect_working_tree(&repo, &self.root, "")?;

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
        let root = self.root.as_path();

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

    /// Syncs the git index with HEAD.
    ///
    /// `GitStore` creates commits by building tree objects directly, bypassing
    /// the git index. This means the index can become stale. Before operations
    /// that consult the index (like `git merge`), we reset it to match HEAD.
    pub(crate) fn sync_index_to_head(&self) -> Result<()> {
        let output = self.run_git(&["reset"])?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::Git(format!("git reset failed: {stderr}")));
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

    /// Fetches the body of `path` as it existed at commit `sha`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::RevisionUnknown`] if `sha` does not resolve to a commit,
    /// [`Error::BodyAtRevisionMissing`] if the path did not exist at that
    /// revision, or [`Error::Git`] for any other git failure.
    pub(crate) fn fetch_body_at(&self, path: &RelPath, sha: &str) -> Result<String> {
        // Verify the SHA resolves to a commit first. Without this, `git show
        // <unknown-40-hex>:<path>` returns "exists on disk, but not in
        // '<sha>'" — indistinguishable from a real missing-path error.
        let verify = self.run_git(&[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{sha}^{{commit}}"),
        ])?;
        if !verify.status.success() {
            return Err(Error::RevisionUnknown {
                sha: sha.to_string(),
            });
        }

        let spec = format!("{sha}:{p}", p = path.as_str());
        let output = self.run_git(&["show", &spec])?;
        if output.status.success() {
            return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        let lower = stderr.to_lowercase();
        if lower.contains("does not exist in")
            || lower.contains("exists on disk, but not in")
            || (lower.contains("path") && lower.contains("does not exist"))
        {
            return Err(Error::BodyAtRevisionMissing {
                path: path.as_str().to_string(),
                sha: sha.to_string(),
            });
        }
        Err(Error::Git(stderr.trim().to_string()))
    }
}

#[cfg(test)]
mod default_commit_message_tests {
    use super::*;

    #[test]
    fn single_file_added() {
        let statuses = vec![FileStatus {
            path: "a.md".to_string(),
            change: FileChange::Added,
        }];
        assert_eq!(GitRepo::default_commit_message(&statuses), "rdm: add a.md");
    }

    #[test]
    fn single_file_modified() {
        let statuses = vec![FileStatus {
            path: "a.md".to_string(),
            change: FileChange::Modified,
        }];
        assert_eq!(
            GitRepo::default_commit_message(&statuses),
            "rdm: update a.md"
        );
    }

    #[test]
    fn single_file_deleted() {
        let statuses = vec![FileStatus {
            path: "a.md".to_string(),
            change: FileChange::Deleted,
        }];
        assert_eq!(
            GitRepo::default_commit_message(&statuses),
            "rdm: delete a.md"
        );
    }

    #[test]
    fn multiple_files() {
        let statuses = vec![
            FileStatus {
                path: "a.md".to_string(),
                change: FileChange::Added,
            },
            FileStatus {
                path: "b.md".to_string(),
                change: FileChange::Modified,
            },
            FileStatus {
                path: "c.md".to_string(),
                change: FileChange::Deleted,
            },
        ];
        let msg = GitRepo::default_commit_message(&statuses);
        assert!(msg.starts_with("rdm: update 3 files"));
        assert!(msg.contains("- add a.md"));
        assert!(msg.contains("- update b.md"));
        assert!(msg.contains("- delete c.md"));
    }
}

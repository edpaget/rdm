//! [`GitRepo`] merge porcelain: listing unmerged files, detecting an
//! in-progress merge, aborting, and resolving individual conflicts.

use rdm_core::error::{Error, Result};

use crate::ResolveResult;
use crate::error::{self, GitError};
use crate::repo::GitRepo;

impl GitRepo {
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
    /// Returns [`GitError::NoMergeInProgress`] if no merge is active.
    /// Returns [`GitError::Git`] if `git merge --abort` fails.
    pub fn git_merge_abort(&mut self) -> error::Result<()> {
        if !self.git_is_merge_in_progress()? {
            return Err(GitError::NoMergeInProgress);
        }
        let output = self.run_git(&["merge", "--abort"])?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(GitError::Git(format!("git merge --abort failed: {stderr}")));
        }
        self.reopen()?;
        Ok(())
    }

    /// Marks a conflicted file as resolved and optionally completes the merge.
    ///
    /// If this was the last unmerged file, the merge is automatically
    /// completed with `git commit --no-edit`.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::NoMergeInProgress`] if no merge is active.
    /// Returns [`GitError::NotConflicted`] if the file is not in the unmerged list.
    /// Returns [`GitError::Git`] if `git add` or `git commit` fails.
    pub fn git_resolve_conflict(&mut self, path: &str) -> error::Result<ResolveResult> {
        if !self.git_is_merge_in_progress()? {
            return Err(GitError::NoMergeInProgress);
        }

        let unmerged = self.git_list_unmerged()?;
        if !unmerged.iter().any(|p| p == path) {
            return Err(GitError::NotConflicted(path.to_string()));
        }

        // Stage the resolved file
        let output = self.run_git(&["add", path])?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(GitError::Git(format!("git add failed: {stderr}")));
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
                return Err(GitError::Git(format!(
                    "git commit --no-edit failed: {stderr}"
                )));
            }
            self.reopen()?;
            merge_completed = true;
        }

        Ok(ResolveResult {
            path: path.to_string(),
            remaining: remaining_count,
            merge_completed,
        })
    }
}

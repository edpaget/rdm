//! [`GitRepo`] remote porcelain: listing/adding/removing remotes, fetch,
//! push, pull, and ahead/behind sync status.

use rdm_core::conflict;
use rdm_core::error::{Error, Result};

use crate::error::{self, GitError};
use crate::repo::GitRepo;
use crate::{MergeConflictResult, PullOutcome, PullResult, PushResult, RemoteInfo, SyncStatus};

impl GitRepo {
    /// Returns the current branch name, or `None` if HEAD is detached or unborn.
    ///
    /// # Errors
    ///
    /// Returns `Error::Git` if `git` is not installed or the command fails.
    pub fn current_branch_name(&self) -> Result<Option<String>> {
        rdm_git::current_branch_at(&self.root)
    }

    /// Lists all configured git remotes with their fetch URLs.
    ///
    /// Returns remotes sorted alphabetically by name.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::Git`] if the repository configuration cannot be read.
    pub fn git_remote_list(&self) -> error::Result<Vec<RemoteInfo>> {
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
    /// Returns [`GitError::DuplicateRemote`] if a remote with the given name
    /// already exists. Returns [`GitError::Git`] if the configuration cannot be
    /// written.
    pub fn git_remote_add(&mut self, name: &str, url: &str) -> error::Result<()> {
        let existing = self.git_remote_list()?;
        if existing.iter().any(|r| r.name == name) {
            return Err(GitError::DuplicateRemote(name.to_string()));
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
        self.reopen()?;

        Ok(())
    }

    /// Removes a git remote by name.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::RemoteNotFound`] if no remote with the given name
    /// exists. Returns [`GitError::Git`] if the configuration cannot be written.
    pub fn git_remote_remove(&mut self, name: &str) -> error::Result<()> {
        let existing = self.git_remote_list()?;
        if !existing.iter().any(|r| r.name == name) {
            return Err(GitError::RemoteNotFound(name.to_string()));
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
        self.reopen()?;

        Ok(())
    }

    /// Fetches from a named git remote using the `git` CLI.
    ///
    /// Verifies the remote exists first, then shells out to `git fetch`.
    /// After a successful fetch, the repository is reopened to refresh refs.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::RemoteNotFound`] if no remote with the given name exists.
    /// Returns [`GitError::Git`] if `git` is not found or the fetch fails.
    pub fn git_fetch(&mut self, remote_name: &str) -> error::Result<()> {
        let existing = self.git_remote_list()?;
        if !existing.iter().any(|r| r.name == remote_name) {
            return Err(GitError::RemoteNotFound(remote_name.to_string()));
        }

        let output = self.run_git(&["fetch", remote_name])?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(GitError::Git(format!(
                "git fetch {remote_name} failed: {stderr}"
            )));
        }

        // Reopen repo to refresh refs
        self.reopen()?;

        Ok(())
    }

    /// Pushes the current branch to a named git remote.
    ///
    /// Verifies the remote exists, determines the current branch, then shells
    /// out to `git push`. If `force` is true, `--force` is added.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::RemoteNotFound`] if no remote with the given name exists.
    /// Returns [`GitError::PushRejected`] if the push is rejected (non-fast-forward).
    /// Returns [`GitError::Git`] if HEAD is detached, `git` is not found, or the
    /// push fails for another reason.
    pub fn git_push(&mut self, remote_name: &str, force: bool) -> error::Result<PushResult> {
        let existing = self.git_remote_list()?;
        if !existing.iter().any(|r| r.name == remote_name) {
            return Err(GitError::RemoteNotFound(remote_name.to_string()));
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
                return Err(GitError::PushRejected(format!(
                    "remote has commits you don't have locally ({remote_name}/{branch})"
                )));
            }
            return Err(GitError::Git(format!(
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
        self.reopen()?;

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
    /// Returns [`GitError::RemoteNotFound`] if no remote with the given name exists.
    /// Returns [`GitError::Git`] if HEAD is detached, `git` is not found, or the
    /// merge fails for a non-conflict reason.
    pub fn git_pull(&mut self, remote_name: &str) -> error::Result<PullOutcome> {
        let existing = self.git_remote_list()?;
        if !existing.iter().any(|r| r.name == remote_name) {
            return Err(GitError::RemoteNotFound(remote_name.to_string()));
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
                return Err(GitError::Git(
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
                return Err(GitError::Git(format!("git merge failed: {stderr}")));
            }

            // Clean merge succeeded
            self.reopen()?;
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
            return Err(GitError::Git(format!(
                "git merge --ff-only failed: {stderr}"
            )));
        }

        // Reopen repo to refresh state
        self.reopen()?;

        Ok(PullOutcome::Success(PullResult {
            remote: remote_name.to_string(),
            branch,
            commits_merged: behind,
            changed: true,
        }))
    }

    /// Computes the ahead/behind status between the local branch and a remote
    /// tracking branch.
    ///
    /// Returns `Ok(None)` if HEAD is detached, unborn, or no tracking ref
    /// exists for the remote (e.g., before the first fetch).
    ///
    /// # Errors
    ///
    /// Returns [`GitError::RemoteNotFound`] if no remote with the given name exists.
    /// Returns [`GitError::Git`] if the repository state cannot be read.
    pub fn git_sync_status(&self, remote_name: &str) -> error::Result<Option<SyncStatus>> {
        let existing = self.git_remote_list()?;
        if !existing.iter().any(|r| r.name == remote_name) {
            return Err(GitError::RemoteNotFound(remote_name.to_string()));
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
            return Err(GitError::Git(format!(
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
}

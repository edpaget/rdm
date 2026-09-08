//! Git-backed [`Store`] implementation, with commits created via gitoxide.
//!
//! [`GitStore`] wraps [`FsStore`] and only ever flushes to disk on
//! [`Store::commit`] — it never creates a git commit. Reads, writes, and
//! deletes are delegated to the inner `FsStore`.
//!
//! # Staging is session-scoped; committing is too
//!
//! [`Store::commit`] flushes a batch to disk **and journals exactly which
//! paths it wrote** to the calling session's changeset (see
//! [`rdm_core::session`]). Two commands in two shells sharing one `$RDM_ROOT`
//! therefore accumulate two disjoint changesets on one working tree.
//!
//! Two entry points turn a changeset into a git commit, and they mean
//! different things:
//!
//! - [`GitStore::commit_changeset`] — **the default.** Builds the commit tree
//!   from HEAD plus exactly this session's journaled paths, reconciling the
//!   generated indexes in memory. A path another session left dirty is
//!   structurally unreachable.
//! - [`GitStore::commit_whole_tree`] — the explicitly-named machine-global
//!   escape hatch. Rebuilds from the whole working directory, sweeping up
//!   whatever anyone left dirty. Reserved for fixture seeding and the user's
//!   own `--all` opt-in.
//!
//! [`GitStore::discard_changeset`] / [`GitStore::discard_whole_tree`] are the
//! same split for the destructive side. The documented contract and the
//! implemented behavior are the same statement: nothing here is
//! machine-global unless its name says so.
//!
//! All git logic — low-level plumbing and high-level porcelain — lives on the
//! [`GitRepo`] collaborator (see the `repo`, `commit`, `remote`, and
//! `merge` modules). `GitStore` is a thin adapter composing an `FsStore`
//! with a `GitRepo`, exposing the git capability via [`GitStore::git`] and
//! [`GitStore::git_mut`].

#![warn(missing_docs)]

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use rdm_core::conflict::ConflictItem;
use rdm_core::error::{Error, Result};
use rdm_core::session::journal::{JournalEntry, JournalKind};
use rdm_core::session::{self, ResolvedSession, SessionPaths};
use rdm_core::store::{DirEntry, RelPath, Store, VersionedStore};
use rdm_store_fs::FsStore;

pub mod error;

mod commit;
mod merge;
mod remote;
mod repo;

pub use commit::{ChangesetScope, CommitReport, CommitScope};
/// HEAD/commit info, re-exported from [`rdm_git`] as the return type of
/// [`GitRepo::head_commit_info`] / [`GitRepo::commit_messages_since`].
pub use rdm_git::HeadCommitInfo;
pub use repo::GitRepo;

/// The kind of file change detected by [`GitRepo::git_status_report`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileChange {
    /// A new file not present in HEAD.
    Added,
    /// An existing file whose content differs from HEAD.
    Modified,
    /// A file present in HEAD but missing from the working directory.
    Deleted,
}

/// A single file's status as reported by [`GitRepo::git_status_report`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileStatus {
    /// The relative path of the file within the repository.
    pub path: String,
    /// The kind of change detected.
    pub change: FileChange,
}

/// Uncommitted working-tree changes, partitioned into what *this* changeset
/// authored, what rdm generated for it, and what some other session left
/// dirty.
///
/// Returned by [`GitRepo::git_status_report`] (the whole-tree view, where
/// `others` is always empty) and by [`GitStore::status_report_scoped`] (the
/// changeset view). It is the only way to observe working-tree changes from
/// outside this crate. The generated `INDEX.md` files are rewritten by every
/// mutation, and a shared plan repo can hold several sessions' uncommitted
/// work at once: without this partition a session cannot tell its own edits
/// from derived output or from a neighbour's, and so cannot predict what its
/// `rdm commit` will land.
///
/// # The one rule every consumer follows
///
/// **Gate on [`total`](Self::total) / [`is_clean`](Self::is_clean); report on
/// [`user`](Self::user).**
///
/// An action that rewrites the whole working tree — `rdm commit --all`,
/// `rdm discard --all` — must be gated by the raw truth, because a tree
/// holding *only* regenerated indexes is still dirty and must still be
/// committable (otherwise it stays dirty forever and `rdm remote pull`
/// refuses to run). But every count and listing shown to a human or an agent
/// comes from `user`, with `derived` and `others` surfaced separately and by
/// name so nothing is hidden.
///
/// **One partition, not two filters.** The changeset split and the derived
/// split are decided together in a single pass, so the view a user reads and
/// the set a scoped commit lands can never disagree.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StatusReport {
    /// Changes this changeset authored — everything of its own that is not
    /// generated output.
    pub user: Vec<FileStatus>,
    /// Changes to files rdm generates for this changeset, per
    /// [`rdm_core::paths::is_derived_path`].
    pub derived: Vec<FileStatus>,
    /// Dirty paths this changeset does not claim: another live session's
    /// uncommitted work, or an orphaned changeset's.
    ///
    /// Always empty in the whole-tree view, where by definition everything is
    /// in scope.
    pub others: Vec<FileStatus>,
}

impl StatusReport {
    /// Returns whether the working tree matches HEAD exactly — no changes of
    /// any kind, this session's or anyone else's.
    ///
    /// This, not `user.is_empty()`, is the correct gate for a whole-tree
    /// commit or discard.
    pub fn is_clean(&self) -> bool {
        self.user.is_empty() && self.derived.is_empty() && self.others.is_empty()
    }

    /// Returns whether *this changeset* has nothing to commit.
    ///
    /// Distinct from [`is_clean`](Self::is_clean): the tree can be dirty with
    /// another session's work while this changeset owns nothing at all — the
    /// case a scoped `rdm commit` must report rather than sweep.
    pub fn is_changeset_clean(&self) -> bool {
        self.user.is_empty() && self.derived.is_empty()
    }

    /// Returns the total number of changed files across every list.
    pub fn total(&self) -> usize {
        self.user.len() + self.derived.len() + self.others.len()
    }

    /// Returns the path-sorted union of every list.
    ///
    /// This is the raw truth about what a *whole-tree* commit will contain —
    /// feed it to [`GitRepo::default_commit_message`], never `user` alone,
    /// since a derived-only tree has an empty `user` list. A scoped commit's
    /// message comes from [`changeset`](Self::changeset) instead, so an
    /// auto-generated message can never name another session's file.
    pub fn all(&self) -> Vec<FileStatus> {
        let mut all: Vec<FileStatus> = self
            .user
            .iter()
            .chain(self.derived.iter())
            .chain(self.others.iter())
            .cloned()
            .collect();
        all.sort_by(|a, b| a.path.cmp(&b.path));
        all
    }

    /// Returns the path-sorted union of exactly this changeset's entries.
    ///
    /// The scoped counterpart to [`all`](Self::all), and the only correct
    /// input to a scoped commit's default message.
    pub fn changeset(&self) -> Vec<FileStatus> {
        let mut all: Vec<FileStatus> = self
            .user
            .iter()
            .chain(self.derived.iter())
            .cloned()
            .collect();
        all.sort_by(|a, b| a.path.cmp(&b.path));
        all
    }

    /// Returns the one-line summary a caller prints after committing this
    /// report's changes.
    ///
    /// Lives here, not in each interface, so `rdm commit` and the MCP
    /// `rdm_commit` tool can never disagree about how the same report is
    /// summarized. Follows the crate rule above: the count is `user`, and
    /// `derived` is named separately rather than folded in or hidden.
    ///
    /// Only meaningful when the report is not [`is_clean`](Self::is_clean) —
    /// callers gate on that first and print their own no-op message.
    pub fn commit_summary(&self) -> String {
        let derived = self.derived.len();
        if self.user.is_empty() {
            format!("Committed {derived} regenerated index file(s).")
        } else if derived > 0 {
            format!(
                "Committed {} file(s) (plus {derived} regenerated index file(s)).",
                self.user.len()
            )
        } else {
            format!("Committed {} file(s).", self.user.len())
        }
    }

    /// Returns the one-line summary a caller prints after discarding this
    /// report's changes.
    ///
    /// The discard counterpart to [`commit_summary`](Self::commit_summary),
    /// shared by `rdm discard` and the MCP `rdm_discard` tool for the same
    /// reason. There is no user-empty branch here: `git_discard` restores
    /// everything, so a derived-only discard still reports `0 file(s)` plus
    /// the named regenerated count.
    ///
    /// Only meaningful when the report is not [`is_clean`](Self::is_clean).
    pub fn discard_summary(&self) -> String {
        let derived = self.derived.len();
        if derived > 0 {
            format!(
                "Discarded {} file(s) (plus {derived} regenerated index file(s)).",
                self.user.len()
            )
        } else {
            format!("Discarded {} file(s).", self.user.len())
        }
    }

    /// Returns the one-line note naming what this action deliberately left
    /// alone, or `None` when nothing belongs to another session.
    ///
    /// Shared by `rdm status`, `rdm commit`, `rdm discard` and their MCP
    /// counterparts so the three can never describe the same situation
    /// differently.
    pub fn others_summary(&self) -> Option<String> {
        if self.others.is_empty() {
            return None;
        }
        Some(format!(
            "{} file(s) belong to other changesets and were left untouched \
             (`rdm session list` to see them, `--all` to include them).",
            self.others.len()
        ))
    }
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
/// `FsStore` **and journals the paths it wrote** to the calling session's
/// changeset — it never creates a git commit. Turning a changeset into a git
/// commit is [`GitStore::commit_changeset`]; sweeping the whole working tree
/// regardless of who wrote it is the explicitly-named
/// [`GitStore::commit_whole_tree`]. Staging is session-scoped and so is the
/// default commit: the two no longer contradict each other. All git logic is
/// delegated to the composed [`GitRepo`], reachable via
/// [`git`](Self::git)/[`git_mut`](Self::git_mut).
pub struct GitStore {
    inner: FsStore,
    git: GitRepo,
    /// Lazily resolved once per store; see [`GitStore::session`].
    session: OnceLock<ResolvedSession>,
    /// Where this repo's session state lives, or `None` when no state
    /// directory could be determined at all.
    session_paths: Option<SessionPaths>,
}

impl GitStore {
    /// Opens a `GitStore` for an existing git repository.
    ///
    /// Both halves of the `INDEX.md` merge driver are ensured on every open:
    /// the `.gitattributes` entries in the worktree and the
    /// `[merge "rdm-index"]` section in the repo-local `.git/config`. Both are
    /// idempotent no-ops once installed, and both are **best-effort** here —
    /// an installation failure warns and the open still succeeds, so a
    /// read-only mount or restrictive CI checkout can still be read. (Only the
    /// explicit [`GitStore::init`] hard-fails on them.)
    ///
    /// Ensuring `.gitattributes` here is what backfills repos created or
    /// cloned before the merge driver shipped, with no user action. The write
    /// is a normal working-tree change: it shows up in `rdm status` until the
    /// next `rdm commit` lands it, at which point the mapping travels with
    /// clones.
    ///
    /// # Errors
    ///
    /// Returns `Error::Git` if the path is not inside a git repository.
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        let repo = gix::open(&root)
            .map_err(|e| Error::Git(e.to_string()))?
            .into_sync();
        let git = GitRepo::new(root.clone(), repo);
        // Best-effort: a repo with an unwritable .git/config (read-only
        // mount, restrictive CI checkout) must still open for reads — the
        // merge driver is a convenience, not a prerequisite for the store.
        if let Err(e) = git.ensure_merge_driver_config() {
            eprintln!("warning: could not install INDEX.md merge driver: {e}");
        }
        // Best-effort for the same reason. This is the backfill path: repos
        // opened (rather than `rdm init`ed) never got the worktree half.
        if let Err(e) = git.ensure_gitattributes() {
            eprintln!("warning: could not install INDEX.md merge attributes: {e}");
        }
        Ok(Self::compose_journaling(FsStore::new(&root), git))
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

        let git = GitRepo::new(root.clone(), repo.into_sync());
        git.ensure_gitattributes()?;
        git.ensure_merge_driver_config()?;
        Ok(Self::compose_journaling(FsStore::new(&root), git))
    }

    /// Clones a remote git repository and opens a `GitStore` for it.
    ///
    /// This is the remote counterpart to [`GitStore::init`]. It shells out to
    /// `git clone` to fetch the remote repository into `root`. When `branch`
    /// is `Some`, `--branch <name>` is passed to `git clone` so the specified
    /// branch is checked out.
    ///
    /// As with [`GitStore::new`], both merge-driver halves are ensured
    /// best-effort once the clone is opened — an installation failure warns
    /// rather than failing the clone. A clone inherits `.gitattributes` when
    /// the source committed it; when the source never did, it is re-created
    /// here, so the clone is mapped from its first command.
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
        let output = rdm_git::run_git(&args)?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::Git(format!("git clone failed: {}", stderr.trim())));
        }
        // Open the cloned repo
        let repo = gix::open(&root)
            .map_err(|e| Error::Git(format!("failed to open cloned repo: {e}")))?
            .into_sync();
        let git = GitRepo::new(root.clone(), repo);
        // Best-effort, mirroring `GitStore::new`: never fail a successful
        // clone over an unwritable .git/config.
        if let Err(e) = git.ensure_merge_driver_config() {
            eprintln!("warning: could not install INDEX.md merge driver: {e}");
        }
        // Covers cloning from a source that never committed `.gitattributes`
        // — inheritance alone does not map such a clone.
        if let Err(e) = git.ensure_gitattributes() {
            eprintln!("warning: could not install INDEX.md merge attributes: {e}");
        }
        Ok(Self::compose_journaling(FsStore::new(&root), git))
    }

    /// Composes a store from its two collaborators, resolving where session
    /// state lives.
    ///
    /// The state directory is `<git-dir>/rdm/`: both whole-tree walks
    /// (`build_tree_from_dir` and `collect_working_tree`) skip exactly the
    /// name `.git`, and neither consults `.gitignore`, so this is the only
    /// placement that is invisible to an rdm commit without changing either
    /// walk. Nothing is created here — directories appear lazily on first
    /// write, so a read-only plan repo still opens.
    fn compose(inner: FsStore, git: GitRepo) -> Self {
        let session_paths = Some(SessionPaths::for_git_dir(git.git_dir()));
        Self {
            inner,
            git,
            session: OnceLock::new(),
            session_paths,
        }
    }

    /// Returns this process's resolved session identity for this repo.
    ///
    /// Resolved at most once per store and never fails; see
    /// [`rdm_core::session::resolve_session`] for the rung chain and its
    /// degradation guarantee. `None` only when no state directory could be
    /// determined at all.
    pub fn session(&self) -> Option<&ResolvedSession> {
        let paths = self.session_paths.as_ref()?;
        Some(
            self.session
                .get_or_init(|| session::resolve_system_session(paths)),
        )
    }

    /// Pins this store's session to an explicitly-chosen changeset id.
    ///
    /// Overrides the rung chain for **this store only**, without touching
    /// process-global environment state (which is both `unsafe` to mutate on
    /// a running server and inherently racy). Writes journal to `id`, and
    /// [`session`](Self::session) reports it at [`Rung::Explicit`].
    ///
    /// The motivating caller is a long-lived server told `--changeset <id>`:
    /// it advertises that id on every response, so its writes had better
    /// land there. Pinning after the session has already been resolved is a
    /// no-op — pin at construction.
    #[must_use]
    pub fn with_session_id(self, id: session::SessionId) -> Self {
        let _ = self.session.set(ResolvedSession {
            id,
            rung: session::Rung::Explicit,
            resolve_micros: 0,
            // A pinned id is supplied, never derived from a lease this
            // process minted, so there is nothing for the continuity
            // advisory to warn about.
            lease_bootstrapped: false,
        });
        self
    }

    /// Returns where this repo's session state lives, if anywhere.
    pub fn session_paths(&self) -> Option<&SessionPaths> {
        self.session_paths.as_ref()
    }

    /// Records a flushed batch in this session's changeset journal.
    ///
    /// Best-effort and non-fatal, mirroring the hook logger's
    /// swallow-failures contract: an unwritable state directory must never
    /// fail a mutation. The failure is bounded and reported rather than
    /// silent — the paths become unattributed, and [`commit_changeset`] names
    /// them with recovery routes instead of sweeping them. Called only
    /// *after* a successful flush, so the journal can never claim a path that
    /// was not written.
    ///
    /// [`commit_changeset`]: Self::commit_changeset
    fn record_journal(&self, touched: &[(RelPath, JournalKind, Option<String>)]) {
        if touched.is_empty() {
            return;
        }
        let Some(paths) = self.session_paths.as_ref() else {
            return;
        };
        let Some(resolved) = self.session() else {
            return;
        };
        let entries: Vec<JournalEntry> = touched
            .iter()
            .map(|(path, kind, digest)| JournalEntry {
                path: path.as_str().to_string(),
                kind: *kind,
                // Base-blob identity per journaled path: the scoped commit
                // uses it to refuse another session's bytes.
                digest: digest.clone(),
            })
            .collect();
        let _ = session::journal::record(paths, &resolved.id, &entries);
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

    /// Creates a git commit from the **whole** working-directory state.
    ///
    /// The explicitly-named machine-global escape hatch. It sweeps up
    /// whatever any session left dirty, so it is reserved for the two places
    /// that legitimately want that: seeding a fixture, and the user's own
    /// `--all` opt-in on `rdm commit`. Everything else — the CLI's default
    /// `rdm commit`, the `Done:` hook, the MCP commit tool, `rdm init
    /// --remote`, `rdm bootstrap --init` — goes through
    /// [`commit_changeset`](Self::commit_changeset).
    ///
    /// No-op if the working tree already matches HEAD.
    ///
    /// # Errors
    /// Returns [`Error::Git`] if the commit cannot be created.
    pub fn commit_whole_tree(&self, message: &str) -> Result<CommitReport> {
        self.git.git_commit(message)
    }

    /// Creates a git commit containing HEAD plus exactly this session's
    /// changeset.
    ///
    /// This is the blessed committer. It resolves this process's session,
    /// reads its journal, and builds the commit tree from HEAD plus only
    /// those paths — so a path another session left dirty is structurally
    /// unreachable rather than filtered out late.
    ///
    /// `extra_paths` is the explicit include-these-paths capability for
    /// writes that genuinely cannot route through the `Store` (a back-filled
    /// `.gitattributes`, say). It applies to **this commit only** and is
    /// supplied by the caller that actually wrote those paths — it is not a
    /// standing exemption list.
    ///
    /// On success the landed paths are removed from the journal. That is
    /// correctness, not hygiene: see
    /// [`journal::truncate`](rdm_core::session::journal::truncate).
    ///
    /// The returned [`ScopedCommit`] carries everything a porcelain needs to
    /// report — the sha, what landed, what was skipped as missing, and
    /// whether the changeset was empty while the tree was dirty — so no
    /// caller has to re-derive any of it.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Git`] if the tree cannot be built, the derived
    /// indexes cannot be reconciled, or HEAD moves twice under the commit.
    pub fn commit_changeset(
        &self,
        message: Option<&str>,
        extra_paths: &[String],
    ) -> Result<ScopedCommit> {
        let id = self.session().map(|s| s.id.clone());
        self.commit_changeset_id(id.as_ref(), message, extra_paths)
    }

    /// Composes the store and journals any `.gitattributes` back-fill the
    /// constructor's `ensure_gitattributes` just performed.
    ///
    /// Eager on purpose. A back-fill and the commit that should carry it are
    /// usually two *different processes* (`rdm init` writes it; a later
    /// `rdm commit` lands it), and the second process finds the mapping
    /// already present and so writes — and latches — nothing. Journaling at
    /// the moment of the write is the only point at which the fact is still
    /// known. Lazy creation still holds for every repo that is already
    /// mapped: no write, no journal, no state directory.
    fn compose_journaling(inner: FsStore, git: GitRepo) -> Self {
        let store = Self::compose(inner, git);
        store.journal_pending_side_writes();
        store
    }

    /// Commits a *named* changeset — the orphan-recovery entry point behind
    /// `rdm commit --changeset <id>`.
    ///
    /// Identical to [`commit_changeset`](Self::commit_changeset) except that
    /// the changeset is chosen explicitly rather than resolved from this
    /// process.
    ///
    /// # Errors
    ///
    /// As [`commit_changeset`](Self::commit_changeset).
    pub fn commit_changeset_id(
        &self,
        id: Option<&session::SessionId>,
        message: Option<&str>,
        extra_paths: &[String],
    ) -> Result<ScopedCommit> {
        // A `.gitattributes` back-fill from a store-less site (pull's
        // post-merge re-ensure, the whole-tree discard) must join a changeset
        // before the journal is read, or it can never be committed at all.
        self.journal_pending_side_writes();
        let journal = self.read_changeset(id)?;
        let owned = Self::owned_paths(&journal, extra_paths);
        let report = self.git.git_status_report_scoped(&owned)?;

        let mut scope = ChangesetScope {
            extra_writes: extra_paths.to_vec(),
            ..ChangesetScope::default()
        };
        for entry in &journal {
            if rdm_core::paths::is_derived_path(&entry.path) {
                if entry.kind == JournalKind::Write {
                    scope.derived.push(entry.path.clone());
                } else {
                    scope.deletes.push(entry.path.clone());
                }
            } else if entry.kind == JournalKind::Write {
                scope.writes.push(entry.path.clone());
                // Carry the base-blob identity through so the tree builder can
                // refuse a path another session has overwritten since.
                if let Some(digest) = &entry.digest {
                    scope.digests.insert(entry.path.clone(), digest.clone());
                }
            } else {
                scope.deletes.push(entry.path.clone());
            }
        }

        if scope.is_empty() {
            // Nothing attributed to this changeset. Deliberately NOT a
            // whole-tree sweep and deliberately not silence: report the
            // unattributed paths so the caller can recover them.
            return Ok(ScopedCommit {
                changeset: id.map(|i| i.to_string()),
                sha: None,
                committed: Vec::new(),
                skipped_missing: Vec::new(),
                report,
            });
        }

        let message = message
            .map(str::to_string)
            // Derived from the changeset's own entries, never the whole tree,
            // so an auto-generated message can never name another session's
            // file.
            .unwrap_or_else(|| GitRepo::default_commit_message(&report.changeset()));
        let outcome = self.git.git_commit_changeset(&message, &scope)?;

        if outcome.sha.is_some()
            && let (Some(paths), Some(id)) = (self.session_paths.as_ref(), id)
        {
            let _ = session::journal::truncate(paths, id, &outcome.committed);
        }

        Ok(ScopedCommit {
            changeset: id.map(|i| i.to_string()),
            sha: outcome.sha,
            committed: outcome.committed,
            skipped_missing: outcome.skipped_missing,
            report,
        })
    }

    /// Returns the three-way working-tree status from this session's point of
    /// view.
    ///
    /// See [`StatusReport`]: one partition into `user` / `derived` / `others`,
    /// not two independent filters.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Git`] if the repository state cannot be read.
    pub fn status_report_scoped(&self) -> Result<StatusReport> {
        self.journal_pending_side_writes();
        let id = self.session().map(|s| s.id.clone());
        let journal = self.read_changeset(id.as_ref())?;
        let owned = Self::owned_paths(&journal, &[]);
        self.git.git_status_report_scoped(&owned)
    }

    /// Restores **only** this session's changeset to HEAD, leaving every other
    /// session's uncommitted work — and its rows in the shared indexes —
    /// intact.
    ///
    /// The shipped `rdm discard` design. Concretely:
    ///
    /// 1. this changeset's non-derived paths are restored to HEAD (added ones
    ///    removed, modified/deleted ones written back);
    /// 2. the changeset's journal is cleared;
    /// 3. the derived indexes are regenerated **from the resulting disk
    ///    state**, so another session's still-uncommitted rows survive — and
    ///    that regeneration is journaled to this (now empty) changeset, so the
    ///    session owns what it just rewrote;
    /// 4. the `.gitattributes` merge-driver mapping is re-ensured, exactly as
    ///    the whole-tree discard does.
    ///
    /// Returns the report describing what was discarded.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Git`] if the HEAD tree cannot be read or files cannot
    /// be written, or a core error if the indexes cannot be regenerated.
    pub fn discard_changeset(&mut self) -> Result<StatusReport> {
        let report = self.status_report_scoped()?;
        if report.is_changeset_clean() {
            let _ = self.git.ensure_gitattributes();
            return Ok(report);
        }
        self.git.restore_paths_to_head(&report.user)?;

        if let (Some(paths), Some(id)) = (
            self.session_paths.as_ref(),
            self.session().map(|s| s.id.clone()),
        ) {
            let _ = session::journal::discard_changeset(paths, &id);
        }

        // Regenerate from what is actually on disk now: the point is that
        // another session's uncommitted rows must survive this discard.
        rdm_core::ops::index::generate_index(self)?;
        Store::commit(self)?;

        // Reinstate the merge-driver mapping the restore may have removed.
        // Best-effort by design: a discard must never fail because of it.
        if self.git.ensure_gitattributes().unwrap_or(false) {
            self.journal_side_write(crate::repo::GITATTRIBUTES_PATH);
        }
        Ok(report)
    }

    /// Restores the **whole** working tree to HEAD, destroying every
    /// session's uncommitted work.
    ///
    /// The explicitly-named destructive escape hatch behind
    /// `rdm discard --force --all`. Callers must warn, naming the other live
    /// changesets, before invoking it.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Git`] if the HEAD tree cannot be read or files cannot
    /// be written.
    pub fn discard_whole_tree(&self) -> Result<()> {
        self.git.git_discard()
    }

    /// Records a path this store wrote outside the `Store` write path.
    ///
    /// The route that makes a `.gitattributes` back-fill committable: the
    /// file is written by [`GitRepo::ensure_gitattributes`] with no `Store`
    /// batch behind it, so without this it would be journaled by nobody and
    /// therefore permanently uncommittable under scoping — cancelling the
    /// merge-driver back-fill entirely.
    fn journal_side_write(&self, path: &str) {
        let Ok(rel) = RelPath::new(path) else { return };
        // No digest: this write did not go through the store, so there is no
        // staged content to identify. The commit-time check fails open on a
        // digest-less entry, which is the right answer here.
        self.record_journal(&[(rel, JournalKind::Write, None)]);
    }

    /// Journals any `.gitattributes` write latched by a site that has no
    /// `Store` in reach.
    ///
    /// Drained at open (where the back-fill happens) and again before every
    /// scoped commit and scoped status (which catches the re-ensures inside
    /// the whole-tree discard and `git_pull`'s post-merge path). A repo that
    /// is already mapped writes nothing, latches nothing, and therefore still
    /// creates no session state on open.
    pub fn journal_pending_side_writes(&self) {
        if self.git.take_gitattributes_written() {
            self.journal_side_write(crate::repo::GITATTRIBUTES_PATH);
        }
    }

    /// Reads a changeset's journal, degrading to empty on an unreadable
    /// state directory.
    fn read_changeset(
        &self,
        id: Option<&session::SessionId>,
    ) -> Result<Vec<session::journal::JournalEntry>> {
        let (Some(paths), Some(id)) = (self.session_paths.as_ref(), id) else {
            return Ok(Vec::new());
        };
        // A read failure degrades to "this changeset owns nothing", which
        // reports the tree as unattributed rather than sweeping it. Never a
        // hang, never a silent whole-tree commit.
        Ok(session::journal::read_journal(paths, id).unwrap_or_default())
    }

    /// The path set a changeset claims: its journal plus any caller-supplied
    /// per-commit includes.
    fn owned_paths(
        journal: &[session::journal::JournalEntry],
        extra: &[String],
    ) -> std::collections::BTreeSet<String> {
        journal
            .iter()
            .map(|e| e.path.clone())
            .chain(extra.iter().cloned())
            .collect()
    }
}

/// What a scoped commit did, from one source so every porcelain reports the
/// same facts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScopedCommit {
    /// The changeset that was committed, when one could be resolved.
    pub changeset: Option<String>,
    /// The new commit's SHA, or `None` when nothing was committed.
    pub sha: Option<String>,
    /// The paths this commit landed.
    pub committed: Vec<String>,
    /// Journaled paths skipped because their working-tree file has vanished.
    pub skipped_missing: Vec<String>,
    /// The three-way status as it was before the commit.
    pub report: StatusReport,
}

impl ScopedCommit {
    /// Returns whether the changeset was empty while the tree was dirty.
    ///
    /// The rung-4 fragmentation case, and the case of a session that mutated
    /// before scoping shipped. A caller must NOT print "Nothing to commit."
    /// here — the tree is dirty and those paths need recovering.
    pub fn unattributed_dirt(&self) -> bool {
        self.sha.is_none() && !self.report.others.is_empty()
    }

    /// The one-line note naming journaled paths whose working-tree file has
    /// vanished, or `None` when none did.
    ///
    /// Every porcelain must print this on **every** branch, the `sha: None`
    /// ones included. Skipping a vanished path is deliberately not fatal, but
    /// it must never be silent: the journal is truncated only on a *successful*
    /// commit, so those paths are still claimed by this changeset while the
    /// files backing them are gone. A caller told nothing but
    /// `Nothing to commit.` would have no way to learn that its own tracked
    /// work had disappeared underneath it.
    pub fn skipped_summary(&self) -> Option<String> {
        if self.skipped_missing.is_empty() {
            return None;
        }
        Some(format!(
            "skipped {} journaled path(s) no longer on disk (still in this changeset's journal): {}",
            self.skipped_missing.len(),
            self.skipped_missing.join(", ")
        ))
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
        self.inner.write(path, content)
    }

    fn delete(&mut self, path: &RelPath) -> Result<()> {
        self.inner.delete(path)
    }

    fn commit(&mut self) -> Result<()> {
        // Staging is the only workflow here: flush to disk and never create
        // a git commit. `commit_changeset` lands exactly this journal;
        // `commit_whole_tree` is the named machine-global escape hatch.
        //
        // The batch's membership is snapshotted *before* the flush (which
        // drains the staging overlay) and journaled *after* it succeeds, so
        // the journal can be neither a superset of what landed nor a claim
        // about a batch that failed. Journaling writes only inside
        // `<git-dir>/rdm/` and is best-effort — commit behavior is unchanged.
        let touched = self.inner.staged_paths();
        self.inner.commit()?;
        self.record_journal(&touched);
        Ok(())
    }

    fn discard(&mut self) {
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

// Compile-time assertion: GitStore must implement Send + Sync.
// Catch regressions at the store crate level (not just downstream in rdm-mcp
// when wrapping GitStore in Mutex<AppStore>). Fails at library build time
// if GitStore loses either trait.
static_assertions::assert_impl_all!(GitStore: Send, Sync);

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
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
    fn init_writes_gitattributes_merge_entries() {
        let dir = TempDir::new().unwrap();
        let _store = GitStore::init(dir.path()).unwrap();
        let attrs = std::fs::read_to_string(dir.path().join(".gitattributes")).unwrap();
        assert!(
            attrs.contains("INDEX.md merge=rdm-index"),
            "expected root INDEX.md merge entry, got: {attrs}"
        );
        assert!(
            attrs.contains("**/INDEX.md merge=rdm-index"),
            "expected wildcard INDEX.md merge entry, got: {attrs}"
        );
    }

    #[test]
    fn init_writes_merge_driver_git_config() {
        let dir = TempDir::new().unwrap();
        let _store = GitStore::init(dir.path()).unwrap();
        let config = std::fs::read_to_string(dir.path().join(".git").join("config")).unwrap();
        assert!(
            config.contains("[merge \"rdm-index\"]"),
            "expected merge driver section, got: {config}"
        );
        assert!(
            config.contains("driver = rdm --root . index"),
            "expected driver command to invoke rdm index with an explicit --root . \
             (the driver subprocess has no ambient RDM_ROOT/cwd discovery), got: {config}"
        );
        assert!(
            config.contains("%A") && config.contains("%P"),
            "expected driver command to use %A/%P placeholders, got: {config}"
        );
    }

    #[test]
    fn init_gitattributes_and_config_are_idempotent() {
        let dir = TempDir::new().unwrap();
        let _store1 = GitStore::init(dir.path()).unwrap();
        let _store2 = GitStore::init(dir.path()).unwrap();

        let attrs = std::fs::read_to_string(dir.path().join(".gitattributes")).unwrap();
        assert_eq!(
            attrs.matches("merge=rdm-index").count(),
            2,
            "expected exactly two merge=rdm-index entries (root + wildcard), got: {attrs}"
        );

        let config = std::fs::read_to_string(dir.path().join(".git").join("config")).unwrap();
        assert_eq!(
            config.matches("[merge \"rdm-index\"]").count(),
            1,
            "expected exactly one merge driver section, got: {config}"
        );
    }

    #[test]
    fn init_preserves_existing_gitattributes_content() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join(".gitattributes"), "*.bin binary").unwrap();
        let _store = GitStore::init(dir.path()).unwrap();

        let attrs = std::fs::read_to_string(dir.path().join(".gitattributes")).unwrap();
        assert!(
            attrs.contains("*.bin binary"),
            "expected pre-existing content to survive, got: {attrs}"
        );
        assert!(attrs.contains("INDEX.md merge=rdm-index"));
        assert!(attrs.contains("**/INDEX.md merge=rdm-index"));
    }

    /// Builds a repo whose HEAD deliberately has no `.gitattributes` at all,
    /// so the backfill paths have something to backfill.
    fn repo_without_committed_gitattributes() -> TempDir {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();
        // `init` writes it; remove it before the seed commit so HEAD lacks it.
        std::fs::remove_file(dir.path().join(".gitattributes")).unwrap();
        store
            .write(&RelPath::new("seed.md").unwrap(), "seed".to_string())
            .unwrap();
        store.commit().unwrap();
        store.commit_whole_tree("seed: add seed.md").unwrap();
        assert!(!dir.path().join(".gitattributes").exists());
        dir
    }

    #[test]
    fn new_backfills_gitattributes_when_absent() {
        let dir = repo_without_committed_gitattributes();

        // Opening (not initializing) must install the worktree half.
        let _store = GitStore::new(dir.path()).unwrap();

        let attrs = std::fs::read_to_string(dir.path().join(".gitattributes")).unwrap();
        assert!(
            attrs.contains("INDEX.md merge=rdm-index"),
            "expected root INDEX.md merge entry after open, got: {attrs}"
        );
        assert!(
            attrs.contains("**/INDEX.md merge=rdm-index"),
            "expected wildcard INDEX.md merge entry after open, got: {attrs}"
        );
    }

    #[test]
    fn open_is_idempotent_and_preserves_existing_gitattributes_content() {
        let dir = repo_without_committed_gitattributes();
        std::fs::write(dir.path().join(".gitattributes"), "*.bin binary\n").unwrap();

        let _s1 = GitStore::new(dir.path()).unwrap();
        let _s2 = GitStore::new(dir.path()).unwrap();

        let attrs = std::fs::read_to_string(dir.path().join(".gitattributes")).unwrap();
        assert!(
            attrs.contains("*.bin binary"),
            "expected hand-written content to survive, got: {attrs}"
        );
        assert_eq!(
            attrs.matches("merge=rdm-index").count(),
            2,
            "expected exactly two merge=rdm-index entries after two opens, got: {attrs}"
        );
    }

    #[test]
    fn new_does_not_fail_when_gitattributes_is_unwritable() {
        let dir = TempDir::new().unwrap();
        gix::init(dir.path()).unwrap();
        // A directory at that path makes the write deterministically fail —
        // no chmod/root-user flakiness.
        std::fs::create_dir(dir.path().join(".gitattributes")).unwrap();

        let store = GitStore::new(dir.path());
        assert!(
            store.is_ok(),
            "a repo whose .gitattributes cannot be written must still open for reads"
        );
    }

    #[test]
    fn discard_reinstates_gitattributes_it_removed() {
        // Shape 1: untracked (`Added`) — the restore loop deletes it outright.
        let dir = repo_without_committed_gitattributes();
        let store = GitStore::new(dir.path()).unwrap();
        assert!(dir.path().join(".gitattributes").exists());

        store.git().git_discard().unwrap();

        let attrs = std::fs::read_to_string(dir.path().join(".gitattributes")).unwrap_or_default();
        assert!(
            attrs.contains("merge=rdm-index"),
            "discarding an untracked .gitattributes must not un-map the repo, got: {attrs}"
        );
    }

    #[test]
    fn discard_reinstates_gitattributes_when_head_predates_the_mapping() {
        // Shape 2: tracked, but HEAD's blob predates the mapping — the restore
        // loop reverts to a version without the marker.
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();
        std::fs::write(dir.path().join(".gitattributes"), "*.bin binary\n").unwrap();
        store
            .write(&RelPath::new("seed.md").unwrap(), "seed".to_string())
            .unwrap();
        store.commit().unwrap();
        store
            .commit_whole_tree("seed: pre-mapping .gitattributes")
            .unwrap();

        // Reopen: the mapping is appended, making the file `Modified`.
        let store = GitStore::new(dir.path()).unwrap();
        store.git().git_discard().unwrap();

        let attrs = std::fs::read_to_string(dir.path().join(".gitattributes")).unwrap();
        assert!(
            attrs.contains("*.bin binary"),
            "HEAD's own content must still be restored, got: {attrs}"
        );
        assert!(
            attrs.contains("merge=rdm-index"),
            "discarding to a pre-mapping HEAD must not un-map the repo, got: {attrs}"
        );
    }

    #[test]
    fn git_status_report_partitions_generated_indexes_from_user_changes() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();
        store
            .write(&RelPath::new("INDEX.md").unwrap(), "# Index\n".to_string())
            .unwrap();
        store
            .write(
                &RelPath::new("projects/demo/INDEX.md").unwrap(),
                "# demo\n".to_string(),
            )
            .unwrap();
        store
            .write(
                &RelPath::new("projects/demo/roadmaps/auth/roadmap.md").unwrap(),
                "seed".to_string(),
            )
            .unwrap();
        store.commit().unwrap();
        store.commit_whole_tree("seed: plan repo").unwrap();

        // One user edit plus the two indexes a mutation regenerates.
        std::fs::write(
            dir.path().join("projects/demo/roadmaps/auth/roadmap.md"),
            "edited",
        )
        .unwrap();
        std::fs::write(dir.path().join("INDEX.md"), "# Index (regenerated)\n").unwrap();
        std::fs::write(
            dir.path().join("projects/demo/INDEX.md"),
            "# demo (regenerated)\n",
        )
        .unwrap();

        let report = store.git().git_status_report().unwrap();
        assert_eq!(
            report.user.len(),
            1,
            "expected exactly the one user edit, got: {:?}",
            report.user
        );
        assert_eq!(
            report.user[0].path,
            "projects/demo/roadmaps/auth/roadmap.md"
        );
        assert_eq!(
            report.derived.len(),
            2,
            "expected both regenerated indexes, got: {:?}",
            report.derived
        );
        assert_eq!(report.total(), 3);
        assert!(!report.is_clean());

        let all = report.all();
        let paths: Vec<&str> = all.iter().map(|s| s.path.as_str()).collect();
        assert_eq!(
            paths,
            vec![
                "INDEX.md",
                "projects/demo/INDEX.md",
                "projects/demo/roadmaps/auth/roadmap.md",
            ],
            "all() must be the path-sorted union"
        );
    }

    #[test]
    fn status_report_is_clean_only_when_nothing_at_all_differs() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();
        store
            .write(&RelPath::new("INDEX.md").unwrap(), "# Index\n".to_string())
            .unwrap();
        store.commit().unwrap();
        store.commit_whole_tree("seed: plan repo").unwrap();

        assert!(store.git().git_status_report().unwrap().is_clean());

        // Derived-only dirty tree: no user changes, but decidedly not clean.
        std::fs::write(dir.path().join("INDEX.md"), "# Index (regenerated)\n").unwrap();
        let report = store.git().git_status_report().unwrap();
        assert!(report.user.is_empty());
        assert_eq!(report.derived.len(), 1);
        assert!(
            !report.is_clean(),
            "a derived-only tree must still be committable — gating commit on \
             user.is_empty() would leave it dirty forever"
        );
    }

    fn report_of(user: usize, derived: usize) -> StatusReport {
        let mk = |n: usize, prefix: &str| {
            (0..n)
                .map(|i| FileStatus {
                    path: format!("{prefix}{i}.md"),
                    change: FileChange::Modified,
                })
                .collect()
        };
        StatusReport {
            user: mk(user, "user-"),
            derived: mk(derived, "derived-"),
            others: Vec::new(),
        }
    }

    // The CLI's `rdm commit`/`rdm discard` and the MCP `rdm_commit`/`rdm_discard`
    // tools both render their summary through these two methods, so covering the
    // methods covers both interfaces and pins them to the same wording.
    #[test]
    fn commit_summary_covers_all_three_branches() {
        assert_eq!(report_of(2, 0).commit_summary(), "Committed 2 file(s).");
        assert_eq!(
            report_of(1, 2).commit_summary(),
            "Committed 1 file(s) (plus 2 regenerated index file(s))."
        );
        assert_eq!(
            report_of(0, 2).commit_summary(),
            "Committed 2 regenerated index file(s)."
        );
    }

    #[test]
    fn discard_summary_covers_both_branches() {
        assert_eq!(report_of(2, 0).discard_summary(), "Discarded 2 file(s).");
        assert_eq!(
            report_of(1, 2).discard_summary(),
            "Discarded 1 file(s) (plus 2 regenerated index file(s))."
        );
    }

    #[test]
    fn skipped_summary_is_none_only_when_nothing_vanished() {
        let clean = ScopedCommit::default();
        assert_eq!(clean.skipped_summary(), None);

        let vanished = ScopedCommit {
            skipped_missing: vec!["projects/demo/tasks/a.md".into()],
            ..ScopedCommit::default()
        };
        let note = vanished.skipped_summary().expect("a skip must be reported");
        assert!(note.contains("projects/demo/tasks/a.md"), "{note}");
        // The journal is truncated only on a successful commit, so the caller
        // must be told the path is still claimed — otherwise "skipped" reads as
        // "dropped", and the recovery route is invisible.
        assert!(note.contains("journal"), "{note}");
    }

    #[test]
    fn git_status_report_treats_gitattributes_as_a_user_change() {
        let dir = repo_without_committed_gitattributes();
        let store = GitStore::new(dir.path()).unwrap();

        let report = store.git().git_status_report().unwrap();
        assert!(
            report.user.iter().any(|s| s.path == ".gitattributes"),
            "the merge mapping must be landable, so it is a user change: {:?}",
            report.user
        );
        assert!(report.derived.is_empty());
    }

    #[test]
    fn new_opens_existing_repo() {
        let dir = TempDir::new().unwrap();
        gix::init(dir.path()).unwrap();
        let store = GitStore::new(dir.path());
        assert!(store.is_ok());
    }

    #[test]
    fn new_adds_merge_driver_config_to_legacy_repo() {
        let dir = TempDir::new().unwrap();
        gix::init(dir.path()).unwrap();
        let _store = GitStore::new(dir.path()).unwrap();

        let config = std::fs::read_to_string(dir.path().join(".git").join("config")).unwrap();
        assert!(
            config.contains("[merge \"rdm-index\"]"),
            "expected merge driver section to be added on open, got: {config}"
        );
    }

    /// A repo whose `.git/config` is unwritable (read-only mount, restrictive
    /// CI checkout) must still open for reads — the merge-driver install is
    /// best-effort, never a prerequisite for the store.
    #[cfg(unix)]
    #[test]
    fn new_succeeds_when_git_config_is_read_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        gix::init(dir.path()).unwrap();
        let config_path = dir.path().join(".git").join("config");
        let mut perms = std::fs::metadata(&config_path).unwrap().permissions();
        perms.set_mode(0o444);
        std::fs::set_permissions(&config_path, perms).unwrap();

        let store = GitStore::new(dir.path());
        assert!(
            store.is_ok(),
            "read-only .git/config must not prevent opening the store: {:?}",
            store.err()
        );
        let config = std::fs::read_to_string(&config_path).unwrap();
        assert!(
            !config.contains("[merge \"rdm-index\"]"),
            "driver section must not appear when config is unwritable"
        );

        // Restore write permission so TempDir cleanup can't be affected.
        let mut perms = std::fs::metadata(&config_path).unwrap().permissions();
        perms.set_mode(0o644);
        std::fs::set_permissions(&config_path, perms).unwrap();
    }

    #[test]
    fn new_is_idempotent_across_repeated_opens() {
        let dir = TempDir::new().unwrap();
        gix::init(dir.path()).unwrap();
        let _store1 = GitStore::new(dir.path()).unwrap();
        let _store2 = GitStore::new(dir.path()).unwrap();

        let config = std::fs::read_to_string(dir.path().join(".git").join("config")).unwrap();
        assert_eq!(
            config.matches("[merge \"rdm-index\"]").count(),
            1,
            "expected exactly one merge driver section after repeated opens, got: {config}"
        );
    }

    #[test]
    fn new_preserves_existing_custom_merge_driver_section() {
        let dir = TempDir::new().unwrap();
        gix::init(dir.path()).unwrap();
        let config_path = dir.path().join(".git").join("config");
        {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .append(true)
                .open(&config_path)
                .unwrap();
            writeln!(file, "\n[merge \"rdm-index\"]\n\tdriver = custom-driver %A").unwrap();
        }
        let _store = GitStore::new(dir.path()).unwrap();

        let config = std::fs::read_to_string(&config_path).unwrap();
        assert_eq!(
            config.matches("[merge \"rdm-index\"]").count(),
            1,
            "expected the custom section not to be duplicated, got: {config}"
        );
        assert!(
            config.contains("custom-driver"),
            "expected pre-existing custom driver to survive, got: {config}"
        );
    }

    #[test]
    fn new_fails_on_non_repo() {
        let dir = TempDir::new().unwrap();
        let result = GitStore::new(dir.path());
        assert!(result.is_err());
    }

    #[test]
    fn commit_whole_tree_creates_git_commit() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();
        let path = RelPath::new("hello.md").unwrap();
        store.write(&path, "world".to_string()).unwrap();
        store.commit().unwrap();
        store.commit_whole_tree("test message").unwrap();

        let repo = gix::open(dir.path()).unwrap();
        let mut head = repo.head().unwrap();
        let commit = head.peel_to_commit().unwrap();
        let msg = String::from_utf8_lossy(commit.message_raw_sloppy());
        assert_eq!(msg, "test message");
    }

    #[test]
    fn delete_is_committed() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();

        // Write the file, then land a real commit.
        let path = RelPath::new("doomed.md").unwrap();
        store.write(&path, "bye".to_string()).unwrap();
        store.commit().unwrap();
        store.commit_whole_tree("seed: add doomed.md").unwrap();
        assert!(dir.path().join("doomed.md").exists());

        // Delete, then land a real commit reflecting the delete.
        store.delete(&path).unwrap();
        store.commit().unwrap();
        store.commit_whole_tree("delete doomed.md").unwrap();
        assert!(!dir.path().join("doomed.md").exists());

        // Verify the delete is reflected in a real commit: the latest commit
        // message matches, and the tree no longer contains the file.
        let repo = gix::open(dir.path()).unwrap();
        let mut head = repo.head().unwrap();
        let commit = head.peel_to_commit().unwrap();
        let msg = String::from_utf8_lossy(commit.message_raw_sloppy());
        assert_eq!(msg, "delete doomed.md");

        let output = git_cmd()
            .args(["show", "--stat", "HEAD"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        let shown = String::from_utf8_lossy(&output.stdout);
        assert!(
            shown.contains("doomed.md"),
            "expected doomed.md in HEAD commit stat, got: {shown}"
        );
    }

    #[test]
    fn discard_does_not_create_commit() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();

        // Create an initial commit so HEAD exists
        let path = RelPath::new("init.md").unwrap();
        store.write(&path, "init".to_string()).unwrap();
        store.commit().unwrap();
        store.commit_whole_tree("seed: add init.md").unwrap();

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
    fn read_your_own_writes() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();
        let path = RelPath::new("staged.md").unwrap();
        store.write(&path, "staged content".to_string()).unwrap();
        assert_eq!(store.read(&path).unwrap(), "staged content");
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
        store
            .commit_whole_tree("seed: add keep.md and doomed.md")
            .unwrap();

        // Now make changes directly on disk (simulating staging mode)
        std::fs::write(dir.path().join("keep.md"), "modified").unwrap();
        std::fs::write(dir.path().join("added.md"), "new file").unwrap();
        std::fs::remove_file(dir.path().join("doomed.md")).unwrap();

        let status = store.git().git_status_all().unwrap();
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
        store
            .commit_whole_tree("seed: add keep.md and doomed.md")
            .unwrap();

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
        let status = store.git().git_status_all().unwrap();
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
        store.commit_whole_tree("seed: add init.md").unwrap();

        let repo = gix::open(dir.path()).unwrap();
        let head_before = repo.head().unwrap().peel_to_commit().unwrap().id().detach();

        // Git commit when clean should be a no-op
        store.commit_whole_tree("should not appear").unwrap();

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
        store.commit_whole_tree("seed: add init.md").unwrap();

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
        store.commit_whole_tree("seed: add init.md").unwrap();

        let bare_dir = setup_bare_remote(&mut store, "origin");
        store.git_mut().git_fetch("origin").unwrap();

        // Make two local commits
        store
            .write(&RelPath::new("local1.md").unwrap(), "local1".to_string())
            .unwrap();
        store.commit().unwrap();
        store.commit_whole_tree("add local1.md").unwrap();
        store
            .write(&RelPath::new("local2.md").unwrap(), "local2".to_string())
            .unwrap();
        store.commit().unwrap();
        store.commit_whole_tree("add local2.md").unwrap();

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
        store.commit_whole_tree("seed: add init.md").unwrap();

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
        store.commit_whole_tree("seed: add init.md").unwrap();

        let bare_dir = setup_bare_remote(&mut store, "origin");
        store.git_mut().git_fetch("origin").unwrap();

        // Make local commit
        store
            .write(&RelPath::new("local.md").unwrap(), "local".to_string())
            .unwrap();
        store.commit().unwrap();
        store.commit_whole_tree("add local.md").unwrap();

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
        store.commit_whole_tree("seed: add init.md").unwrap();

        let bare_dir = setup_bare_remote(&mut store, "origin");
        store.git_mut().git_fetch("origin").unwrap();

        // Make two local commits
        store
            .write(&RelPath::new("a.md").unwrap(), "a".to_string())
            .unwrap();
        store.commit().unwrap();
        store.commit_whole_tree("add a.md").unwrap();
        store
            .write(&RelPath::new("b.md").unwrap(), "b".to_string())
            .unwrap();
        store.commit().unwrap();
        store.commit_whole_tree("add b.md").unwrap();

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
        store.commit_whole_tree("seed: add init.md").unwrap();

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
        store.commit_whole_tree("add local.md").unwrap();

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
        store.commit_whole_tree("seed: add init.md").unwrap();

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
        store.commit_whole_tree("add local.md").unwrap();

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
        store.commit_whole_tree("seed: add init.md").unwrap();

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
        store.commit_whole_tree("seed: add init.md").unwrap();

        let bare_dir = setup_bare_remote(&mut store, "origin");
        store.git_mut().git_fetch("origin").unwrap();

        // Make a local commit (different file from remote)
        store
            .write(&RelPath::new("local.md").unwrap(), "local".to_string())
            .unwrap();
        store.commit().unwrap();
        store.commit_whole_tree("add local.md").unwrap();

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

    /// Seeds a diverged legacy repo: HEAD carries no `.gitattributes` at all,
    /// the remote is one commit ahead, and the local side is one commit ahead
    /// of the fork point. Returns the repo dir and the bare remote.
    ///
    /// The caller reopens the store (which backfills `.gitattributes`) and
    /// pulls.
    fn seed_diverged_legacy_repo() -> (TempDir, TempDir) {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();
        // Drop the worktree half before the seed commit, so HEAD is a repo
        // that predates the merge mapping.
        std::fs::remove_file(dir.path().join(".gitattributes")).unwrap();
        store
            .write(&RelPath::new("init.md").unwrap(), "init".to_string())
            .unwrap();
        store.commit().unwrap();
        store.commit_whole_tree("seed: add init.md").unwrap();

        let bare_dir = setup_bare_remote(&mut store, "origin");
        store.git_mut().git_fetch("origin").unwrap();

        // Local side moves ahead.
        store
            .write(&RelPath::new("local.md").unwrap(), "local".to_string())
            .unwrap();
        store.commit().unwrap();
        store.commit_whole_tree("add local.md").unwrap();

        // Remote side moves ahead too, on a different file — so the merge
        // itself is clean and the only thing that can block is the tree.
        let clone_dir = TempDir::new().unwrap();
        git_cmd()
            .args(["clone"])
            .arg(bare_dir.path())
            .arg(clone_dir.path())
            .output()
            .unwrap();
        std::fs::write(clone_dir.path().join("remote.md"), "remote").unwrap();
        for args in [
            vec!["add", "."],
            vec!["commit", "-m", "remote commit"],
            vec!["push"],
        ] {
            git_cmd()
                .args(&args)
                .current_dir(clone_dir.path())
                .output()
                .unwrap();
        }

        assert!(
            !dir.path().join(".gitattributes").exists(),
            "fixture must start without the mapping on disk"
        );
        (dir, bare_dir)
    }

    #[test]
    fn pull_is_not_blocked_by_the_backfilled_merge_mapping() {
        let (dir, bare_dir) = seed_diverged_legacy_repo();

        // Reopening is what backfills `.gitattributes`, dirtying a tree the
        // user never touched.
        let mut store = GitStore::new(dir.path()).unwrap();
        assert!(
            !store.git().git_status_all().unwrap().is_empty(),
            "the backfill must genuinely dirty the tree, or this proves nothing"
        );

        // Before the guard existed this returned "cannot pull with
        // uncommitted changes — commit or discard first", and `rdm discard`
        // could not clear it because discard re-ensures the mapping.
        let outcome = store.git_mut().git_pull("origin").unwrap();
        match outcome {
            PullOutcome::Success(result) => assert!(result.changed),
            PullOutcome::Conflict(_) => panic!("expected a clean merge, got conflict"),
        }

        assert!(dir.path().join("local.md").exists());
        assert!(dir.path().join("remote.md").exists());
        let attrs = std::fs::read_to_string(dir.path().join(".gitattributes")).unwrap();
        assert!(
            attrs.contains("merge=rdm-index"),
            "the mapping restored for the merge must be re-ensured after it, got: {attrs}"
        );

        let _ = bare_dir;
    }

    /// Seeds a *behind-only* legacy repo: HEAD carries no `.gitattributes`, and
    /// the remote is one commit ahead — a commit that adds its own mapping,
    /// exactly as any peer's `rdm commit` does once the backfill has shipped.
    /// The local side never moves, so the pull takes the fast-forward path.
    fn seed_behind_legacy_repo() -> (TempDir, TempDir) {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();
        std::fs::remove_file(dir.path().join(".gitattributes")).unwrap();
        store
            .write(&RelPath::new("init.md").unwrap(), "init".to_string())
            .unwrap();
        store.commit().unwrap();
        store.commit_whole_tree("seed: add init.md").unwrap();

        let bare_dir = setup_bare_remote(&mut store, "origin");

        let clone_dir = TempDir::new().unwrap();
        git_cmd()
            .args(["clone"])
            .arg(bare_dir.path())
            .arg(clone_dir.path())
            .output()
            .unwrap();
        std::fs::write(
            clone_dir.path().join(".gitattributes"),
            "INDEX.md merge=rdm-index\n**/INDEX.md merge=rdm-index\n",
        )
        .unwrap();
        std::fs::write(clone_dir.path().join("remote.md"), "remote").unwrap();
        for args in [
            vec!["add", "."],
            vec!["commit", "-m", "remote commit"],
            vec!["push"],
        ] {
            git_cmd()
                .args(&args)
                .current_dir(clone_dir.path())
                .output()
                .unwrap();
        }

        assert!(
            !dir.path().join(".gitattributes").exists(),
            "fixture must start without the mapping on disk"
        );
        (dir, bare_dir)
    }

    #[test]
    fn fast_forward_pull_is_not_blocked_by_the_backfilled_merge_mapping() {
        let (dir, bare_dir) = seed_behind_legacy_repo();

        // Reopening backfills `.gitattributes` as an untracked file — right
        // where the incoming fast-forward wants to write its committed copy.
        let mut store = GitStore::new(dir.path()).unwrap();
        assert!(
            store
                .git()
                .git_status_all()
                .unwrap()
                .iter()
                .any(|fs| fs.path == ".gitattributes"),
            "the backfill must genuinely dirty the tree, or this proves nothing"
        );

        // Before the guard covered this path, git refused the fast-forward with
        // "untracked working tree files would be overwritten by merge", and
        // `rdm discard` could not clear it — the repo was wedged for good.
        let outcome = store.git_mut().git_pull("origin").unwrap();
        match outcome {
            PullOutcome::Success(result) => assert!(result.changed),
            PullOutcome::Conflict(_) => panic!("expected a clean fast-forward, got conflict"),
        }

        assert!(dir.path().join("remote.md").exists());
        let attrs = std::fs::read_to_string(dir.path().join(".gitattributes")).unwrap();
        assert!(
            attrs.contains("merge=rdm-index"),
            "the mapping restored for the merge must be re-ensured after it, got: {attrs}"
        );

        let _ = bare_dir;
    }

    #[test]
    fn fast_forward_pull_still_tolerates_unrelated_user_dirt() {
        let (dir, bare_dir) = seed_behind_legacy_repo();
        let mut store = GitStore::new(dir.path()).unwrap();

        // The guard partitions the tree on the fast-forward path too, but must
        // not start *refusing* on user dirt there: git has always allowed a
        // fast-forward whose incoming changes do not collide with local edits.
        std::fs::write(dir.path().join("scratch.md"), "mine").unwrap();

        let outcome = store.git_mut().git_pull("origin").unwrap();
        match outcome {
            PullOutcome::Success(result) => assert!(result.changed),
            PullOutcome::Conflict(_) => panic!("expected a clean fast-forward, got conflict"),
        }
        assert_eq!(
            std::fs::read_to_string(dir.path().join("scratch.md")).unwrap(),
            "mine",
            "the user's uncommitted file must survive the fast-forward untouched"
        );

        let _ = bare_dir;
    }

    #[test]
    fn pull_still_refuses_when_the_user_edited_gitattributes() {
        let (dir, bare_dir) = seed_diverged_legacy_repo();
        let mut store = GitStore::new(dir.path()).unwrap();

        // A user line alongside rdm's mapping. The carve-out is byte-exact, so
        // this file is no longer "rdm's own write" and must still block —
        // otherwise the pull would silently discard the user's edit.
        let path = dir.path().join(".gitattributes");
        let mut attrs = std::fs::read_to_string(&path).unwrap();
        attrs.push_str("*.bin binary\n");
        std::fs::write(&path, &attrs).unwrap();

        let err = store.git_mut().git_pull("origin").unwrap_err();
        assert!(
            err.to_string()
                .contains("cannot pull with uncommitted changes"),
            "a user-edited .gitattributes must still block the pull, got: {err}"
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            attrs,
            "the refused pull must leave the user's edit untouched"
        );

        let _ = bare_dir;
    }

    #[test]
    fn is_rdm_mapping_write_matches_only_rdms_own_write() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();
        std::fs::remove_file(dir.path().join(".gitattributes")).unwrap();
        store
            .write(&RelPath::new("seed.md").unwrap(), "seed".to_string())
            .unwrap();
        store.commit().unwrap();
        store.commit_whole_tree("seed: no gitattributes").unwrap();

        let store = GitStore::new(dir.path()).unwrap();
        let mapping = store
            .git()
            .git_status_all()
            .unwrap()
            .into_iter()
            .find(|fs| fs.path == ".gitattributes")
            .expect("the backfill must have written it");
        assert!(store.git().is_rdm_mapping_write(&mapping).unwrap());

        // Any other path is never rdm's mapping write, whatever its content.
        std::fs::write(dir.path().join("seed.md"), "edited").unwrap();
        let seed = store
            .git()
            .git_status_all()
            .unwrap()
            .into_iter()
            .find(|fs| fs.path == "seed.md")
            .unwrap();
        assert!(!store.git().is_rdm_mapping_write(&seed).unwrap());

        // A user line appended to the mapping takes it out of the carve-out.
        let path = dir.path().join(".gitattributes");
        let mut attrs = std::fs::read_to_string(&path).unwrap();
        attrs.push_str("*.bin binary\n");
        std::fs::write(&path, attrs).unwrap();
        assert!(!store.git().is_rdm_mapping_write(&mapping).unwrap());
    }

    #[test]
    fn pull_diverged_conflicting_detects_conflicts() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();
        store
            .write(&RelPath::new("shared.md").unwrap(), "original".to_string())
            .unwrap();
        store.commit().unwrap();
        store.commit_whole_tree("seed: add shared.md").unwrap();

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
        store.commit_whole_tree("update shared.md locally").unwrap();

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
        store.commit_whole_tree("seed: add shared.md").unwrap();

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
        store.commit_whole_tree("update shared.md locally").unwrap();

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
        store.commit_whole_tree("seed: add init.md").unwrap();

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
        store.commit_whole_tree("add a.md").unwrap();
        store
            .write(&RelPath::new("b.md").unwrap(), "b".to_string())
            .unwrap();
        store.commit().unwrap();
        store.commit_whole_tree("add b.md").unwrap();
        store
            .write(&RelPath::new("c.md").unwrap(), "c".to_string())
            .unwrap();
        store.commit().unwrap();
        store.commit_whole_tree("add c.md").unwrap();

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
        store.commit_whole_tree("seed: init plan repo").unwrap();

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
    fn clone_remote_adds_merge_driver_config() {
        let (_source, bare) = make_bare_plan_repo();
        let target = TempDir::new().unwrap();
        let target_path = target.path().join("cloned");

        let _store =
            GitStore::clone_remote(bare.path().to_str().unwrap(), &target_path, None).unwrap();

        let config = std::fs::read_to_string(target_path.join(".git").join("config")).unwrap();
        assert!(
            config.contains("[merge \"rdm-index\"]"),
            "expected merge driver section after clone, got: {config}"
        );

        let attrs = std::fs::read_to_string(target_path.join(".gitattributes")).unwrap();
        assert!(
            attrs.contains("merge=rdm-index"),
            "expected cloned .gitattributes to carry the merge entries from source, got: {attrs}"
        );
    }

    #[test]
    fn clone_remote_backfills_gitattributes_when_source_lacks_it() {
        // Complements `clone_remote_adds_merge_driver_config`, which only
        // covers inheriting a committed `.gitattributes` from the source.
        let source = TempDir::new().unwrap();
        let mut store = GitStore::init(source.path()).unwrap();
        std::fs::remove_file(source.path().join(".gitattributes")).unwrap();
        store
            .write(&RelPath::new("INDEX.md").unwrap(), "# Index\n".to_string())
            .unwrap();
        store.commit().unwrap();
        store.commit_whole_tree("seed: no gitattributes").unwrap();

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

        let target = TempDir::new().unwrap();
        let target_path = target.path().join("cloned");
        let _clone =
            GitStore::clone_remote(bare.path().to_str().unwrap(), &target_path, None).unwrap();

        let attrs = std::fs::read_to_string(target_path.join(".gitattributes")).unwrap();
        assert!(
            attrs.contains("INDEX.md merge=rdm-index"),
            "expected root INDEX.md merge entry in the clone, got: {attrs}"
        );
        assert!(
            attrs.contains("**/INDEX.md merge=rdm-index"),
            "expected wildcard INDEX.md merge entry in the clone, got: {attrs}"
        );
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

    // ---- session state: sited so both whole-tree walks miss it ----

    /// Journal a real batch the way `Store::commit` does, under a pinned
    /// explicit id so the test never depends on the runner's process tree.
    fn journal_a_batch(store: &mut GitStore, key: &str) -> RelPath {
        let path = RelPath::new(key).unwrap();
        store.write(&path, "body".to_string()).unwrap();
        store.commit().unwrap();
        path
    }

    /// Opens a store on a repo whose `INDEX.md` merge mapping is ALREADY
    /// installed, so opening performs no `.gitattributes` back-fill and
    /// therefore journals nothing of its own.
    ///
    /// Without this the assertions below would be measuring the store's own
    /// (correct, and separately tested) journaling of that back-fill rather
    /// than the property under test.
    fn store_already_mapped(dir: &TempDir) -> GitStore {
        gix::init(dir.path()).unwrap();
        std::fs::write(
            dir.path().join(".gitattributes"),
            "INDEX.md merge=rdm-index\n**/INDEX.md merge=rdm-index\n",
        )
        .unwrap();
        GitStore::new(dir.path()).unwrap()
    }

    #[test]
    fn commit_journals_exactly_the_flushed_paths() {
        let dir = TempDir::new().unwrap();
        let mut store = store_already_mapped(&dir);
        journal_a_batch(&mut store, "a.md");

        let paths = store.session_paths().unwrap().clone();
        let id = store.session().unwrap().id.clone();
        let entries = rdm_core::session::journal::read_journal(&paths, &id).unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(names, vec!["a.md"], "not a superset, not empty");
        assert_eq!(entries[0].kind, JournalKind::Write);

        // A staged delete journals as a delete, so the removal stays
        // reproducible from the journal alone.
        let path = RelPath::new("a.md").unwrap();
        store.delete(&path).unwrap();
        store.commit().unwrap();
        let entries = rdm_core::session::journal::read_journal(&paths, &id).unwrap();
        assert_eq!(entries[0].kind, JournalKind::Delete);
    }

    #[test]
    fn commit_journals_the_content_identity_of_what_it_flushed() {
        // Base-blob identity per journaled path: without it the scoped commit
        // can only say "this changeset touched this path", which after a
        // concurrent overwrite commits someone else's bytes.
        let dir = TempDir::new().unwrap();
        let mut store = store_already_mapped(&dir);
        journal_a_batch(&mut store, "a.md");

        let paths = store.session_paths().unwrap().clone();
        let id = store.session().unwrap().id.clone();
        let entries = rdm_core::session::journal::read_journal(&paths, &id).unwrap();
        assert_eq!(
            entries[0].digest.as_deref(),
            Some(rdm_core::store::content_digest("body").as_str())
        );

        // A delete carries no digest — there are no bytes to identify.
        let path = RelPath::new("a.md").unwrap();
        store.delete(&path).unwrap();
        store.commit().unwrap();
        let entries = rdm_core::session::journal::read_journal(&paths, &id).unwrap();
        assert_eq!(entries[0].kind, JournalKind::Delete);
        assert_eq!(entries[0].digest, None);
    }

    #[test]
    fn a_side_write_journals_without_a_digest_so_the_commit_check_fails_open() {
        // `.gitattributes` is written outside the store, so there is no
        // staged content to identify. It must still be committable.
        let dir = TempDir::new().unwrap();
        gix::init(dir.path()).unwrap();
        let store = GitStore::new(dir.path()).unwrap();

        let paths = store.session_paths().unwrap().clone();
        let id = store.session().unwrap().id.clone();
        let entries = rdm_core::session::journal::read_journal(&paths, &id).unwrap();
        let attrs = entries
            .iter()
            .find(|e| e.path == crate::repo::GITATTRIBUTES_PATH)
            .expect("the back-filled .gitattributes must be journaled");
        assert_eq!(
            attrs.digest, None,
            "a store-bypassing write has no staged content to identify"
        );
    }

    #[test]
    fn a_changeset_path_another_session_overwrote_is_refused_rather_than_committed() {
        let dir = TempDir::new().unwrap();
        let mut store = store_already_mapped(&dir);
        let path = RelPath::new("projects/demo/tasks/fix-bug.md").unwrap();
        store.write(&path, "mine".to_string()).unwrap();
        store.commit().unwrap();

        // Another session lands its own bytes at the same path after this
        // changeset flushed but before it committed.
        std::fs::write(dir.path().join(path.as_str()), "theirs").unwrap();

        let err = store.commit_changeset(Some("mine"), &[]).unwrap_err();
        match err {
            Error::ChangesetPathOverwritten { item, .. } => assert_eq!(item, "task/fix-bug"),
            other => panic!("expected ChangesetPathOverwritten, got {other:?}"),
        }
    }

    /// The delete-side mirror of the write guard above.
    ///
    /// A delete journals `digest: None` by construction, so there is nothing
    /// to compare bytes against. What there is, is the state a deleting
    /// session leaves at the path: **absent**. A path that is present again at
    /// commit time therefore holds content this session did not put there, and
    /// applying the delete would destroy it with exit 0 on both sides.
    ///
    /// This replaces `a_stale_delete_still_destroys_a_concurrently_recreated_path`,
    /// which locked the pre-guard behavior. See docs/lost-update-evaluation.md
    /// § Selected mechanism.
    ///
    /// No `serial_scoped()` guard: that helper serializes only tests that pin
    /// `RDM_SESSION` process-globally, and this one passes its changeset
    /// message and resolves its session exactly as its neighbor does.
    #[test]
    fn a_stale_delete_over_a_concurrently_recreated_path_is_refused() {
        let dir = TempDir::new().unwrap();
        let mut store = store_already_mapped(&dir);
        let path = RelPath::new("projects/demo/tasks/doomed.md").unwrap();

        // Seed the doomed file and land it, so HEAD really carries the path
        // the delete will remove.
        store.write(&path, "mine".to_string()).unwrap();
        store.commit().unwrap();
        let seed_sha = store
            .commit_changeset(Some("seed"), &[])
            .unwrap()
            .sha
            .expect("the seed must land");

        // Journal the delete. No digest is recorded for it — there is no
        // staged content to identify.
        store.delete(&path).unwrap();
        store.commit().unwrap();

        // Another session recreates the path with its own bytes after this
        // changeset flushed but before it commits.
        std::fs::write(dir.path().join(path.as_str()), "theirs").unwrap();

        let err = store.commit_changeset(Some("mine"), &[]).unwrap_err();
        match &err {
            Error::ChangesetDeletePathRecreated { item, .. } => {
                assert_eq!(item, "task/doomed", "the refusal names the item");
            }
            other => panic!("expected ChangesetDeletePathRecreated, got {other:?}"),
        }

        // A refusal, not a partial landing: HEAD is still the seed commit and
        // the other session's bytes are untouched on disk.
        let head = store
            .git
            .head_commit_info()
            .unwrap()
            .expect("the seed commit")
            .sha;
        assert_eq!(head, seed_sha, "nothing landed: HEAD is still the seed");
        assert_eq!(
            store.fetch_body_at(&path, &head).unwrap(),
            "mine",
            "the seed's content for the path is still what HEAD carries"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join(path.as_str())).ok(),
            Some("theirs".to_string()),
            "and the other session's recreated bytes survive on disk"
        );
    }

    /// A multi-path delete (the `rdm roadmap delete` shape, which removes a
    /// roadmap's whole directory) is all-or-nothing, exactly like the write
    /// guard: one recreated path refuses the entire commit rather than landing
    /// the siblings.
    ///
    /// `read_journal` returns a `BTreeMap`-sorted list, so the path the error
    /// names is deterministic and can be asserted on directly.
    #[test]
    fn one_recreated_path_refuses_a_whole_multi_path_delete() {
        let dir = TempDir::new().unwrap();
        let mut store = store_already_mapped(&dir);
        let roadmap = RelPath::new("projects/demo/roadmaps/gone/roadmap.md").unwrap();
        let phase = RelPath::new("projects/demo/roadmaps/gone/phase-1-a.md").unwrap();

        store.write(&roadmap, "the roadmap".to_string()).unwrap();
        store.write(&phase, "the phase".to_string()).unwrap();
        store.commit().unwrap();
        let seed_sha = store
            .commit_changeset(Some("seed"), &[])
            .unwrap()
            .sha
            .expect("the seed must land");

        // The whole directory is deleted in one changeset.
        store.delete(&roadmap).unwrap();
        store.delete(&phase).unwrap();
        store.commit().unwrap();

        // Another session recreates only ONE of the two paths.
        std::fs::write(dir.path().join(roadmap.as_str()), "theirs").unwrap();

        let err = store.commit_changeset(Some("mine"), &[]).unwrap_err();
        match &err {
            Error::ChangesetDeletePathRecreated { item, .. } => {
                assert_eq!(item, "roadmap/gone", "the refusal names the recreated item");
            }
            other => panic!("expected ChangesetDeletePathRecreated, got {other:?}"),
        }

        // All-or-nothing: the sibling phase is still at HEAD, because nothing
        // landed at all.
        assert_eq!(
            store.fetch_body_at(&phase, &seed_sha).unwrap(),
            "the phase",
            "the sibling delete must not have landed on its own"
        );
    }

    /// Case 1 of four the guard must NOT trip on: two deletes, each its own
    /// changeset, one after the other.
    #[test]
    fn sequential_deletes_in_one_session_never_trip_the_delete_guard() {
        let dir = TempDir::new().unwrap();
        let mut store = store_already_mapped(&dir);
        let p1 = RelPath::new("projects/demo/tasks/one.md").unwrap();
        let p2 = RelPath::new("projects/demo/tasks/two.md").unwrap();

        store.write(&p1, "one".to_string()).unwrap();
        store.write(&p2, "two".to_string()).unwrap();
        store.commit().unwrap();
        store.commit_changeset(Some("seed"), &[]).unwrap();

        store.delete(&p1).unwrap();
        store.commit().unwrap();
        let first = store
            .commit_changeset(Some("drop one"), &[])
            .expect("the first delete must land");
        assert!(first.sha.is_some());
        assert!(first.committed.iter().any(|p| p == p1.as_str()));

        store.delete(&p2).unwrap();
        store.commit().unwrap();
        let second = store
            .commit_changeset(Some("drop two"), &[])
            .expect("the second delete must land too");
        assert!(second.sha.is_some());
        assert!(second.committed.iter().any(|p| p == p2.as_str()));
    }

    /// Case 2: a write and then a delete of the same path inside ONE
    /// uncommitted changeset. The session's own earlier write left the path
    /// present; its later delete left it absent, and absent is the state the
    /// guard reads.
    #[test]
    fn a_write_then_delete_in_one_uncommitted_batch_commits() {
        let dir = TempDir::new().unwrap();
        let mut store = store_already_mapped(&dir);
        let path = RelPath::new("projects/demo/tasks/edited.md").unwrap();

        store.write(&path, "v1".to_string()).unwrap();
        store.commit().unwrap();
        store.commit_changeset(Some("seed"), &[]).unwrap();

        // Flush 1: journals a Write plus its digest.
        store.write(&path, "v2".to_string()).unwrap();
        store.commit().unwrap();
        // Flush 2, same changeset: journals a Delete, which collapses over
        // the Write in `read_journal`.
        store.delete(&path).unwrap();
        store.commit().unwrap();

        let landed = store
            .commit_changeset(Some("edit then drop"), &[])
            .expect("a session's own write-then-delete must commit");
        let sha = landed.sha.expect("the changeset must produce a commit");
        assert!(
            store.fetch_body_at(&path, &sha).is_err(),
            "the delete really applied: the path is gone from the landed tree"
        );
    }

    /// Case 3: a create and then a delete of the same path inside ONE
    /// uncommitted changeset, on a path that was never in HEAD. The changeset
    /// is a no-op for it, so it must neither refuse nor claim to have
    /// committed anything.
    #[test]
    fn a_create_then_delete_in_one_uncommitted_batch_commits() {
        let dir = TempDir::new().unwrap();
        let mut store = store_already_mapped(&dir);
        let seed = RelPath::new("projects/demo/tasks/keep.md").unwrap();
        let path = RelPath::new("projects/demo/tasks/ephemeral.md").unwrap();

        store.write(&seed, "keep".to_string()).unwrap();
        store.commit().unwrap();
        store.commit_changeset(Some("seed"), &[]).unwrap();

        store.write(&path, "new".to_string()).unwrap();
        store.commit().unwrap();
        store.delete(&path).unwrap();
        store.commit().unwrap();

        // The changeset now also carries a real change so the commit is not
        // trivially empty.
        store.write(&seed, "kept, edited".to_string()).unwrap();
        store.commit().unwrap();

        let landed = store
            .commit_changeset(Some("create then drop"), &[])
            .expect("a session's own create-then-delete must commit");
        assert!(landed.sha.is_some());
        assert!(
            !landed.committed.iter().any(|p| p == path.as_str()),
            "the path was never in HEAD, so the changeset is a no-op for it; \
             committed: {:?}",
            landed.committed
        );
    }

    /// Case 4: another session already deleted this path and landed the
    /// deletion. The path is absent from disk AND from HEAD, so this
    /// changeset's own delete is a no-op — not a conflict.
    #[test]
    fn a_path_another_session_already_deleted_and_landed_commits() {
        let dir = TempDir::new().unwrap();
        let mut store = store_already_mapped(&dir);
        let seed = RelPath::new("projects/demo/tasks/keep.md").unwrap();
        let path = RelPath::new("projects/demo/tasks/shared.md").unwrap();

        store.write(&seed, "keep".to_string()).unwrap();
        store.write(&path, "shared".to_string()).unwrap();
        store.commit().unwrap();
        store.commit_changeset(Some("seed"), &[]).unwrap();

        // This session journals the delete and flushes it: the file leaves
        // disk here.
        store.delete(&path).unwrap();
        store.commit().unwrap();

        // Another session's changeset lands the very same deletion first.
        let paths = store.session_paths().unwrap().clone();
        let other = rdm_core::session::SessionId::new("other-session").unwrap();
        rdm_core::session::journal::record(
            &paths,
            &other,
            &[rdm_core::session::journal::JournalEntry {
                path: path.as_str().to_string(),
                kind: JournalKind::Delete,
                digest: None,
            }],
        )
        .unwrap();
        let theirs = store
            .commit_changeset_id(Some(&other), Some("theirs"), &[])
            .expect("the other session's delete lands cleanly");
        let their_sha = theirs.sha.expect("the other session must commit");
        assert!(
            store.fetch_body_at(&path, &their_sha).is_err(),
            "the other session's delete is what removed the path from HEAD"
        );

        // Give this session something of its own to land, then commit. Its
        // journal still claims the delete; the path is absent from both disk
        // and HEAD, so the guard must let it through as a no-op.
        store.write(&seed, "kept, edited".to_string()).unwrap();
        store.commit().unwrap();
        let landed = store
            .commit_changeset(Some("mine"), &[])
            .expect("a delete another session already landed must not refuse");
        assert!(landed.sha.is_some());
        assert!(
            !landed.committed.iter().any(|p| p == path.as_str()),
            "the path was already gone from HEAD, so this changeset did not \
             commit it; committed: {:?}",
            landed.committed
        );
    }

    /// Delete-then-recreate never reaches the delete guard.
    ///
    /// `read_journal` collapses a path to its **last** recorded kind (a
    /// `BTreeMap` keyed on path, documented as last-kind-wins), so a path this
    /// changeset deleted and then recreated journals as `Write` and lands in
    /// `ChangesetScope::writes` with a digest. The *write* guard owns it. This
    /// asserts that routing directly, so no delete-guard test can pass
    /// vacuously by mistaking this shape for the delete window.
    #[test]
    fn a_delete_then_recreate_is_routed_to_the_write_guard_not_the_delete_guard() {
        let dir = TempDir::new().unwrap();
        let mut store = store_already_mapped(&dir);
        let path = RelPath::new("projects/demo/tasks/fix-bug.md").unwrap();

        store.write(&path, "original".to_string()).unwrap();
        store.commit().unwrap();
        store.commit_changeset(Some("seed"), &[]).unwrap();

        store.delete(&path).unwrap();
        store.commit().unwrap();
        store.write(&path, "a-recreated".to_string()).unwrap();
        store.commit().unwrap();

        // Another session overwrites the recreated file before this changeset
        // commits.
        std::fs::write(dir.path().join(path.as_str()), "theirs").unwrap();

        let err = store.commit_changeset(Some("mine"), &[]).unwrap_err();
        match &err {
            Error::ChangesetPathOverwritten { item, .. } => {
                assert_eq!(item, "task/fix-bug");
            }
            Error::ChangesetDeletePathRecreated { .. } => panic!(
                "the delete guard must never see this path: the journal \
                 collapsed it to a Write, so the write guard owns it"
            ),
            other => panic!("expected ChangesetPathOverwritten, got {other:?}"),
        }
    }

    #[test]
    fn a_legacy_journal_line_without_a_digest_still_commits() {
        // An in-flight changeset created before digests existed must not be
        // bricked by the new check.
        let dir = TempDir::new().unwrap();
        let mut store = store_already_mapped(&dir);
        let path = RelPath::new("projects/demo/tasks/legacy.md").unwrap();
        store.write(&path, "content".to_string()).unwrap();
        store.commit().unwrap();

        // Rewrite the journal in the pre-digest format.
        let paths = store.session_paths().unwrap().clone();
        let id = store.session().unwrap().id.clone();
        let line = format!(
            "{{\"paths\":[{{\"path\":\"{}\",\"kind\":\"write\"}}]}}\n",
            path.as_str()
        );
        std::fs::write(
            rdm_core::session::journal::changeset_path(&paths, &id),
            line,
        )
        .unwrap();

        let committed = store.commit_changeset(Some("legacy"), &[]).unwrap();
        assert!(
            committed.sha.is_some(),
            "a digest-less entry must fail open, not block the commit"
        );
    }

    #[test]
    fn an_empty_flush_journals_nothing() {
        let dir = TempDir::new().unwrap();
        let mut store = store_already_mapped(&dir);
        store.commit().unwrap();
        let paths = store.session_paths().unwrap().clone();
        let id = store.session().unwrap().id.clone();
        assert!(
            !rdm_core::session::journal::changeset_path(&paths, &id).exists(),
            "an empty batch must not leave a journal claiming it happened"
        );
    }

    #[test]
    fn session_state_lives_inside_the_git_dir() {
        let dir = TempDir::new().unwrap();
        let store = store_already_mapped(&dir);
        let base = store.session_paths().unwrap().base().to_path_buf();
        assert!(
            base.starts_with(store.git_dir()),
            "session state must sit inside the git dir, which both tree walks skip; got {}",
            base.display()
        );
        assert!(
            !base.exists(),
            "state directories are created lazily on first write, never at open"
        );
    }

    #[test]
    fn git_status_ignores_session_state_dir() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();
        journal_a_batch(&mut store, "a.md");
        store.commit_whole_tree("seed").unwrap();

        // The only on-disk difference from HEAD is now the lease + journal.
        let paths = store.session_paths().unwrap().clone();
        assert!(
            paths.changesets_dir().exists(),
            "the journal really was written"
        );
        journal_a_batch(&mut store, "a.md"); // same content: no tree change
        let report = store.git().git_status_report().unwrap();
        assert!(
            report.is_clean(),
            "session state must be invisible to status, got {:?}",
            report.all()
        );
    }

    #[test]
    fn journal_presence_does_not_change_commit_tree_or_message() {
        fn tree_and_message(with_journal: bool) -> (String, String) {
            let dir = TempDir::new().unwrap();
            let mut store = GitStore::init(dir.path()).unwrap();
            let path = RelPath::new("a.md").unwrap();
            store.write(&path, "body".to_string()).unwrap();
            store.commit().unwrap();
            if !with_journal {
                // Delete the whole state dir: the commit path must not notice.
                let base = store.session_paths().unwrap().base().to_path_buf();
                std::fs::remove_dir_all(&base).unwrap();
            }
            let statuses = store.git().git_status_report().unwrap().all();
            let message = GitRepo::default_commit_message(&statuses);
            store.commit_whole_tree(&message).unwrap();
            let out = git_cmd()
                .args(["rev-parse", "HEAD^{tree}"])
                .current_dir(dir.path())
                .output()
                .unwrap();
            (
                String::from_utf8_lossy(&out.stdout).trim().to_string(),
                message,
            )
        }

        let (with_tree, with_message) = tree_and_message(true);
        let (without_tree, without_message) = tree_and_message(false);
        assert_eq!(with_tree, without_tree, "journal presence changed the tree");
        assert_eq!(with_message, without_message);
        assert!(!with_tree.is_empty());
    }

    #[test]
    fn commit_leaves_git_status_dirty_since_it_never_touches_git() {
        let dir = TempDir::new().unwrap();
        let mut store = GitStore::init(dir.path()).unwrap();
        let path = RelPath::new("hello.md").unwrap();
        store.write(&path, "world".to_string()).unwrap();
        store.commit().unwrap();

        // `Store::commit` only flushes to disk — it never touches git. So
        // `git status` still reports the file as untracked/dirty.
        let output = git_cmd()
            .args(["status", "--porcelain"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        let status = String::from_utf8_lossy(&output.stdout);
        assert!(
            !status.trim().is_empty(),
            "expected dirty git status since Store::commit never commits, got: {status}"
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
        store.commit_whole_tree("seed: add a.md").unwrap();

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
        store.commit_whole_tree("add a.md v1").unwrap();
        let v1_sha = store.head_sha().unwrap();

        store.write(&path, "v2".to_string()).unwrap();
        store.commit().unwrap();
        store.commit_whole_tree("update a.md to v2").unwrap();
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
        store.commit_whole_tree("seed: add seed.md").unwrap();
        let early_sha = store.head_sha().unwrap();

        // Later commit: introduce later.md.
        store
            .write(&RelPath::new("later.md").unwrap(), "later".to_string())
            .unwrap();
        store.commit().unwrap();
        store.commit_whole_tree("add later.md").unwrap();

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

    /// Verifies that gix initializes repositories with the files-based ref
    /// format (not reftable). This test documents the current ref format
    /// behavior and will help detect if that changes in future gix versions.
    ///
    /// Note: a reftable-format repo still writes `.git/HEAD` as a regular
    /// symref-style file for backward compatibility, so the HEAD checks below
    /// are only sanity checks of a conventional repo layout. The sole
    /// discriminator between the two formats is the presence or absence of
    /// the `.git/reftable/` directory, asserted last.
    #[test]
    fn gix_init_uses_files_ref_format() {
        let dir = TempDir::new().unwrap();
        let _store = GitStore::init(dir.path()).unwrap();

        let git_dir = dir.path().join(".git");
        assert!(git_dir.exists(), "expected .git directory to exist");

        // Sanity: HEAD exists as a regular symref-style file. (True under
        // both files and reftable formats — not a format discriminator.)
        let head_file = git_dir.join("HEAD");
        assert!(head_file.exists(), "expected .git/HEAD file to exist");
        assert!(
            head_file.is_file(),
            "expected .git/HEAD to be a regular file"
        );

        // Sanity: HEAD contains a ref pointer (not a hash). Also true under
        // reftable, which keeps a symref-style HEAD for backward compat.
        let head_content =
            std::fs::read_to_string(&head_file).expect("should be able to read HEAD file");
        assert!(
            head_content.starts_with("ref: "),
            "expected HEAD to contain a ref pointer, got: {}",
            head_content
        );

        // The actual ref-format discriminator: reftable format creates a
        // .git/reftable/ directory; the files format does not.
        let reftable_dir = git_dir.join("reftable");
        assert!(
            !reftable_dir.exists(),
            "did not expect .git/reftable/ directory (reftable format); \
             gix appears to be using reftable now"
        );
    }

    // ---- scoped commit / discard: attribution lives in the tree builder ----

    /// Seeds a plan repo with one project and an initial whole-tree commit,
    /// then returns a store pinned to `id`.
    fn scoped_repo(dir: &TempDir, id: &str) -> GitStore {
        let mut store = GitStore::init(dir.path()).unwrap();
        rdm_core::ops::init::init_with_config(&mut store, rdm_core::config::Config::default())
            .unwrap();
        rdm_core::ops::mutate(&mut store, "demo", |s| {
            rdm_core::ops::project::create_project(s, "demo", "Demo")
        })
        .unwrap();
        store.commit_whole_tree("seed").unwrap();
        drop(store);
        unsafe { std::env::set_var(rdm_core::session::RDM_SESSION_ENV, id) };
        GitStore::new(dir.path()).unwrap()
    }

    fn make_task(store: &mut GitStore, slug: &str) {
        make_task_in(store, "demo", slug);
    }

    /// Like [`make_task`], but in a named project — the cross-changeset tests
    /// need a project whose `project.md` is *not* in HEAD.
    fn make_task_in(store: &mut GitStore, project: &str, slug: &str) {
        rdm_core::ops::mutate(store, project, |s| {
            rdm_core::ops::task::create_task(
                s,
                rdm_core::ops::task::CreateTask {
                    project,
                    slug,
                    title: slug,
                    priority: rdm_core::model::Priority::Medium,
                    tags: None,
                    body: Some("body"),
                },
            )
            .map(|_| ())
        })
        .unwrap();
    }

    /// Every path in HEAD's tree — not just the ones the tip commit changed.
    ///
    /// The coherence assertions below are about what the commit's *tree*
    /// contains (including paths inherited from HEAD such as
    /// `projects/demo/INDEX.md`), so `show --name-only` would make the
    /// dangling-link check pass vacuously.
    fn head_tree_paths(dir: &TempDir) -> Vec<String> {
        git_in(dir, &["ls-tree", "-r", "--name-only", "HEAD"])
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(str::to_string)
            .collect()
    }

    /// Every markdown link target in a rendered index body.
    ///
    /// Well-defined because `display::format_top_level_index` emits one row
    /// per project whose only link target is `projects/<name>/INDEX.md`.
    fn index_link_targets(index: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut rest = index;
        while let Some(open) = rest.find("](") {
            rest = &rest[open + 2..];
            if let Some(close) = rest.find(')') {
                out.push(rest[..close].to_string());
                rest = &rest[close + 1..];
            } else {
                break;
            }
        }
        out
    }

    /// AC-3 assertion 1: the committed index names no path the tree lacks.
    fn assert_no_dangling_links(index: &str, tree: &[String]) {
        for target in index_link_targets(index) {
            assert!(
                tree.contains(&target),
                "the committed index links {target}, absent from the committed tree: {tree:?}"
            );
        }
    }

    /// AC-3 assertion 2: no `projects/<p>/INDEX.md` without its manifest.
    ///
    /// A general scan rather than a hardcoded project name, so it also catches
    /// a regression under some future project.
    fn assert_no_orphan_project_index(tree: &[String]) {
        for path in tree {
            let segments: Vec<&str> = path.split('/').collect();
            if segments.len() == 3 && segments[0] == "projects" && segments[2] == "INDEX.md" {
                let manifest = format!("projects/{}/project.md", segments[1]);
                assert!(
                    tree.contains(&manifest),
                    "the committed tree holds {path} but not {manifest}: {tree:?}"
                );
            }
        }
    }

    /// Rebinds the process-global session id and reopens the store.
    ///
    /// A fresh construction is how [`scoped_repo`] binds identity, so it must
    /// be re-done rather than reusing the previous handle — otherwise the two
    /// "sessions" silently merge and the test proves nothing.
    fn switch_session(dir: &TempDir, id: &str) -> GitStore {
        unsafe { std::env::set_var(rdm_core::session::RDM_SESSION_ENV, id) };
        GitStore::new(dir.path()).unwrap()
    }

    /// Runs `git` against `dir` with the inherited git environment cleared.
    ///
    /// Load-bearing: this suite runs from inside the repo's own pre-commit
    /// hook, where `GIT_DIR`/`GIT_INDEX_FILE` point at the outer repository —
    /// without this, every query below silently reads the wrong repo.
    fn git_in(dir: &TempDir, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .args(args)
            .current_dir(dir.path())
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    fn tree_paths(dir: &TempDir) -> Vec<String> {
        let out = std::process::Command::new("git")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .args(["show", "--name-only", "--pretty=format:", "HEAD"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(str::to_string)
            .collect()
    }

    // These tests pin RDM_SESSION process-globally, so they must not run
    // concurrently with each other. `serial_scoped` is a plain mutex held for
    // the body of each.
    fn serial_scoped() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn a_scoped_commit_contains_only_journaled_paths() {
        let _guard = serial_scoped();
        let dir = TempDir::new().unwrap();
        let mut store = scoped_repo(&dir, "unit-a");
        make_task(&mut store, "mine");

        // An unjournaled dirty path: written straight to disk, owned by no
        // changeset. It must be structurally unreachable, not filtered late.
        std::fs::write(dir.path().join("stranger.md"), "not mine\n").unwrap();

        let outcome = store.commit_changeset(Some("scoped"), &[]).unwrap();
        assert!(outcome.sha.is_some(), "the commit should have landed");
        let landed = tree_paths(&dir);
        assert!(
            landed.iter().any(|p| p == "projects/demo/tasks/mine.md"),
            "own path missing: {landed:?}"
        );
        assert!(
            !landed.iter().any(|p| p == "stranger.md"),
            "an unjournaled dirty path entered the tree: {landed:?}"
        );
        assert!(
            dir.path().join("stranger.md").exists(),
            "the foreign path must be left alone on disk"
        );
    }

    #[test]
    fn a_landed_changeset_is_truncated_out_of_its_journal() {
        let _guard = serial_scoped();
        let dir = TempDir::new().unwrap();
        let mut store = scoped_repo(&dir, "unit-trunc");
        make_task(&mut store, "once");
        store.commit_changeset(Some("first"), &[]).unwrap();

        let paths = store.session_paths().unwrap().clone();
        let id = store.session().unwrap().id.clone();
        let left = rdm_core::session::journal::read_journal(&paths, &id).unwrap();
        assert!(
            left.is_empty(),
            "landed paths must be truncated out of the journal, got {left:?}"
        );

        // A second commit therefore re-commits nothing — which is what stops
        // it from sweeping another session's later edit to the same path.
        let again = store.commit_changeset(Some("second"), &[]).unwrap();
        assert!(
            again.sha.is_none(),
            "a re-commit landed something: {again:?}"
        );
    }

    #[test]
    fn the_same_changeset_twice_against_one_head_yields_one_tree_oid() {
        let _guard = serial_scoped();
        let mut oids = Vec::new();
        for _ in 0..2 {
            let dir = TempDir::new().unwrap();
            let mut store = scoped_repo(&dir, "unit-det");
            make_task(&mut store, "deterministic");
            store.commit_changeset(Some("det"), &[]).unwrap();
            oids.push(
                git_in(&dir, &["rev-parse", "HEAD^{tree}"])
                    .trim()
                    .to_string(),
            );
        }
        assert_eq!(
            oids[0], oids[1],
            "committing the same changeset against the same HEAD must be deterministic"
        );
    }

    // ---- cross-changeset derived-index dependencies ----
    //
    // Session A creates a project and does not commit; session B creates a
    // document under it and commits first. B's `projects/alt/INDEX.md` has a
    // parent entity visible in neither HEAD nor B's own changeset. See
    // `docs/scoping-model-decision.md` § "INDEX.md Consistency in Partial
    // Commits".

    /// Arranges the three-session fixture and returns B's store, uncommitted.
    ///
    /// Session `a_id` creates project `alt` and leaves it staged; session
    /// `b_id` creates `b-task` under it. The caller commits.
    fn arrange_orphaned_parent(dir: &TempDir, a_id: &str, b_id: &str) -> GitStore {
        let mut store_a = scoped_repo(dir, a_id);
        rdm_core::ops::mutate(&mut store_a, "alt", |s| {
            rdm_core::ops::project::create_project(s, "alt", "Alt").map(|_| ())
        })
        .unwrap();
        drop(store_a);

        // Vacuity guards: without these, a future change that makes A's
        // project land early would turn the whole scenario green for the
        // wrong reason.
        assert!(
            dir.path().join("projects/alt/project.md").exists(),
            "fixture: A's project.md was never written to disk"
        );
        assert!(
            !head_tree_paths(dir)
                .iter()
                .any(|p| p == "projects/alt/project.md"),
            "fixture: A's project already landed in HEAD, the scenario is vacuous"
        );

        let mut store_b = switch_session(dir, b_id);
        make_task_in(&mut store_b, "alt", "b-task");
        store_b
    }

    #[test]
    fn a_commit_under_a_project_another_session_has_not_landed_still_lands() {
        let _guard = serial_scoped();
        let dir = TempDir::new().unwrap();
        let store_b = arrange_orphaned_parent(&dir, "unit-orphan-a", "unit-orphan-b");

        // The `unwrap` is the reproduction: before the seed-side prune this
        // panics on `Error::Git("failed to reconcile the generated indexes
        // against HEAD: project not found: alt …")`.
        let outcome = store_b.commit_changeset(Some("land b"), &[]).unwrap();
        assert!(outcome.sha.is_some(), "B's commit should have landed");

        let tree = head_tree_paths(&dir);
        let index = git_in(&dir, &["show", "HEAD:INDEX.md"]);
        assert_no_dangling_links(&index, &tree);
        assert_no_orphan_project_index(&tree);
    }

    /// The accepted consequence of dropping the orphaned subtree at the seed.
    ///
    /// B's own task lands in the tree, but the root index B commits carries no
    /// row for it: emitting `projects/alt/INDEX.md` for a project whose
    /// `project.md` this commit does not contain is exactly the orphan the
    /// coherence assertion forbids. The divergence is one-directional
    /// (`tree ⊇ index`) and self-healing — see
    /// `the_deferred_rows_return_once_the_owning_session_lands_its_project`
    /// and `docs/scoping-model-decision.md` § "INDEX.md Consistency".
    #[test]
    fn b_task_is_deliberately_absent_from_the_index_b_commits() {
        let _guard = serial_scoped();
        let dir = TempDir::new().unwrap();
        let store_b = arrange_orphaned_parent(&dir, "unit-omit-a", "unit-omit-b");
        store_b.commit_changeset(Some("land b"), &[]).unwrap();

        let tree = head_tree_paths(&dir);
        assert!(
            tree.iter().any(|p| p == "projects/alt/tasks/b-task.md"),
            "B's own document must still land in the tree: {tree:?}"
        );
        assert!(
            !tree.iter().any(|p| p == "projects/alt/project.md"),
            "A's uncommitted manifest must not have been swept in: {tree:?}"
        );
        let index = git_in(&dir, &["show", "HEAD:INDEX.md"]);
        assert!(
            !index.contains("projects/alt/INDEX.md"),
            "accepted: the deferred row must be absent, not dangling: {index}"
        );
    }

    #[test]
    fn the_deferred_rows_return_once_the_owning_session_lands_its_project() {
        let _guard = serial_scoped();
        let dir = TempDir::new().unwrap();
        let store_b = arrange_orphaned_parent(&dir, "unit-heal-a", "unit-heal-b");
        store_b.commit_changeset(Some("land b"), &[]).unwrap();
        drop(store_b);

        let store_a = switch_session(&dir, "unit-heal-a");
        store_a.commit_changeset(Some("land alt"), &[]).unwrap();

        let tree = head_tree_paths(&dir);
        for want in ["projects/alt/project.md", "projects/alt/INDEX.md"] {
            assert!(
                tree.iter().any(|p| p == want),
                "{want} missing after the owning session landed: {tree:?}"
            );
        }
        let index = git_in(&dir, &["show", "HEAD:INDEX.md"]);
        assert!(
            index.contains("projects/alt/INDEX.md"),
            "the deferred root-index row did not return: {index}"
        );
        let project_index = git_in(&dir, &["show", "HEAD:projects/alt/INDEX.md"]);
        assert!(
            project_index.contains("b-task"),
            "B's task did not reappear in the reconciled project index: {project_index}"
        );
        assert_no_dangling_links(&index, &tree);
        assert_no_orphan_project_index(&tree);
    }

    #[test]
    fn the_same_orphaned_changeset_twice_against_one_head_yields_one_tree_oid() {
        let _guard = serial_scoped();
        let mut oids = Vec::new();
        for _ in 0..2 {
            let dir = TempDir::new().unwrap();
            let store_b = arrange_orphaned_parent(&dir, "unit-odet-a", "unit-odet-b");
            store_b.commit_changeset(Some("det"), &[]).unwrap();
            oids.push(
                git_in(&dir, &["rev-parse", "HEAD^{tree}"])
                    .trim()
                    .to_string(),
            );
        }
        assert_eq!(
            oids[0], oids[1],
            "the seed-side prune sits on the determinism-critical path"
        );
    }

    // ---- compare-and-swap on HEAD: the lost-update guard ----

    /// Moves HEAD forward by one commit that keeps HEAD's tree byte-for-byte.
    ///
    /// A tree-preserving nudge is exactly what the compare-and-swap has to
    /// notice: nothing the committing changeset owns changed, so only the
    /// *parent* check can catch it. Deliberately does not go through
    /// [`GitStore`], so it cannot contend on the advisory commit lock the
    /// commit under test is already holding.
    fn nudge_head(root: &Path) {
        let repo = gix::open(root).unwrap();
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        let tree = head.tree_id().unwrap().detach();
        let parent = head.id().detach();
        let sig = gix::actor::Signature {
            name: "rival".into(),
            email: "rival@localhost".into(),
            time: gix::date::Time::now_local_or_utc(),
        };
        let mut buf = gix::date::parse::TimeBuf::default();
        let sig_ref = sig.to_ref(&mut buf);
        repo.commit_as(sig_ref, sig_ref, "HEAD", "rival nudge", tree, [parent])
            .unwrap();
    }

    /// Installs a pre-ref-update hook that nudges HEAD on the first
    /// `nudge_attempts` commit attempts, and returns the fire counter.
    fn race_head_for(root: &Path, nudge_attempts: usize) -> std::sync::Arc<AtomicUsize> {
        let root = root.to_path_buf();
        let fired = std::sync::Arc::new(AtomicUsize::new(0));
        let counter = fired.clone();
        crate::commit::test_hooks::set_pre_ref_update(Box::new(move || {
            if counter.fetch_add(1, Ordering::SeqCst) < nudge_attempts {
                nudge_head(&root);
            }
        }));
        fired
    }

    #[test]
    fn a_head_that_moves_mid_commit_is_rebuilt_against_and_retried_once() {
        let _guard = serial_scoped();
        let dir = TempDir::new().unwrap();
        let mut store = scoped_repo(&dir, "unit-cas-retry");
        make_task(&mut store, "racer");

        // Move HEAD exactly once, inside the read-HEAD → build-tree →
        // update-ref window, so the first compare-and-swap must lose.
        let fired = race_head_for(dir.path(), 1);
        let outcome = store.commit_changeset(Some("scoped"), &[]);
        crate::commit::test_hooks::clear_pre_ref_update();

        let outcome = outcome.expect("the retry after one HEAD move must succeed");
        assert_eq!(
            fired.load(Ordering::SeqCst),
            2,
            "the commit should have made exactly two attempts"
        );
        let sha = outcome.sha.expect("nothing landed after the retry");

        // The retry rebuilt against the rival commit rather than clobbering
        // it: HEAD is ours, and the rival is in our ancestry.
        assert_eq!(
            git_in(&dir, &["rev-parse", "HEAD"]).trim(),
            sha,
            "HEAD is not the retried commit"
        );
        assert!(
            git_in(&dir, &["log", "--format=%s", "-3"]).contains("rival nudge"),
            "the rival commit was lost rather than rebuilt against"
        );
        assert!(
            tree_paths(&dir)
                .iter()
                .any(|p| p == "projects/demo/tasks/racer.md"),
            "the changeset did not land on the retry"
        );
    }

    #[test]
    fn a_head_that_moves_on_every_attempt_errors_rather_than_losing_the_commit() {
        let _guard = serial_scoped();
        let dir = TempDir::new().unwrap();
        let mut store = scoped_repo(&dir, "unit-cas-give-up");
        make_task(&mut store, "starved");
        let before = git_in(&dir, &["rev-parse", "HEAD"]).trim().to_string();

        // A hook that fires on *every* attempt: the retry loses too.
        let fired = race_head_for(dir.path(), usize::MAX);
        let outcome = store.commit_changeset(Some("scoped"), &[]);
        crate::commit::test_hooks::clear_pre_ref_update();

        let err = outcome.expect_err("a permanently-moving HEAD must error, not spin or clobber");
        let msg = err.to_string();
        assert!(
            msg.contains("moved HEAD twice") && msg.contains("re-run `rdm commit`"),
            "the error must name the collision and the retry: {msg}"
        );
        assert_eq!(
            fired.load(Ordering::SeqCst),
            2,
            "the loop must give up after two attempts, not retry unboundedly"
        );

        // Nothing was lost: the rivals landed, ours did not, and the journal
        // still holds it so the advertised re-run lands it.
        assert!(
            !tree_paths(&dir)
                .iter()
                .any(|p| p == "projects/demo/tasks/starved.md"),
            "a failed commit must not have half-landed"
        );
        assert_ne!(
            git_in(&dir, &["rev-parse", "HEAD"]).trim(),
            before,
            "the rival commits should still be on HEAD"
        );
        let paths = store.session_paths().unwrap().clone();
        let id = store.session().unwrap().id.clone();
        let left = rdm_core::session::journal::read_journal(&paths, &id).unwrap();
        assert!(
            left.iter()
                .any(|e| e.path == "projects/demo/tasks/starved.md"),
            "a failed commit must not truncate the journal: {left:?}"
        );

        let retried = store.commit_changeset(Some("scoped"), &[]).unwrap();
        assert!(retried.sha.is_some(), "the advertised re-run did not land");
        assert!(
            tree_paths(&dir)
                .iter()
                .any(|p| p == "projects/demo/tasks/starved.md"),
            "the re-run did not land the changeset"
        );
    }

    #[test]
    fn a_commit_proceeds_when_the_advisory_lock_cannot_be_taken() {
        let _guard = serial_scoped();
        let dir = TempDir::new().unwrap();
        let mut store = scoped_repo(&dir, "unit-lock-busy");
        make_task(&mut store, "unlocked");

        // A fresh foreign lock: not stale, so it is never stolen. The commit
        // must give up waiting and proceed — the compare-and-swap, not the
        // lock, is what makes it correct, and the `Done:` hook path must never
        // be blocked by a lock it cannot take.
        let lock = dir.path().join(".git").join("rdm").join("commit.lock");
        std::fs::create_dir_all(lock.parent().unwrap()).unwrap();
        std::fs::write(&lock, b"held by a live process").unwrap();

        let outcome = store
            .commit_changeset(Some("scoped"), &[])
            .expect("a contended lock must not fail the commit");
        assert!(
            outcome.sha.is_some(),
            "the commit did not land while the lock was held: {outcome:?}"
        );
        assert!(
            tree_paths(&dir)
                .iter()
                .any(|p| p == "projects/demo/tasks/unlocked.md"),
            "the changeset did not land while the lock was held"
        );
        assert_eq!(
            std::fs::read(&lock).unwrap(),
            b"held by a live process",
            "the holder's lock file must be left untouched"
        );
    }

    #[test]
    fn a_journaled_path_that_vanished_is_skipped_not_fatal() {
        let _guard = serial_scoped();
        let dir = TempDir::new().unwrap();
        let mut store = scoped_repo(&dir, "unit-missing");
        make_task(&mut store, "vanishing");
        std::fs::remove_file(dir.path().join("projects/demo/tasks/vanishing.md")).unwrap();

        let outcome = store.commit_changeset(Some("skip"), &[]).unwrap();
        assert!(
            outcome
                .skipped_missing
                .iter()
                .any(|p| p == "projects/demo/tasks/vanishing.md"),
            "the vanished path was not reported as skipped: {outcome:?}"
        );
    }

    #[test]
    fn a_scoped_discard_leaves_foreign_paths_and_their_index_rows() {
        let _guard = serial_scoped();
        let dir = TempDir::new().unwrap();

        let mut mine = scoped_repo(&dir, "unit-disc-a");
        make_task(&mut mine, "discarded");
        drop(mine);

        unsafe { std::env::set_var(rdm_core::session::RDM_SESSION_ENV, "unit-disc-b") };
        let mut theirs = GitStore::new(dir.path()).unwrap();
        make_task(&mut theirs, "survivor");
        drop(theirs);

        unsafe { std::env::set_var(rdm_core::session::RDM_SESSION_ENV, "unit-disc-a") };
        let mut mine = GitStore::new(dir.path()).unwrap();
        mine.discard_changeset().unwrap();

        assert!(
            !dir.path().join("projects/demo/tasks/discarded.md").exists(),
            "the discarding session's own file survived"
        );
        assert!(
            dir.path().join("projects/demo/tasks/survivor.md").exists(),
            "a scoped discard destroyed another session's file"
        );
        let index = std::fs::read_to_string(dir.path().join("projects/demo/INDEX.md")).unwrap();
        assert!(
            index.contains("survivor"),
            "the regenerated index dropped the other session's row: {index}"
        );
        assert!(
            !index.contains("discarded"),
            "the regenerated index kept the discarded row: {index}"
        );
    }

    #[test]
    fn status_partitions_into_three_buckets_in_one_pass() {
        let _guard = serial_scoped();
        let dir = TempDir::new().unwrap();
        let mut mine = scoped_repo(&dir, "unit-status-a");
        make_task(&mut mine, "mine-status");
        drop(mine);

        unsafe { std::env::set_var(rdm_core::session::RDM_SESSION_ENV, "unit-status-b") };
        let mut theirs = GitStore::new(dir.path()).unwrap();
        make_task(&mut theirs, "theirs-status");
        drop(theirs);

        unsafe { std::env::set_var(rdm_core::session::RDM_SESSION_ENV, "unit-status-a") };
        let mine = GitStore::new(dir.path()).unwrap();
        let report = mine.status_report_scoped().unwrap();

        assert!(
            report
                .user
                .iter()
                .any(|f| f.path.ends_with("mine-status.md")),
            "own path missing from `user`: {report:?}"
        );
        assert!(
            report
                .others
                .iter()
                .any(|f| f.path.ends_with("theirs-status.md")),
            "the other session's path is not in `others`: {report:?}"
        );
        assert!(
            report
                .derived
                .iter()
                .all(|f| rdm_core::paths::is_derived_path(&f.path)),
            "a non-derived path landed in `derived`: {report:?}"
        );
        assert!(!report.is_clean(), "is_clean must still mean the raw truth");
        assert!(!report.is_changeset_clean(), "this changeset does own work");
        assert_eq!(
            report.total(),
            report.user.len() + report.derived.len() + report.others.len(),
            "total must cover every bucket"
        );
    }
}

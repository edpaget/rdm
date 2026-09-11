//! [`GitRepo`] local commit, status, discard, and tree-walking machinery.
//!
//! These methods build commits by writing tree objects directly (bypassing the
//! git index) and compare the working tree against HEAD for status/discard.
//!
//! # Two commit scopes
//!
//! Attribution lives in the tree builder itself, not in a late filter. Every
//! commit declares a [`CommitScope`]:
//!
//! - [`CommitScope::WholeTree`] rebuilds the tree from the working directory,
//!   exactly as rdm always did. It is the machine-global escape hatch, and it
//!   sweeps up whatever any other session left dirty.
//! - [`CommitScope::Changeset`] builds `HEAD` plus **only** the paths the
//!   caller names in a [`ChangesetScope`]. A path this caller did not write is
//!   structurally unreachable — it is never read, never blobbed, and never
//!   enters the tree.
//!
//! The `ChangesetScope` a `GitStore` passes here comes from
//! [`rdm_core::session::journal`], which records exactly which paths each
//! session flushed. This module does not resolve session identity itself: it
//! takes the path list it is given, so it stays testable without any ambient
//! process state.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use gix::object::tree::EntryKind;
use gix::objs::tree::EntryMode;
use rdm_core::error::{Error, Result};
use rdm_core::lock::AdvisoryLock;
use rdm_core::store::{RelPath, Store};

use crate::repo::GitRepo;
use crate::{FileChange, FileStatus, HeadCommitInfo, StatusReport};

/// Exactly the paths one changeset covers, split by what happened to them.
///
/// Built by [`GitStore`](crate::GitStore) from a session's journal, or by a
/// caller that wrote paths outside the `Store` entirely (see
/// [`extra_writes`](Self::extra_writes)). Nothing else is committable through
/// [`CommitScope::Changeset`] — a scoped commit is inexpressible without
/// naming the paths it covers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChangesetScope {
    /// Non-derived paths whose working-tree content this changeset wrote.
    pub writes: Vec<String>,
    /// Paths this changeset removed.
    pub deletes: Vec<String>,
    /// Derived paths (`INDEX.md`) this changeset journaled.
    ///
    /// These are **never** read from disk: the on-disk index already carries
    /// other sessions' rows. They are reconciled in memory as HEAD plus this
    /// changeset — see [`GitRepo::create_git_commit`].
    pub derived: Vec<String>,
    /// Paths a caller wrote outside the `Store` and is vouching for on this
    /// commit only.
    ///
    /// This is the explicit include-these-paths capability, not a standing
    /// exemption list: the paths are supplied per commit by the caller that
    /// wrote them (e.g. a back-filled `.gitattributes`), and nothing persists
    /// between commits.
    pub extra_writes: Vec<String>,
    /// The content identity this changeset journaled for each write, when the
    /// journal recorded one.
    ///
    /// Base-blob identity per journaled path: a path whose working-tree
    /// content no longer matches what this changeset wrote belongs to whoever
    /// overwrote it, and committing it here would land their bytes under this
    /// message.
    ///
    /// **This map covers writes only, and its fail-open answer is scoped to
    /// them.** A *write* absent from this map — a legacy journal line, or an
    /// [`extra_writes`](Self::extra_writes) entry that never entered the store
    /// — is committed without a digest comparison, which is the deliberate
    /// fail-open answer for writes.
    ///
    /// It says nothing at all about [`deletes`](Self::deletes), which never
    /// appear here by construction: a delete journals `digest: None` because
    /// there are no bytes to identify. Deletes are **not** committed
    /// unchecked — they are guarded by a separate commit-time *presence*
    /// check in [`GitRepo::build_changeset_tree`]'s delete loop, which refuses
    /// with [`Error::ChangesetDeletePathRecreated`] when a path this changeset
    /// deleted is present on disk again at commit time.
    ///
    /// [`GitRepo::build_changeset_tree`]: crate::repo::GitRepo
    pub digests: std::collections::BTreeMap<String, String>,
}

impl ChangesetScope {
    /// Returns whether the scope names no paths at all.
    pub fn is_empty(&self) -> bool {
        self.writes.is_empty()
            && self.deletes.is_empty()
            && self.derived.is_empty()
            && self.extra_writes.is_empty()
    }

    /// Returns every non-derived write, journaled or caller-supplied.
    fn all_writes(&self) -> impl Iterator<Item = &String> {
        self.writes.iter().chain(self.extra_writes.iter())
    }
}

/// What a commit's tree is built from.
#[derive(Clone, Copy, Debug)]
pub enum CommitScope<'a> {
    /// Rebuild the tree from the whole working directory.
    ///
    /// The machine-global escape hatch: this commits whatever is on disk,
    /// including paths other sessions left dirty. Reserved for fixture
    /// seeding and the explicit `--all` opt-ins.
    WholeTree,
    /// Build `HEAD` plus exactly the named changeset.
    Changeset(&'a ChangesetScope),
}

/// [`GitRepo::build_changeset_tree`]'s return: the new tree oid, the paths
/// that landed (`committed`), the journaled paths skipped as vanished
/// (`skipped`), and the wider `settled` set a caller truncates a session's
/// journal against. See that method's doc comment for what each Vec holds.
type ChangesetTreeResult = (gix::ObjectId, Vec<String>, Vec<String>, Vec<String>);

/// What a commit actually did, reported from one place so every porcelain
/// (CLI, server) states the same facts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommitReport {
    /// The new commit's SHA, or `None` when there was nothing to commit.
    pub sha: Option<String>,
    /// Repo-relative paths this commit landed, sorted.
    pub committed: Vec<String>,
    /// Journaled paths skipped because their working-tree file is gone.
    ///
    /// A concurrent discard, or a manual `rm`. Skipping is deliberate: a
    /// vanished path must never fail an otherwise-good commit.
    pub skipped_missing: Vec<String>,
    /// Paths this changeset can now prove are correctly reflected at the
    /// resulting tree, whether or not this specific commit changed their
    /// blob.
    ///
    /// Wider than `committed`: it also covers a no-op write-back (content
    /// already equal to HEAD) and every non-deferred reconciled derived path.
    /// It excludes `skipped_missing` paths and any derived path deferred for
    /// an unlanded project. This is the set a caller truncates a session's
    /// journal against, so an idempotent write still clears the journal even
    /// when the commit itself is a no-op (`sha: None`).
    pub(crate) settled: Vec<String>,
}

/// How long a scoped commit will wait for the advisory lock before giving up
/// and proceeding unlocked.
///
/// Deliberately far below the default 30s `hook_timeout_secs`: the `Done:`
/// hook path reaches this code, and a lock must never be what makes it miss
/// its deadline.
#[cfg(not(test))]
const COMMIT_LOCK_WAIT: Duration = Duration::from_secs(5);

/// The test-build value of [`COMMIT_LOCK_WAIT`], shortened so the
/// contended-lock arm (give up waiting, proceed unlocked) can be exercised
/// without stalling the suite for the production five seconds. Only the
/// duration differs — the arm under test is the same code.
#[cfg(test)]
const COMMIT_LOCK_WAIT: Duration = Duration::from_millis(120);

/// How stale a lock file must be before another process takes it over.
///
/// Bounds the damage from a process killed between acquiring and releasing.
const COMMIT_LOCK_STALE_AFTER: Duration = Duration::from_secs(30);

/// Where the advisory commit lock lives for a given git dir.
///
/// Shared by [`acquire_commit_lock`] and its tests so the two can never
/// disagree about which file is the lock.
pub(crate) fn commit_lock_path(git_dir: &Path) -> PathBuf {
    git_dir.join("rdm").join("commit.lock")
}

/// Test seam fired between building the scoped tree and the compare-and-swap
/// on HEAD.
///
/// The compare-and-swap is only reachable in a losing state if HEAD moves
/// inside that window, which no test could otherwise arrange deterministically.
/// Installing a hook here lets a test move HEAD at exactly that instant and
/// assert both the rebuild-and-retry arm and the give-up-after-two arm.
#[cfg(test)]
pub(crate) mod test_hooks {
    use std::cell::RefCell;

    thread_local! {
        static PRE_REF_UPDATE: RefCell<Option<Box<dyn FnMut()>>> =
            const { RefCell::new(None) };
    }

    /// Installs `f`, to be called once per commit attempt until cleared.
    pub(crate) fn set_pre_ref_update(f: Box<dyn FnMut()>) {
        PRE_REF_UPDATE.with(|h| *h.borrow_mut() = Some(f));
    }

    /// Removes any installed hook.
    pub(crate) fn clear_pre_ref_update() {
        PRE_REF_UPDATE.with(|h| *h.borrow_mut() = None);
    }

    pub(super) fn fire_pre_ref_update() {
        PRE_REF_UPDATE.with(|h| {
            if let Ok(mut slot) = h.try_borrow_mut()
                && let Some(f) = slot.as_mut()
            {
                f();
            }
        });
    }
}

#[cfg(test)]
use test_hooks::fire_pre_ref_update;

#[cfg(not(test))]
#[inline]
fn fire_pre_ref_update() {}

/// Takes the best-effort advisory lock guarding the read-HEAD → build-tree →
/// update-ref window of a scoped commit.
///
/// Best-effort by construction: failing to take the lock **proceeds anyway**
/// rather than erroring, because the compare-and-swap on HEAD is the actual
/// correctness mechanism and the lock is only there to make the common case
/// avoid a wasted rebuild.
///
/// The wait/staleness state machine itself lives in
/// [`rdm_core::lock::AdvisoryLock`], shared with the filesystem store's flush
/// precondition so the repo cannot carry two divergent lock implementations.
/// The *durations* stay here, because this path shortens them under
/// `cfg(test)` and that one does not.
fn acquire_commit_lock(git_dir: &Path) -> AdvisoryLock {
    AdvisoryLock::acquire(
        commit_lock_path(git_dir),
        COMMIT_LOCK_WAIT,
        COMMIT_LOCK_STALE_AFTER,
    )
}

/// Sorts tree entries the way git requires: by name, with directory names
/// compared as if they carried a trailing `/`.
///
/// A plain byte comparison gets this wrong (e.g. a `foo` directory vs a
/// `foo.md` blob) and produces trees `git fsck` rejects with `treeNotSorted`.
/// Shared by both tree builders so the hazard cannot be re-derived in one of
/// them: identical content must yield an identical tree either way.
fn sort_tree_entries(entries: &mut [gix::objs::tree::Entry]) {
    entries.sort_by(|a, b| {
        let sort_key = |e: &gix::objs::tree::Entry| -> Vec<u8> {
            let name = &*e.filename;
            if e.mode == EntryMode::from(EntryKind::Tree) {
                name.iter().chain(b"/").copied().collect()
            } else {
                name.to_vec()
            }
        };
        sort_key(a).cmp(&sort_key(b))
    });
}

/// Names the project a store path lives under, if any.
///
/// `projects/<p>/<something>/…` yields `Some(<p>)`. Everything else yields
/// `None` — including a top-level file literally named `projects/foo` (two
/// segments, no subtree), `projects/` alone, and any path outside `projects/`.
/// Total and allocation-free; the returned name borrows from `path`.
fn project_segment(path: &str) -> Option<&str> {
    let mut segments = path.split('/');
    if segments.next()? != "projects" {
        return None;
    }
    let project = segments.next()?;
    // A third non-empty segment is what distinguishes a project *subtree*
    // from a top-level file that merely happens to be named `projects/foo`.
    let third = segments.next()?;
    if project.is_empty() || third.is_empty() {
        return None;
    }
    Some(project)
}

/// Removes every `projects/<p>/**` entry whose `projects/<p>/project.md` is
/// absent from the same map, and returns the dropped project names, sorted.
///
/// The `project.md` manifest is the sentinel `list_roadmaps`/`list_tasks`/
/// `list_reviews` check before enumerating a project, so a subtree without one
/// makes [`rdm_core::ops::index::generate_index`] raise `ProjectNotFound`.
/// Pruning such a subtree out of the *seed* is what lets index generation
/// succeed for every other project.
///
/// Deterministic by construction: ordered collections throughout, never a
/// `HashSet`, because the caller sits on the commit's tree-oid path.
fn drop_orphaned_project_subtrees(files: &mut BTreeMap<String, String>) -> Vec<String> {
    let mut owned: BTreeSet<&str> = BTreeSet::new();
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for key in files.keys() {
        if let Some(project) = project_segment(key) {
            seen.insert(project);
            if rdm_core::paths::is_project_manifest(key) {
                owned.insert(project);
            }
        }
    }
    // Collected as owned `String`s before mutating, to end the borrow of
    // `files` that `owned`/`seen` hold.
    let orphans: BTreeSet<String> = seen.difference(&owned).map(|p| (*p).to_string()).collect();
    if orphans.is_empty() {
        return Vec::new();
    }
    files.retain(|key, _| project_segment(key).is_none_or(|p| !orphans.contains(p)));
    orphans.into_iter().collect()
}

/// An in-memory nested tree assembled from a flat `path -> blob oid` map.
#[derive(Default)]
struct TreeNode {
    blobs: BTreeMap<String, gix::ObjectId>,
    dirs: BTreeMap<String, TreeNode>,
}

impl TreeNode {
    fn insert(&mut self, path: &str, oid: gix::ObjectId) {
        match path.split_once('/') {
            None => {
                self.blobs.insert(path.to_string(), oid);
            }
            Some((head, rest)) => {
                self.dirs
                    .entry(head.to_string())
                    .or_default()
                    .insert(rest, oid);
            }
        }
    }

    fn write(&self, repo: &gix::Repository) -> Result<gix::ObjectId> {
        let mut entries: Vec<gix::objs::tree::Entry> = Vec::new();
        for (name, oid) in &self.blobs {
            entries.push(gix::objs::tree::Entry {
                mode: EntryMode::from(EntryKind::Blob),
                filename: name.clone().into(),
                oid: *oid,
            });
        }
        for (name, node) in &self.dirs {
            let subtree = node.write(repo)?;
            entries.push(gix::objs::tree::Entry {
                mode: EntryMode::from(EntryKind::Tree),
                filename: name.clone().into(),
                oid: subtree,
            });
        }
        sort_tree_entries(&mut entries);
        let tree = gix::objs::Tree { entries };
        Ok(repo
            .write_object(&tree)
            .map_err(|e| Error::Git(format!("failed to write tree: {e}")))?
            .detach())
    }
}

/// Builds nested trees from a flat `path -> blob oid` map.
///
/// The scoped counterpart to
/// [`build_tree_from_dir`](GitRepo::build_tree_from_dir). Both share
/// [`sort_tree_entries`], so identical content produces an identical tree oid
/// through either builder.
fn write_tree_from_map(
    repo: &gix::Repository,
    entries: &BTreeMap<String, gix::ObjectId>,
) -> Result<gix::ObjectId> {
    let mut root = TreeNode::default();
    for (path, oid) in entries {
        root.insert(path, *oid);
    }
    root.write(repo)
}

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
    ///
    /// # Scope
    ///
    /// [`CommitScope::WholeTree`] preserves the historical behavior
    /// byte-for-byte: rebuild from disk, one attempt, no compare-and-swap.
    ///
    /// [`CommitScope::Changeset`] instead seeds a flat `path -> blob oid` map
    /// from HEAD and applies **only** the named changeset on top — writes read
    /// from the working tree, deletes removed, derived indexes reconciled in
    /// memory. A path no one named is not filtered out late; it is never
    /// reachable. The ref update is a compare-and-swap against the HEAD the
    /// map was seeded from, with one rebuild-and-retry.
    ///
    /// # Residual race
    ///
    /// A concurrent committer can still land between the successful
    /// compare-and-swap and this process's post-commit index sync, and a path
    /// this changeset journaled may have been overwritten by another session
    /// between the journal write and the read here (attribution is
    /// path-level, not content-level). Closing that window is
    /// `plan-repo-concurrency`'s phase 6 and is deliberately **not** attempted
    /// here.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Git`] if the tree cannot be built, the derived
    /// indexes cannot be reconciled, or the ref update fails twice in a row
    /// against a moving HEAD.
    pub(crate) fn create_git_commit(
        &self,
        message: &str,
        scope: CommitScope<'_>,
    ) -> Result<CommitReport> {
        match scope {
            CommitScope::WholeTree => self.create_whole_tree_commit(message),
            CommitScope::Changeset(changeset) => self.create_scoped_commit(message, changeset),
        }
    }

    /// The historical whole-tree commit: rebuild the tree from disk and move
    /// HEAD to it.
    fn create_whole_tree_commit(&self, message: &str) -> Result<CommitReport> {
        let repo = self.repo.to_thread_local();
        let root = self.root.clone();
        let tree_id = self.build_tree_from_dir(&repo, &root)?;

        let parents: Vec<gix::ObjectId> = repo
            .head()
            .ok()
            .and_then(|mut h| h.peel_to_commit().ok())
            .map(|c| c.id().detach())
            .into_iter()
            .collect();

        let sig = self.commit_signature(&repo);
        let mut time_buf = gix::date::parse::TimeBuf::default();
        let sig_ref = sig.to_ref(&mut time_buf);

        let id = repo
            .commit_as(sig_ref, sig_ref, "HEAD", message, tree_id, parents)
            .map_err(|e| Error::Git(format!("failed to create commit: {e}")))?;

        // We built the tree directly, bypassing the git index.  Sync the index
        // to HEAD so that `git status` doesn't report every touched file as
        // modified.
        self.sync_index_to_head()?;

        Ok(CommitReport {
            sha: Some(id.detach().to_string()),
            committed: self.git_status_all()?.into_iter().map(|s| s.path).collect(),
            skipped_missing: Vec::new(),
            settled: Vec::new(),
        })
    }

    /// Builds and lands a commit containing HEAD plus exactly `changeset`.
    fn create_scoped_commit(
        &self,
        message: &str,
        changeset: &ChangesetScope,
    ) -> Result<CommitReport> {
        // Best-effort: the compare-and-swap below is what makes this correct.
        let _lock = acquire_commit_lock(self.git_dir());

        let mut attempt = 0u8;
        loop {
            // Re-opened per attempt: a retry exists precisely because another
            // process moved HEAD, so this handle must not carry a snapshot of
            // the ref state the losing attempt was built against.
            let repo = self.repo.to_thread_local();
            let head = self.head_commit_oid(&repo);
            let head_tree = head.and_then(|oid| {
                repo.find_object(oid)
                    .ok()
                    .and_then(|o| o.try_into_commit().ok())
                    .and_then(|c| c.tree_id().ok())
                    .map(|t| t.detach())
            });

            let (tree_id, committed, skipped_missing, settled) =
                self.build_changeset_tree(&repo, changeset, head)?;

            if Some(tree_id) == head_tree {
                // Nothing this changeset owns differs from HEAD. `settled`
                // still carries every path this changeset can now prove is
                // correctly reflected at HEAD, so the caller can truncate the
                // journal even though nothing landed.
                return Ok(CommitReport {
                    sha: None,
                    committed: Vec::new(),
                    skipped_missing,
                    settled,
                });
            }

            let sig = self.commit_signature(&repo);
            let commit = gix::objs::Commit {
                tree: tree_id,
                parents: head.into_iter().collect(),
                author: sig.clone(),
                committer: sig,
                encoding: None,
                message: message.into(),
                extra_headers: Vec::new(),
            };
            let commit_id = repo
                .write_object(&commit)
                .map_err(|e| Error::Git(format!("failed to write commit object: {e}")))?
                .detach();

            // No-op outside `cfg(test)`; see `test_hooks`.
            fire_pre_ref_update();

            if self.compare_and_swap_head(&repo, head, commit_id, message)? {
                self.sync_index_to_head()?;
                return Ok(CommitReport {
                    sha: Some(commit_id.to_string()),
                    committed,
                    skipped_missing,
                    settled,
                });
            }

            attempt += 1;
            if attempt > 1 {
                return Err(Error::Git(
                    "another session moved HEAD twice while this commit was being built — \
                     nothing was lost; re-run `rdm commit` to retry against the new HEAD"
                        .to_string(),
                ));
            }
        }
    }

    /// Resolves the committer/author signature, falling back to rdm's own.
    fn commit_signature(&self, repo: &gix::Repository) -> gix::actor::Signature {
        let default_sig = || gix::actor::Signature {
            name: "rdm".into(),
            email: "rdm@localhost".into(),
            time: gix::date::Time::now_local_or_utc(),
        };
        match repo.committer() {
            Some(Ok(s)) => s.to_owned().unwrap_or_else(|_| default_sig()),
            _ => default_sig(),
        }
    }

    /// Returns HEAD's commit oid, or `None` on an unborn HEAD.
    fn head_commit_oid(&self, repo: &gix::Repository) -> Option<gix::ObjectId> {
        repo.head()
            .ok()
            .and_then(|mut h| h.peel_to_commit().ok())
            .map(|c| c.id().detach())
    }

    /// Points HEAD at `new` only if it still points at `expected`.
    ///
    /// Returns `Ok(false)` when HEAD moved under us (the caller rebuilds and
    /// retries once), `Ok(true)` on success.
    fn compare_and_swap_head(
        &self,
        repo: &gix::Repository,
        expected: Option<gix::ObjectId>,
        new: gix::ObjectId,
        message: &str,
    ) -> Result<bool> {
        use gix::refs::transaction::{Change, LogChange, PreviousValue, RefEdit, RefLog};

        let previous = match expected {
            Some(oid) => PreviousValue::MustExistAndMatch(gix::refs::Target::Object(oid)),
            None => PreviousValue::MustNotExist,
        };
        let summary = message.lines().next().unwrap_or("commit");
        let edit = RefEdit {
            change: Change::Update {
                log: LogChange {
                    mode: RefLog::AndReference,
                    force_create_reflog: false,
                    message: format!("commit: {summary}").into(),
                },
                expected: previous,
                new: gix::refs::Target::Object(new),
            },
            name: "HEAD"
                .try_into()
                .map_err(|e| Error::Git(format!("invalid ref name HEAD: {e}")))?,
            deref: true,
        };
        match repo.edit_reference(edit) {
            Ok(_) => Ok(true),
            Err(e) => {
                // Distinguish "HEAD moved" from a genuine failure by looking
                // at HEAD itself rather than parsing an error string.
                let fresh = gix::open(&self.root)
                    .ok()
                    .and_then(|r| self.head_commit_oid(&r));
                if fresh != expected {
                    Ok(false)
                } else {
                    Err(Error::Git(format!("failed to update HEAD: {e}")))
                }
            }
        }
    }

    /// Builds the scoped tree: HEAD's entries plus exactly this changeset.
    ///
    /// Returns the tree oid, the paths that landed (their blob actually
    /// changed), the journaled paths skipped because their working-tree file
    /// has since vanished, and the wider set of paths this changeset can now
    /// prove are correctly reflected at the resulting tree — `settled` —
    /// which is `committed` plus every non-derived write whose content
    /// already matched HEAD (so it never entered `committed`) plus every
    /// non-derived delete that reached past the recreated-check, including a
    /// no-op delete of a path already absent (e.g. another session deleted
    /// and landed the same path first), plus every non-deferred reconciled
    /// derived path. `settled` excludes `skipped` paths and any derived path
    /// `reconcile_derived` deferred for an unlanded project, so a caller can
    /// safely truncate a session's journal against it even on a fully no-op
    /// commit — write or delete.
    fn build_changeset_tree(
        &self,
        repo: &gix::Repository,
        changeset: &ChangesetScope,
        head: Option<gix::ObjectId>,
    ) -> Result<ChangesetTreeResult> {
        let mut entries = self.collect_tree_at(repo, head)?;
        let mut committed: Vec<String> = Vec::new();
        let mut skipped: Vec<String> = Vec::new();

        // Derived paths are reconciled from HEAD-plus-this-changeset in
        // memory, never read from disk: the on-disk index already carries
        // other sessions' rows, which is precisely the defect this scoping
        // exists to fix.
        let derived = self.reconcile_derived(repo, changeset, head)?;

        for path in changeset.all_writes() {
            if rdm_core::paths::is_derived_path(path) {
                continue;
            }
            let file = self.root.join(path);
            let content = match std::fs::read(&file) {
                Ok(c) => c,
                Err(_) => {
                    // Vanished under us (a concurrent discard, or a manual
                    // rm). Report it; never fail the commit over it.
                    skipped.push(path.clone());
                    continue;
                }
            };
            // Content-level attribution. Path-level attribution alone commits
            // whatever is at the path now, which after a concurrent overwrite
            // is another session's work landing under this message. Fail open
            // on a digest-less entry (a legacy journal line, a caller-vouched
            // extra write) and on non-UTF8 content, which the digest cannot
            // describe.
            if let Some(journaled) = changeset.digests.get(path)
                && let Ok(text) = std::str::from_utf8(&content)
                && &rdm_core::store::content_digest(text) != journaled
            {
                return Err(Error::ChangesetPathOverwritten {
                    item: rdm_core::paths::describe_path(path),
                    path: path.clone(),
                });
            }
            let blob = repo
                .write_blob(&content)
                .map_err(|e| Error::Git(format!("failed to write blob for {path}: {e}")))?
                .detach();
            if entries.get(path) != Some(&blob) {
                committed.push(path.clone());
            }
            entries.insert(path.clone(), blob);
        }

        // The delete-side half of the same commit-time content discipline the
        // write branch above applies — but a *presence* test, not a digest
        // comparison. A delete journals `digest: None` by construction (there
        // are no bytes to identify), so there is nothing to compare bytes to;
        // what there is, is the state a deleting session leaves behind:
        // **absent**.
        //
        // The reference point is the WORKING TREE AT COMMIT TIME, never HEAD —
        // exactly as the write guard compares against `std::fs::read(&file)`
        // above and never consults HEAD. A HEAD basis would conflate "another
        // session changed this" with "I changed this earlier in my own
        // uncommitted batch", and would therefore refuse the stage-then-batch
        // workflow rdm prescribes.
        //
        // Two branches, both keyed on existence:
        //
        //   * absent at commit time → the path is as this session left it, so
        //     the delete proceeds. This covers write-then-delete and
        //     create-then-delete inside one uncommitted changeset, and a path
        //     another session already deleted and landed.
        //   * present at commit time → someone refilled a path this session
        //     emptied. Applying the delete would destroy their content with
        //     exit 0 on both sides, so refuse instead.
        //
        // Derived indexes are exempt for the same reason they are exempt from
        // the write loop: whatever regenerates them (`rdm index`, a pull, a
        // merge resolution) may legitimately refill a derived index this
        // changeset deleted. Their
        // commit-time correctness is `reconcile_derived`'s, not this loop's.
        //
        // A delete-then-*recreate* within one changeset never reaches here:
        // `read_journal` collapses a path to its last recorded kind, so the
        // recreate journals as a `Write` and the write guard above owns it.
        //
        // Every non-derived delete that reaches past the recreated-check is
        // settled, whether or not it actually removed an entry from the tree:
        // `entries.remove(path).is_some()` is false exactly when the path was
        // already absent (e.g. another session deleted and landed it first),
        // which is the state this delete asked for just as much as one that
        // removed a live entry. A derived delete is excluded here (not
        // untracked — every derived delete that actually removes an entry
        // already lands in `committed` below, same as before) because its
        // commit-time correctness is `reconcile_derived`'s, not this loop's,
        // matching the write guard's own derived exemption above.
        let mut settled_deletes: Vec<String> = Vec::new();
        for path in &changeset.deletes {
            if !rdm_core::paths::is_derived_path(path) && self.root.join(path).exists() {
                return Err(Error::ChangesetDeletePathRecreated {
                    item: rdm_core::paths::describe_path(path),
                    path: path.clone(),
                });
            }
            if entries.remove(path).is_some() {
                committed.push(path.clone());
            }
            if !rdm_core::paths::is_derived_path(path) {
                settled_deletes.push(path.clone());
            }
        }

        for (path, content) in &derived {
            let blob = repo
                .write_blob(content)
                .map_err(|e| Error::Git(format!("failed to write blob for {path}: {e}")))?
                .detach();
            if entries.get(path) != Some(&blob) {
                committed.push(path.clone());
            }
            entries.insert(path.clone(), blob);
        }

        committed.sort();
        committed.dedup();
        skipped.sort();

        // `settled`: every path this changeset can now prove is correctly
        // reflected at the tree just built, whether or not this specific
        // commit changed its blob. Starts from `committed` (paths that DID
        // change), then widens to every non-derived write that already
        // matched HEAD (skipping only what vanished — those stay journaled),
        // every non-derived delete that reached past the recreated-check
        // (including a no-op delete of a path already absent — settled_deletes,
        // above), then every non-deferred reconciled derived path (`derived`'s
        // keys already exclude anything `reconcile_derived` deferred for an
        // unlanded project).
        let mut settled = committed.clone();
        for path in changeset.all_writes() {
            if !rdm_core::paths::is_derived_path(path) && !skipped.contains(path) {
                settled.push(path.clone());
            }
        }
        settled.extend(settled_deletes);
        settled.extend(derived.keys().cloned());
        settled.sort();
        settled.dedup();

        let tree_id = write_tree_from_map(repo, &entries)?;
        Ok((tree_id, committed, skipped, settled))
    }

    /// Regenerates the derived indexes this changeset journaled, from HEAD
    /// plus this changeset only.
    ///
    /// Projects HEAD's document bytes into an in-memory store, applies this
    /// changeset's non-derived writes and deletes, re-runs
    /// [`rdm_core::ops::index::generate_index`] there, and returns **only**
    /// the derived paths this changeset journaled. Every other index stays at
    /// its HEAD oid, so an unrelated project's index is never silently
    /// rewritten by an unrelated session's commit.
    ///
    /// The `rdm-index` merge driver is ruled out by name here: it fires only
    /// during a merge, and a scoped commit performs none.
    ///
    /// Deterministic by construction — ordered maps throughout, no timestamps,
    /// no hash-iteration ordering — so committing the same changeset twice
    /// against the same HEAD yields the same tree oid.
    ///
    /// Ordinary mutations journal no derived paths any more, so the
    /// `changeset.derived.is_empty()` early return makes this a structural
    /// no-op for them; the remaining producers are the explicit `rdm index`
    /// command and the merge/clone reconciliation paths. The derived-path
    /// class itself is deleted by a later phase of `retire-generated-index`.
    fn reconcile_derived(
        &self,
        repo: &gix::Repository,
        changeset: &ChangesetScope,
        head: Option<gix::ObjectId>,
    ) -> Result<BTreeMap<String, Vec<u8>>> {
        if changeset.derived.is_empty() {
            return Ok(BTreeMap::new());
        }
        // Non-UTF-8 blobs are skipped when projecting into the in-memory
        // store (documents are text), but they keep their HEAD oid in the
        // tree, so nothing is dropped from the commit.
        let mut files: BTreeMap<String, String> = self
            .collect_blobs_at(repo, head)?
            .into_iter()
            .filter_map(|(path, bytes)| String::from_utf8(bytes).ok().map(|s| (path, s)))
            .collect();

        for path in changeset.all_writes() {
            if rdm_core::paths::is_derived_path(path) {
                continue;
            }
            match std::fs::read_to_string(self.root.join(path)) {
                Ok(content) => {
                    files.insert(path.clone(), content);
                }
                Err(_) => {
                    files.remove(path);
                }
            }
        }
        for path in &changeset.deletes {
            files.remove(path);
        }

        // `files` is now exactly `HEAD ∪ this changeset's writes − its
        // deletes`, so "absent from both HEAD and this changeset" reduces to
        // "absent from `files`". Drop every `projects/<p>/**` subtree whose
        // `project.md` is missing from that projection: such a parent is owned
        // by a *third* session that has not committed yet, and without the
        // drop `list_reviews`/`list_roadmaps`/`list_tasks` raise
        // `ProjectNotFound` and abort an otherwise-good commit with a
        // misleading `project not found` for a project the user did just
        // create.
        //
        // The fix point is the SEED, not the output path, and this was traced
        // against the real code:
        //
        //   * `generate_index` is ONE whole-store call, so there is no
        //     per-path regeneration to skip.
        //   * "Skip the offending derived path and fall back to its HEAD blob"
        //     fails twice over: the orphan has no HEAD blob (the owning
        //     session never committed the project), and the ROOT `INDEX.md`
        //     is not skipped at all yet still fails, because `list_projects`
        //     enumerates `projects/*` *directories* — this changeset's own
        //     write under the orphaned project creates that directory in the
        //     seeded store.
        //
        // The cost is a one-directional divergence (`tree ⊇ index`) that is
        // never dangling and heals on the owning session's next commit. See
        // `docs/scoping-model-decision.md` § "INDEX.md Consistency in Partial
        // Commits".
        let _deferred = drop_orphaned_project_subtrees(&mut files);

        let seed: Vec<(&str, &str)> = files
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let mut mem = rdm_core::store::MemoryStore::with_contents(seed);
        rdm_core::ops::index::generate_index(&mut mem).map_err(|e| {
            Error::Git(format!(
                "failed to reconcile the generated indexes against HEAD: {e}"
            ))
        })?;
        mem.commit()?;

        let mut out = BTreeMap::new();
        for path in &changeset.derived {
            let Ok(rel) = RelPath::new(path) else {
                continue;
            };
            // This skip is load-bearing, not merely defensive: a
            // `projects/<p>/INDEX.md` this changeset journaled for a subtree
            // the seed pruned above was never generated, so it is simply not
            // produced here. That is what keeps the commit free of an orphan
            // project index — one whose `project.md` the commit does not also
            // contain — and leaves the path journaled for the next commit.
            if let Ok(content) = mem.read(&rel) {
                out.insert(path.clone(), content.into_bytes());
            }
        }
        Ok(out)
    }

    /// Creates a whole-tree git commit with the given message.
    ///
    /// This is the low-level machine-global commit primitive.
    /// **`pub(crate)` on purpose**: outside this crate the only commit entry
    /// points are [`GitStore::commit_changeset`] (the scoped default) and
    /// [`GitStore::commit_whole_tree`] (the explicitly-named escape hatch), so
    /// a new out-of-crate committer that tries to sweep the tree fails to
    /// compile rather than drifting in silently.
    ///
    /// Unlike [`Store::commit`], which only ever stages (flushes to disk) and
    /// never creates a git commit, this always commits from the current
    /// working-directory state — including paths other sessions left dirty.
    ///
    /// Returns a `CommitReport` with `sha: None` if the working directory
    /// already matches HEAD (no-op).
    ///
    /// # Errors
    ///
    /// Returns `Error::Git` if the commit cannot be created.
    ///
    /// [`Store::commit`]: rdm_core::store::Store::commit
    /// [`GitStore::commit_changeset`]: crate::GitStore::commit_changeset
    /// [`GitStore::commit_whole_tree`]: crate::GitStore::commit_whole_tree
    pub(crate) fn git_commit(&self, message: &str) -> Result<CommitReport> {
        // Deliberately the raw list: a whole-tree commit rewrites the whole
        // tree, so it is gated by the raw truth, not by what a user authored.
        let status = self.git_status_all()?;
        if status.is_empty() {
            return Ok(CommitReport::default());
        }
        self.create_git_commit(message, CommitScope::WholeTree)
    }

    /// Creates a commit containing HEAD plus exactly `changeset`.
    ///
    /// The scoped counterpart to [`git_commit`](Self::git_commit), and the
    /// path every ordinary committer takes. See
    /// [`create_git_commit`](Self::create_git_commit) for the mechanism and
    /// the documented residual race.
    ///
    /// # Errors
    ///
    /// Returns `Error::Git` if the tree cannot be built, the derived indexes
    /// cannot be reconciled, or HEAD moves twice under the commit.
    pub(crate) fn git_commit_changeset(
        &self,
        message: &str,
        changeset: &ChangesetScope,
    ) -> Result<CommitReport> {
        if changeset.is_empty() {
            return Ok(CommitReport::default());
        }
        self.create_git_commit(message, CommitScope::Changeset(changeset))
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
    /// Used to generate a message when none is supplied explicitly. A scoped
    /// commit feeds it [`StatusReport::changeset`](crate::StatusReport::changeset)
    /// — never [`all`](crate::StatusReport::all) — so an auto-generated
    /// message can never name another session's file.
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

    /// Compares the working directory to HEAD and returns the raw, unfiltered
    /// list of changes.
    ///
    /// Walks the working directory tree and the HEAD tree, reporting files
    /// that are added, modified, or deleted.
    ///
    /// Deliberately `pub(crate)`: outside this crate the only entry point is
    /// [`git_status_report`](Self::git_status_report), so no out-of-crate call
    /// site can reacquire a list that conflates generated `INDEX.md` output
    /// with the user's own edits. The three legitimate raw callers are all
    /// in-crate whole-tree operations — [`git_commit`](Self::git_commit),
    /// [`git_discard`](Self::git_discard), and the pull clean-tree guard.
    ///
    /// # Errors
    ///
    /// Returns `Error::Git` if the repository state cannot be read.
    pub(crate) fn git_status_all(&self) -> Result<Vec<FileStatus>> {
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

    /// Compares the working directory to HEAD, splitting the result into
    /// user-authored changes and rdm-generated output.
    ///
    /// This is the sole entry point for observing working-tree changes from
    /// outside this crate. See [`StatusReport`] for the contract every
    /// consumer follows: gate destructive whole-tree actions on
    /// [`StatusReport::is_clean`], but report counts and listings from
    /// [`StatusReport::user`].
    ///
    /// Membership of `derived` is decided by
    /// [`rdm_core::paths::is_derived_path`].
    ///
    /// # Errors
    ///
    /// Returns `Error::Git` if the repository state cannot be read.
    pub fn git_status_report(&self) -> Result<StatusReport> {
        let (derived, user) = self
            .git_status_all()?
            .into_iter()
            .partition(|fs| rdm_core::paths::is_derived_path(&fs.path));
        Ok(StatusReport {
            user,
            derived,
            others: Vec::new(),
            unattributed: Vec::new(),
        })
    }

    /// Compares the working directory to HEAD and partitions the result into
    /// **four** buckets in ONE pass: this changeset's user-authored edits,
    /// this changeset's regenerated indexes, dirt a real other changeset
    /// claims, and dirt no changeset claims at all.
    ///
    /// `owned` is exactly the path set the calling session's journal claims.
    /// `all_owned` is the union of every live-or-orphaned changeset's claimed
    /// paths (this session's included) — a non-owned path found in
    /// `all_owned` goes to `others` (a real other changeset owns it); a
    /// non-owned path absent from `all_owned` goes to `unattributed` (no
    /// changeset owns it — a raw write outside rdm, or dirt predating
    /// session-scoped commits). The changeset filter and the derived filter
    /// are deliberately not two independent filters over the same list — one
    /// partition, so the view a user reads and the set a commit lands can
    /// never drift apart.
    ///
    /// [`StatusReport::is_clean`] still means "nothing at all differs" and
    /// [`StatusReport::all`] still covers everything, so the destructive-action
    /// gates keep working against the raw truth.
    ///
    /// # Errors
    ///
    /// Returns `Error::Git` if the repository state cannot be read.
    pub(crate) fn git_status_report_scoped(
        &self,
        owned: &std::collections::BTreeSet<String>,
        all_owned: &std::collections::BTreeSet<String>,
    ) -> Result<StatusReport> {
        let mut report = StatusReport {
            user: Vec::new(),
            derived: Vec::new(),
            others: Vec::new(),
            unattributed: Vec::new(),
        };
        for fs in self.git_status_all()? {
            if owned.contains(&fs.path) {
                if rdm_core::paths::is_derived_path(&fs.path) {
                    report.derived.push(fs);
                } else {
                    report.user.push(fs);
                }
            } else if all_owned.contains(&fs.path) {
                report.others.push(fs);
            } else {
                report.unattributed.push(fs);
            }
        }
        Ok(report)
    }

    /// Restores the working directory to match HEAD.
    ///
    /// Overwrites modified files, deletes added files, and restores deleted
    /// files. This is a destructive operation.
    ///
    /// # Behavior note — not literally HEAD-exact
    ///
    /// After the restore loop this re-ensures the rdm-managed
    /// `.gitattributes` merge-driver mapping (best-effort, errors swallowed so
    /// a discard can never fail on it). Without that, a discard would silently
    /// un-map the repo from the `INDEX.md` merge driver in both failure
    /// shapes: an as-yet-uncommitted `.gitattributes` is `Added` and would be
    /// deleted outright, and a tracked one whose HEAD blob predates the
    /// mapping would be reverted to a version without it. The mapping is
    /// re-appended either way, so the post-discard tree may differ from HEAD
    /// by exactly that file.
    ///
    /// Doing this here rather than in the CLI command is what makes every
    /// caller of this shared primitive inherit the fix.
    ///
    /// # Errors
    ///
    /// Returns `Error::Git` if the HEAD tree cannot be read or files cannot
    /// be written.
    pub fn git_discard(&self) -> Result<()> {
        // Deliberately the raw list: a discard restores the whole tree.
        let status = self.git_status_all()?;
        if status.is_empty() {
            // Still re-ensure: a clean tree can nevertheless be un-mapped if
            // HEAD's `.gitattributes` predates the mapping.
            let _ = self.ensure_gitattributes();
            return Ok(());
        }

        self.restore_paths_to_head(&status)?;

        // Reinstate the merge-driver mapping the restore may have just
        // removed (see the behavior note above). Best-effort by design: a
        // discard must never fail because of it.
        let _ = self.ensure_gitattributes();

        Ok(())
    }

    /// Restores exactly the listed paths to their HEAD content.
    ///
    /// The restore half of [`git_discard`](Self::git_discard), *without* its
    /// re-ensure of the `.gitattributes` mapping — a caller that wants the
    /// file left at HEAD (the pull guard, which restores it only so
    /// `git merge` will accept the tree) would be defeated by it.
    ///
    /// `status` entries must come from [`git_status_all`](Self::git_status_all)
    /// on this same repo. A `Modified`/`Deleted` path absent from HEAD is
    /// skipped rather than treated as an error.
    ///
    /// # Errors
    ///
    /// Returns `Error::Git` if the HEAD tree cannot be read or a file cannot
    /// be written or removed.
    pub(crate) fn restore_paths_to_head(&self, status: &[FileStatus]) -> Result<()> {
        let repo = self.repo.to_thread_local();
        let head_files = self.collect_head_blobs(&repo)?;
        let root = self.root.as_path();

        for fs in status {
            Self::restore_one_to_head(root, &head_files, fs)?;
        }

        Ok(())
    }

    /// Restores a single path's working-tree content to its state in
    /// `head_files` — the per-file body shared by
    /// [`restore_paths_to_head`](Self::restore_paths_to_head) and
    /// [`restore_paths_to_head_scoped`](Self::restore_paths_to_head_scoped).
    fn restore_one_to_head(
        root: &Path,
        head_files: &BTreeMap<String, Vec<u8>>,
        fs: &FileStatus,
    ) -> Result<()> {
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
        Ok(())
    }

    /// Restores exactly the listed paths to HEAD, skipping any whose on-disk
    /// content no longer matches what this changeset journaled.
    ///
    /// The discard-side counterpart of the write/delete content guards in
    /// [`build_changeset_tree`](Self::build_changeset_tree): a discard is
    /// destructive exactly where a scoped commit would otherwise refuse, so
    /// it applies the same fail-open digest/presence check per path — but as
    /// a per-path **skip**, not an all-or-nothing refusal, since (unlike a
    /// commit's single tree object) a discard has no atomicity constraint
    /// forcing it to abandon the whole batch over one contested path.
    ///
    /// `digests` maps a journaled **write**'s path to the digest of the
    /// content this session flushed there; `deletes` is the set of paths
    /// this session journaled as **deleted**. A path present in neither map
    /// (a legacy digest-less journal line) restores unconditionally —
    /// fail-open, exactly mirroring `build_changeset_tree`'s policy. So does
    /// a path whose on-disk content has vanished or is not valid UTF-8: the
    /// guard can only refuse a *comparison it can make*, never a missing one.
    ///
    /// Returns `(restored, skipped)` — the paths actually restored (or
    /// removed, for a path this changeset added) and the paths left
    /// untouched because another session's content or recreation was
    /// detected there since this changeset last wrote them.
    ///
    /// # Errors
    ///
    /// Returns `Error::Git` if the HEAD tree cannot be read or a file cannot
    /// be written or removed.
    pub(crate) fn restore_paths_to_head_scoped(
        &self,
        status: &[FileStatus],
        digests: &BTreeMap<String, String>,
        deletes: &BTreeSet<String>,
    ) -> Result<(Vec<String>, Vec<String>)> {
        let repo = self.repo.to_thread_local();
        let head_files = self.collect_head_blobs(&repo)?;
        let root = self.root.as_path();

        let mut restored = Vec::new();
        let mut skipped = Vec::new();

        for fs in status {
            if deletes.contains(&fs.path) {
                if root.join(&fs.path).exists() {
                    // Someone recreated a path this session deleted since —
                    // applying the delete would destroy their content.
                    skipped.push(fs.path.clone());
                    continue;
                }
            } else if let Some(journaled) = digests.get(&fs.path)
                && let Ok(content) = std::fs::read(root.join(&fs.path))
                && let Ok(text) = std::str::from_utf8(&content)
                && &rdm_core::store::content_digest(text) != journaled
            {
                // Overwritten by someone else since this session wrote it.
                skipped.push(fs.path.clone());
                continue;
            }

            Self::restore_one_to_head(root, &head_files, fs)?;
            restored.push(fs.path.clone());
        }

        Ok((restored, skipped))
    }

    /// Returns whether `fs` is the `.gitattributes` merge-driver mapping rdm
    /// wrote itself, and nothing else.
    ///
    /// True only when the path is `.gitattributes` and its working-tree
    /// content is *exactly* what
    /// [`ensure_gitattributes`](Self::ensure_gitattributes) produces from
    /// HEAD's content. A user's own edit to the file — whether or not the
    /// mapping is also present — therefore answers `false` and is never
    /// discarded on their behalf.
    ///
    /// # Errors
    ///
    /// Returns `Error::Git` if the HEAD tree cannot be read.
    pub(crate) fn is_rdm_mapping_write(&self, fs: &FileStatus) -> Result<bool> {
        if fs.path != crate::repo::GITATTRIBUTES_PATH {
            return Ok(false);
        }
        let repo = self.repo.to_thread_local();
        let head_files = self.collect_head_blobs(&repo)?;
        let head_content = head_files
            .get(&fs.path)
            .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
            .unwrap_or_default();
        let working = std::fs::read_to_string(self.root.join(&fs.path)).unwrap_or_default();
        Ok(working == crate::repo::gitattributes_with_mapping(&head_content))
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
        self.collect_tree_at(repo, self.head_commit_oid(repo))
    }

    /// Collects all files from an arbitrary commit's tree as
    /// `path -> blob_oid`.
    ///
    /// `None` (an unborn HEAD) yields an empty map, so a scoped commit seeded
    /// from it *is* the whole tree and lands with zero parents.
    fn collect_tree_at(
        &self,
        repo: &gix::Repository,
        commit: Option<gix::ObjectId>,
    ) -> Result<BTreeMap<String, gix::ObjectId>> {
        let mut files = BTreeMap::new();
        let Some(oid) = commit else {
            return Ok(files);
        };
        let tree = repo
            .find_object(oid)
            .map_err(|e| Error::Git(format!("failed to find commit {oid}: {e}")))?
            .try_into_commit()
            .map_err(|e| Error::Git(format!("{oid} is not a commit: {e}")))?
            .tree()
            .map_err(|e| Error::Git(format!("failed to get tree for {oid}: {e}")))?;
        self.walk_tree(repo, &tree, "", &mut files)?;
        Ok(files)
    }

    /// Collects all file contents from the HEAD tree as `path -> bytes`.
    fn collect_head_blobs(&self, repo: &gix::Repository) -> Result<BTreeMap<String, Vec<u8>>> {
        self.collect_blobs_at(repo, self.head_commit_oid(repo))
    }

    /// Collects all file contents from an arbitrary commit's tree as
    /// `path -> bytes`.
    fn collect_blobs_at(
        &self,
        repo: &gix::Repository,
        commit: Option<gix::ObjectId>,
    ) -> Result<BTreeMap<String, Vec<u8>>> {
        let mut files = BTreeMap::new();
        let Some(oid) = commit else {
            return Ok(files);
        };
        let tree = repo
            .find_object(oid)
            .map_err(|e| Error::Git(format!("failed to find commit {oid}: {e}")))?
            .try_into_commit()
            .map_err(|e| Error::Git(format!("{oid} is not a commit: {e}")))?
            .tree()
            .map_err(|e| Error::Git(format!("failed to get tree for {oid}: {e}")))?;
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
mod commit_lock_tests {
    use super::*;
    use std::time::Instant;
    use tempfile::TempDir;

    /// Backdates `path`'s mtime by `age`, so the staleness branch can be
    /// exercised without waiting `COMMIT_LOCK_STALE_AFTER` real seconds.
    fn backdate(path: &Path, age: Duration) {
        let when = std::time::SystemTime::now() - age;
        let f = std::fs::OpenOptions::new().write(true).open(path).unwrap();
        f.set_modified(when).unwrap();
    }

    #[test]
    fn a_free_lock_is_taken_and_released_on_drop() {
        let dir = TempDir::new().unwrap();
        let lock_path = commit_lock_path(dir.path());
        {
            let lock = acquire_commit_lock(dir.path());
            assert!(lock.held(), "a free lock should have been taken");
            assert!(lock_path.exists(), "the lock file should exist while held");
        }
        assert!(
            !lock_path.exists(),
            "dropping the guard must release the lock"
        );
    }

    #[test]
    fn a_stale_lock_is_taken_over_rather_than_waited_out() {
        let dir = TempDir::new().unwrap();
        let lock_path = commit_lock_path(dir.path());
        std::fs::create_dir_all(lock_path.parent().unwrap()).unwrap();
        std::fs::write(&lock_path, b"pid 1 (killed)").unwrap();
        backdate(&lock_path, COMMIT_LOCK_STALE_AFTER + Duration::from_secs(5));

        let started = Instant::now();
        let lock = acquire_commit_lock(dir.path());
        assert!(
            lock.held(),
            "a lock older than COMMIT_LOCK_STALE_AFTER must be taken over, \
             not waited out — otherwise a killed process blocks the bounded \
             hook path"
        );
        assert!(
            started.elapsed() < COMMIT_LOCK_WAIT,
            "takeover must be immediate, not after the wait deadline"
        );
        assert_ne!(
            std::fs::read(&lock_path).unwrap(),
            b"pid 1 (killed)".to_vec(),
            "the dead holder's bytes must not survive the takeover"
        );
    }

    #[test]
    fn a_lock_still_fresh_is_left_alone_and_the_caller_proceeds_unlocked() {
        let dir = TempDir::new().unwrap();
        let lock_path = commit_lock_path(dir.path());
        std::fs::create_dir_all(lock_path.parent().unwrap()).unwrap();
        std::fs::write(&lock_path, b"held by a live process").unwrap();

        {
            let lock = acquire_commit_lock(dir.path());
            assert!(
                !lock.held(),
                "a fresh foreign lock must not be stolen; the caller proceeds \
                 unlocked instead"
            );
        }
        assert_eq!(
            std::fs::read(&lock_path).unwrap(),
            b"held by a live process",
            "releasing an unheld lock must not delete the holder's file"
        );
    }

    #[test]
    fn an_unusable_lock_directory_degrades_to_proceeding_unlocked() {
        let dir = TempDir::new().unwrap();
        // `<git_dir>/rdm` occupied by a regular file: `create_dir_all` fails.
        std::fs::write(dir.path().join("rdm"), b"not a directory").unwrap();

        let lock = acquire_commit_lock(dir.path());
        assert!(
            !lock.held(),
            "failing to create the lock directory must degrade to unlocked, \
             never error or hang — the compare-and-swap is the correctness \
             mechanism, the lock is only an optimization"
        );
    }
}

#[cfg(test)]
mod orphaned_subtree_tests {
    use super::*;

    fn map(paths: &[&str]) -> BTreeMap<String, String> {
        paths
            .iter()
            .map(|p| ((*p).to_string(), "body".to_string()))
            .collect()
    }

    fn keys(files: &BTreeMap<String, String>) -> Vec<String> {
        files.keys().cloned().collect()
    }

    #[test]
    fn drops_a_subtree_with_no_manifest() {
        let mut files = map(&[
            "projects/alt/tasks/b.md",
            "projects/alt/roadmaps/r/roadmap.md",
        ]);
        let dropped = drop_orphaned_project_subtrees(&mut files);
        assert_eq!(dropped, vec!["alt".to_string()]);
        assert!(keys(&files).is_empty(), "{:?}", keys(&files));
    }

    #[test]
    fn keeps_a_project_that_has_its_manifest() {
        let paths = [
            "projects/demo/project.md",
            "projects/demo/INDEX.md",
            "projects/demo/tasks/a.md",
        ];
        let mut files = map(&paths);
        assert!(drop_orphaned_project_subtrees(&mut files).is_empty());
        assert_eq!(keys(&files).len(), paths.len());
    }

    #[test]
    fn never_touches_paths_outside_projects() {
        let mut files = map(&["rdm.toml", "INDEX.md", ".gitattributes"]);
        assert!(drop_orphaned_project_subtrees(&mut files).is_empty());
        assert_eq!(keys(&files).len(), 3);
    }

    #[test]
    fn ignores_a_top_level_file_named_projects_foo() {
        // Two segments, no subtree: treating this as a project named `foo`
        // would delete an unrelated file from the seed.
        let mut files = map(&["projects/foo", "projects/demo/project.md"]);
        assert!(drop_orphaned_project_subtrees(&mut files).is_empty());
        assert!(keys(&files).iter().any(|k| k == "projects/foo"));
    }

    #[test]
    fn is_a_no_op_on_an_empty_map() {
        let mut files: BTreeMap<String, String> = BTreeMap::new();
        assert!(drop_orphaned_project_subtrees(&mut files).is_empty());
        assert!(files.is_empty());
    }

    #[test]
    fn returns_the_dropped_names_sorted() {
        let mut files = map(&[
            "projects/zeta/tasks/z.md",
            "projects/alpha/tasks/a.md",
            "projects/mid/tasks/m.md",
            "projects/kept/project.md",
        ]);
        let dropped = drop_orphaned_project_subtrees(&mut files);
        assert_eq!(dropped, vec!["alpha", "mid", "zeta"]);
        assert_eq!(keys(&files), vec!["projects/kept/project.md"]);
    }

    #[test]
    fn project_segment_is_total() {
        assert_eq!(project_segment("projects/demo/project.md"), Some("demo"));
        assert_eq!(project_segment("projects/demo/tasks/a.md"), Some("demo"));
        assert_eq!(project_segment("projects/foo"), None);
        assert_eq!(project_segment("projects/"), None);
        assert_eq!(project_segment("projects"), None);
        assert_eq!(project_segment("projects//a.md"), None);
        assert_eq!(project_segment("projects/demo/"), None);
        assert_eq!(project_segment("rdm.toml"), None);
        assert_eq!(project_segment(""), None);
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

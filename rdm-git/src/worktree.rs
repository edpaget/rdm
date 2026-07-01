//! Git worktree lifecycle management for the **project repo**.
//!
//! These helpers operate on the git repository rdm is invoked in (the project
//! or "code" repo), keeping worktrees and branches keyed to plan items. They
//! are deliberately separate from the plan-repo `GitStore` (in `rdm-store-git`):
//! every function shells out to `git` "at" a path, mirroring the repo-agnostic
//! inspection helpers in [`crate`] ([`discover_git_dir`](crate::discover_git_dir),
//! [`current_branch_at`](crate::current_branch_at), …).
//!
//! An rdm-managed worktree is identified by a marker file `rdm-item` written
//! into the worktree's private admin directory (`$GIT_DIR/worktrees/<name>/`).
//! Because that directory is removed by git whenever the worktree is pruned or
//! removed — by any means — there is no stale registry to maintain: [`list`]
//! and [`remove`] simply enumerate `git worktree list --porcelain` and keep the
//! entries that still carry a marker.

use std::path::{Path, PathBuf};
use std::process::Output;

/// Errors that can occur during worktree operations.
///
/// Variants are matchable so the CLI can map each to an actionable message.
#[derive(Debug)]
pub enum WorktreeError {
    /// The path is not inside a git repository.
    NotAGitRepo(PathBuf),
    /// The discovered project repo is actually the plan repo — the two must be
    /// distinct directories.
    IsPlanRepo(PathBuf),
    /// The `git` executable could not be found.
    GitMissing,
    /// The working directory to run git in does not exist or is not a directory.
    ///
    /// Distinguished from [`GitMissing`](WorktreeError::GitMissing) so an invalid
    /// or stale working directory is reported actionably instead of being
    /// misclassified as a missing git installation.
    NoSuchDirectory(PathBuf),
    /// The worktree at this path has uncommitted changes.
    Dirty(PathBuf),
    /// The branch has commits not merged into HEAD and was not force-deleted.
    UnmergedBranch(String),
    /// The target path is a worktree but was not created by rdm (no marker).
    NotRdmWorktree(PathBuf),
    /// No rdm worktree matches the given item or path.
    NotFound(String),
    /// A git command failed; carries the trimmed stderr.
    Git(String),
}

impl std::fmt::Display for WorktreeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WorktreeError::NotAGitRepo(p) => {
                write!(f, "not inside a git repository: {}", p.display())
            }
            WorktreeError::IsPlanRepo(p) => write!(
                f,
                "current repository is the plan repo ({}) — run `rdm worktree` from your project (code) repository instead",
                p.display()
            ),
            WorktreeError::GitMissing => {
                write!(f, "git is not installed — install git to use worktrees")
            }
            WorktreeError::NoSuchDirectory(p) => write!(
                f,
                "directory does not exist: {} — run rdm from inside your project checkout",
                p.display()
            ),
            WorktreeError::Dirty(p) => write!(
                f,
                "worktree has uncommitted changes: {} — commit/stash them or pass --force",
                p.display()
            ),
            WorktreeError::UnmergedBranch(b) => write!(
                f,
                "branch '{b}' has commits not merged into HEAD — pass --force to delete anyway"
            ),
            WorktreeError::NotRdmWorktree(p) => {
                write!(f, "{} is not an rdm-managed worktree", p.display())
            }
            WorktreeError::NotFound(item) => write!(f, "no rdm worktree found for '{item}'"),
            WorktreeError::Git(msg) => write!(f, "git error: {msg}"),
        }
    }
}

impl std::error::Error for WorktreeError {}

/// A convenience `Result` alias for worktree operations.
pub type Result<T> = std::result::Result<T, WorktreeError>;

/// A plan-item reference, the same syntax used by `Done:` commit directives.
///
/// - `<roadmap>/<phase-stem-or-number>` → [`ItemRef::Phase`]
/// - `task/<slug>` → [`ItemRef::Task`] (`task` is a reserved roadmap prefix)
/// - `<roadmap>` (no `/`) → [`ItemRef::Roadmap`] — the whole roadmap, one
///   worktree shared by all its phases
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemRef {
    /// A roadmap phase.
    Phase {
        /// The roadmap slug.
        roadmap: String,
        /// The phase stem (or number, pre-canonicalization).
        stem: String,
    },
    /// A standalone task.
    Task {
        /// The task slug.
        slug: String,
    },
    /// A whole roadmap — keys the single worktree all of its phases share.
    Roadmap {
        /// The roadmap slug.
        roadmap: String,
    },
}

impl ItemRef {
    /// Parses an item reference of the form `<roadmap>/<phase>`, `task/<slug>`,
    /// or a bare `<roadmap>` (no `/`, keying the whole roadmap).
    ///
    /// # Errors
    ///
    /// Returns [`WorktreeError::NotFound`] if the input is empty or, when a `/`
    /// is present, either side is empty.
    pub fn parse(s: &str) -> Result<ItemRef> {
        let s = s.trim();
        let Some((left, right)) = s.split_once('/') else {
            // No separator → the whole roadmap. Existence is validated later by
            // `resolve_item`.
            if s.is_empty() {
                return Err(WorktreeError::NotFound(
                    "empty item — use <roadmap>, <roadmap>/<phase>, or task/<slug>".to_string(),
                ));
            }
            return Ok(ItemRef::Roadmap {
                roadmap: s.to_string(),
            });
        };
        let left = left.trim();
        let right = right.trim();
        if left.is_empty() || right.is_empty() {
            return Err(WorktreeError::NotFound(format!(
                "'{s}' is not a valid item — use <roadmap>, <roadmap>/<phase>, or task/<slug>"
            )));
        }
        if left.eq_ignore_ascii_case("task") {
            Ok(ItemRef::Task {
                slug: right.to_string(),
            })
        } else {
            Ok(ItemRef::Phase {
                roadmap: left.to_string(),
                stem: right.to_string(),
            })
        }
    }

    /// Returns the canonical item string (`<roadmap>/<stem>`, `task/<slug>`, or
    /// a bare `<roadmap>`).
    pub fn canonical(&self) -> String {
        match self {
            ItemRef::Phase { roadmap, stem } => format!("{roadmap}/{stem}"),
            ItemRef::Task { slug } => format!("task/{slug}"),
            ItemRef::Roadmap { roadmap } => roadmap.clone(),
        }
    }

    /// Returns the git branch name for this item.
    ///
    /// Phases map to `phase/<roadmap>/<stem>`; tasks to `task/<slug>`; whole
    /// roadmaps to `roadmap/<slug>`.
    pub fn branch_name(&self) -> String {
        match self {
            ItemRef::Phase { roadmap, stem } => format!("phase/{roadmap}/{stem}"),
            ItemRef::Task { slug } => format!("task/{slug}"),
            ItemRef::Roadmap { roadmap } => format!("roadmap/{roadmap}"),
        }
    }

    /// Returns the worktree directory name (the branch with `/` → `-`).
    pub fn dir_name(&self) -> String {
        self.branch_name().replace('/', "-")
    }

    /// Inverts [`branch_name`](ItemRef::branch_name): parses a git branch name
    /// back into the item it keys, or `None` when the branch does not follow the
    /// `phase/<roadmap>/<stem>`, `task/<slug>`, or `roadmap/<slug>` convention.
    ///
    /// This is the signal that lets [`current`] recognize a hand-made worktree —
    /// or the main checkout sitting on an item branch — without a marker.
    pub fn from_branch(branch: &str) -> Option<ItemRef> {
        let branch = branch.trim();
        if let Some(rest) = branch.strip_prefix("phase/") {
            let (roadmap, stem) = rest.split_once('/')?;
            if roadmap.is_empty() || stem.is_empty() {
                return None;
            }
            Some(ItemRef::Phase {
                roadmap: roadmap.to_string(),
                stem: stem.to_string(),
            })
        } else if let Some(slug) = branch.strip_prefix("task/") {
            let slug = slug.trim();
            if slug.is_empty() {
                return None;
            }
            Some(ItemRef::Task {
                slug: slug.to_string(),
            })
        } else if let Some(roadmap) = branch.strip_prefix("roadmap/") {
            let roadmap = roadmap.trim();
            if roadmap.is_empty() {
                return None;
            }
            Some(ItemRef::Roadmap {
                roadmap: roadmap.to_string(),
            })
        } else {
            None
        }
    }
}

/// Information about an rdm-managed worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeInfo {
    /// The canonical item reference (`<roadmap>/<stem>`, `task/<slug>`, or a
    /// bare `<roadmap>`).
    pub item: String,
    /// The git branch checked out in the worktree.
    pub branch: String,
    /// The absolute path to the worktree.
    pub path: PathBuf,
    /// Whether [`add`] created the worktree (`false` for an idempotent hit).
    /// Not meaningful for entries returned by [`list`].
    pub created: bool,
    /// Whether the worktree has uncommitted changes. Populated by [`list`];
    /// always `false` from [`add`].
    pub dirty: bool,
}

/// The plan-item context of the current checkout, as reported by [`current`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrentWorktree {
    /// Canonical item reference (`<roadmap>/<stem>`, `task/<slug>`, or a bare
    /// `<roadmap>`).
    pub item: String,
    /// The branch checked out in the current working tree.
    pub branch: String,
    /// Absolute path to the current working tree's top level.
    pub path: PathBuf,
    /// `true` when an `rdm-item` marker is present (an rdm-created worktree);
    /// `false` when the item was inferred from the branch name alone — a
    /// hand-made worktree, or the main checkout sitting on an item branch. In
    /// both cases the item *is* recognized; the flag only distinguishes how.
    pub rdm_managed: bool,
}

/// Options for [`remove`].
#[derive(Debug, Clone, Copy, Default)]
pub struct RemoveOptions {
    /// Remove even if the worktree is dirty (and force-delete the branch).
    pub force: bool,
    /// Also delete the worktree's branch after removal.
    pub delete_branch: bool,
}

/// Options for [`prune`].
#[derive(Debug, Clone, Copy, Default)]
pub struct PruneOptions {
    /// Also delete each pruned worktree's branch.
    pub delete_branch: bool,
    /// Remove (and force-delete the branch of) even a dirty worktree.
    pub force: bool,
    /// Report what would be removed without removing anything.
    pub dry_run: bool,
}

/// What [`prune`] did (or would do) for a single candidate worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PruneAction {
    /// The worktree was removed.
    Removed,
    /// `dry_run` was set; the worktree is a candidate but was left in place.
    WouldRemove,
    /// The worktree was dirty and `force` was not set, so it was skipped.
    SkippedDirty,
    /// The worktree was removed, but its branch was retained because it is not
    /// merged into HEAD (branch deletion was requested without `force`). The
    /// worktree removal itself succeeded — this is a partial success, not a
    /// failure. Carries a human-readable `reason` naming the retained branch.
    RemovedBranchKept {
        /// Why the branch was kept (e.g. `branch '<name>' not merged into HEAD`).
        reason: String,
    },
    /// Removal was attempted but failed; carries the error message.
    Failed(String),
}

/// The outcome of [`prune`] for one done-item worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PruneResult {
    /// The canonical item reference the worktree keys to.
    pub item: String,
    /// The git branch the worktree had checked out.
    pub branch: String,
    /// The path of the worktree.
    pub path: PathBuf,
    /// What happened to it.
    pub action: PruneAction,
}

/// Returns whether the plan item named by the canonical `item` string resolves
/// to a `done` status against the plan `store`.
///
/// A phase is done when its frontmatter status is [`rdm_core::model::PhaseStatus::Done`];
/// a task when its status is [`rdm_core::model::TaskStatus::Done`]; a bare roadmap when
/// every one of its phases is terminal
/// ([`rdm_core::ops::roadmap::computed_status`] == [`rdm_core::ops::roadmap::RoadmapStatus::Done`]).
/// Any item that fails to parse, does not exist, or cannot be read is treated as
/// not-done (returns `false`), so a worktree is never pruned on a read error.
fn item_is_done(store: &impl rdm_core::store::Store, project: &str, item: &str) -> bool {
    let Ok(parsed) = ItemRef::parse(item) else {
        return false;
    };
    match parsed {
        ItemRef::Phase { roadmap, stem } => {
            rdm_core::io::load_phase(store, project, &roadmap, &stem)
                .map(|d| d.frontmatter.status == rdm_core::model::PhaseStatus::Done)
                .unwrap_or(false)
        }
        ItemRef::Task { slug } => rdm_core::io::load_task(store, project, &slug)
            .map(|d| d.frontmatter.status == rdm_core::model::TaskStatus::Done)
            .unwrap_or(false),
        ItemRef::Roadmap { roadmap } => {
            match rdm_core::ops::phase::list_phases(store, project, &roadmap) {
                Ok(phases) => {
                    let statuses: Vec<_> =
                        phases.iter().map(|(_, d)| d.frontmatter.status).collect();
                    rdm_core::ops::roadmap::computed_status(&statuses)
                        == rdm_core::ops::roadmap::RoadmapStatus::Done
                }
                Err(_) => false,
            }
        }
    }
}

/// Removes every rdm worktree whose plan item is already `done`, in one pass.
///
/// Enumerates [`list`], keeps only worktrees whose `item` resolves to a `done`
/// status against the plan `store` (via the private `item_is_done` helper), and for each such
/// candidate: skips it as [`PruneAction::SkippedDirty`] if it is dirty and
/// `opts.force` is unset; records [`PruneAction::WouldRemove`] without touching
/// it when `opts.dry_run` is set; otherwise removes it (via the internal
/// `remove_info`, reusing this single [`list`] pass rather than re-scanning)
/// honoring `opts.delete_branch` / `opts.force`, and maps the outcome:
/// [`PruneAction::Removed`] on full success; [`PruneAction::RemovedBranchKept`]
/// when the worktree was removed but its unmerged branch was retained
/// ([`WorktreeError::UnmergedBranch`], force off); [`PruneAction::SkippedDirty`]
/// if it went dirty between the scan and removal; [`PruneAction::Failed`] on any
/// other error. Non-done worktrees are not candidates and are omitted from the
/// returned vector entirely.
///
/// A single worktree's removal failure is captured in its [`PruneResult`]
/// rather than aborting the whole batch.
///
/// # Errors
///
/// Returns [`WorktreeError::Git`] if `git worktree list` fails, or
/// [`WorktreeError::GitMissing`] if `git` is not installed. Propagates
/// [`WorktreeError::NoSuchDirectory`] from the [`list`] pass if `repo_root` does
/// not exist (or is not a directory).
pub fn prune(
    repo_root: &Path,
    store: &impl rdm_core::store::Store,
    project: &str,
    opts: PruneOptions,
) -> Result<Vec<PruneResult>> {
    let worktrees = list(repo_root)?;
    let mut results = Vec::new();
    for wt in worktrees {
        if !item_is_done(store, project, &wt.item) {
            continue;
        }
        let action = if wt.dirty && !opts.force {
            PruneAction::SkippedDirty
        } else if opts.dry_run {
            PruneAction::WouldRemove
        } else {
            match remove_info(
                repo_root,
                &wt,
                RemoveOptions {
                    force: opts.force,
                    delete_branch: opts.delete_branch,
                },
            ) {
                Ok(()) => PruneAction::Removed,
                // The worktree WAS removed; only the unmerged-branch cleanup was
                // declined. Report the partial success distinctly so counts stay
                // accurate and the orphaned branch is visible.
                Err(WorktreeError::UnmergedBranch(b)) => PruneAction::RemovedBranchKept {
                    reason: format!("branch '{b}' not merged into HEAD — pass --force to delete"),
                },
                // The worktree went dirty between the scan and removal: treat it
                // like the scan-time dirty skip, not an opaque failure.
                Err(WorktreeError::Dirty(_)) => PruneAction::SkippedDirty,
                // NOTE: a branch-delete failure for any OTHER reason (e.g. a raw
                // Git error) still maps to Failed even though the worktree was
                // removed — a narrower latent misreport class intentionally left
                // out of this phase's scope.
                Err(e) => PruneAction::Failed(e.to_string()),
            }
        };
        results.push(PruneResult {
            item: wt.item,
            branch: wt.branch,
            path: wt.path,
            action,
        });
    }
    Ok(results)
}

/// Discovers the project repo's **main** working tree from `cwd`.
///
/// Runs `git rev-parse --show-toplevel` to confirm `cwd` is inside a working
/// tree, then resolves the primary working tree via `main_worktree`.
/// Anchoring on the main working tree means `rdm worktree` invoked from inside a
/// *linked* worktree still places new worktrees as siblings of the primary
/// checkout rather than nesting them under the current worktree.
///
/// When the canonical repo is **bare** (no working tree of its own),
/// `--show-toplevel` fails; this falls back to `--is-bare-repository` +
/// `--absolute-git-dir` and anchors on the bare git dir, so worktree operations
/// work both from a linked worktree of a bare repo and from inside the bare dir
/// itself.
///
/// # Errors
///
/// Returns [`WorktreeError::NotAGitRepo`] if `cwd` is not inside a git
/// repository (and is not a bare repository), [`WorktreeError::GitMissing`] if
/// `git` is not installed, [`WorktreeError::NoSuchDirectory`] if `cwd` does not
/// exist, or [`WorktreeError::Git`] if the `git` process cannot be spawned.
pub fn discover_project_repo(cwd: &Path) -> Result<PathBuf> {
    let output = run_git_at(cwd, &["rev-parse", "--show-toplevel"])?;
    if output.status.success() {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path.is_empty() {
            // From a linked worktree, `--show-toplevel` is that worktree's own
            // root; anchor on the main working tree instead, falling back to the
            // toplevel. For a bare-canonical repo the first `git worktree list`
            // entry is the bare dir itself — exactly the dir we run git in.
            return Ok(main_worktree(cwd).unwrap_or_else(|| PathBuf::from(path)));
        }
    }
    // `--show-toplevel` fails inside a *bare* repository (no working tree).
    // Detect that case and anchor on the bare git dir so worktree ops still work
    // when the canonical repo is bare.
    let is_bare = run_git_at(cwd, &["rev-parse", "--is-bare-repository"])?;
    if is_bare.status.success() && String::from_utf8_lossy(&is_bare.stdout).trim() == "true" {
        let git_dir = run_git_at(cwd, &["rev-parse", "--absolute-git-dir"])?;
        if git_dir.status.success() {
            let dir = String::from_utf8_lossy(&git_dir.stdout).trim().to_string();
            if !dir.is_empty() {
                return Ok(PathBuf::from(dir));
            }
        }
    }
    Err(WorktreeError::NotAGitRepo(cwd.to_path_buf()))
}

/// Returns the path of the repository's main (primary) working tree, as listed
/// first by `git worktree list --porcelain`.
///
/// Returns `None` on any failure so callers can fall back to the current
/// top-level directory.
fn main_worktree(cwd: &Path) -> Option<PathBuf> {
    let output = run_git_at(cwd, &["worktree", "list", "--porcelain"]).ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Git always lists the primary working tree first.
    stdout
        .lines()
        .find_map(|line| line.strip_prefix("worktree "))
        .map(|rest| PathBuf::from(rest.trim()))
}

/// Computes the sibling worktree path for `item` relative to `repo_root`.
///
/// `<parent-of-repo>/<repo-dir-name>__worktrees/<branch-with-slashes-as-dashes>`.
///
/// A bare canonical repo's directory usually carries a `.git`/`.bare` suffix (or
/// is literally `.bare`); the suffix is stripped so siblings are named after the
/// project, not the git dir. If stripping empties the name (the `<repo>/.bare`
/// layout), the anchor falls back to the parent directory's name.
///
/// The `unwrap_or` fallbacks guard the degenerate case of a repo at the
/// filesystem root (no parent / no file name); they never trigger for a real
/// project checkout.
fn worktree_path(repo_root: &Path, item: &ItemRef) -> PathBuf {
    let raw_name = repo_root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "repo".to_string());
    let stem = raw_name
        .strip_suffix(".git")
        .or_else(|| raw_name.strip_suffix(".bare"))
        .unwrap_or(raw_name.as_str());
    let repo_name = if stem.is_empty() {
        repo_root
            .parent()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "repo".to_string())
    } else {
        stem.to_string()
    };
    let parent = repo_root.parent().unwrap_or(repo_root);
    parent
        .join(format!("{repo_name}__worktrees"))
        .join(item.dir_name())
}

/// Adds (or reuses) a worktree for `item` on `branch`, based on `base`.
///
/// Idempotent: if a marker-matched worktree for `item` already exists, returns
/// it with `created: false` instead of erroring. If `branch` already exists
/// without a worktree, it is reused; otherwise it is created from `base`
/// (defaulting to current `HEAD`).
///
/// # Errors
///
/// Returns [`WorktreeError::Git`] if any git command fails or the marker
/// cannot be written, [`WorktreeError::GitMissing`] if `git` is not installed,
/// or [`WorktreeError::NoSuchDirectory`] if `repo_root` does not exist (or is
/// not a directory). Also propagates errors from the [`list`] call used for the
/// idempotency check.
pub fn add(
    repo_root: &Path,
    item: &ItemRef,
    branch: &str,
    base: Option<&str>,
) -> Result<WorktreeInfo> {
    // Idempotency: an existing rdm worktree for this item short-circuits.
    let canonical = item.canonical();
    for existing in list(repo_root)? {
        if existing.item == canonical {
            return Ok(WorktreeInfo {
                created: false,
                ..existing
            });
        }
    }

    let path = worktree_path(repo_root, item);
    let path_str = path.to_string_lossy().to_string();

    let branch_exists = run_git_at(
        repo_root,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ],
    )
    .map(|o| o.status.success())
    .unwrap_or(false);

    let output = if branch_exists {
        // Reuse the existing branch (e.g. after a manual `git worktree remove`).
        run_git_at(repo_root, &["worktree", "add", &path_str, branch])?
    } else {
        let base = base.unwrap_or("HEAD");
        run_git_at(
            repo_root,
            &["worktree", "add", "-b", branch, &path_str, base],
        )?
    };
    if !output.status.success() {
        return Err(WorktreeError::Git(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }

    write_marker(&path, &canonical, branch)?;

    Ok(WorktreeInfo {
        item: canonical,
        branch: branch.to_string(),
        path,
        created: true,
        dirty: false,
    })
}

/// Lists all rdm-managed worktrees of the project repo at `repo_root`.
///
/// Only worktrees carrying an `rdm-item` marker are returned. The `dirty` flag
/// reflects whether each worktree has uncommitted changes.
///
/// # Errors
///
/// Returns [`WorktreeError::Git`] if `git worktree list` fails,
/// [`WorktreeError::GitMissing`] if `git` is not installed, or
/// [`WorktreeError::NoSuchDirectory`] if `repo_root` does not exist (or is not a
/// directory).
pub fn list(repo_root: &Path) -> Result<Vec<WorktreeInfo>> {
    let output = run_git_at(repo_root, &["worktree", "list", "--porcelain"])?;
    if !output.status.success() {
        return Err(WorktreeError::Git(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);

    let mut infos = Vec::new();
    for block in stdout.split("\n\n") {
        let mut wt_path: Option<PathBuf> = None;
        for line in block.lines() {
            if let Some(rest) = line.strip_prefix("worktree ") {
                wt_path = Some(PathBuf::from(rest.trim()));
            }
        }
        let Some(wt_path) = wt_path else { continue };
        let Some((item, branch)) = read_marker(&wt_path) else {
            continue;
        };
        let dirty = is_dirty(&wt_path);
        infos.push(WorktreeInfo {
            item,
            branch,
            path: wt_path,
            created: false,
            dirty,
        });
    }
    Ok(infos)
}

/// Reports the plan item the checkout containing `cwd` corresponds to, if any.
///
/// Detection prefers the `rdm-item` marker written by [`add`]; failing that, it
/// inverts the checked-out branch name via [`ItemRef::from_branch`]
/// (`phase/<roadmap>/<stem>`, `task/<slug>`, or `roadmap/<slug>`) so a hand-made
/// worktree — or the main checkout sitting on an item branch — is still
/// recognized. Returns
/// `Ok(None)` when the checkout carries no marker and is not on an item branch
/// (e.g. the main checkout on `main`).
///
/// Read-only: never mutates the repository. This is the detection primitive the
/// `rdm-do` skill uses to decide whether to reuse the current worktree or create
/// a new one.
///
/// # Errors
///
/// Returns [`WorktreeError::NotAGitRepo`] if `cwd` is not inside a git working
/// tree, [`WorktreeError::GitMissing`] if `git` is not installed,
/// [`WorktreeError::NoSuchDirectory`] if `cwd` does not exist (or is not a
/// directory), or [`WorktreeError::Git`] if a git command fails.
pub fn current(cwd: &Path) -> Result<Option<CurrentWorktree>> {
    // NOTE: unlike `discover_project_repo`, `current` is intentionally NOT
    // bare-repo aware — it reports the checkout `cwd` sits in, so from inside a
    // bare dir (no working tree) it returns `NotAGitRepo`. That case is out of
    // this phase's AC scope (add/list/remove only); from a linked worktree it
    // works. Tracked as a follow-up task.
    let output = run_git_at(cwd, &["rev-parse", "--show-toplevel"])?;
    if !output.status.success() {
        return Err(WorktreeError::NotAGitRepo(cwd.to_path_buf()));
    }
    let toplevel = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if toplevel.is_empty() {
        return Err(WorktreeError::NotAGitRepo(cwd.to_path_buf()));
    }
    let toplevel = PathBuf::from(toplevel);

    let branch =
        crate::current_branch_at(&toplevel).map_err(|e| WorktreeError::Git(e.to_string()))?;

    // Prefer the rdm marker (an rdm-created worktree).
    if let Some((item, marker_branch)) = read_marker(&toplevel) {
        return Ok(Some(CurrentWorktree {
            item,
            branch: branch.unwrap_or(marker_branch),
            path: toplevel,
            rdm_managed: true,
        }));
    }

    // Fall back to inverting the branch-name convention.
    if let Some(branch) = branch
        && let Some(item) = ItemRef::from_branch(&branch)
    {
        return Ok(Some(CurrentWorktree {
            item: item.canonical(),
            branch,
            path: toplevel,
            rdm_managed: false,
        }));
    }

    Ok(None)
}

/// Removes the rdm worktree identified by `target` (a canonical item ref or a
/// path).
///
/// Refuses a dirty worktree unless `opts.force`. When `opts.delete_branch` is
/// set, deletes the branch afterward (`git branch -d`, escalating to `-D` when
/// `opts.force`).
///
/// # Errors
///
/// Returns [`WorktreeError::NotFound`] if no rdm worktree matches `target`,
/// [`WorktreeError::NotRdmWorktree`] if `target` resolves to a worktree without
/// a marker, [`WorktreeError::Dirty`] if the worktree is dirty and `force` is
/// not set, [`WorktreeError::UnmergedBranch`] if the branch is unmerged and
/// `force` is not set, [`WorktreeError::Git`] on a git failure, or
/// [`WorktreeError::NoSuchDirectory`] if `repo_root` does not exist (or is not a
/// directory).
pub fn remove(repo_root: &Path, target: &str, opts: RemoveOptions) -> Result<()> {
    let worktrees = list(repo_root)?;
    let target_path = Path::new(target);
    // Canonicalize the target once so a user-supplied path matches git's
    // reported worktree path even when they differ only by symlinks (e.g. the
    // path printed by `add` vs. the `/private`-resolved path on macOS).
    let target_canon = target_path.canonicalize().ok();
    let info = worktrees.iter().find(|w| {
        w.item == target
            || w.path == target_path
            || w.path.ends_with(target)
            || target_canon
                .as_deref()
                .zip(w.path.canonicalize().ok())
                .is_some_and(|(tc, wc)| tc == wc)
    });

    let info = match info {
        Some(i) => i,
        None => {
            // Distinguish "a worktree at this path but not ours" from "unknown".
            if target_path.exists() && target_path.join(".git").exists() {
                return Err(WorktreeError::NotRdmWorktree(target_path.to_path_buf()));
            }
            return Err(WorktreeError::NotFound(target.to_string()));
        }
    };

    remove_info(repo_root, info, opts)
}

/// Removes an already-resolved worktree, given its [`WorktreeInfo`].
///
/// This is the removal primitive shared by [`remove`] (which resolves a target
/// string to a `WorktreeInfo` first) and [`prune`] (which passes each candidate
/// straight from its single [`list`] pass, avoiding an O(n²) re-scan). It
/// re-checks dirtiness against `info.path` at removal time rather than trusting
/// the possibly-stale `info.dirty` from an earlier scan, narrowing the
/// time-of-check/time-of-use window and letting callers categorize a worktree
/// that went dirty since the scan as dirty rather than as an opaque failure.
///
/// Refuses a dirty worktree unless `opts.force`. When `opts.delete_branch` is
/// set, deletes the branch afterward (`git branch -d`, escalating to `-D` when
/// `opts.force`).
///
/// # Errors
///
/// Returns [`WorktreeError::Dirty`] if the worktree is dirty and `force` is not
/// set, [`WorktreeError::UnmergedBranch`] if the branch is unmerged and `force`
/// is not set (raised only *after* the worktree itself was successfully
/// removed — a partial success), or [`WorktreeError::Git`] on a git failure.
fn remove_info(repo_root: &Path, info: &WorktreeInfo, opts: RemoveOptions) -> Result<()> {
    if is_dirty(&info.path) && !opts.force {
        return Err(WorktreeError::Dirty(info.path.clone()));
    }

    let path_str = info.path.to_string_lossy().to_string();
    let mut args = vec!["worktree", "remove"];
    if opts.force {
        args.push("--force");
    }
    args.push(&path_str);
    let output = run_git_at(repo_root, &args)?;
    if !output.status.success() {
        return Err(WorktreeError::Git(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }

    if opts.delete_branch {
        let flag = if opts.force { "-D" } else { "-d" };
        let output = run_git_at(repo_root, &["branch", flag, &info.branch])?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("not fully merged") {
                return Err(WorktreeError::UnmergedBranch(info.branch.clone()));
            }
            return Err(WorktreeError::Git(stderr.trim().to_string()));
        }
    }

    Ok(())
}

/// Resolves the private admin directory for the worktree at `wt_path`.
///
/// Linked worktrees have a `.git` *file* containing `gitdir: <admin-dir>`. The
/// main worktree has a `.git` *directory* and is never rdm-managed, so this
/// returns `None` for it.
fn admin_dir(wt_path: &Path) -> Option<PathBuf> {
    let dot_git = wt_path.join(".git");
    let contents = std::fs::read_to_string(&dot_git).ok()?;
    let line = contents.lines().next()?;
    let path = line.strip_prefix("gitdir:")?.trim();
    Some(PathBuf::from(path))
}

/// Writes the `rdm-item` marker into the worktree's admin directory.
fn write_marker(wt_path: &Path, item: &str, branch: &str) -> Result<()> {
    let Some(admin) = admin_dir(wt_path) else {
        return Err(WorktreeError::Git(format!(
            "could not locate admin directory for worktree {}",
            wt_path.display()
        )));
    };
    let marker = admin.join("rdm-item");
    std::fs::write(&marker, format!("{item}\n{branch}\n"))
        .map_err(|e| WorktreeError::Git(format!("failed to write marker: {e}")))?;
    Ok(())
}

/// Reads the `rdm-item` marker for the worktree at `wt_path`, returning
/// `(item, branch)` when present.
fn read_marker(wt_path: &Path) -> Option<(String, String)> {
    let admin = admin_dir(wt_path)?;
    let contents = std::fs::read_to_string(admin.join("rdm-item")).ok()?;
    let mut lines = contents.lines();
    let item = lines.next()?.trim().to_string();
    let branch = lines.next().unwrap_or("").trim().to_string();
    if item.is_empty() {
        return None;
    }
    Some((item, branch))
}

/// Returns `true` if the worktree at `path` has uncommitted changes.
///
/// Fails open: if `git status` cannot be run the worktree is reported clean.
/// This only relaxes the rdm-level [`remove`] guard, never data safety — the
/// underlying `git worktree remove` (invoked without `--force` unless the
/// caller forces) refuses a dirty worktree on its own.
fn is_dirty(path: &Path) -> bool {
    match run_git_at(path, &["status", "--porcelain"]) {
        Ok(output) => !output.stdout.is_empty(),
        Err(_) => false,
    }
}

/// Runs a git command in `path`, mapping a missing `git` binary to
/// [`WorktreeError::GitMissing`] and a missing/invalid working directory to
/// [`WorktreeError::NoSuchDirectory`].
fn run_git_at(path: &Path, args: &[&str]) -> Result<Output> {
    match crate::process::git_command(Some(path), args) {
        Ok(o) => Ok(o),
        Err(e) => {
            // A spawn failure against an invalid working directory is reported
            // differently per platform, so disambiguate by inspecting the path
            // itself rather than the errno. A nonexistent cwd is ENOENT
            // (`NotFound`) everywhere; a cwd that is a *file* surfaces as
            // `NotFound` on macOS but `NotADirectory` (errno 20) on Linux. In
            // either case `is_dir` is false, so an invalid cwd is always
            // reported actionably rather than as a missing git install.
            // (`exists()` is insufficient — a file cwd exists but is not a dir.)
            if !path.is_dir() {
                Err(WorktreeError::NoSuchDirectory(path.to_path_buf()))
            } else if e.kind() == std::io::ErrorKind::NotFound {
                // Real directory, but `git` itself could not be spawned.
                Err(WorktreeError::GitMissing)
            } else {
                Err(WorktreeError::Git(format!("failed to run git: {e}")))
            }
        }
    }
}

// ---------- Plan-repo canonicalization glue ----------
//
// These helpers bridge a raw item reference (as a user types it) to a validated
// [`ItemRef`] against the plan `store`, and discover the distinct project repo.
// They live here — beside [`ItemRef`] — so both the CLI and the MCP server share
// one implementation rather than duplicating the resolution logic.

/// Canonicalizes an item reference against the plan `store`, validating that it
/// exists before any git mutation. A numeric phase identifier is resolved to its
/// stem.
///
/// # Errors
///
/// Returns [`WorktreeError::NotFound`] if `raw` is not a valid item reference,
/// if the roadmap/phase number cannot be resolved, or if the referenced phase or
/// task does not exist in `project`.
pub fn resolve_item(
    store: &impl rdm_core::store::Store,
    project: &str,
    raw: &str,
) -> Result<ItemRef> {
    let item = ItemRef::parse(raw)?;
    match item {
        ItemRef::Phase { roadmap, stem } => {
            let resolved = rdm_core::ops::phase::resolve_phase_stem(
                store, project, &roadmap, &stem,
            )
            .map_err(|_| {
                WorktreeError::NotFound(format!(
                    "unknown item '{roadmap}/{stem}' — check `rdm phase list`"
                ))
            })?;
            // Confirm the phase actually exists (resolve_phase_stem passes
            // non-numeric stems through unverified).
            rdm_core::io::load_phase(store, project, &roadmap, &resolved).map_err(|_| {
                WorktreeError::NotFound(format!(
                    "phase '{roadmap}/{resolved}' not found — check `rdm phase list`"
                ))
            })?;
            Ok(ItemRef::Phase {
                roadmap,
                stem: resolved,
            })
        }
        ItemRef::Task { slug } => {
            rdm_core::io::load_task(store, project, &slug).map_err(|_| {
                WorktreeError::NotFound(format!("task '{slug}' not found — check `rdm task list`"))
            })?;
            Ok(ItemRef::Task { slug })
        }
        ItemRef::Roadmap { roadmap } => {
            rdm_core::io::load_roadmap(store, project, &roadmap).map_err(|_| {
                WorktreeError::NotFound(format!(
                    "roadmap '{roadmap}' not found — check `rdm roadmap list`"
                ))
            })?;
            Ok(ItemRef::Roadmap { roadmap })
        }
    }
}

/// Best-effort canonicalization of a `remove` target.
///
/// An item reference with a numeric phase identifier is resolved to its stem
/// against the plan `store`; anything that does not parse or resolve as an item
/// (a filesystem path, or an item whose phase no longer exists) is returned
/// verbatim so it can still match by path or stored canonical string.
pub fn resolve_target(store: &impl rdm_core::store::Store, project: &str, raw: &str) -> String {
    let Ok(item) = ItemRef::parse(raw) else {
        return raw.to_string();
    };
    // Only a numeric phase stem needs plan-repo resolution; everything else is
    // already canonical.
    if let ItemRef::Phase { roadmap, stem } = &item
        && stem.parse::<u32>().is_ok()
    {
        if let Ok(resolved) =
            rdm_core::ops::phase::resolve_phase_stem(store, project, roadmap, stem)
        {
            return format!("{roadmap}/{resolved}");
        }
        return raw.to_string();
    }
    item.canonical()
}

/// Discovers the project (code) repo from `cwd` and refuses if it is the plan
/// repo at `plan_root` — the two must be distinct directories.
///
/// Compares canonicalized paths so symlinks (e.g. macOS `/private`) don't mask a
/// match between the project repo and the plan repo.
///
/// # Errors
///
/// Returns [`WorktreeError::IsPlanRepo`] if the discovered repo is the plan repo,
/// or any error propagated from [`discover_project_repo`].
pub fn discover_distinct_project_repo(cwd: &Path, plan_root: &Path) -> Result<PathBuf> {
    let repo = discover_project_repo(cwd)?;
    let repo_canon = repo.canonicalize().unwrap_or_else(|_| repo.clone());
    let root_canon = plan_root
        .canonicalize()
        .unwrap_or_else(|_| plan_root.to_path_buf());
    if repo_canon == root_canon {
        return Err(WorktreeError::IsPlanRepo(repo));
    }
    Ok(repo)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_phase_item() {
        let item = ItemRef::parse("agent-worktree-runs/phase-1-foo").unwrap();
        assert_eq!(
            item,
            ItemRef::Phase {
                roadmap: "agent-worktree-runs".to_string(),
                stem: "phase-1-foo".to_string(),
            }
        );
    }

    #[test]
    fn parse_task_item() {
        let item = ItemRef::parse("task/fix-bug").unwrap();
        assert_eq!(
            item,
            ItemRef::Task {
                slug: "fix-bug".to_string()
            }
        );
    }

    #[test]
    fn parse_task_prefix_case_insensitive() {
        assert_eq!(
            ItemRef::parse("TASK/fix-bug").unwrap(),
            ItemRef::Task {
                slug: "fix-bug".to_string()
            }
        );
    }

    #[test]
    fn parse_phase_number_kept_verbatim() {
        // Numeric stems pass through; the CLI canonicalizes against the plan repo.
        assert_eq!(
            ItemRef::parse("my-roadmap/2").unwrap(),
            ItemRef::Phase {
                roadmap: "my-roadmap".to_string(),
                stem: "2".to_string(),
            }
        );
    }

    #[test]
    fn parse_bare_slug_is_roadmap() {
        // A reference with no `/` keys the whole roadmap (one worktree per
        // roadmap). Existence is validated later by `resolve_item`.
        assert_eq!(
            ItemRef::parse("fix-worktree-review-firing").unwrap(),
            ItemRef::Roadmap {
                roadmap: "fix-worktree-review-firing".to_string(),
            }
        );
    }

    #[test]
    fn parse_rejects_empty_side() {
        assert!(ItemRef::parse("foo/").is_err());
        assert!(ItemRef::parse("/bar").is_err());
    }

    #[test]
    fn parse_rejects_empty_input() {
        // An empty (or whitespace-only) ref is not a roadmap — it errors.
        assert!(matches!(
            ItemRef::parse(""),
            Err(WorktreeError::NotFound(_))
        ));
        assert!(matches!(
            ItemRef::parse("   "),
            Err(WorktreeError::NotFound(_))
        ));
    }

    #[test]
    fn phase_branch_and_dir_names() {
        let item = ItemRef::Phase {
            roadmap: "agent-worktree-runs".to_string(),
            stem: "phase-1-worktree-lifecycle".to_string(),
        };
        assert_eq!(
            item.branch_name(),
            "phase/agent-worktree-runs/phase-1-worktree-lifecycle"
        );
        assert_eq!(
            item.dir_name(),
            "phase-agent-worktree-runs-phase-1-worktree-lifecycle"
        );
        assert_eq!(
            item.canonical(),
            "agent-worktree-runs/phase-1-worktree-lifecycle"
        );
    }

    #[test]
    fn task_branch_and_dir_names() {
        let item = ItemRef::Task {
            slug: "fix-bug".to_string(),
        };
        assert_eq!(item.branch_name(), "task/fix-bug");
        assert_eq!(item.dir_name(), "task-fix-bug");
        assert_eq!(item.canonical(), "task/fix-bug");
    }

    #[test]
    fn roadmap_branch_and_dir_names() {
        let item = ItemRef::Roadmap {
            roadmap: "fix-worktree-review-firing".to_string(),
        };
        assert_eq!(item.branch_name(), "roadmap/fix-worktree-review-firing");
        assert_eq!(item.dir_name(), "roadmap-fix-worktree-review-firing");
        assert_eq!(item.canonical(), "fix-worktree-review-firing");
    }

    #[test]
    fn from_branch_inverts_phase_and_task() {
        assert_eq!(
            ItemRef::from_branch("phase/agent-worktree-runs/phase-1-foo"),
            Some(ItemRef::Phase {
                roadmap: "agent-worktree-runs".to_string(),
                stem: "phase-1-foo".to_string(),
            })
        );
        assert_eq!(
            ItemRef::from_branch("task/fix-bug"),
            Some(ItemRef::Task {
                slug: "fix-bug".to_string()
            })
        );
    }

    #[test]
    fn from_branch_roundtrips_branch_name() {
        for item in [
            ItemRef::Phase {
                roadmap: "rm".to_string(),
                stem: "phase-2-x".to_string(),
            },
            ItemRef::Task {
                slug: "do-thing".to_string(),
            },
        ] {
            assert_eq!(ItemRef::from_branch(&item.branch_name()), Some(item));
        }
    }

    #[test]
    fn from_branch_inverts_roadmap() {
        assert_eq!(
            ItemRef::from_branch("roadmap/fix-worktree-review-firing"),
            Some(ItemRef::Roadmap {
                roadmap: "fix-worktree-review-firing".to_string(),
            })
        );
    }

    #[test]
    fn from_branch_roadmap_roundtrips() {
        let item = ItemRef::Roadmap {
            roadmap: "my-roadmap".to_string(),
        };
        assert_eq!(ItemRef::from_branch(&item.branch_name()), Some(item));
    }

    #[test]
    fn from_branch_rejects_non_convention() {
        assert_eq!(ItemRef::from_branch("main"), None);
        assert_eq!(ItemRef::from_branch("feature/foo"), None);
        // `phase/` with no stem segment.
        assert_eq!(ItemRef::from_branch("phase/only-two"), None);
        assert_eq!(ItemRef::from_branch("task/"), None);
        // `roadmap/` with no slug.
        assert_eq!(ItemRef::from_branch("roadmap/"), None);
    }

    #[test]
    fn worktree_path_is_sibling() {
        let item = ItemRef::Task {
            slug: "fix-bug".to_string(),
        };
        let path = worktree_path(Path::new("/home/me/Projects/rdm"), &item);
        assert_eq!(
            path,
            PathBuf::from("/home/me/Projects/rdm__worktrees/task-fix-bug")
        );
    }

    #[test]
    fn worktree_path_strips_git_suffix() {
        let item = ItemRef::Task {
            slug: "fix-bug".to_string(),
        };
        // A bare canonical repo `<name>.git`: the `.git` suffix is stripped so the
        // sibling anchor is named after the project, not the git dir.
        assert_eq!(
            worktree_path(Path::new("/p/canonical.git"), &item),
            PathBuf::from("/p/canonical__worktrees/task-fix-bug")
        );
        // The `<repo>/.bare` layout: stripping `.bare` empties the name, so the
        // anchor falls back to the parent directory's name (`foo`), never empty.
        assert_eq!(
            worktree_path(Path::new("/p/foo/.bare"), &item),
            PathBuf::from("/p/foo/foo__worktrees/task-fix-bug")
        );
    }

    // ---------- Plan-repo canonicalization glue ----------

    /// Builds a plan store fixture with one roadmap (`my-roadmap`) carrying phase
    /// number 2 (`phase-2-do-thing`) and one task (`fix-bug`).
    fn plan_fixture() -> (tempfile::TempDir, rdm_store_fs::FsStore) {
        let dir = tempfile::tempdir().unwrap();
        let mut store = rdm_store_fs::FsStore::new(dir.path());
        rdm_core::ops::init::init(&mut store).unwrap();
        rdm_core::ops::project::create_project(&mut store, "proj", "Proj").unwrap();
        rdm_core::ops::roadmap::create_roadmap(
            &mut store,
            rdm_core::ops::roadmap::CreateRoadmap {
                project: "proj",
                slug: "my-roadmap",
                title: "My Roadmap",
                ..Default::default()
            },
        )
        .unwrap();
        rdm_core::ops::phase::create_phase(
            &mut store,
            rdm_core::ops::phase::CreatePhase {
                project: "proj",
                roadmap: "my-roadmap",
                slug: "do-thing",
                title: "Do Thing",
                number: Some(2),
                ..Default::default()
            },
        )
        .unwrap();
        rdm_core::ops::task::create_task(
            &mut store,
            rdm_core::ops::task::CreateTask {
                project: "proj",
                slug: "fix-bug",
                title: "Fix Bug",
                priority: rdm_core::model::Priority::Medium,
                ..Default::default()
            },
        )
        .unwrap();
        (dir, store)
    }

    #[test]
    fn resolve_item_resolves_phase_number_to_stem() {
        let (_dir, store) = plan_fixture();
        let item = resolve_item(&store, "proj", "my-roadmap/2").unwrap();
        assert_eq!(
            item,
            ItemRef::Phase {
                roadmap: "my-roadmap".to_string(),
                stem: "phase-2-do-thing".to_string(),
            }
        );
    }

    #[test]
    fn resolve_item_validates_roadmap() {
        let (_dir, store) = plan_fixture();
        assert_eq!(
            resolve_item(&store, "proj", "my-roadmap").unwrap(),
            ItemRef::Roadmap {
                roadmap: "my-roadmap".to_string(),
            }
        );
        assert!(matches!(
            resolve_item(&store, "proj", "no-such-roadmap"),
            Err(WorktreeError::NotFound(_))
        ));
    }

    #[test]
    fn resolve_item_unknown_phase_is_not_found() {
        let (_dir, store) = plan_fixture();
        // Numeric identifier with no matching phase.
        assert!(matches!(
            resolve_item(&store, "proj", "my-roadmap/99"),
            Err(WorktreeError::NotFound(_))
        ));
        // Non-numeric stem that does not exist passes resolve_phase_stem but
        // fails the load_phase existence check.
        assert!(matches!(
            resolve_item(&store, "proj", "my-roadmap/phase-9-nope"),
            Err(WorktreeError::NotFound(_))
        ));
    }

    #[test]
    fn resolve_item_validates_task() {
        let (_dir, store) = plan_fixture();
        assert_eq!(
            resolve_item(&store, "proj", "task/fix-bug").unwrap(),
            ItemRef::Task {
                slug: "fix-bug".to_string()
            }
        );
        assert!(matches!(
            resolve_item(&store, "proj", "task/nope"),
            Err(WorktreeError::NotFound(_))
        ));
    }

    #[test]
    fn resolve_target_resolves_phase_number() {
        let (_dir, store) = plan_fixture();
        assert_eq!(
            resolve_target(&store, "proj", "my-roadmap/2"),
            "my-roadmap/phase-2-do-thing"
        );
    }

    #[test]
    fn resolve_target_passes_through_unparseable_and_tasks() {
        let (_dir, store) = plan_fixture();
        // A filesystem path (leading slash → empty roadmap) is returned verbatim.
        assert_eq!(resolve_target(&store, "proj", "/some/path"), "/some/path");
        // A task ref is returned canonically.
        assert_eq!(
            resolve_target(&store, "proj", "task/fix-bug"),
            "task/fix-bug"
        );
        // An unresolvable numeric phase falls back to the raw input verbatim.
        assert_eq!(
            resolve_target(&store, "proj", "my-roadmap/99"),
            "my-roadmap/99"
        );
    }

    // ---------- prune ----------

    fn git_available() -> bool {
        std::process::Command::new("git")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn run_git(dir: &Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t.com")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t.com")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// Builds a plan store with `my-roadmap` carrying a done phase (number 1,
    /// stem `phase-1-done-phase`) and an open phase (number 2, stem
    /// `phase-2-open-phase`), plus a project git repo (nested under a tempdir so
    /// its sibling `__worktrees` dir is cleaned up). Returns
    /// `(plan_dir, repo_root, store, repo_parent)`; keep both tempdirs alive.
    fn prune_fixture() -> (
        tempfile::TempDir,
        PathBuf,
        rdm_store_fs::FsStore,
        tempfile::TempDir,
    ) {
        let plan_dir = tempfile::tempdir().unwrap();
        let mut store = rdm_store_fs::FsStore::new(plan_dir.path());
        rdm_core::ops::init::init(&mut store).unwrap();
        rdm_core::ops::project::create_project(&mut store, "proj", "Proj").unwrap();
        rdm_core::ops::roadmap::create_roadmap(
            &mut store,
            rdm_core::ops::roadmap::CreateRoadmap {
                project: "proj",
                slug: "my-roadmap",
                title: "My Roadmap",
                ..Default::default()
            },
        )
        .unwrap();
        rdm_core::ops::phase::create_phase(
            &mut store,
            rdm_core::ops::phase::CreatePhase {
                project: "proj",
                roadmap: "my-roadmap",
                slug: "done-phase",
                title: "Done Phase",
                number: Some(1),
                ..Default::default()
            },
        )
        .unwrap();
        rdm_core::ops::phase::create_phase(
            &mut store,
            rdm_core::ops::phase::CreatePhase {
                project: "proj",
                roadmap: "my-roadmap",
                slug: "open-phase",
                title: "Open Phase",
                number: Some(2),
                ..Default::default()
            },
        )
        .unwrap();
        rdm_core::ops::phase::update_phase(
            &mut store,
            "proj",
            "my-roadmap",
            "phase-1-done-phase",
            Some(rdm_core::model::PhaseStatus::Done),
            rdm_core::ops::update::TagsUpdate::Keep,
            rdm_core::ops::update::BodyUpdate::Keep,
            None,
            None,
            None,
        )
        .unwrap();

        let repo_parent = tempfile::tempdir().unwrap();
        let repo = repo_parent.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        run_git(&repo, &["init", "-b", "main"]);
        std::fs::write(repo.join("README.md"), "# project").unwrap();
        run_git(&repo, &["add", "."]);
        run_git(&repo, &["commit", "-m", "initial"]);

        (plan_dir, repo, store, repo_parent)
    }

    fn done_item() -> ItemRef {
        ItemRef::Phase {
            roadmap: "my-roadmap".to_string(),
            stem: "phase-1-done-phase".to_string(),
        }
    }

    fn open_item() -> ItemRef {
        ItemRef::Phase {
            roadmap: "my-roadmap".to_string(),
            stem: "phase-2-open-phase".to_string(),
        }
    }

    #[test]
    fn prune_removes_done_keeps_open() {
        if !git_available() {
            return;
        }
        let (_plan, repo, store, _parent) = prune_fixture();
        let done = done_item();
        let open = open_item();
        add(&repo, &done, &done.branch_name(), None).unwrap();
        add(&repo, &open, &open.branch_name(), None).unwrap();

        let results = prune(&repo, &store, "proj", PruneOptions::default()).unwrap();
        // Only the done worktree is a candidate, and it was removed.
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].item, done.canonical());
        assert_eq!(results[0].action, PruneAction::Removed);

        // The open worktree survives; the done one is gone.
        let remaining = list(&repo).unwrap();
        let items: Vec<&str> = remaining.iter().map(|w| w.item.as_str()).collect();
        assert!(items.contains(&open.canonical().as_str()));
        assert!(!items.contains(&done.canonical().as_str()));
    }

    #[test]
    fn prune_skips_dirty_done_without_force() {
        if !git_available() {
            return;
        }
        let (_plan, repo, store, _parent) = prune_fixture();
        let done = done_item();
        let info = add(&repo, &done, &done.branch_name(), None).unwrap();
        std::fs::write(info.path.join("scratch.txt"), "wip").unwrap();

        let results = prune(&repo, &store, "proj", PruneOptions::default()).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].action, PruneAction::SkippedDirty);
        // Still present.
        assert_eq!(list(&repo).unwrap().len(), 1);

        // With --force it is removed.
        let results = prune(
            &repo,
            &store,
            "proj",
            PruneOptions {
                force: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(results[0].action, PruneAction::Removed);
        assert!(list(&repo).unwrap().is_empty());
    }

    #[test]
    fn prune_dry_run_removes_nothing() {
        if !git_available() {
            return;
        }
        let (_plan, repo, store, _parent) = prune_fixture();
        let done = done_item();
        add(&repo, &done, &done.branch_name(), None).unwrap();

        let results = prune(
            &repo,
            &store,
            "proj",
            PruneOptions {
                dry_run: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].action, PruneAction::WouldRemove);
        // Nothing was actually removed.
        assert_eq!(list(&repo).unwrap().len(), 1);
    }

    #[test]
    fn prune_resolves_roadmap_and_task_arms() {
        if !git_available() {
            return;
        }
        // The shipping model is one worktree per roadmap (ItemRef::Roadmap) plus
        // per-task worktrees (ItemRef::Task), so exercise those resolver arms of
        // `item_is_done` end-to-end through `prune` — not just the Phase arm.
        let (_plan, repo, mut store, _parent) = prune_fixture();

        // A second roadmap whose only phase is done → roadmap resolves to Done.
        rdm_core::ops::roadmap::create_roadmap(
            &mut store,
            rdm_core::ops::roadmap::CreateRoadmap {
                project: "proj",
                slug: "done-roadmap",
                title: "Done Roadmap",
                ..Default::default()
            },
        )
        .unwrap();
        rdm_core::ops::phase::create_phase(
            &mut store,
            rdm_core::ops::phase::CreatePhase {
                project: "proj",
                roadmap: "done-roadmap",
                slug: "only",
                title: "Only Phase",
                number: Some(1),
                ..Default::default()
            },
        )
        .unwrap();
        rdm_core::ops::phase::update_phase(
            &mut store,
            "proj",
            "done-roadmap",
            "phase-1-only",
            Some(rdm_core::model::PhaseStatus::Done),
            rdm_core::ops::update::TagsUpdate::Keep,
            rdm_core::ops::update::BodyUpdate::Keep,
            None,
            None,
            None,
        )
        .unwrap();

        // A done task and an open task.
        rdm_core::ops::task::create_task(
            &mut store,
            rdm_core::ops::task::CreateTask {
                project: "proj",
                slug: "done-task",
                title: "Done Task",
                priority: rdm_core::model::Priority::Medium,
                ..Default::default()
            },
        )
        .unwrap();
        rdm_core::ops::task::update_task(
            &mut store,
            "proj",
            "done-task",
            Some(rdm_core::model::TaskStatus::Done),
            None,
            rdm_core::ops::update::TagsUpdate::Keep,
            rdm_core::ops::update::BodyUpdate::Keep,
            None,
            None,
            None,
        )
        .unwrap();
        rdm_core::ops::task::create_task(
            &mut store,
            rdm_core::ops::task::CreateTask {
                project: "proj",
                slug: "open-task",
                title: "Open Task",
                priority: rdm_core::model::Priority::Medium,
                ..Default::default()
            },
        )
        .unwrap();

        // Worktrees: a done & a not-done roadmap, a done & an open task.
        // `my-roadmap` (from the fixture) has one done + one open phase → not done.
        let done_rm = ItemRef::Roadmap {
            roadmap: "done-roadmap".to_string(),
        };
        let open_rm = ItemRef::Roadmap {
            roadmap: "my-roadmap".to_string(),
        };
        let done_tk = ItemRef::Task {
            slug: "done-task".to_string(),
        };
        let open_tk = ItemRef::Task {
            slug: "open-task".to_string(),
        };
        for item in [&done_rm, &open_rm, &done_tk, &open_tk] {
            add(&repo, item, &item.branch_name(), None).unwrap();
        }

        let results = prune(&repo, &store, "proj", PruneOptions::default()).unwrap();
        let removed: Vec<&str> = results
            .iter()
            .filter(|r| r.action == PruneAction::Removed)
            .map(|r| r.item.as_str())
            .collect();
        // Only the done roadmap and done task are candidates, and both removed.
        assert_eq!(removed.len(), 2);
        assert!(removed.contains(&"done-roadmap"));
        assert!(removed.contains(&"task/done-task"));

        // The not-done roadmap and open task survive.
        let remaining = list(&repo).unwrap();
        let items: Vec<&str> = remaining.iter().map(|w| w.item.as_str()).collect();
        assert!(items.contains(&"my-roadmap"));
        assert!(items.contains(&"task/open-task"));
        assert!(!items.contains(&"done-roadmap"));
        assert!(!items.contains(&"task/done-task"));
    }

    #[test]
    fn prune_removes_worktree_keeps_unmerged_branch() {
        if !git_available() {
            return;
        }
        let (_plan, repo, store, _parent) = prune_fixture();
        let done = done_item();
        let info = add(&repo, &done, &done.branch_name(), None).unwrap();

        // Make an unmerged commit on the worktree's branch so `git branch -d`
        // (force off) will refuse it after the worktree is removed.
        std::fs::write(info.path.join("feature.txt"), "work").unwrap();
        run_git(&info.path, &["add", "."]);
        run_git(&info.path, &["commit", "-m", "feature work"]);

        let results = prune(
            &repo,
            &store,
            "proj",
            PruneOptions {
                delete_branch: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(results.len(), 1);
        // Partial success: worktree removed, branch retained (unmerged).
        match &results[0].action {
            PruneAction::RemovedBranchKept { reason } => {
                assert!(!reason.is_empty(), "reason should explain the retention");
            }
            other => panic!("expected RemovedBranchKept, got {other:?}"),
        }

        // The worktree is gone from the list, so a later prune can't re-clean it.
        assert!(list(&repo).unwrap().is_empty());

        // The branch survives — it was not deleted.
        let out = std::process::Command::new("git")
            .args(["branch", "--list", &done.branch_name()])
            .current_dir(&repo)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .output()
            .unwrap();
        assert!(
            !String::from_utf8_lossy(&out.stdout).trim().is_empty(),
            "unmerged branch should be retained"
        );
    }

    #[test]
    fn remove_info_rechecks_dirty_returns_dirty() {
        if !git_available() {
            return;
        }
        let (_plan, repo, _store, _parent) = prune_fixture();
        let done = done_item();
        add(&repo, &done, &done.branch_name(), None).unwrap();

        // Re-resolve the worktree via `list`, then dirty it *after* that scan.
        let info = list(&repo)
            .unwrap()
            .into_iter()
            .find(|w| w.item == done.canonical())
            .unwrap();
        std::fs::write(info.path.join("scratch.txt"), "wip").unwrap();

        // `remove_info` must re-check dirtiness at removal time (not trust the
        // stale `info.dirty` from the scan) and refuse without force.
        let err = remove_info(&repo, &info, RemoveOptions::default()).unwrap_err();
        assert!(matches!(err, WorktreeError::Dirty(_)));
        assert_eq!(list(&repo).unwrap().len(), 1, "worktree must survive");
    }

    #[test]
    fn discover_distinct_project_repo_refuses_plan_repo() {
        let dir = tempfile::tempdir().unwrap();
        // A real git repo so `git rev-parse --show-toplevel` succeeds. Clear the
        // GIT_* env vars (as `run_git_at` does) so `git init` targets the tempdir
        // even when this test runs inside a git hook, which exports GIT_DIR etc.
        let initialized = std::process::Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !initialized {
            return; // git not available — nothing to assert.
        }
        let root = dir.path();
        let err = discover_distinct_project_repo(root, root).unwrap_err();
        assert!(matches!(err, WorktreeError::IsPlanRepo(_)));
    }
}

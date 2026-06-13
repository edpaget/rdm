//! Git worktree lifecycle management for the **project repo**.
//!
//! These helpers operate on the git repository rdm is invoked in (the project
//! or "code" repo), keeping worktrees and branches keyed to plan items. They
//! are deliberately separate from the plan-repo [`GitStore`](crate::GitStore):
//! every function shells out to `git` "at" a path via [`run_git_at`], mirroring
//! the existing project-repo helpers in [`crate`] (`discover_git_dir`,
//! `current_branch_at`, …).
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
}

impl ItemRef {
    /// Parses an item reference of the form `<roadmap>/<phase>` or
    /// `task/<slug>`.
    ///
    /// # Errors
    ///
    /// Returns [`WorktreeError::NotFound`] if the input has no `/` separator or
    /// either side is empty.
    pub fn parse(s: &str) -> Result<ItemRef> {
        let s = s.trim();
        let Some((left, right)) = s.split_once('/') else {
            return Err(WorktreeError::NotFound(format!(
                "'{s}' is not a valid item — use <roadmap>/<phase> or task/<slug>"
            )));
        };
        let left = left.trim();
        let right = right.trim();
        if left.is_empty() || right.is_empty() {
            return Err(WorktreeError::NotFound(format!(
                "'{s}' is not a valid item — use <roadmap>/<phase> or task/<slug>"
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

    /// Returns the canonical item string (`<roadmap>/<stem>` or `task/<slug>`).
    pub fn canonical(&self) -> String {
        match self {
            ItemRef::Phase { roadmap, stem } => format!("{roadmap}/{stem}"),
            ItemRef::Task { slug } => format!("task/{slug}"),
        }
    }

    /// Returns the git branch name for this item.
    ///
    /// Phases map to `phase/<roadmap>/<stem>`; tasks to `task/<slug>`.
    pub fn branch_name(&self) -> String {
        match self {
            ItemRef::Phase { roadmap, stem } => format!("phase/{roadmap}/{stem}"),
            ItemRef::Task { slug } => format!("task/{slug}"),
        }
    }

    /// Returns the worktree directory name (the branch with `/` → `-`).
    pub fn dir_name(&self) -> String {
        self.branch_name().replace('/', "-")
    }
}

/// Information about an rdm-managed worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeInfo {
    /// The canonical item reference (`<roadmap>/<stem>` or `task/<slug>`).
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

/// Options for [`remove`].
#[derive(Debug, Clone, Copy, Default)]
pub struct RemoveOptions {
    /// Remove even if the worktree is dirty (and force-delete the branch).
    pub force: bool,
    /// Also delete the worktree's branch after removal.
    pub delete_branch: bool,
}

/// Discovers the project repo's top-level directory from `cwd`.
///
/// Runs `git rev-parse --show-toplevel`.
///
/// # Errors
///
/// Returns [`WorktreeError::NotAGitRepo`] if `cwd` is not inside a git
/// repository, [`WorktreeError::GitMissing`] if `git` is not installed, or
/// [`WorktreeError::Git`] if the `git` process cannot be spawned.
pub fn discover_project_repo(cwd: &Path) -> Result<PathBuf> {
    let output = run_git_at(cwd, &["rev-parse", "--show-toplevel"])?;
    if !output.status.success() {
        return Err(WorktreeError::NotAGitRepo(cwd.to_path_buf()));
    }
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if path.is_empty() {
        return Err(WorktreeError::NotAGitRepo(cwd.to_path_buf()));
    }
    Ok(PathBuf::from(path))
}

/// Computes the sibling worktree path for `item` relative to `repo_root`.
///
/// `<parent-of-repo>/<repo-dir-name>__worktrees/<branch-with-slashes-as-dashes>`.
///
/// The `unwrap_or` fallbacks guard the degenerate case of a repo at the
/// filesystem root (no parent / no file name); they never trigger for a real
/// project checkout.
fn worktree_path(repo_root: &Path, item: &ItemRef) -> PathBuf {
    let repo_name = repo_root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "repo".to_string());
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
/// cannot be written, or [`WorktreeError::GitMissing`] if `git` is not
/// installed. Also propagates errors from the [`list`] call used for the
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
/// Returns [`WorktreeError::Git`] if `git worktree list` fails, or
/// [`WorktreeError::GitMissing`] if `git` is not installed.
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
/// `force` is not set, or [`WorktreeError::Git`] on a git failure.
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

    if info.dirty && !opts.force {
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
/// [`WorktreeError::GitMissing`].
fn run_git_at(path: &Path, args: &[&str]) -> Result<Output> {
    match std::process::Command::new("git")
        .args(args)
        .current_dir(path)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
    {
        Ok(o) => Ok(o),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(WorktreeError::GitMissing),
        Err(e) => Err(WorktreeError::Git(format!("failed to run git: {e}"))),
    }
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
    fn parse_rejects_no_slash() {
        assert!(matches!(
            ItemRef::parse("no-slash"),
            Err(WorktreeError::NotFound(_))
        ));
    }

    #[test]
    fn parse_rejects_empty_side() {
        assert!(ItemRef::parse("foo/").is_err());
        assert!(ItemRef::parse("/bar").is_err());
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
}

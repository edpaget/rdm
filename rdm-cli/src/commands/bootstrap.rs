use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::paths;

/// Runs the `rdm bootstrap` command.
///
/// Clones the plan repo at `plan_repo_url` into `target` (resolved from
/// `path_override` or the default XDG data dir), or fast-forwards it if the
/// clone already exists. Validates the result is a plan repo, or runs
/// `rdm init` on an otherwise-empty remote if `init_if_empty` is set.
pub fn run(
    plan_repo_url: &str,
    path_override: Option<PathBuf>,
    branch: Option<String>,
    init_if_empty: bool,
) -> Result<()> {
    let target = resolve_target_path(path_override)?;
    let state = classify_target(&target)?;

    let final_path = match state {
        TargetState::Empty => {
            clone_fresh(plan_repo_url, &target, branch.as_deref(), init_if_empty)?
        }
        TargetState::PlanRepo => update_existing(&target)?,
        TargetState::NonPlanGitRepo => bail!(
            "target '{}' is a git repo but is not an rdm plan repo (no rdm.toml). \
             Pick a different --path, or delete this directory and re-run.",
            target.display()
        ),
        TargetState::NonEmptyNonGit => bail!(
            "target '{}' exists and is not empty. \
             Pick a different --path, or delete this directory and re-run.",
            target.display()
        ),
    };

    print_success(&final_path);
    Ok(())
}

/// What we found at the target path before acting.
enum TargetState {
    /// Missing or empty — safe to clone.
    Empty,
    /// Existing git working tree with an `rdm.toml`.
    PlanRepo,
    /// Existing git working tree with no `rdm.toml`.
    NonPlanGitRepo,
    /// Exists, non-empty, no `.git` — unsafe to touch.
    NonEmptyNonGit,
}

/// Classifies the current state of the target directory.
fn classify_target(target: &Path) -> Result<TargetState> {
    if !target.exists() {
        return Ok(TargetState::Empty);
    }
    let has_entries = std::fs::read_dir(target)
        .with_context(|| format!("failed to read {}", target.display()))?
        .next()
        .is_some();
    if !has_entries {
        return Ok(TargetState::Empty);
    }
    if target.join(".git").exists() {
        if target.join("rdm.toml").exists() {
            Ok(TargetState::PlanRepo)
        } else {
            Ok(TargetState::NonPlanGitRepo)
        }
    } else {
        Ok(TargetState::NonEmptyNonGit)
    }
}

/// Resolves `--path` or falls back to `$XDG_DATA_HOME/rdm/plan-repo`.
fn resolve_target_path(path_override: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(p) = path_override {
        return Ok(p);
    }
    let data_dir = paths::default_data_dir()
        .context("cannot determine default plan-repo path — set --path or ensure $HOME is set")?;
    Ok(data_dir.join("plan-repo"))
}

/// Clones into an empty target, then validates or initializes.
fn clone_fresh(
    url: &str,
    target: &Path,
    branch: Option<&str>,
    init_if_empty: bool,
) -> Result<PathBuf> {
    println!("Cloning {url} → {}", target.display());
    drop(
        rdm_store_git::GitStore::clone_remote(url, target, branch)
            .context("failed to clone plan repo")?,
    );

    if target.join("rdm.toml").exists() {
        return Ok(target.to_path_buf());
    }

    if !init_if_empty {
        bail!(
            "cloned repo has no rdm.toml — '{url}' is not a plan repo. \
             Re-run with --init to initialize an empty remote as a plan repo."
        );
    }

    let mut store = rdm_store_git::GitStore::new(target).context("failed to open cloned repo")?;
    rdm_core::ops::init::init_with_config(&mut store, rdm_core::config::Config::default())
        .context("failed to initialize cloned repo as a plan repo")?;
    store
        .git_commit("rdm: initialize plan repo via bootstrap --init")
        .context("failed to commit initial plan repo state")?;
    Ok(target.to_path_buf())
}

/// Fast-forwards an existing plan-repo clone.
fn update_existing(target: &Path) -> Result<PathBuf> {
    let mut store =
        rdm_store_git::GitStore::new(target).context("failed to open existing plan repo")?;

    let remotes = store.git_remote_list().context("failed to list remotes")?;
    let remote_name = remotes
        .iter()
        .find(|r| r.name == "origin")
        .map(|r| r.name.clone())
        .or_else(|| remotes.first().map(|r| r.name.clone()));
    let Some(remote_name) = remote_name else {
        // No remote configured — nothing to update. Treat as success.
        println!(
            "Plan repo already present at {} (no remote configured, skipping fetch).",
            target.display()
        );
        return Ok(target.to_path_buf());
    };

    println!(
        "Plan repo already at {}; fetching {remote_name}…",
        target.display()
    );
    match store
        .git_pull(&remote_name)
        .context("failed to fast-forward plan repo")?
    {
        rdm_store_git::PullOutcome::Success(result) => {
            if result.changed {
                println!(
                    "Fast-forwarded {} commit(s) from {}/{}.",
                    result.commits_merged, result.remote, result.branch
                );
            } else {
                println!("Already up to date.");
            }
        }
        rdm_store_git::PullOutcome::Conflict(conflict) => {
            bail!(
                "merge conflict while updating plan repo ({} file(s) need resolution). \
                 Resolve conflicts in {} manually before re-running.",
                conflict.conflicted_files.len(),
                target.display()
            );
        }
    }
    Ok(target.to_path_buf())
}

/// Prints the post-bootstrap success banner.
fn print_success(path: &Path) {
    println!();
    println!("Plan repo ready at {}", path.display());
    println!("  export RDM_ROOT=\"{}\"", path.display());
}

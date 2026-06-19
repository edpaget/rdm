//! Single git-subprocess spawner shared by every git call site in the crate.
//!
//! All process spawning routes through [`git_command`], which builds the
//! `git` command and scrubs the ambient `GIT_DIR`/`GIT_WORK_TREE`/
//! `GIT_INDEX_FILE` environment variables once. The two error domains in this
//! crate ([`rdm_core::error::Error`](rdm_core::error::Error) for the store and
//! [`WorktreeError`](crate::worktree::WorktreeError) for worktrees) each map
//! the raw [`std::io::Result`] themselves, so behavior is identical to the
//! previously duplicated spawners.

use std::path::Path;
use std::process::Output;

/// Spawns `git` with `args`, optionally in `cwd`, returning the raw output.
///
/// The ambient `GIT_DIR`, `GIT_WORK_TREE`, and `GIT_INDEX_FILE` environment
/// variables are removed so the command operates on the intended repository
/// rather than one inherited from the caller's environment.
pub(crate) fn git_command(cwd: Option<&Path>, args: &[&str]) -> std::io::Result<Output> {
    let mut cmd = std::process::Command::new("git");
    cmd.args(args)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE");
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    cmd.output()
}

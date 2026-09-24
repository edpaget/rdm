//! Shared, isolated `git` test helper for `rdm-git`'s own unit tests
//! (`source.rs`, `worktree.rs`) and its integration test
//! (`tests/worktree.rs`, which pulls this file in verbatim via `#[path]`
//! since a `cfg(test)` module in the library crate is invisible to a
//! separate integration-test binary).
//!
//! # Why ambient global/system git config is isolated by default
//!
//! Every fixture in this crate spawns real `git` subprocesses. Left alone,
//! those subprocesses read the *invoking developer's* `~/.gitconfig` and
//! `/etc/gitconfig` — settings that were never meant to influence a test and
//! that vary machine to machine. That is not hypothetical: `2c55784` fixed a
//! path-handling regression (`unified_diff_argv` returning an empty hunk set
//! when `GitSourceRepo` is rooted at a subdirectory of the checkout) that
//! passed every existing test locally yet failed for a real user, because
//! their `~/.gitconfig` carried `diff.relative = true` — "a real config
//! users set", in that commit's own words. A test suite that never sets
//! `diff.relative` at all would have caught the original bug but could still
//! pass while regressing the fix, since it would never actually exercise the
//! hostile setting.
//!
//! [`git`] closes that gap the other direction: it points `GIT_CONFIG_GLOBAL`
//! and `GIT_CONFIG_SYSTEM` at `/dev/null`, so no fixture can *accidentally*
//! inherit a real global/system config, hostile or otherwise. Reading
//! `/dev/null` as a config file yields a valid, empty config — not an error —
//! so this is silent by design. Only `--local` (the repository's own
//! `.git/config`) is left untouched, since that is the layer a fixture is
//! expected to set up itself (e.g. via a plain `git config <key> <value>`
//! run through [`git`]).
//!
//! # Deliberately injecting a hostile setting
//!
//! A scenario that wants to prove a guard survives a setting sourced from the
//! *global* config layer specifically (rather than the local one) uses
//! [`write_global_config`] to create a scratch config file inside its own
//! `TempDir` — never a real `~/.gitconfig` or `/etc/gitconfig`, both of which
//! this module never opens, let alone writes — and [`git_with_global`] to run
//! a command with `GIT_CONFIG_GLOBAL` pointed at that file instead of
//! `/dev/null`. See `rdm-git/src/source.rs`'s
//! `unified_diff_finds_hunks_from_a_subdirectory_of_the_checkout` test for a
//! worked example that exercises the `2c55784` regression through both the
//! local and the global config layer.
//!
//! # Why source repos are nested inside their `TempDir`
//!
//! `worktree::add` places a new worktree at
//! `repo_root.parent()/<repo-name>__worktrees/<item>` — a **sibling** of the
//! repo root, matching real usage (a worktree for `~/Projects/rdm` belongs at
//! `~/Projects/rdm__worktrees`, not nested inside it). If a fixture's repo
//! root *were* the bare `TempDir` itself, that sibling directory would land
//! in the system temp directory, one level above anything `TempDir::drop`
//! cleans up — leaking a populated git worktree on every run. `c647ab0` fixed
//! this by rooting every repo one level down (`dir.path().join("repo")`), so
//! the sibling lands inside the `TempDir` and is swept away with it; this
//! module does not change that shape, it only isolates the `git` calls those
//! fixtures already made. See `rdm-cli/tests/suite_hygiene/`
//! (`temp_hygiene` and `git_config`, run by `cargo nextest run --profile
//! suite-hygiene`) for the regression tests that pin both.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Runs `git` in `dir`, asserting success and returning the raw [`Output`].
///
/// Isolated the same way [`crate::process::git_command`] hardens rdm's own
/// production git subprocesses against interactive prompts, plus:
///
/// - `GIT_DIR`/`GIT_WORK_TREE`/`GIT_INDEX_FILE` cleared, so the command
///   targets `dir` even when the test runs inside a git hook (which exports
///   these pointing at the invoking repository).
/// - A fixed author/committer identity, so a commit succeeds even when the
///   invoking environment has none configured.
/// - `GIT_CONFIG_GLOBAL`/`GIT_CONFIG_SYSTEM` redirected to `/dev/null`, so
///   the developer's real `~/.gitconfig`/`/etc/gitconfig` can never reach the
///   child process — see the module doc for why.
///
/// Use [`git_with_global`] instead when a scenario deliberately wants a
/// specific setting to reach git through the global config layer.
///
/// # Panics
///
/// Panics (via `assert!`) if the command exits non-zero, printing the
/// argv and stderr.
pub fn git(dir: &Path, args: &[&str]) -> Output {
    run(dir, args, None)
}

/// Like [`git`], but overrides `GIT_CONFIG_GLOBAL` with `global_config_path`
/// instead of `/dev/null` — for a scenario that deliberately wants a
/// hostile (or otherwise non-default) setting to reach git through the
/// GLOBAL config layer, the same layer a real `~/.gitconfig` occupies. Pair
/// with [`write_global_config`] to create that file without ever touching a
/// real global config.
///
/// `GIT_CONFIG_SYSTEM` stays isolated to `/dev/null`, and `--local` (the
/// repository's own config) is unaffected either way.
///
/// # Panics
///
/// Panics (via `assert!`) if the command exits non-zero, printing the
/// argv and stderr.
pub fn git_with_global(dir: &Path, args: &[&str], global_config_path: &Path) -> Output {
    run(dir, args, Some(global_config_path))
}

fn run(dir: &Path, args: &[&str], global_override: Option<&Path>) -> Output {
    let mut cmd = Command::new("git");
    cmd.args(args)
        .current_dir(dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@test.com")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@test.com")
        .env("GIT_CONFIG_SYSTEM", "/dev/null");
    match global_override {
        Some(path) => cmd.env("GIT_CONFIG_GLOBAL", path),
        None => cmd.env("GIT_CONFIG_GLOBAL", "/dev/null"),
    };
    let out = cmd.output().unwrap();
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

/// Writes a scratch global-config file inside `dir` (a directory the caller
/// owns, typically its own `TempDir`) with `contents`, and returns its path
/// for use with [`git_with_global`].
///
/// Never opens, reads, or writes a real `~/.gitconfig` or `/etc/gitconfig` —
/// the file this returns lives entirely inside `dir`.
///
/// `#[cfg(test)]` here is restating what is already true of this whole file
/// (it is only ever reached via a `#[cfg(test)] mod` declaration or a
/// `#[path]` include from an integration test) so that
/// `rdm-core/tests/no_raw_fs_write_audit.rs` — which scans file text and
/// cannot see gating applied to the external `mod` statement in `lib.rs` —
/// recognizes this raw `fs::write` as test-only without a separate allowlist
/// entry, the same way it already does for a `#[cfg(test)] mod tests { .. }`
/// block.
#[cfg(test)]
pub fn write_global_config(dir: &Path, contents: &str) -> PathBuf {
    let path = dir.join("hostile.gitconfig");
    std::fs::write(&path, contents).unwrap();
    path
}

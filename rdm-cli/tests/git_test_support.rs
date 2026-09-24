//! Shared, isolated `git` test helper for `rdm-cli`'s integration tests that
//! drive real project-repo git state: `cli_worktree.rs`, `cli_gate.rs`,
//! `cli_verify.rs`, `cli_review_change.rs`, `cli_phase.rs`, `cli_task.rs`,
//! and the `workflow_review/`, `workflow_passes/`, `distribution/`,
//! `cli_loops/` and `golden_json/` test binaries (through
//! `common/plan_fixture.rs`). Included via
//! `#[path = "git_test_support.rs"] mod git_test_support;` from each — it is
//! deliberately NOT shared with any other `cli_*.rs` file, and NOT exposed
//! as a public API of `rdm-git` (see that crate's own copy at
//! `rdm-git/src/git_test_support.rs`), since a cross-crate boundary means
//! this side cannot import the other's `cfg(test)` module. The two copies
//! are an accepted, documented duplication rather than a defect — see
//! `CLAUDE.md`'s Dogfooding entry for `scripts/verify-git-config-isolation.sh`.
//!
//! # Why ambient global/system git config is isolated by default
//!
//! Every fixture here spawns real `git` subprocesses, and left alone those
//! subprocesses would read the *invoking developer's* `~/.gitconfig` and
//! `/etc/gitconfig`. That is not hypothetical: `2c55784` fixed a path bug
//! (`unified_diff_argv` returning an empty hunk set when rooted at a
//! subdirectory of the checkout) that passed locally yet failed for a real
//! user because their `~/.gitconfig` carried `diff.relative = true` — "a
//! real config users set", in that commit's own words. [`git`] closes the
//! gap the other direction: `GIT_CONFIG_GLOBAL`/`GIT_CONFIG_SYSTEM` are
//! redirected to `/dev/null` (a valid, empty config — not an error), so no
//! fixture can accidentally inherit a real global/system config. Only
//! `--local` is left untouched, since that is the layer a fixture sets up
//! itself.
//!
//! # Deliberately injecting a hostile setting
//!
//! A scenario that wants a setting to reach git through the *global* layer
//! specifically uses [`write_global_config`] to create a scratch file inside
//! its own `TempDir` — never a real `~/.gitconfig`/`/etc/gitconfig` — and
//! [`git_with_global`] to run a command with `GIT_CONFIG_GLOBAL` pointed at
//! it instead of `/dev/null`. See `cli_review_change.rs`'s
//! `change_comment_anchors_under_a_hostile_ambient_git_config` test.
//!
//! # Why source repos are nested inside their `TempDir`
//!
//! `rdm worktree add` places a new worktree at
//! `repo_root.parent()/<repo-name>__worktrees/<item>` — a sibling of the
//! repo root. A fixture whose repo root IS the bare `TempDir` puts that
//! sibling in the system temp directory, where `TempDir::drop` never reaches
//! it — `c647ab0` fixed this by rooting every repo one level down. This
//! module does not change that shape, only the isolation of the `git` calls
//! those fixtures make. See `scripts/verify-worktree-temp-hygiene.sh`.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Runs `git` in `dir`, asserting success and returning the raw [`Output`].
///
/// Isolated from the developer's real global/system git config — see the
/// module doc. Use [`git_with_global`] to deliberately inject a setting
/// through the global layer instead.
///
/// # Panics
///
/// Panics (via `assert!`) if the command exits non-zero, printing the argv
/// and stderr.
pub fn git(dir: &Path, args: &[&str]) -> Output {
    run(dir, args, None)
}

/// Like [`git`], but overrides `GIT_CONFIG_GLOBAL` with `global_config_path`
/// instead of `/dev/null`. Pair with [`write_global_config`].
///
/// # Panics
///
/// Panics (via `assert!`) if the command exits non-zero, printing the argv
/// and stderr.
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
/// for use with [`git_with_global`]. Never opens, reads, or writes a real
/// `~/.gitconfig` or `/etc/gitconfig`.
pub fn write_global_config(dir: &Path, contents: &str) -> PathBuf {
    let path = dir.join("hostile.gitconfig");
    std::fs::write(&path, contents).unwrap();
    path
}

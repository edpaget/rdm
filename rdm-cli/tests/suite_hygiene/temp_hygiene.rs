//! `rdm_git::worktree::add` places a worktree at
//! `repo_root.parent()/<repo-name>__worktrees/<item>`, a sibling of the repo
//! root. A fixture whose git root *is* its `TempDir` therefore puts that
//! sibling in the temp directory, where `TempDir::drop` never reaches it
//! (~44,000 such directories had accumulated before `c647ab0`). The nested
//! run's `TMPDIR` is a scratch directory this test owns, so the count is the
//! run's own.

use tempfile::TempDir;

use crate::nested::{Nested, describe_leaks, worktree_leaks};
use crate::workflow_support::Failure;

fn must<T>(r: Result<T, Failure>) -> T {
    r.unwrap_or_else(|f| panic!("{f}"))
}

/// Both whole crates rather than a list of binaries: the check's sensitivity
/// is exactly the number of worktree-creating fixtures it runs, so a list
/// that must be edited whenever a fixture is added would silently stop
/// catching things. Non-vacuity is `mutants::tempdir_rooted_fixture_leaks_a_worktree`.
#[test]
fn worktree_suites_leave_no_worktree_in_tmpdir() {
    let dir = TempDir::new().expect("tempdir");
    let nested = Nested::in_checkout(dir.path()).args(["-p", "rdm-git", "-p", "rdm-cli"]);
    let tmpdir = nested.tmpdir();
    let run = must(nested.run());
    assert_eq!(
        run.code,
        0,
        "the worktree suites did not pass — fix them before judging leakage:\n{}",
        run.tail()
    );
    let leaks = must(worktree_leaks(&tmpdir));
    assert!(leaks.is_empty(), "{}", describe_leaks(&leaks));
}

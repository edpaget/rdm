//! Negative controls for [`crate::temp_hygiene`] and [`crate::git_config`]:
//! each re-runs its scenario in a working-tree mirror of the checkout with a
//! regression planted ([`crate::mirror`]; family `suite-hygiene`), and must
//! show the failure the fixed check exists to catch. The checkout is only
//! ever read.
//!
//! One family carries both plants, and each control is attributable to one:
//!
//! - M1 runs under a neutral git config (global and system both
//!   `/dev/null`), so the stripped isolation is inert and only the re-rooted
//!   fixture matters.
//! - M2 asserts only on test failures, which the re-rooted fixture (it leaks,
//!   but passes) never causes.
//!
//! The nested run happens inside the mirror, `--frozen`, with the family's
//! own `CARGO_TARGET_DIR`, `RUSTFLAGS=""` and no debuginfo, holding the
//! family lock for the build and the run; only the targets the scenario runs
//! are built. An anchor that does not occur exactly once is
//! [`Failure::Infra`], so a control whose regression was not planted fails
//! as not-run, never as a pass.

use std::path::Path;

use tempfile::TempDir;

use crate::git_config::{FILTER, Hostile};
use crate::mirror::{CARGO_BUILD_REMOVALS, Edit, Mirror};
use crate::nested::{Nested, TEST_RUN_FAILED, worktree_leaks};
use crate::workflow_support::Failure;

fn must<T>(r: Result<T, Failure>) -> T {
    r.unwrap_or_else(|f| panic!("negative control not run: {f}"))
}

const FAMILY: &str = "suite-hygiene";

/// `init_project_repo` rooted back at its `TempDir`: the shape `c647ab0`
/// fixed.
const TEMPDIR_ROOTED_FIXTURE: &str = r##"    let dir = TempDir::new().unwrap();
    let root = dir.path().to_path_buf();
    git(&root, &["init", "-b", "main"]);
    std::fs::write(root.join("README.md"), "# project").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-m", "initial commit"]);
    SourceRepo { _dir: dir, root }
"##;

/// The global/system isolation lines of both `git_test_support` copies.
const ISOLATION: &str = r#"        .env("GIT_CONFIG_SYSTEM", "/dev/null");
    match global_override {
        Some(path) => cmd.env("GIT_CONFIG_GLOBAL", path),
        None => cmd.env("GIT_CONFIG_GLOBAL", "/dev/null"),
    };"#;

/// [`ISOLATION`] stripped: the fixture's git inherits the run's config.
const NO_ISOLATION: &str = "        ;\n    let _ = global_override;";

fn edits() -> Vec<Edit> {
    vec![
        Edit::FnBody {
            file: "rdm-git/tests/worktree.rs",
            name: "fixture rooted at its TempDir",
            sig: "fn init_project_repo(",
            body: TEMPDIR_ROOTED_FIXTURE,
        },
        Edit::Replace {
            file: "rdm-git/src/git_test_support.rs",
            name: "rdm-git fixture git-config isolation stripped",
            from: ISOLATION,
            to: NO_ISOLATION,
        },
        Edit::Replace {
            file: "rdm-cli/tests/git_test_support.rs",
            name: "rdm-cli fixture git-config isolation stripped",
            from: ISOLATION,
            to: NO_ISOLATION,
        },
    ]
}

/// A nested run inside `mirror`, building into its own target dir.
fn in_mirror(mirror: &Mirror, scratch: &Path) -> Nested {
    let mut nested = Nested::in_dir(&mirror.src(), scratch);
    for key in CARGO_BUILD_REMOVALS {
        nested = nested.env_remove(*key);
    }
    nested
        .args(["--frozen"])
        .env("CARGO_TARGET_DIR", mirror.target_dir())
        // A planted edit may leave a warning; `-D warnings` from the caller's
        // environment must not turn that into a not-run control.
        .env("RUSTFLAGS", "")
        .env("CARGO_PROFILE_DEV_DEBUG", "0")
}

/// M1 — `temp_hygiene`'s control: with the fixture rooted at its `TempDir`,
/// rdm-git's worktree suite passes but leaves worktrees in `TMPDIR`.
#[test]
fn tempdir_rooted_fixture_leaks_a_worktree() {
    let dir = TempDir::new().expect("tempdir");
    let mirror = must(Mirror::prepare(FAMILY, &edits(), dir.path()));
    eprintln!("{}", mirror.describe());
    let nested = in_mirror(&mirror, &dir.path().join("run"))
        .args(["-p", "rdm-git", "--test", "worktree"])
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null");
    let tmpdir = nested.tmpdir();
    let run = must(nested.run());
    assert_eq!(
        run.code,
        0,
        "inconclusive: the mutant worktree suite did not pass:\n{}",
        run.tail()
    );
    let leaks = must(worktree_leaks(&tmpdir));
    assert!(
        !leaks.is_empty(),
        "the TempDir-rooted fixture leaked nothing, so temp_hygiene would stay green \
         through a real regression — check that rdm-git's worktree suite still calls \
         init_project_repo"
    );
    eprintln!("the planted fixture leaked {} worktree dir(s)", leaks.len());
}

/// M2 — `git_config`'s control: with both `git_test_support` copies'
/// isolation stripped, the hostile run's tests fail (exit 100: tests ran and
/// failed, not a build or setup failure).
#[test]
fn stripped_git_config_isolation_fails_the_hostile_run() {
    let dir = TempDir::new().expect("tempdir");
    let mirror = must(Mirror::prepare(FAMILY, &edits(), dir.path()));
    eprintln!("{}", mirror.describe());
    let hostile = Hostile::new(&dir.path().join("hostile"));
    let nested = hostile.apply(in_mirror(&mirror, &dir.path().join("run")).args([
        "-p",
        "rdm-git",
        "-p",
        "rdm-cli",
        "--lib",
        "--test",
        "worktree",
        "--test",
        "cli_worktree",
        "--test",
        "cli_gate",
        "--test",
        "cli_verify",
        "--test",
        "cli_review_change",
        "-E",
        FILTER,
    ]));
    let run = must(nested.run());
    assert_eq!(
        run.code,
        TEST_RUN_FAILED,
        "with the isolation stripped the hostile run still passed, so git_config is \
         vacuous — check that the selected suites route every fixture commit through \
         git_test_support::git:\n{}",
        run.tail()
    );
}

//! The change-review and worktree suites must produce the same results under
//! a hostile global/system git config as under a clean one. `2c55784` fixed a
//! bug (`unified_diff_argv` returning an empty hunk set under
//! `diff.relative = true`) that passed every local run yet failed for a real
//! user whose `~/.gitconfig` carried that setting. `git_test_support` isolates
//! every fixture git call from the global/system layers, while the `rdm`
//! binary and rdm-git's production code still see whatever the run's
//! environment provides — so a regression in a production guard, not only a
//! fixture, shows up here.
//!
//! The clean result set is "every selected test passes" (the default
//! `cargo nextest run`, and `temp_hygiene`'s clean whole-crate run, prove
//! it), so the hostile run passing is the identical-results property.
//! Non-vacuity is `mutants::stripped_git_config_isolation_fails_the_hostile_run`.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

use crate::nested::{Nested, describe_leaks, rust_home_pins, worktree_leaks};
use crate::workflow_support::Failure;

fn must<T>(r: Result<T, Failure>) -> T {
    r.unwrap_or_else(|f| panic!("{f}"))
}

/// The suites implicated by change-review and worktree testing: rdm-git's
/// unit and integration tests (except its bare `tests::` module, whose local
/// fixture helper is deliberately unisolated and exercises HEAD/log/branch
/// primitives outside this scope), and the four rdm-cli binaries that drive
/// real project-repo git state.
pub const FILTER: &str = "(package(rdm-git) and not test(/^tests::/)) or (package(rdm-cli) and \
     (binary(cli_worktree) or binary(cli_gate) or binary(cli_verify) or \
     binary(cli_review_change)))";

/// A hostile git environment under `dir`: a scratch `HOME` whose
/// `.gitconfig` carries settings real users set, a hostile
/// `GIT_CONFIG_SYSTEM`, and an empty `GNUPGHOME`. Never the real
/// `~/.gitconfig` or `/etc/gitconfig`.
pub struct Hostile {
    home: PathBuf,
    system: PathBuf,
    gnupg: PathBuf,
    /// Created by the configured `diff.external` driver if git ever runs it.
    pub marker: PathBuf,
}

impl Hostile {
    /// Writes the fixture under `dir`.
    ///
    /// - `user.*`: a real `~/.gitconfig` has an identity; one without is
    ///   incomplete, not hostile.
    /// - `diff.relative`: the `2c55784` trigger.
    /// - `diff.external`: a driver replaces git's diff machinery, so a
    ///   `git diff` without `--no-ext-diff` reports nothing; the driver
    ///   touches [`Hostile::marker`] so an execution is observable.
    /// - `init.defaultBranch = weird`, `advice.detachedHead`, `core.pager =
    ///   false`: nonstandard defaults, noisy advice, a pager.
    /// - `commit.gpgsign`: a fixture commit that reaches it fails loudly (the
    ///   empty `GNUPGHOME` makes that fast rather than a prompt).
    pub fn new(dir: &Path) -> Self {
        let home = dir.join("home");
        let gnupg = dir.join("gnupghome");
        std::fs::create_dir_all(&home).expect("hostile home");
        std::fs::create_dir_all(&gnupg).expect("hostile gnupghome");
        set_mode(&gnupg, 0o700);
        let marker = dir.join("external-diff-ran");
        let driver = dir.join("hostile-diff-driver.sh");
        std::fs::write(
            &driver,
            format!("#!/bin/sh\n: > '{}'\nexit 0\n", marker.display()),
        )
        .expect("diff driver");
        set_mode(&driver, 0o755);
        std::fs::write(
            home.join(".gitconfig"),
            format!(
                "[user]\n\tname = Hostile Developer\n\temail = hostile@example.com\n\
                 [diff]\n\trelative = true\n\texternal = {}\n\
                 [init]\n\tdefaultBranch = weird\n\
                 [advice]\n\tdetachedHead = true\n\
                 [core]\n\tpager = false\n\
                 [commit]\n\tgpgsign = true\n",
                driver.display()
            ),
        )
        .expect("hostile .gitconfig");
        let system = dir.join("system-gitconfig");
        std::fs::write(&system, "[diff]\n\trelative = true\n").expect("hostile system config");
        Self {
            home,
            system,
            gnupg,
            marker,
        }
    }

    /// Applies the hostile environment to `nested`, pinning the Rust homes
    /// that would otherwise follow `HOME`, and removing any inherited
    /// `GIT_CONFIG_GLOBAL` (which would override the hostile `HOME`).
    pub fn apply(&self, mut nested: Nested) -> Nested {
        nested = nested
            .env_remove("GIT_CONFIG_GLOBAL")
            .env_remove("XDG_CONFIG_HOME")
            .env("HOME", &self.home)
            .env("GIT_CONFIG_SYSTEM", &self.system)
            .env("GNUPGHOME", &self.gnupg);
        for (key, value) in rust_home_pins() {
            nested = nested.env(key, value);
        }
        nested
    }
}

fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).expect("chmod");
}

/// The `-p` scope every run adds: `-E` selects what runs, not what builds.
pub fn scope() -> Vec<OsString> {
    ["-p", "rdm-git", "-p", "rdm-cli", "-E", FILTER]
        .map(OsString::from)
        .to_vec()
}

#[test]
fn hostile_git_config_changes_no_result() {
    let dir = TempDir::new().expect("tempdir");
    let hostile = Hostile::new(&dir.path().join("hostile"));
    let nested = hostile.apply(Nested::in_checkout(&dir.path().join("run")).args(scope()));
    let tmpdir = nested.tmpdir();
    let run = must(nested.run());
    assert_eq!(
        run.code,
        0,
        "the implicated suites did NOT pass under a hostile HOME/GIT_CONFIG_SYSTEM — a \
         fixture or a production guard is reading the ambient git config (see \
         rdm-git/src/git_test_support.rs and rdm-cli/tests/git_test_support.rs):\n{}",
        run.tail()
    );
    assert!(
        !hostile.marker.exists(),
        "a git diff ran the hostile `diff.external` driver: a production `git diff` is \
         missing `--no-ext-diff`"
    );
    let leaks = must(worktree_leaks(&tmpdir));
    assert!(
        leaks.is_empty(),
        "under the hostile config: {}",
        describe_leaks(&leaks)
    );
}

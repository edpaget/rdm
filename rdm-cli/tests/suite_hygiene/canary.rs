//! The hook-environment canary (task `canary-git-env-isolation-regression`).
//!
//! git exports `GIT_DIR`/`GIT_INDEX_FILE` (a linked worktree's gitdir) to the
//! hooks it runs, and hk's pre-commit step runs the suite from one. On
//! 2026-09-24 a test that spawned `git init` without scrubbing them acted on
//! the invoking repository and rewrote its shared config to
//! `core.bare = true`. The canary reproduces that environment — without the
//! hk scrub — but points it at a throwaway repository with a linked worktree
//! inside the test's `TempDir`, never at this checkout: a whole-suite nested
//! run must leave every byte of the canary untouched.
//! [`an_unscrubbed_git_init_flips_the_canary`] is the negative control: the
//! same fixture and environment, and one unscrubbed `git init`, must trip the
//! same detector.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

use crate::nested::Nested;
use crate::plan_fixture::Sandbox;
use crate::workflow_support::{Failure, infra};

fn must<T>(r: Result<T, Failure>) -> T {
    r.unwrap_or_else(|f| panic!("{f}"))
}

/// A throwaway repository (`<TempDir>/canary/repo`) with one commit and a
/// linked worktree (`<TempDir>/canary/wt`).
struct Canary {
    dir: TempDir,
    sandbox: Sandbox,
    repo: PathBuf,
    wt: PathBuf,
}

impl Canary {
    fn new() -> Result<Self, Failure> {
        let dir = TempDir::new().map_err(infra)?;
        let sandbox = Sandbox::new(&dir.path().join("sandbox"))?;
        let base = dir.path().join("canary");
        let repo = base.join("repo");
        std::fs::create_dir_all(&repo).map_err(infra)?;
        sandbox.git(&repo, &["init", "-b", "main"])?;
        std::fs::write(repo.join("README.md"), "# canary\n").map_err(infra)?;
        sandbox.git(&repo, &["add", "README.md"])?;
        sandbox.git(&repo, &["commit", "-m", "canary"])?;
        sandbox.git(&repo, &["worktree", "add", "../wt"])?;
        Ok(Self {
            wt: base.join("wt"),
            dir,
            sandbox,
            repo,
        })
    }

    /// The linked worktree's gitdir, what git exports as `GIT_DIR` to a hook
    /// running in it.
    fn gitdir(&self) -> PathBuf {
        self.repo.join(".git").join("worktrees").join("wt")
    }

    /// The repository-locating variables git exports to a hook run in the
    /// linked worktree.
    fn hook_env(&self) -> Vec<(&'static str, PathBuf)> {
        vec![
            ("GIT_DIR", self.gitdir()),
            ("GIT_INDEX_FILE", self.gitdir().join("index")),
            ("GIT_PREFIX", PathBuf::new()),
            ("GIT_EDITOR", PathBuf::from(":")),
        ]
    }

    /// Every file under the repository's `.git` and the linked worktree.
    fn snapshot(&self) -> Result<BTreeMap<PathBuf, Vec<u8>>, Failure> {
        let mut files = BTreeMap::new();
        for root in [self.repo.join(".git"), self.wt.clone()] {
            walk(&root, &mut files)?;
        }
        Ok(files)
    }

    /// `core.bare` of the canary, read through a scrubbed git.
    fn core_bare(&self) -> Result<String, Failure> {
        let git_dir = self.repo.join(".git");
        let out = self.sandbox.git(
            self.dir.path(),
            &[
                "--git-dir",
                &git_dir.to_string_lossy(),
                "config",
                "--get",
                "core.bare",
            ],
        )?;
        Ok(out.trim().to_owned())
    }
}

fn walk(dir: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) -> Result<(), Failure> {
    for entry in std::fs::read_dir(dir).map_err(infra)? {
        let entry = entry.map_err(infra)?;
        let path = entry.path();
        if entry.file_type().map_err(infra)?.is_dir() {
            walk(&path, files)?;
        } else {
            files.insert(path.clone(), std::fs::read(&path).map_err(infra)?);
        }
    }
    Ok(())
}

/// The paths whose bytes differ between two snapshots (added, removed or
/// changed).
fn differing(
    before: &BTreeMap<PathBuf, Vec<u8>>,
    after: &BTreeMap<PathBuf, Vec<u8>>,
) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = before
        .keys()
        .chain(after.keys())
        .filter(|p| before.get(*p) != after.get(*p))
        .cloned()
        .collect();
    paths.sort();
    paths.dedup();
    paths
}

#[test]
fn hook_git_env_reaches_no_repository() {
    let canary = must(Canary::new());
    let before = must(canary.snapshot());
    let mut nested = Nested::in_checkout(&canary.dir.path().join("run"));
    for (key, value) in canary.hook_env() {
        nested = nested.env(key, value);
    }
    let run = must(nested.run());
    assert_eq!(
        run.code,
        0,
        "the suite did not pass under a hook's GIT_DIR/GIT_INDEX_FILE (a test that \
         inherits them acts on the repository they name):\n{}",
        run.tail()
    );
    let changed = differing(&before, &must(canary.snapshot()));
    assert!(
        changed.is_empty(),
        "a test reached the repository named by the inherited hook environment and \
         changed:\n{}\nEvery git (or rdm) child a test spawns must drop \
         git_test_support::REPO_REDIRECT_VARS (see rdm-cli/tests/git_test_support.rs).",
        changed
            .iter()
            .map(|p| format!("  {}", p.display()))
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert_eq!(must(canary.core_bare()), "false", "the canary turned bare");
}

#[test]
fn an_unscrubbed_git_init_flips_the_canary() {
    let canary = must(Canary::new());
    let before = must(canary.snapshot());
    let victim = canary.dir.path().join("victim");
    let mut cmd = canary.sandbox.command("git");
    cmd.arg("init").arg(&victim).current_dir(canary.dir.path());
    for (key, value) in canary.hook_env() {
        cmd.env(key, value);
    }
    let out = cmd.output().expect("spawn git init");
    assert!(
        out.status.success(),
        "inconclusive: the unscrubbed git init failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let changed = differing(&before, &must(canary.snapshot()));
    assert!(
        changed.contains(&canary.repo.join(".git").join("config")),
        "the detector did not see the unscrubbed git init rewrite the shared config: \
         {changed:?}"
    );
    assert_eq!(
        must(canary.core_bare()),
        "true",
        "the unscrubbed git init did not flip core.bare"
    );
}

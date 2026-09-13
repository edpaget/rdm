//! [`GitSourceRepo`]: the git-backed implementation of rdm-core's read-only
//! [`SourceRepo`] port, used to derive and resolve a `change/<sha>` review's
//! comment anchors against the project's source repository.
//!
//! Every method here spawns `git` with a **read-only** subcommand —
//! `rev-parse`, `merge-base`, `show`, or `diff` — and that list is the whole
//! contract: the port exposes no write, so reviewing a change can never
//! mutate the repository being reviewed. The allow-list is asserted
//! directly in this module's tests over the argv each method builds.

use std::path::{Path, PathBuf};

use rdm_core::error::{Error, Result};
use rdm_core::source::SourceRepo;

use crate::run_git_at;

/// A source repository rooted at a filesystem path, read through `git`.
#[derive(Debug, Clone)]
pub struct GitSourceRepo {
    root: PathBuf,
}

impl GitSourceRepo {
    /// Wraps the repository containing `root`.
    ///
    /// No validation happens here — a path that is not inside a git
    /// repository surfaces as an [`Error::Git`] from the first call.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The filesystem path this repository is rooted at.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }
}

/// The argv [`GitSourceRepo::rev_parse`] builds for `rev`.
///
/// Split out (with its siblings below) so the read-only command allow-list
/// is testable without spawning git.
#[must_use]
pub fn rev_parse_argv(rev: &str) -> Vec<String> {
    vec![
        "rev-parse".to_string(),
        "--verify".to_string(),
        "--quiet".to_string(),
        format!("{rev}^{{commit}}"),
    ]
}

/// The argv [`GitSourceRepo::merge_base`] builds.
#[must_use]
pub fn merge_base_argv(a: &str, b: &str) -> Vec<String> {
    vec!["merge-base".to_string(), a.to_string(), b.to_string()]
}

/// The argv [`GitSourceRepo::file_at`] builds.
#[must_use]
pub fn file_at_argv(rev: &str, path: &str) -> Vec<String> {
    vec!["show".to_string(), format!("{rev}:{path}")]
}

/// The argv [`GitSourceRepo::unified_diff`] builds.
#[must_use]
pub fn unified_diff_argv(base: &str, head: &str, path: &str) -> Vec<String> {
    vec![
        "diff".to_string(),
        "--unified=0".to_string(),
        "--no-color".to_string(),
        format!("{base}..{head}"),
        "--".to_string(),
        path.to_string(),
    ]
}

/// Runs `argv` in this repository, returning `None` on a non-zero exit.
fn run_ok(root: &Path, argv: &[String]) -> Result<Option<Vec<u8>>> {
    let borrowed: Vec<&str> = argv.iter().map(String::as_str).collect();
    let out = run_git_at(root, &borrowed)?;
    if !out.status.success() {
        return Ok(None);
    }
    Ok(Some(out.stdout))
}

/// Trims trailing whitespace off a single-line git result, mapping an empty
/// result to `None`.
fn single_line(bytes: Vec<u8>) -> Option<String> {
    let s = String::from_utf8_lossy(&bytes).trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

impl SourceRepo for GitSourceRepo {
    fn rev_parse(&self, rev: &str) -> Result<Option<String>> {
        Ok(run_ok(&self.root, &rev_parse_argv(rev))?.and_then(single_line))
    }

    fn merge_base(&self, a: &str, b: &str) -> Result<Option<String>> {
        Ok(run_ok(&self.root, &merge_base_argv(a, b))?.and_then(single_line))
    }

    fn file_at(&self, rev: &str, path: &str) -> Result<Option<String>> {
        let Some(bytes) = run_ok(&self.root, &file_at_argv(rev, path))? else {
            return Ok(None);
        };
        // A binary or otherwise non-UTF-8 blob cannot be quoted; surface it
        // rather than lossily converting (which would let an anchor "match"
        // replacement characters).
        String::from_utf8(bytes).map(Some).map_err(|_| {
            Error::Git(format!(
                "{rev}:{path} is not valid UTF-8 — a change review can only anchor comments in text files"
            ))
        })
    }

    fn unified_diff(&self, base: &str, head: &str, path: &str) -> Result<Option<String>> {
        let Some(bytes) = run_ok(&self.root, &unified_diff_argv(base, head, path))? else {
            return Ok(None);
        };
        let text = String::from_utf8_lossy(&bytes).to_string();
        // `git diff` exits 0 with empty output when the range does not touch
        // the path at all — report that as "untouched", not "empty diff".
        if text.trim().is_empty() {
            return Ok(None);
        }
        Ok(Some(text))
    }

    fn head(&self) -> Result<Option<String>> {
        self.rev_parse("HEAD")
    }

    fn current_branch(&self) -> Result<Option<String>> {
        crate::current_branch_at(&self.root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::TempDir;

    /// The complete set of git subcommands this module is permitted to run.
    /// Adding to it is a deliberate act: the port is read-only by
    /// construction, and this is where that construction is enforced.
    const READ_ONLY_COMMANDS: &[&str] = &["rev-parse", "merge-base", "show", "diff"];

    #[test]
    fn every_built_argv_starts_with_a_read_only_subcommand() {
        let argvs = vec![
            rev_parse_argv("HEAD"),
            merge_base_argv("main", "topic"),
            file_at_argv("abc123", "src/lib.rs"),
            unified_diff_argv("base", "head", "src/lib.rs"),
        ];
        assert_eq!(argvs.len(), 4);
        for argv in &argvs {
            let subcommand = argv.first().map(String::as_str).unwrap_or_default();
            assert!(
                READ_ONLY_COMMANDS.contains(&subcommand),
                "git subcommand {subcommand:?} is not in the read-only allow-list"
            );
        }
    }

    #[test]
    fn no_built_argv_mentions_a_mutating_subcommand() {
        let joined = [
            rev_parse_argv("HEAD"),
            merge_base_argv("a", "b"),
            file_at_argv("a", "p"),
            unified_diff_argv("a", "b", "p"),
        ]
        .concat()
        .join(" ");
        for forbidden in ["add", "commit", "checkout", "write-tree", "reset", "push"] {
            assert!(
                !joined.split_whitespace().any(|tok| tok == forbidden),
                "argv unexpectedly contains {forbidden:?}: {joined}"
            );
        }
    }

    /// A `git` command in `dir` with an identity configured and the ambient
    /// git environment cleared — these tests must work unchanged when run
    /// from inside a git hook, which exports `GIT_DIR`/`GIT_WORK_TREE`/
    /// `GIT_INDEX_FILE` pointing at the invoking repository.
    fn git(dir: &Path, args: &[&str]) {
        let out = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .env("GIT_AUTHOR_NAME", "test")
            .env("GIT_AUTHOR_EMAIL", "test@test.com")
            .env("GIT_COMMITTER_NAME", "test")
            .env("GIT_COMMITTER_EMAIL", "test@test.com")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn seed() -> TempDir {
        let dir = TempDir::new().unwrap();
        let p = dir.path();
        git(p, &["init", "-b", "main"]);
        std::fs::write(p.join("a.txt"), "one\ntwo\nthree\n").unwrap();
        git(p, &["add", "."]);
        git(p, &["commit", "-m", "base"]);
        git(p, &["checkout", "-b", "topic"]);
        std::fs::write(p.join("a.txt"), "one\nTWO\nthree\n").unwrap();
        git(p, &["add", "."]);
        git(p, &["commit", "-m", "edit"]);
        dir
    }

    #[test]
    fn rev_parse_resolves_symbolic_and_abbreviated_revs() {
        let dir = seed();
        let repo = GitSourceRepo::new(dir.path());
        let head = repo.rev_parse("HEAD").unwrap().unwrap();
        assert_eq!(head.len(), 40);
        assert_eq!(repo.rev_parse(&head[..7]).unwrap().unwrap(), head);
        assert_eq!(repo.rev_parse("topic").unwrap().unwrap(), head);
        assert_eq!(repo.head().unwrap().unwrap(), head);
        assert_eq!(repo.current_branch().unwrap().unwrap(), "topic");
    }

    #[test]
    fn rev_parse_returns_none_for_an_unknown_rev() {
        let dir = seed();
        let repo = GitSourceRepo::new(dir.path());
        assert_eq!(repo.rev_parse("no-such-rev").unwrap(), None);
    }

    #[test]
    fn merge_base_and_diff_and_file_reads_work() {
        let dir = seed();
        let repo = GitSourceRepo::new(dir.path());
        let base = repo.merge_base("main", "topic").unwrap().unwrap();
        let head = repo.rev_parse("topic").unwrap().unwrap();
        assert_eq!(
            repo.file_at(&head, "a.txt").unwrap().unwrap(),
            "one\nTWO\nthree\n"
        );
        assert_eq!(
            repo.file_at(&base, "a.txt").unwrap().unwrap(),
            "one\ntwo\nthree\n"
        );
        let diff = repo.unified_diff(&base, &head, "a.txt").unwrap().unwrap();
        assert!(diff.contains("@@"), "expected hunk headers in {diff}");
        // An untouched path is `None`, not an empty diff.
        assert_eq!(
            repo.unified_diff(&base, &head, "missing.txt").unwrap(),
            None
        );
    }

    #[test]
    fn file_at_returns_none_for_a_missing_path() {
        let dir = seed();
        let repo = GitSourceRepo::new(dir.path());
        let head = repo.rev_parse("HEAD").unwrap().unwrap();
        assert_eq!(repo.file_at(&head, "nope.txt").unwrap(), None);
    }

    #[test]
    fn file_at_errors_on_non_utf8_content() {
        let dir = seed();
        let p = dir.path();
        std::fs::write(p.join("bin.dat"), [0xff_u8, 0xfe, 0x00, 0x01]).unwrap();
        git(p, &["add", "."]);
        git(p, &["commit", "-m", "binary"]);
        let repo = GitSourceRepo::new(p);
        let head = repo.rev_parse("HEAD").unwrap().unwrap();
        assert!(repo.file_at(&head, "bin.dat").is_err());
    }

    #[test]
    fn reading_never_mutates_the_repository() {
        let dir = seed();
        let repo = GitSourceRepo::new(dir.path());
        let before_head = repo.rev_parse("HEAD").unwrap().unwrap();
        let porcelain = |dir: &Path| {
            Command::new("git")
                .args(["status", "--porcelain"])
                .current_dir(dir)
                .env_remove("GIT_DIR")
                .env_remove("GIT_WORK_TREE")
                .env_remove("GIT_INDEX_FILE")
                .output()
                .unwrap()
                .stdout
        };
        let status_before = porcelain(dir.path());

        let base = repo.merge_base("main", "topic").unwrap().unwrap();
        let _ = repo.file_at(&before_head, "a.txt").unwrap();
        let _ = repo.unified_diff(&base, &before_head, "a.txt").unwrap();
        let _ = repo.current_branch().unwrap();

        assert_eq!(repo.rev_parse("HEAD").unwrap().unwrap(), before_head);
        let status_after = porcelain(dir.path());
        assert_eq!(status_before, status_after);
    }
}

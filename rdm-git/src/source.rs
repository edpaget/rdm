//! [`GitSourceRepo`]: the git-backed implementation of rdm-core's read-only
//! [`SourceRepo`] port, used to derive and resolve a `change/<sha>` review's
//! comment anchors against the project's source repository.
//!
//! Every method here spawns `git` with a **read-only** subcommand —
//! `rev-parse`, `merge-base`, `show`, or `diff` — but that list is only the
//! starting point. Revision operands are rejected if option-shaped, command-
//! specific option boundaries prevent reinterpretation, and file/diff reads
//! resolve operands to commits before constructing object/range expressions.

use std::path::{Path, PathBuf};

use rdm_core::error::{Error, Result};
use rdm_core::source::{SourceObjectKind, SourceRepo};

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
        "--end-of-options".to_string(),
        format!("{rev}^{{commit}}"),
    ]
}

/// The argv [`GitSourceRepo::merge_base`] builds.
#[must_use]
pub fn merge_base_argv(a: &str, b: &str) -> Vec<String> {
    vec![
        "merge-base".to_string(),
        "--".to_string(),
        a.to_string(),
        b.to_string(),
    ]
}

/// The argv [`GitSourceRepo::file_at`] builds after resolving `rev` to a commit.
#[must_use]
pub fn file_at_argv(rev: &str, path: &str) -> Vec<String> {
    vec![
        "show".to_string(),
        "--end-of-options".to_string(),
        format!("{rev}:{path}"),
    ]
}

/// The argv [`GitSourceRepo::object_kind_at`] builds after resolving `rev` to
/// a commit.
///
/// `git ls-tree <tree> -- <path>` reports a tree entry's recorded mode/type
/// directly from the tree object, with **no need for the entry's own object
/// to exist locally** — the one property that makes this usable for a
/// gitlink: the commit it names lives in the *submodule's* object database,
/// essentially never the superproject's, so `git cat-file -t <rev>:<path>`
/// (which requires actually opening that object) reliably fails on exactly
/// the gitlink case this method exists to detect. `--full-tree` is required
/// for the same reason [`unified_diff_argv`] needs `:(top)`/`--no-relative`:
/// without it, `ls-tree`'s pathspec is matched relative to the invoking cwd,
/// silently returning nothing for a path that does exist when `GitSourceRepo`
/// is rooted at a subdirectory of the checkout.
#[must_use]
pub fn ls_tree_kind_argv(rev: &str, path: &str) -> Vec<String> {
    vec![
        "ls-tree".to_string(),
        "--full-tree".to_string(),
        "--end-of-options".to_string(),
        rev.to_string(),
        "--".to_string(),
        path.to_string(),
    ]
}

/// Parses one `git ls-tree` output line (`<mode> <type> <sha>\t<path>`) into
/// its [`SourceObjectKind`], returning `None` for an empty result (the path
/// does not exist in the tree) or an unrecognized type.
fn parse_ls_tree_kind(bytes: &[u8]) -> Option<SourceObjectKind> {
    let line = String::from_utf8_lossy(bytes);
    let line = line.lines().next()?;
    let kind = line.split_whitespace().nth(1)?;
    match kind {
        "blob" => Some(SourceObjectKind::Blob),
        "tree" => Some(SourceObjectKind::Tree),
        "commit" => Some(SourceObjectKind::Gitlink),
        _ => None,
    }
}

/// The argv [`GitSourceRepo::unified_diff`] builds after resolving both commits.
///
/// Two guards make this agree with [`file_at_argv`] about what `path` means,
/// and BOTH are required.
///
/// `git show <rev>:<path>` always resolves from the **repository root**, but
/// `GitSourceRepo` is routinely rooted at a subdirectory of the checkout — it
/// is built from the invoking cwd, so a linked worktree's own HEAD is what
/// `change/HEAD` pins. Unless the diff agrees, a comment anchored from a
/// subdirectory reads its file successfully but computes an empty hunk set,
/// and is refused as "not touched by `<base>..<head>`" for a file the change
/// really does modify.
///
/// - `:(top)` on the pathspec anchors it at the repository root, since git
///   otherwise resolves a `diff` pathspec relative to the current directory.
///   It is a no-op when the root already is the top level.
/// - `--no-relative` overrides the user's `diff.relative` config. With
///   `diff.relative = true` set in `~/.gitconfig`, git restricts the diff to
///   the current directory and strips the rest — which re-opens exactly the
///   empty-hunk-set bug even with the `:(top)` pathspec, because the output
///   is filtered after the pathspec is matched. rdm's git subprocesses do not
///   clear `GIT_CONFIG_GLOBAL`/`GIT_CONFIG_SYSTEM`, so ambient user config
///   reaches this command and the flag is what makes the result independent
///   of it.
#[must_use]
pub fn unified_diff_argv(base: &str, head: &str, path: &str) -> Vec<String> {
    vec![
        "diff".to_string(),
        "--unified=0".to_string(),
        "--no-color".to_string(),
        "--no-relative".to_string(),
        "--end-of-options".to_string(),
        format!("{base}..{head}"),
        "--".to_string(),
        format!(":(top){path}"),
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

// Reject before any subprocess, including before resolving the other operand.
fn validate_revision_input(rev: &str) -> Result<()> {
    if rev.starts_with('-') {
        return Err(Error::InvalidChangeRevisionInput(rev.to_string()));
    }
    Ok(())
}

impl SourceRepo for GitSourceRepo {
    fn rev_parse(&self, rev: &str) -> Result<Option<String>> {
        validate_revision_input(rev)?;
        Ok(run_ok(&self.root, &rev_parse_argv(rev))?.and_then(single_line))
    }

    fn merge_base(&self, a: &str, b: &str) -> Result<Option<String>> {
        validate_revision_input(a)?;
        validate_revision_input(b)?;
        Ok(run_ok(&self.root, &merge_base_argv(a, b))?.and_then(single_line))
    }

    fn file_at(&self, rev: &str, path: &str) -> Result<Option<String>> {
        validate_revision_input(rev)?;
        let Some(commit) = self.rev_parse(rev)? else {
            return Ok(None);
        };
        let Some(bytes) = run_ok(&self.root, &file_at_argv(&commit, path))? else {
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
        validate_revision_input(base)?;
        validate_revision_input(head)?;
        let Some(base) = self.rev_parse(base)? else {
            return Ok(None);
        };
        let Some(head) = self.rev_parse(head)? else {
            return Ok(None);
        };
        let Some(bytes) = run_ok(&self.root, &unified_diff_argv(&base, &head, path))? else {
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

    fn object_kind_at(&self, rev: &str, path: &str) -> Result<Option<SourceObjectKind>> {
        validate_revision_input(rev)?;
        let Some(commit) = self.rev_parse(rev)? else {
            return Ok(None);
        };
        let Some(bytes) = run_ok(&self.root, &ls_tree_kind_argv(&commit, path))? else {
            return Ok(None);
        };
        Ok(parse_ls_tree_kind(&bytes))
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
    const READ_ONLY_COMMANDS: &[&str] = &["rev-parse", "merge-base", "show", "diff", "ls-tree"];

    #[test]
    fn every_built_argv_starts_with_a_read_only_subcommand() {
        let argvs = vec![
            rev_parse_argv("HEAD"),
            merge_base_argv("main", "topic"),
            file_at_argv("abc123", "src/lib.rs"),
            unified_diff_argv("base", "head", "src/lib.rs"),
            ls_tree_kind_argv("abc123", "src/lib.rs"),
        ];
        assert_eq!(argvs.len(), 5);
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
            ls_tree_kind_argv("a", "p"),
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

    /// `GitSourceRepo` is rooted at the invoking cwd (so a linked worktree's own
    /// HEAD is what `change/HEAD` pins), which means the root is very often a
    /// SUBDIRECTORY of the checkout rather than its top level. `git show
    /// <rev>:<path>` resolves its path from the repository root, so `file_at`
    /// works from anywhere; a bare `git diff -- <path>` pathspec is resolved
    /// relative to the CWD instead. Unless the two agree, a comment anchored
    /// from a subdirectory reads the file fine but sees an empty hunk set and
    /// is refused as "not touched by <base>..<head>" — a confidently false
    /// error about a file the change really does modify.
    #[test]
    fn unified_diff_finds_hunks_from_a_subdirectory_of_the_checkout() {
        let dir = seed();
        let top = dir.path();
        // A sibling directory to stand in. `seed` leaves HEAD on `topic`, so
        // this commit lands only there — it just needs to exist in the working
        // tree for `GitSourceRepo` to be rooted at it.
        std::fs::create_dir_all(top.join("other")).unwrap();
        std::fs::write(top.join("other/keep.txt"), "x\n").unwrap();
        git(top, &["add", "."]);
        git(top, &["commit", "-m", "sibling"]);
        // `diff.relative = true` is a real thing users set. It restricts a diff
        // to the cwd, re-opening the empty-hunk-set bug even with the `:(top)`
        // pathspec — so pin it here: this repo carries the hostile config, and
        // the assertions below fail if `--no-relative` is dropped.
        git(top, &["config", "diff.relative", "true"]);

        let from_top = GitSourceRepo::new(top);
        let base = from_top.merge_base("main", "topic").unwrap().unwrap();
        let head = from_top.rev_parse("topic").unwrap().unwrap();

        // Rooted at a subdirectory — what `discover_source_repo` hands us when
        // the operator runs rdm from anywhere but the checkout's top level.
        let from_sub = GitSourceRepo::new(top.join("other"));

        // The control: reading the file at head already works from here.
        assert_eq!(
            from_sub.file_at(&head, "a.txt").unwrap().unwrap(),
            "one\nTWO\nthree\n",
            "file_at resolves from the repo root, so this half always worked"
        );

        let diff = from_sub
            .unified_diff(&base, &head, "a.txt")
            .unwrap()
            .expect("a.txt IS modified by base..head, so the diff must not be None");
        assert!(
            diff.contains("@@"),
            "expected hunk headers when diffing from a subdirectory, got: {diff}"
        );

        // And an untouched path is still None from here, not a false positive.
        assert_eq!(
            from_sub.unified_diff(&base, &head, "missing.txt").unwrap(),
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
        // The binary-content rejection is a UTF-8-validation concern reached
        // only after the object-kind check has already confirmed Blob — not
        // folded into the kind gate itself.
        assert_eq!(
            repo.object_kind_at(&head, "bin.dat").unwrap(),
            Some(SourceObjectKind::Blob)
        );
    }

    #[test]
    fn object_kind_at_distinguishes_blob_and_tree() {
        let dir = seed();
        let p = dir.path();
        std::fs::create_dir_all(p.join("sub")).unwrap();
        std::fs::write(p.join("sub/nested.txt"), "nested\n").unwrap();
        git(p, &["add", "."]);
        git(p, &["commit", "-m", "add subdirectory"]);
        let repo = GitSourceRepo::new(p);
        let head = repo.rev_parse("HEAD").unwrap().unwrap();
        assert_eq!(
            repo.object_kind_at(&head, "a.txt").unwrap(),
            Some(SourceObjectKind::Blob)
        );
        assert_eq!(
            repo.object_kind_at(&head, "sub").unwrap(),
            Some(SourceObjectKind::Tree)
        );
        assert_eq!(repo.object_kind_at(&head, "nope.txt").unwrap(), None);
    }

    /// `git ls-tree`'s pathspec is matched relative to the invoking cwd by
    /// default — exactly the same trap [`unified_diff_argv`] works around —
    /// so `object_kind_at` must keep working when `GitSourceRepo` is rooted
    /// at a subdirectory of the checkout rather than its top level.
    #[test]
    fn object_kind_at_resolves_from_a_subdirectory_of_the_checkout() {
        let dir = seed();
        let top = dir.path();
        std::fs::create_dir_all(top.join("sub")).unwrap();
        std::fs::write(top.join("sub/nested.txt"), "nested\n").unwrap();
        std::fs::create_dir_all(top.join("other")).unwrap();
        std::fs::write(top.join("other/keep.txt"), "x\n").unwrap();
        git(top, &["add", "."]);
        git(top, &["commit", "-m", "add subdirectories"]);
        let repo = GitSourceRepo::new(top.join("other"));
        let head = repo.rev_parse("HEAD").unwrap().unwrap();
        assert_eq!(
            repo.object_kind_at(&head, "a.txt").unwrap(),
            Some(SourceObjectKind::Blob)
        );
        assert_eq!(
            repo.object_kind_at(&head, "sub").unwrap(),
            Some(SourceObjectKind::Tree)
        );
    }

    /// A gitlink (mode 160000) is a tree entry recording another
    /// repository's commit — built by hand via `update-index --cacheinfo` so
    /// this needs no real submodule content or network access, and the
    /// referenced commit is never fetched into this repository's object
    /// database (exactly the real-world shape: a submodule's commits live in
    /// its own separate repository). `git ls-tree` must still resolve the
    /// entry's recorded type (`commit`) straight from the tree object,
    /// without dereferencing into the submodule to look it up.
    #[test]
    fn object_kind_at_reports_a_gitlink() {
        let dir = seed();
        let p = dir.path();
        let fake_submodule_sha = "a".repeat(40);
        git(
            p,
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("160000,{fake_submodule_sha},vendor/lib"),
            ],
        );
        git(p, &["commit", "-m", "add gitlink"]);
        let repo = GitSourceRepo::new(p);
        let head = repo.rev_parse("HEAD").unwrap().unwrap();
        assert_eq!(
            repo.object_kind_at(&head, "vendor/lib").unwrap(),
            Some(SourceObjectKind::Gitlink)
        );
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
    #[test]
    fn revision_boundaries_are_command_specific() {
        assert_eq!(
            rev_parse_argv("HEAD"),
            [
                "rev-parse",
                "--verify",
                "--quiet",
                "--end-of-options",
                "HEAD^{commit}"
            ]
        );
        assert_eq!(
            merge_base_argv("main", "topic"),
            ["merge-base", "--", "main", "topic"]
        );
        assert_eq!(
            file_at_argv("head", "a.txt"),
            ["show", "--end-of-options", "head:a.txt"]
        );
        assert_eq!(
            unified_diff_argv("base", "head", "a.txt"),
            [
                "diff",
                "--unified=0",
                "--no-color",
                "--no-relative",
                "--end-of-options",
                "base..head",
                "--",
                ":(top)a.txt"
            ]
        );
        assert_eq!(
            ls_tree_kind_argv("head", "a.txt"),
            [
                "ls-tree",
                "--full-tree",
                "--end-of-options",
                "head",
                "--",
                "a.txt"
            ]
        );
    }

    #[test]
    fn hostile_revision_operands_never_create_or_overwrite_files() {
        let dir = seed();
        let repo = GitSourceRepo::new(dir.path());
        let outside = TempDir::new().unwrap();
        for preexisting in [false, true] {
            let output = outside.path().join("output");
            // Include the names produced by the old show/range concatenation.
            let sentinels = [
                output.clone(),
                outside.path().join("output:a.txt"),
                outside.path().join("output..HEAD"),
            ];
            for path in &sentinels {
                if preexisting {
                    std::fs::write(path, b"protected sentinel").unwrap();
                }
            }
            let attack = format!("--output={}", output.display());
            for operand in [attack.as_str(), "--all", "--help", "-p", "--"] {
                let results = [
                    repo.rev_parse(operand),
                    repo.merge_base(operand, "HEAD"),
                    repo.merge_base("HEAD", operand),
                    repo.file_at(operand, "a.txt"),
                    repo.unified_diff(operand, "HEAD", "a.txt"),
                    repo.unified_diff("HEAD", operand, "a.txt"),
                ];
                for path in &sentinels {
                    if preexisting {
                        assert_eq!(std::fs::read(path).unwrap(), b"protected sentinel");
                    } else {
                        assert!(!path.exists(), "unexpected output at {}", path.display());
                    }
                }
                for result in results {
                    assert!(
                        matches!(result, Err(Error::InvalidChangeRevisionInput(value)) if value == operand)
                    );
                }
                assert!(matches!(
                    repo.object_kind_at(operand, "a.txt"),
                    Err(Error::InvalidChangeRevisionInput(value)) if value == operand
                ));
            }
        }
        // Invalid operands must be rejected even if a subprocess could not run
        // in this root, including when the invalid operand is second.
        let missing = GitSourceRepo::new(outside.path().join("absent"));
        assert!(matches!(
            missing.unified_diff("HEAD", "--output=x", "a.txt"),
            Err(Error::InvalidChangeRevisionInput(_))
        ));
        assert!(matches!(
            missing.merge_base("HEAD", "--all"),
            Err(Error::InvalidChangeRevisionInput(_))
        ));
    }

    #[test]
    fn object_resolution_requires_commits_and_peels_annotated_tags() {
        let dir = seed();
        let repo = GitSourceRepo::new(dir.path());
        git(
            dir.path(),
            &["tag", "-a", "release", "-m", "release", "HEAD"],
        );
        let head = repo.head().unwrap().unwrap();
        assert_eq!(repo.rev_parse("release").unwrap(), Some(head.clone()));
        assert_eq!(
            repo.file_at("release", "a.txt").unwrap(),
            repo.file_at(&head, "a.txt").unwrap()
        );
        assert!(
            repo.unified_diff("main", "release", "a.txt")
                .unwrap()
                .is_some()
        );
        for expression in ["HEAD^{tree}", "HEAD:a.txt"] {
            let output = Command::new("git")
                .args(["rev-parse", expression])
                .current_dir(dir.path())
                .env_remove("GIT_DIR")
                .env_remove("GIT_WORK_TREE")
                .env_remove("GIT_INDEX_FILE")
                .output()
                .unwrap();
            assert!(output.status.success());
            let oid = String::from_utf8(output.stdout).unwrap().trim().to_string();
            assert_eq!(oid.len(), 40);
            for rev in [
                oid.as_str(),
                expression,
                "ffffffffffffffffffffffffffffffffffffffff",
            ] {
                assert_eq!(repo.rev_parse(rev).unwrap(), None);
                assert_eq!(repo.file_at(rev, "a.txt").unwrap(), None);
                assert_eq!(repo.unified_diff(rev, "HEAD", "a.txt").unwrap(), None);
                assert_eq!(repo.unified_diff("main", rev, "a.txt").unwrap(), None);
            }
            let tag = if expression.contains("tree") {
                "tree-tag"
            } else {
                "blob-tag"
            };
            git(dir.path(), &["tag", "-a", tag, "-m", tag, &oid]);
            assert_eq!(repo.rev_parse(tag).unwrap(), None);
            assert_eq!(repo.file_at(tag, "a.txt").unwrap(), None);
        }
    }
}

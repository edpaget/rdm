//! Working-tree mirrors for planted-regression builds: the generic half of
//! the negative controls in the `concurrency` binary (its `mutant.rs`) and
//! the `suite_hygiene` binary.
//!
//! A *family* is one named set of planted [`Edit`]s. The checkout is only
//! ever read. The source list is `git ls-files -z --cached --others
//! --exclude-standard` (so uncommitted and untracked-unignored work is
//! mirrored too); the files are mirrored into
//! `$CARGO_TARGET_TMPDIR/rdm-mutants/<family>/src`, with the family's edits
//! applied in memory before the write. A file is written only when its bytes
//! differ, which preserves mtimes so cargo rebuilds incrementally, and mirror
//! files no longer listed are deleted. Whatever builds or runs in the mirror
//! does so under an exclusive lock on `<family>/build.lock` ([`Mirror`] holds
//! it until dropped), so concurrent test processes (two nextest runs, or
//! `cargo test` threads) serialise, and each family has its own
//! `<family>/target` ([`Mirror::target_dir`]).
//!
//! An edit whose anchor does not occur exactly once is a
//! [`Failure::Infra`] naming the family and edit: the negative test fails as
//! not-run, never as a pass. That is fixture construction, like
//! `rdm_devtools::workflow::MutantTree::replace_once`, not an assertion about
//! source text.
//!
//! Included with `#[path = "../common/mirror.rs"] mod mirror;`; it names
//! `crate::plan_fixture` and `crate::workflow_support`.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use crate::plan_fixture::Sandbox;
use crate::workflow_support::{Failure, infra, repo_root};

/// One planted edit.
pub enum Edit {
    /// Replace `from` (which must occur exactly once) with `to`.
    Replace {
        /// The repo-relative file.
        file: &'static str,
        /// A name for error messages.
        name: &'static str,
        /// The anchor.
        from: &'static str,
        /// Its replacement.
        to: &'static str,
    },
    /// Replace the body of the top-level fn whose signature line starts with
    /// `sig` (exactly once, at column 0), up to the next line that is exactly
    /// `}`.
    FnBody {
        /// The repo-relative file.
        file: &'static str,
        /// A name for error messages.
        name: &'static str,
        /// The signature prefix.
        sig: &'static str,
        /// The new body (the lines between the opening and closing braces).
        body: &'static str,
    },
}

impl Edit {
    /// The file the edit applies to.
    pub fn file(&self) -> &'static str {
        match self {
            Self::Replace { file, .. } | Self::FnBody { file, .. } => file,
        }
    }
}

/// Applies `edit` (one of `family`'s) to `text`.
pub fn apply(family: &str, edit: &Edit, text: &str) -> Result<String, Failure> {
    let not_applied = |name: &str, why: String| {
        Failure::Infra(format!(
            "mutant family `{family}` edit `{name}` in {} not applied: {why}",
            edit.file()
        ))
    };
    match edit {
        Edit::Replace { name, from, to, .. } => {
            let count = text.matches(from).count();
            if count != 1 {
                return Err(not_applied(
                    name,
                    format!("its anchor occurs {count} times (expected exactly once): {from:?}"),
                ));
            }
            Ok(text.replacen(from, to, 1))
        }
        Edit::FnBody {
            name, sig, body, ..
        } => {
            let lines: Vec<&str> = text.split_inclusive('\n').collect();
            let starts: Vec<usize> = (0..lines.len())
                .filter(|&i| lines[i].starts_with(sig))
                .collect();
            let [start] = starts[..] else {
                return Err(not_applied(
                    name,
                    format!(
                        "{} lines start with {sig:?} (expected exactly one)",
                        starts.len()
                    ),
                ));
            };
            let open = (start..lines.len())
                .find(|&i| lines[i].trim_end().ends_with('{'))
                .ok_or_else(|| not_applied(name, "the signature never opens a body".into()))?;
            let close = (open + 1..lines.len())
                .find(|&i| lines[i].trim_end() == "}")
                .ok_or_else(|| not_applied(name, "the body never closes at column 0".into()))?;
            let mut out: String = lines[..=open].concat();
            out.push_str(body);
            out.push_str(&lines[close..].concat());
            Ok(out)
        }
    }
}

/// Where `family`'s mirror, target dir and lock live.
pub fn family_root(family: &str) -> PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("rdm-mutants")
        .join(family)
}

/// The checkout's source list, read-only.
fn source_list(scratch: &Path) -> Result<Vec<String>, Failure> {
    let sandbox = Sandbox::new(scratch)?;
    let out = sandbox
        .command("git")
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ])
        .env("GIT_OPTIONAL_LOCKS", "0")
        .current_dir(repo_root())
        .stdin(Stdio::null())
        .output()
        .map_err(infra)?;
    if !out.status.success() {
        return Err(Failure::Infra(format!(
            "git ls-files in the checkout failed: {}",
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .split('\0')
        .filter(|p| !p.is_empty() && !p.starts_with("target/"))
        .map(str::to_owned)
        .collect())
}

/// Reads every listed regular file from the checkout, with `edits` applied.
fn planted_sources(
    family: &str,
    edits: &[Edit],
    scratch: &Path,
) -> Result<BTreeMap<String, Vec<u8>>, Failure> {
    let root = repo_root();
    let mut files = BTreeMap::new();
    for rel in source_list(scratch)? {
        let path = root.join(&rel);
        // `--cached` lists a tracked file deleted from the working tree.
        if !fs::symlink_metadata(&path).is_ok_and(|m| m.is_file()) {
            continue;
        }
        files.insert(rel, fs::read(&path).map_err(infra)?);
    }
    for edit in edits {
        let bytes = files.get(edit.file()).ok_or_else(|| {
            Failure::Infra(format!(
                "mutant family `{family}`: {} is not in the source list",
                edit.file()
            ))
        })?;
        let text = String::from_utf8(bytes.clone()).map_err(infra)?;
        let planted = apply(family, edit, &text)?;
        files.insert(edit.file().to_owned(), planted.into_bytes());
    }
    Ok(files)
}

/// Brings the mirror at `src` to exactly `files`, writing only what differs.
/// Returns (written, removed).
fn sync_mirror(src: &Path, files: &BTreeMap<String, Vec<u8>>) -> Result<(usize, usize), Failure> {
    let mut written = 0;
    for (rel, bytes) in files {
        let dest = src.join(rel);
        if fs::read(&dest).is_ok_and(|b| &b == bytes) {
            continue;
        }
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).map_err(infra)?;
        }
        fs::write(&dest, bytes).map_err(infra)?;
        written += 1;
    }
    let listed: BTreeSet<PathBuf> = files.keys().map(|r| src.join(r)).collect();
    let mut removed = 0;
    let mut stack = vec![src.to_owned()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).map_err(infra)?.flatten() {
            let path = entry.path();
            if entry.file_type().map_err(infra)?.is_dir() {
                stack.push(path);
            } else if !listed.contains(&path) {
                fs::remove_file(&path).map_err(infra)?;
                removed += 1;
            }
        }
    }
    Ok((written, removed))
}

/// A synced family mirror, locked until dropped.
pub struct Mirror {
    family: String,
    root: PathBuf,
    _lock: File,
    /// How long acquiring the family lock took.
    pub lock_wait: Duration,
    /// How many files the mirror holds.
    pub files: usize,
    /// How many of them this sync wrote.
    pub written: usize,
    /// How many stale mirror files this sync removed.
    pub removed: usize,
}

impl Mirror {
    /// Plants `edits` into a mirror of the checkout for `family` and syncs
    /// it, taking the family lock first. `scratch` is a directory the caller
    /// owns (the source listing's git runs in a sandbox there).
    ///
    /// # Errors
    ///
    /// [`Failure::Infra`] if an edit's anchor is not found exactly once, or
    /// the listing, lock or sync fails.
    pub fn prepare(family: &str, edits: &[Edit], scratch: &Path) -> Result<Self, Failure> {
        let started = Instant::now();
        let files = planted_sources(family, edits, scratch)?;
        let root = family_root(family);
        fs::create_dir_all(&root).map_err(infra)?;
        let lock = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(root.join("build.lock"))
            .map_err(infra)?;
        lock.lock().map_err(infra)?;
        let lock_wait = started.elapsed();
        let src = root.join("src");
        fs::create_dir_all(&src).map_err(infra)?;
        let (written, removed) = sync_mirror(&src, &files)?;
        Ok(Self {
            family: family.to_owned(),
            root,
            _lock: lock,
            lock_wait,
            files: files.len(),
            written,
            removed,
        })
    }

    /// The family's name.
    pub fn family(&self) -> &str {
        &self.family
    }

    /// The mirrored source tree (the workspace root of the mutant).
    pub fn src(&self) -> PathBuf {
        self.root.join("src")
    }

    /// The family's own cargo target directory.
    pub fn target_dir(&self) -> PathBuf {
        self.root.join("target")
    }

    /// A one-line summary for diagnostics.
    pub fn describe(&self) -> String {
        format!(
            "mutant family `{}`: lock wait {:.2}s, mirror {} files ({} written, {} removed)",
            self.family,
            self.lock_wait.as_secs_f64(),
            self.files,
            self.written,
            self.removed
        )
    }
}

/// The inherited variables a cargo child inside a mirror must not see: they
/// would redirect its build (flags, target dir, jobserver) away from the
/// family's own.
pub const CARGO_BUILD_REMOVALS: &[&str] = &[
    "CARGO_ENCODED_RUSTFLAGS",
    "CARGO_BUILD_RUSTFLAGS",
    "CARGO_BUILD_TARGET_DIR",
    "CARGO_TARGET_DIR",
    "CARGO_MAKEFLAGS",
    "MAKEFLAGS",
    "MFLAGS",
];

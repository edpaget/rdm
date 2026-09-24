//! Isolated mutant builds of `rdm` for the negative controls in
//! [`crate::mutants`].
//!
//! Each [`Family`] is one build carrying all of that family's planted
//! regressions; within a family every negative scenario exercises exactly one
//! of them (the attribution is recorded in `docs/test-migration-inventory.md`
//! § 9).
//!
//! The checkout is only ever read. The source list is
//! `git ls-files -z --cached --others --exclude-standard` (so uncommitted and
//! untracked-unignored work builds too); the files are mirrored into
//! `$CARGO_TARGET_TMPDIR/rdm-mutants/<family>/src`, with the family's edits
//! applied in memory before the write. A file is written only when its bytes
//! differ, which preserves mtimes so cargo rebuilds incrementally, and mirror
//! files no longer listed are deleted. The build runs there with its own
//! `CARGO_TARGET_DIR`, under an exclusive lock on `<family>/build.lock`, so
//! concurrent test processes (two nextest runs, or `cargo test` threads)
//! serialise. The binary is hard-linked (or copied) into the calling test's
//! temp directory while the lock is still held, so a later rebuild cannot
//! swap the inode under a running test.
//!
//! An edit whose anchor does not occur exactly once is a
//! [`Failure::Infra`] naming the family and edit: the negative test fails as
//! not-run, never as a pass. That is fixture construction, like
//! `rdm_devtools::workflow::MutantTree::replace_once`, not an assertion about
//! source text.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use serde_json::Value;

use crate::git_test_support::repo_redirect_removals;
use crate::plan_fixture::Sandbox;
use crate::workflow_support::{Failure, infra, repo_root};

/// A planted-regression build.
#[derive(Clone, Copy, Debug)]
pub enum Family {
    /// Session identity: the harness check skipped, the continuity advisory
    /// silenced, and the create-path lease sweep removed.
    Session,
    /// The changeset journal: read-modify-write `truncate`, the journal lock
    /// stripped from both sides, and the unlinking `discard_changeset`.
    Journal,
    /// Lost updates: the flush precondition and the commit-time delete guard
    /// both short-circuited.
    LostUpdate,
}

/// One planted edit.
enum Edit {
    /// Replace `from` (which must occur exactly once) with `to`.
    Replace {
        file: &'static str,
        name: &'static str,
        from: &'static str,
        to: &'static str,
    },
    /// Replace the body of the top-level fn whose signature line starts with
    /// `sig` (exactly once, at column 0), up to the next line that is exactly
    /// `}`.
    FnBody {
        file: &'static str,
        name: &'static str,
        sig: &'static str,
        body: &'static str,
    },
}

impl Edit {
    fn file(&self) -> &'static str {
        match self {
            Self::Replace { file, .. } | Self::FnBody { file, .. } => file,
        }
    }
}

/// `truncate` as it was before the append-only fix: read, filter, and
/// rewrite, with the barrier inside the read → write window.
const RMW_TRUNCATE: &str = r#"    let landed_paths: Vec<String> = landed.iter().map(|e| e.path.clone()).collect();
    let remaining: Vec<JournalEntry> = read_journal(paths, id)?
        .into_iter()
        .filter(|e| !landed_paths.contains(&e.path))
        .collect();
    harness_barrier(HARNESS_JOURNAL_BARRIER);
    let path = changeset_path(paths, id);
    if remaining.is_empty() {
        return match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(Error::Io(e)),
        };
    }
    let line = serde_json::to_string(&JournalLine { paths: remaining })
        .map_err(|e| Error::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, e)))?;
    std::fs::write(&path, format!("{line}\n"))?;
    Ok(())
"#;

/// `discard_changeset` as it was before it joined the append protocol: a
/// bare unlink, with the barrier where the fix parks (after the read, before
/// the destruction).
const UNLINK_DISCARD: &str = r#"    harness_barrier(HARNESS_JOURNAL_BARRIER);
    match std::fs::remove_file(changeset_path(paths, id)) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(Error::Io(e)),
    }
"#;

impl Family {
    /// The family's directory name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::Journal => "journal",
            Self::LostUpdate => "lost-update",
        }
    }

    fn edits(self) -> Vec<Edit> {
        match self {
            Self::Session => vec![
                Edit::Replace {
                    file: "rdm-core/src/session/mod.rs",
                    name: "harness check skipped (phase-10 merge bug)",
                    from: "if let Some((var, raw)) = active_harness_var(env) {",
                    to: "if let Some((var, raw)) = active_harness_var(env) && false {",
                },
                Edit::Replace {
                    file: "rdm-core/src/session/mod.rs",
                    name: "continuity advisory silenced",
                    from: "    Some(lines.join(\"\\n\"))\n",
                    to: "    let _ = lines;\n    None\n",
                },
                Edit::Replace {
                    file: "rdm-core/src/session/lease.rs",
                    name: "create-path lease sweep removed",
                    from: "    gc(paths, procs);\n    let lease = Lease {",
                    to: "    let lease = Lease {",
                },
            ],
            Self::Journal => vec![
                Edit::FnBody {
                    file: "rdm-core/src/session/journal.rs",
                    name: "read-modify-write truncate",
                    sig: "pub fn truncate(",
                    body: RMW_TRUNCATE,
                },
                Edit::Replace {
                    file: "rdm-core/src/session/journal.rs",
                    name: "append lock stripped",
                    from: "LockMode::Shared => file.try_lock_shared(),",
                    to: "LockMode::Shared => Ok(()),",
                },
                Edit::Replace {
                    file: "rdm-core/src/session/journal.rs",
                    name: "compaction lock stripped",
                    from: "LockMode::Exclusive => file.try_lock(),",
                    to: "LockMode::Exclusive => Ok(()),",
                },
                Edit::FnBody {
                    file: "rdm-core/src/session/journal.rs",
                    name: "unlinking discard",
                    sig: "pub fn discard_changeset(",
                    body: UNLINK_DISCARD,
                },
            ],
            Self::LostUpdate => vec![
                Edit::Replace {
                    file: "rdm-store-fs/src/lib.rs",
                    name: "flush precondition neutered",
                    from: "if &current != baseline {",
                    to: "if false && &current != baseline {",
                },
                Edit::Replace {
                    file: "rdm-store-git/src/commit.rs",
                    name: "delete guard short-circuited",
                    from: "self.root.join(path).exists()",
                    to: "false",
                },
            ],
        }
    }
}

/// Applies `edit` to `text`.
fn apply(family: Family, edit: &Edit, text: &str) -> Result<String, Failure> {
    let not_applied = |name: &str, why: String| {
        Failure::Infra(format!(
            "mutant family `{}` edit `{name}` in {} not applied: {why}",
            family.name(),
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

/// Where a family's mirror, target dir and lock live.
fn family_root(family: Family) -> PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("rdm-mutants")
        .join(family.name())
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

/// Reads every listed regular file from the checkout, with `family`'s edits
/// applied.
fn planted_sources(family: Family, scratch: &Path) -> Result<BTreeMap<String, Vec<u8>>, Failure> {
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
    for edit in family.edits() {
        let bytes = files.get(edit.file()).ok_or_else(|| {
            Failure::Infra(format!(
                "mutant family `{}`: {} is not in the source list",
                family.name(),
                edit.file()
            ))
        })?;
        let text = String::from_utf8(bytes.clone()).map_err(infra)?;
        let planted = apply(family, &edit, &text)?;
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

/// Builds `family`'s mutant and places its binary in `into` (a directory the
/// calling test owns). Returns the binary's path.
pub fn build(family: Family, into: &Path) -> Result<PathBuf, Failure> {
    let started = Instant::now();
    let files = planted_sources(family, &into.join("mutant-sandbox"))?;
    let root = family_root(family);
    fs::create_dir_all(&root).map_err(infra)?;
    let lock = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(root.join("build.lock"))
        .map_err(infra)?;
    lock.lock().map_err(infra)?;
    let locked = Instant::now();

    let src = root.join("src");
    fs::create_dir_all(&src).map_err(infra)?;
    let (written, removed) = sync_mirror(&src, &files)?;

    let mut cmd = Command::new(env!("CARGO"));
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("RDM_") {
            cmd.env_remove(key);
        }
    }
    for key in repo_redirect_removals() {
        cmd.env_remove(key);
    }
    for key in [
        "CARGO_ENCODED_RUSTFLAGS",
        "CARGO_BUILD_RUSTFLAGS",
        "CARGO_BUILD_TARGET_DIR",
        "CARGO_TARGET_DIR",
        "CARGO_MAKEFLAGS",
        "MAKEFLAGS",
        "MFLAGS",
    ] {
        cmd.env_remove(key);
    }
    let out = cmd
        .args([
            "build",
            "-p",
            "rdm-cli",
            "--bin",
            "rdm",
            "--frozen",
            "--message-format=json-render-diagnostics",
        ])
        .env("CARGO_TARGET_DIR", root.join("target"))
        // A planted edit may leave a warning; `-D warnings` from the caller's
        // environment must not turn that into a not-run control.
        .env("RUSTFLAGS", "")
        // No debuginfo: it dominates the size of a cold dependency graph
        // (three of which live under the target dir), and a mutant is only
        // ever executed, never debugged.
        .env("CARGO_PROFILE_DEV_DEBUG", "0")
        .current_dir(&src)
        .stdin(Stdio::null())
        .output()
        .map_err(infra)?;
    if !out.status.success() {
        return Err(Failure::Infra(format!(
            "mutant family `{}` failed to build:\n{}",
            family.name(),
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    let exe = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|m| m["reason"] == "compiler-artifact" && m["target"]["name"] == "rdm")
        .find_map(|m| m["executable"].as_str().map(PathBuf::from))
        .ok_or_else(|| {
            Failure::Infra(format!(
                "mutant family `{}`: cargo reported no `rdm` executable",
                family.name()
            ))
        })?;
    let dest = into.join(format!("rdm-mutant-{}", family.name()));
    if fs::hard_link(&exe, &dest).is_err() {
        fs::copy(&exe, &dest).map_err(infra)?;
    }
    drop(lock);
    eprintln!(
        "mutant family `{}`: lock wait {:.2}s, mirror {} files ({written} written, {removed} removed), \
         build {:.2}s",
        family.name(),
        (locked - started).as_secs_f64(),
        files.len(),
        locked.elapsed().as_secs_f64(),
    );
    Ok(dest)
}

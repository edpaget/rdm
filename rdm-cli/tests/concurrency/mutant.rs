//! Isolated mutant builds of `rdm` for the negative controls in
//! [`crate::mutants`].
//!
//! Each [`Family`] is one build carrying all of that family's planted
//! regressions; within a family every negative scenario exercises exactly one
//! of them (the attribution is recorded in `docs/test-migration-inventory.md`
//! § 9).
//!
//! The checkout is only ever read: the family's edits are planted into a
//! working-tree mirror ([`crate::mirror`], shared with the `suite_hygiene`
//! binary), and the build runs there with the family's own
//! `CARGO_TARGET_DIR`, under the family lock the [`Mirror`] holds. The binary
//! is hard-linked (or copied) into the calling test's temp directory while the
//! lock is still held, so a later rebuild cannot swap the inode under a
//! running test.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use serde_json::Value;

use crate::git_test_support::repo_redirect_removals;
use crate::mirror::{CARGO_BUILD_REMOVALS, Edit, Mirror};
use crate::workflow_support::{Failure, infra};

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

/// Builds `family`'s mutant and places its binary in `into` (a directory the
/// calling test owns). Returns the binary's path.
pub fn build(family: Family, into: &Path) -> Result<PathBuf, Failure> {
    let mirror = Mirror::prepare(family.name(), &family.edits(), &into.join("mutant-sandbox"))?;
    let started = Instant::now();

    let mut cmd = Command::new(env!("CARGO"));
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("RDM_") {
            cmd.env_remove(key);
        }
    }
    for key in repo_redirect_removals() {
        cmd.env_remove(key);
    }
    for key in CARGO_BUILD_REMOVALS {
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
        .env("CARGO_TARGET_DIR", mirror.target_dir())
        // A planted edit may leave a warning; `-D warnings` from the caller's
        // environment must not turn that into a not-run control.
        .env("RUSTFLAGS", "")
        // No debuginfo: it dominates the size of a cold dependency graph
        // (three of which live under the target dir), and a mutant is only
        // ever executed, never debugged.
        .env("CARGO_PROFILE_DEV_DEBUG", "0")
        .current_dir(mirror.src())
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
    eprintln!(
        "{}, build {:.2}s",
        mirror.describe(),
        started.elapsed().as_secs_f64()
    );
    drop(mirror);
    Ok(dest)
}

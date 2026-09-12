//! Workspace-wide audit: no raw `fs::write`/`std::fs::write` call site may
//! exist in production code outside the three store crates
//! (`rdm-core/src/store/`, `rdm-store-fs/`, `rdm-store-git/`) without being
//! named here with a reason.
//!
//! A raw filesystem write bypasses the plan repo's `Store`, so it is never
//! journaled to a changeset and a scoped `rdm commit` can never land it —
//! exactly the `rdm config set` bug this phase fixes for `rdm.toml`. This
//! test is the regression guard: a newly-introduced raw writer under
//! `$RDM_ROOT` fails it immediately, and a stale allowlist entry (naming a
//! writer that no longer exists) fails it too, so the list stays honest.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Every production (non-test) file that legitimately contains a raw
/// `fs::write`, and why it is exempt from routing through the plan repo's
/// `Store`.
///
/// Each of these writes a location that is either not under `$RDM_ROOT` at
/// all, or is `$RDM_ROOT/.git/`-internal bookkeeping that the `Store` itself
/// could never journal (journaling it would be circular — it records what
/// the `Store` should commit).
const ALLOWED: &[(&str, &str)] = &[
    (
        "rdm-cli/src/paths.rs",
        "save_global_config writes $XDG_CONFIG_HOME, never $RDM_ROOT",
    ),
    (
        "rdm-cli/src/commands/agent_config.rs",
        "write_output writes an arbitrary --out consumer directory, not the plan repo",
    ),
    (
        "rdm-cli/src/commands/hook.rs",
        "installs git hook shims into .git/hooks/, untracked git-internal plumbing",
    ),
    (
        "rdm-core/src/session/journal.rs",
        "truncate writes changeset-journal bookkeeping under $RDM_ROOT/.git/rdm/, deliberately untracked — it records what the Store should commit, so routing it through the Store would be circular",
    ),
    (
        "rdm-core/src/session/lease.rs",
        "create_exclusive/repoint_parent_lease write session lease bookkeeping under $RDM_ROOT/.git/rdm/, deliberately untracked for the same reason as the journal above",
    ),
    (
        "rdm-git/src/worktree.rs",
        "write_marker writes the rdm-item marker into a worktree's .git/worktrees/<name>/ admin dir, git-internal plumbing",
    ),
];

/// Scans `contents` (the text of a single `.rs` file) for every line
/// containing a raw `fs::write(` call, returning `(1-based line number,
/// line text)` for each hit found **outside** any `#[cfg(test)]`-tagged
/// item.
///
/// The skip logic is a brace-depth counter armed the line *after* a
/// test-gating `#[cfg(...)]` attribute line: that next line is expected to
/// open the tagged item's body (`mod tests {` or `fn foo() {`), and every
/// line from there until the matching closing brace is skipped. This covers
/// both a `#[cfg(test)] mod tests { ... }` block and a single `#[cfg(test)]
/// fn foo() { ... }`, without needing a per-line allowlist for fixture
/// writes inside otherwise-production files. A test-gating attribute is not
/// just the bare `#[cfg(test)]` — `#[cfg(all(test, feature = "git"))]` and
/// `#[cfg(any(test, ...))]` also only compile under test — but
/// `#[cfg(not(test))]` means the opposite (production-only) and must never
/// be treated as one.
pub fn scan_file(_path: &str, contents: &str) -> Vec<(usize, String)> {
    let mut hits = Vec::new();
    let mut armed = false;
    let mut skip_depth: i32 = 0;

    let net_braces = |line: &str| -> i32 {
        line.chars().filter(|&c| c == '{').count() as i32
            - line.chars().filter(|&c| c == '}').count() as i32
    };

    for (idx, line) in contents.lines().enumerate() {
        let lineno = idx + 1;

        if skip_depth > 0 {
            skip_depth += net_braces(line);
            if skip_depth < 0 {
                skip_depth = 0;
            }
            continue;
        }

        if armed {
            armed = false;
            skip_depth = net_braces(line).max(0);
            continue;
        }

        if is_cfg_test_line(line) {
            armed = true;
            continue;
        }

        if line.contains("fs::write(") {
            hits.push((lineno, line.to_string()));
        }
    }

    hits
}

/// Returns whether an attribute line only compiles its item under test —
/// `#[cfg(test)]`, `#[cfg(all(test, feature = "git"))]`,
/// `#[cfg(any(test, ...))]` — as opposed to `#[cfg(not(test))]`, which means
/// the opposite (production-only) and must never arm the skip.
fn is_cfg_test_line(line: &str) -> bool {
    let trimmed = line.trim();
    if !trimmed.starts_with("#[cfg(") || !trimmed.ends_with(")]") || trimmed.contains("not(test)") {
        return false;
    }
    trimmed
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .any(|tok| tok == "test")
}

/// The workspace root, derived from this crate's manifest directory
/// (`rdm-core`, a direct sibling of every other workspace member).
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("rdm-core must have a parent directory")
        .to_path_buf()
}

/// Every tracked `*.rs` file in the workspace, excluding the three store
/// crates and any test file, via `git ls-files`.
fn scannable_files(root: &Path) -> Vec<String> {
    let output = std::process::Command::new("git")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .args(["ls-files", "*.rs"])
        .current_dir(root)
        .output()
        .expect("failed to run git ls-files");
    assert!(
        output.status.success(),
        "git ls-files failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_string)
        .filter(|p| {
            !p.starts_with("rdm-core/src/store/")
                && !p.starts_with("rdm-store-fs/")
                && !p.starts_with("rdm-store-git/")
                && !p.contains("/tests/")
        })
        .collect()
}

#[test]
fn no_unlisted_raw_fs_write_outside_store_crates() {
    let root = workspace_root();
    let files = scannable_files(&root);
    assert!(
        files.len() > 10,
        "sanity check: expected many scannable files, got {} — is the workspace root right? ({})",
        files.len(),
        root.display()
    );

    let allowed: BTreeSet<&str> = ALLOWED.iter().map(|(path, _)| *path).collect();
    let mut hit_files: BTreeSet<String> = BTreeSet::new();
    let mut offenders: Vec<String> = Vec::new();

    for rel_path in &files {
        let full_path = root.join(rel_path);
        let Ok(contents) = std::fs::read_to_string(&full_path) else {
            continue;
        };
        let hits = scan_file(rel_path, &contents);
        if hits.is_empty() {
            continue;
        }
        hit_files.insert(rel_path.clone());
        if !allowed.contains(rel_path.as_str()) {
            for (lineno, line) in &hits {
                offenders.push(format!("{rel_path}:{lineno}: {}", line.trim()));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "found raw fs::write outside the allowlist — route through the Store, or add a \
         reasoned ALLOWED entry in rdm-core/tests/no_raw_fs_write_audit.rs:\n{}",
        offenders.join("\n")
    );

    // The other direction: every ALLOWED entry must still have a real hit,
    // so the list can never silently go stale (naming a writer that has
    // since been rerouted through the Store, like `save_repo_config` was).
    let mut stale: Vec<&str> = Vec::new();
    for (path, _reason) in ALLOWED {
        if !hit_files.contains(*path) {
            stale.push(path);
        }
    }
    assert!(
        stale.is_empty(),
        "ALLOWED lists a raw fs::write site with no real hit — remove the stale entry: {stale:?}"
    );
}

#[test]
fn scan_file_detects_a_production_fs_write() {
    let src = "fn write_it() {\n    std::fs::write(&path, data)?;\n    Ok(())\n}\n";
    let hits = scan_file("synthetic.rs", src);
    assert_eq!(
        hits,
        vec![(2, "    std::fs::write(&path, data)?;".to_string())]
    );
}

#[test]
fn scan_file_ignores_a_write_inside_a_cfg_test_mod() {
    let src = "fn production() {\n    // no write here\n}\n\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn seed() {\n        std::fs::write(&path, b\"x\").unwrap();\n    }\n}\n";
    let hits = scan_file("synthetic.rs", src);
    assert!(
        hits.is_empty(),
        "expected the cfg(test) mod's write to be skipped, got {hits:?}"
    );
}

#[test]
fn scan_file_ignores_a_write_inside_a_bare_cfg_test_fn() {
    let src = "#[cfg(test)]\nfn seed_fixture() {\n    std::fs::write(&path, b\"x\").unwrap();\n}\n\nfn production() {\n    std::fs::write(&other, b\"y\").unwrap();\n}\n";
    let hits = scan_file("synthetic.rs", src);
    assert_eq!(
        hits,
        vec![(
            7,
            "    std::fs::write(&other, b\"y\").unwrap();".to_string()
        )],
        "expected only the production write outside the bare cfg(test) fn, got {hits:?}"
    );
}

#[test]
fn scan_file_does_not_skip_a_cfg_not_test_write() {
    // `#[cfg(not(test))]` means the opposite of `#[cfg(test)]` — the item
    // compiles ONLY outside test builds — so a raw fs::write behind it is
    // exactly the production-only write this audit exists to catch. It must
    // never be treated as a test-only skip span.
    let src = "#[cfg(not(test))]\nfn production_only() {\n    std::fs::write(&path, b\"x\").unwrap();\n}\n";
    let hits = scan_file("synthetic.rs", src);
    assert_eq!(
        hits,
        vec![(3, "    std::fs::write(&path, b\"x\").unwrap();".to_string())],
        "expected the #[cfg(not(test))]-guarded write to be reported, not skipped, got {hits:?}"
    );
}

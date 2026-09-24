//! Re-emitting into a stale downstream tree removes the workflow engines rdm
//! once shipped and has since renamed or retired, through both the
//! `--skills` and the `--plugin` adapter, and leaves everything else alone.
//!
//! The cleanup is gated by content fingerprint (`SUPERSEDED_WORKFLOWS` in
//! `rdm-core/src/agent_config.rs` records the SHA-256 of every body each
//! retired path ever held), so the seeds must be genuine historical bodies.
//! They are committed under `tests/fixtures/superseded-workflows/` — one
//! recorded body per retired name, extracted once with `git show`; the
//! source commit and digest of each are recorded in
//! `docs/test-migration-inventory.md` § 8. No test reads the checkout's git
//! history, so a shallow clone runs this. A seed that were not a genuine
//! body would survive the re-emit and fail the removal assertion; the
//! negative control plants exactly that — a genuine body with one byte
//! appended, under a retired name — and requires it to survive.

use std::path::Path;

use crate::support::{Emit, cleanup_report, entries, repo_root};

/// Each retired engine name and its committed genuine body.
const SUPERSEDED: [&str; 4] = [
    "dispatch-phase.js",
    "review-refute-fix.js",
    "autopilot.js",
    "rdm-wf-dispatch-phase.js",
];

/// Files no fingerprint covers: a name the table does not carry, and a
/// user-authored file.
const SURVIVORS: [(&str, &str); 2] = [
    (
        "not-superseded.js",
        "export const meta = { name: \"not-superseded\" };\n",
    ),
    (
        "custom-local.js",
        "my own local engine, rdm did not write this\n",
    ),
];

fn fixture_body(name: &str) -> Vec<u8> {
    let path = repo_root()
        .join("tests/fixtures/superseded-workflows")
        .join(format!("{name}.body"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Seeds `dir` with every superseded body plus the survivors.
fn seed(dir: &Path) {
    std::fs::create_dir_all(dir).expect("create workflows dir");
    for name in SUPERSEDED {
        std::fs::write(dir.join(name), fixture_body(name)).expect("seed a superseded body");
    }
    for (name, text) in SURVIVORS {
        std::fs::write(dir.join(name), text).expect("seed a survivor");
    }
    let seeded = entries(dir);
    for name in SUPERSEDED.iter().chain(SURVIVORS.iter().map(|(n, _)| n)) {
        assert!(seeded.contains(*name), "{name} is present before the emit");
    }
}

/// The removal, survivor and byte-identity checks shared by both adapters.
fn assert_cleaned(workflows: &Path, report: &[String]) {
    let left = entries(workflows);
    for name in SUPERSEDED {
        assert!(
            !left.contains(name),
            "the superseded {name} survived the re-emit: {left:?}\nreport: {report:?}"
        );
    }
    for (name, text) in SURVIVORS {
        assert_eq!(
            std::fs::read_to_string(workflows.join(name))
                .ok()
                .as_deref(),
            Some(text),
            "{name} is not the cleanup's to touch"
        );
    }
    let source = repo_root().join(".claude/workflows");
    let shipped: Vec<String> = left
        .into_iter()
        .filter(|n| !SURVIVORS.iter().any(|(s, _)| s == n))
        .collect();
    assert!(!shipped.is_empty(), "the current engines were emitted");
    for name in shipped {
        assert_eq!(
            std::fs::read(workflows.join(&name)).ok(),
            std::fs::read(source.join(&name)).ok(),
            "{name} is byte-identical to this checkout's copy after the cleanup emit"
        );
    }
    assert!(
        report.iter().any(|l| l.starts_with("Removed ")),
        "the emit reports its removals: {report:?}"
    );
}

#[test]
fn skills_reemit_removes_fingerprinted_orphans() {
    let emit = Emit::new();
    let workflows = emit.path("stale/.claude/workflows");
    seed(&workflows);
    let (_, out) = emit.skills("stale");
    assert_cleaned(&workflows, &cleanup_report(&out));
}

#[test]
fn plugin_reemit_removes_fingerprinted_orphans() {
    let emit = Emit::new();
    let workflows = emit.path("stale-plugin/workflows");
    seed(&workflows);
    let (root, out) = emit.plugin("stale-plugin");
    assert_cleaned(&workflows, &cleanup_report(&out));
    assert!(
        root.join(".claude-plugin/plugin.json").is_file(),
        "the cleanup does not disturb the primary emit"
    );
}

#[test]
fn a_tampered_superseded_body_survives_the_cleanup() {
    let emit = Emit::new();
    let workflows = emit.path("tampered/.claude/workflows");
    std::fs::create_dir_all(&workflows).expect("create workflows dir");
    let mut tampered = fixture_body("autopilot.js");
    tampered.push(b'\n');
    std::fs::write(workflows.join("autopilot.js"), &tampered).expect("plant");
    let (_, out) = emit.skills("tampered");
    assert_eq!(
        std::fs::read(workflows.join("autopilot.js")).ok(),
        Some(tampered),
        "a body no fingerprint matches is kept, even under a retired name"
    );
    assert!(
        cleanup_report(&out)
            .iter()
            .all(|l| !l.starts_with("Removed ")),
        "{:?}",
        cleanup_report(&out)
    );
}

//! The review core's two generated projections — the stamped workflow block
//! and the rendered skill prose — gated by running the real generators, plus
//! byte identity of the shipped engine copies.

use std::path::Path;
use std::process::{Command, Output};

use rdm_devtools::workflow::MutantTree;

use crate::support::repo_root;

const WORKFLOW_GEN: &str = "scripts/gen-workflow-review.sh";
const SKILL_GEN: &str = "scripts/gen-skill-review.sh";

const WORKFLOW_TREE: &[&str] = &[
    WORKFLOW_GEN,
    "scripts/lib/gen-workflow-block.sh",
    ".claude/workflows/lib/review.mjs",
    ".claude/workflows/lib/plan-review.mjs",
    ".claude/workflows/rdm-wf-review-refute-fix.js",
    ".claude/workflows/rdm-wf-plan-review.js",
    "rdm-core/src/templates/workflows/rdm-wf-review-refute-fix.js",
    "rdm-core/src/templates/workflows/rdm-wf-plan-review.js",
    "plugins/rdm/workflows/rdm-wf-review-refute-fix.js",
    "plugins/rdm/workflows/rdm-wf-plan-review.js",
];

const SKILL_TREE: &[&str] = &[
    SKILL_GEN,
    ".claude/workflows/lib/review.mjs",
    "rdm-core/src/templates/skill-review-cli.md",
    "rdm-core/src/templates/skill-plan-review-cli.md",
    ".claude/skills/rdm-review/SKILL.md",
    ".claude/skills/rdm-plan-review/SKILL.md",
];

fn sh(root: &Path, script: &str, args: &[&str]) -> Output {
    Command::new("sh")
        .arg(root.join(script))
        .args(args)
        .current_dir(root)
        .output()
        .unwrap_or_else(|e| panic!("running {script}: {e}"))
}

fn assert_ok(out: &Output, what: &str) {
    assert!(
        out.status.success(),
        "{what} failed ({:?}):\n{}{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn scratch(files: &[&str]) -> MutantTree {
    MutantTree::copy(&repo_root(), files).expect("copy scratch tree")
}

/// Inserts `extra` as a new line right after the first line containing
/// `marker` in `file`.
fn insert_after_marker(root: &Path, file: &str, marker: &str, extra: &str) {
    let path = root.join(file);
    let text = std::fs::read_to_string(&path).expect("read");
    let at = text
        .find(marker)
        .unwrap_or_else(|| panic!("{file} has no {marker:?} marker"));
    let eol = at + text[at..].find('\n').expect("marker line ends");
    let mutated = format!("{}\n{extra}{}", &text[..eol], &text[eol..]);
    std::fs::write(&path, mutated).expect("write");
}

/// Appends `suffix` to the first line starting with `prefix` at or after the
/// first occurrence of `after` (or the file start).
fn edit_line(root: &Path, file: &str, after: Option<&str>, prefix: &str, suffix: &str) {
    let path = root.join(file);
    let text = std::fs::read_to_string(&path).expect("read");
    let from = after.map_or(0, |m| {
        text.find(m)
            .unwrap_or_else(|| panic!("{file} has no {m:?}"))
    });
    let mut offset = from;
    let line_end = loop {
        let rest = &text[offset..];
        let nl = rest.find('\n').expect("a line after the anchor") + offset;
        let line_start = text[..offset].rfind('\n').map_or(0, |i| i + 1);
        if offset == line_start && text[line_start..nl].starts_with(prefix) {
            break nl;
        }
        offset = nl + 1;
    };
    let mutated = format!("{}{suffix}{}", &text[..line_end], &text[line_end..]);
    std::fs::write(&path, mutated).expect("write");
}

#[test]
fn review_block_in_sync_in_every_consumer() {
    let out = sh(&repo_root(), WORKFLOW_GEN, &["--check"]);
    assert_ok(&out, "gen-workflow-review.sh --check on the real tree");
}

#[test]
fn review_block_drift_detected_then_healed() {
    let tree = scratch(WORKFLOW_TREE);
    let root = tree.root();
    assert_ok(
        &sh(root, WORKFLOW_GEN, &["--check"]),
        "scratch --check on a clean copy",
    );
    let consumer = ".claude/workflows/rdm-wf-review-refute-fix.js";
    let original = std::fs::read(root.join(consumer)).unwrap();
    insert_after_marker(
        root,
        consumer,
        ">>> review-refute-fix:begin",
        "const PLANTED_DRIFT = 1;",
    );
    let drifted = sh(root, WORKFLOW_GEN, &["--check"]);
    assert!(
        !drifted.status.success(),
        "--check must fail on planted drift inside the block"
    );
    assert_ok(&sh(root, WORKFLOW_GEN, &[]), "regenerate");
    assert_ok(
        &sh(root, WORKFLOW_GEN, &["--check"]),
        "--check after regenerating",
    );
    assert_eq!(
        std::fs::read(root.join(consumer)).unwrap(),
        original,
        "regeneration restores the exact bytes"
    );
}

#[test]
fn skill_projection_in_sync_code() {
    assert_ok(
        &sh(&repo_root(), SKILL_GEN, &["--check", "--mode", "code"]),
        "shipped code skill --check",
    );
}

#[test]
fn skill_projection_in_sync_plan() {
    assert_ok(
        &sh(&repo_root(), SKILL_GEN, &["--check", "--mode", "plan"]),
        "shipped plan skill --check",
    );
}

#[test]
fn skill_projection_drift_detected_then_healed() {
    let tree = scratch(SKILL_TREE);
    let root = tree.root();
    for mode in ["code", "plan"] {
        assert_ok(
            &sh(root, SKILL_GEN, &["--check", "--mode", mode]),
            "scratch --check on a clean copy",
        );
    }
    // The first shared `//| ` spec line renders into both modes.
    edit_line(
        root,
        ".claude/workflows/lib/review.mjs",
        None,
        "//| ",
        " (planted drift)",
    );
    for mode in ["code", "plan"] {
        let drifted = sh(root, SKILL_GEN, &["--check", "--mode", mode]);
        assert!(
            !drifted.status.success(),
            "--check --mode {mode} must fail on planted source drift"
        );
        assert_ok(&sh(root, SKILL_GEN, &["--mode", mode]), "regenerate");
        assert_ok(
            &sh(root, SKILL_GEN, &["--check", "--mode", mode]),
            "--check after regenerating",
        );
    }
}

#[test]
fn local_skill_projection_in_sync_code() {
    assert_ok(
        &sh(
            &repo_root(),
            SKILL_GEN,
            &["--check", "--target", "local", "--mode", "code"],
        ),
        "local code skill --check",
    );
}

#[test]
fn local_skill_projection_in_sync_plan() {
    assert_ok(
        &sh(
            &repo_root(),
            SKILL_GEN,
            &["--check", "--target", "local", "--mode", "plan"],
        ),
        "local plan skill --check",
    );
}

#[test]
fn unknown_target_rejected() {
    let tree = scratch(SKILL_TREE);
    let out = sh(tree.root(), SKILL_GEN, &["--target", "bogus"]);
    assert!(
        !out.status.success(),
        "an unknown --target must be rejected"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("bogus"),
        "the refusal names the target: {stderr}"
    );
}

fn local_consumer_edit(mode: &str, skill: &str) {
    let tree = scratch(SKILL_TREE);
    let root = tree.root();
    let args = ["--check", "--target", "local", "--mode", mode];
    assert_ok(
        &sh(root, SKILL_GEN, &args),
        "scratch local --check on a clean copy",
    );
    let original = std::fs::read(root.join(skill)).unwrap();
    insert_after_marker(
        root,
        skill,
        "<!-- rdm:review-spec:begin",
        "A hand-patched line.",
    );
    assert!(
        !sh(root, SKILL_GEN, &args).status.success(),
        "a consumer-side edit is detected ({mode})"
    );
    assert_ok(
        &sh(root, SKILL_GEN, &["--target", "local", "--mode", mode]),
        "regenerate",
    );
    assert_ok(&sh(root, SKILL_GEN, &args), "--check after regenerating");
    assert_eq!(
        std::fs::read(root.join(skill)).unwrap(),
        original,
        "regeneration restores the exact bytes"
    );
}

#[test]
fn local_consumer_edit_detected_then_healed_code() {
    local_consumer_edit("code", ".claude/skills/rdm-review/SKILL.md");
}

#[test]
fn local_consumer_edit_detected_then_healed_plan() {
    local_consumer_edit("plan", ".claude/skills/rdm-plan-review/SKILL.md");
}

#[test]
fn local_override_does_not_leak_into_shipped_render() {
    let tree = scratch(SKILL_TREE);
    let root = tree.root();
    let local = ".claude/skills/rdm-review/SKILL.md";
    let shipped = "rdm-core/src/templates/skill-review-cli.md";
    let local_before = std::fs::read(root.join(local)).unwrap();
    let shipped_before = std::fs::read(root.join(shipped)).unwrap();
    edit_line(
        root,
        ".claude/workflows/lib/review.mjs",
        Some(">>> find-refute-verdict:local-code-override:begin"),
        "//|",
        " (planted override change)",
    );
    assert_ok(
        &sh(root, SKILL_GEN, &["--target", "local", "--mode", "code"]),
        "render local/code",
    );
    assert_ne!(
        std::fs::read(root.join(local)).unwrap(),
        local_before,
        "the local-code-override region is consumed by the local code render"
    );
    assert_ok(
        &sh(root, SKILL_GEN, &["--target", "shipped", "--mode", "code"]),
        "render shipped/code",
    );
    assert_eq!(
        std::fs::read(root.join(shipped)).unwrap(),
        shipped_before,
        "the override never leaks into the shipped render"
    );
}

#[test]
fn shipped_workflow_templates_match_local_engines() {
    let root = repo_root();
    let dir = root.join("rdm-core/src/templates/workflows");
    let mut seen = 0;
    for entry in std::fs::read_dir(&dir).expect("templates/workflows") {
        let path = entry.expect("dir entry").path();
        if path.extension().is_none_or(|e| e != "js") {
            continue;
        }
        let name = path.file_name().expect("file name");
        let local = root.join(".claude/workflows").join(name);
        assert_eq!(
            std::fs::read(&path).expect("read shipped"),
            std::fs::read(&local).unwrap_or_else(|e| panic!("{}: {e}", local.display())),
            "{} drifted from its local engine",
            name.to_string_lossy()
        );
        seen += 1;
    }
    assert!(
        seen >= 1,
        "the shipped template directory holds at least one engine"
    );
}

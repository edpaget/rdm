//! Repo-level gate for the plan-review driver's two copies, on
//! `cargo nextest run`.
//!
//! `.claude/workflows/lib/plan-review.mjs` is the single source of truth for
//! the plan-review driver; `.claude/workflows/rdm-wf-plan-review.js` carries a
//! BYTE-IDENTICAL copy of its `plan-review-driver` block, hand-mirrored rather
//! than generator-stamped. Nothing else in the Rust test suite notices when
//! those two drift, so an edit to the lib can silently leave the shipped engine
//! on the old behavior.
//!
//! The test compares the two real artifacts for identity; it greps neither
//! file for a string it hopes to find. The driver's behaviour — the lib and
//! the shipped engine, executed — is `rdm-cli/tests/workflow_review/`
//! `plan_driver.rs` (`plan_driver::*`).

use std::path::{Path, PathBuf};

const BEGIN_MARKER: &str = ">>> plan-review-driver:begin";
const END_MARKER: &str = ">>> plan-review-driver:end";

const LIB: &str = ".claude/workflows/lib/plan-review.mjs";
const WORKFLOW: &str = ".claude/workflows/rdm-wf-plan-review.js";

/// The workspace root — the parent of `rdm-core/`.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("rdm-core has a parent directory")
        .to_path_buf()
}

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// The text strictly between the begin and end markers, plus the 1-based line
/// number the block starts on (so a diff report can name real file lines).
fn extract_block(source: &str, rel: &str) -> (Vec<String>, usize) {
    let mut lines: Vec<String> = Vec::new();
    let mut start_line = 0usize;
    let mut inside = false;
    for (idx, line) in source.lines().enumerate() {
        if !inside && line.contains(BEGIN_MARKER) {
            inside = true;
            start_line = idx + 2; // the line AFTER the marker, 1-based
            continue;
        }
        if inside && line.contains(END_MARKER) {
            break;
        }
        if inside {
            lines.push(line.to_string());
        }
    }
    assert!(
        !lines.is_empty(),
        "{rel}: the plan-review-driver block is EMPTY — a renamed or mistyped \
         marker would make a byte-equality check pass vacuously, so this is a \
         failure, not a skip"
    );
    (lines, start_line)
}

#[test]
fn plan_review_driver_block_is_byte_identical() {
    let lib_src = read(LIB);
    let wf_src = read(WORKFLOW);

    let (lib_block, lib_start) = extract_block(&lib_src, LIB);
    let (wf_block, wf_start) = extract_block(&wf_src, WORKFLOW);

    if lib_block != wf_block {
        let mut report = String::new();
        let max = lib_block.len().max(wf_block.len());
        for i in 0..max {
            let a = lib_block.get(i);
            let b = wf_block.get(i);
            if a != b {
                report.push_str(&format!(
                    "first difference:\n  {}:{}\n    {}\n  {}:{}\n    {}\n",
                    LIB,
                    lib_start + i,
                    a.map(String::as_str).unwrap_or("<end of block>"),
                    WORKFLOW,
                    wf_start + i,
                    b.map(String::as_str).unwrap_or("<end of block>"),
                ));
                break;
            }
        }
        panic!(
            "the plan-review-driver block has DRIFTED between its two copies \
             ({} lines in the lib, {} in the workflow).\n{report}\n\
             The lib is the source of truth: apply the identical replacement \
             strings to {WORKFLOW}'s block. This copy is hand-mirrored, not \
             generator-stamped.",
            lib_block.len(),
            wf_block.len(),
        );
    }
}

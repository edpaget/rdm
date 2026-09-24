//! Repo-level gate for the review pipeline's reasoning-effort threading, on
//! `cargo nextest run`.
//!
//! The dispatch lane resolves `rdm model resolve review-find|review-verify
//! --format json` into a `{model, effort}` profile and hands the pair to the
//! review engines. `scripts/lib/review-effort.test.mjs` drives the real
//! `buildReviewPipeline` (both modes) and the real plan-review driver with a
//! recording fake agent and asserts that `effort` reaches every finder
//! (including its retry) and every refuter `agent()` call when supplied, is
//! absent when not, and is refused before any dispatch when invalid. This test
//! is what makes `cargo nextest run` the gate for it.
//!
//! See phase model-effort-profiles/phase-6-thread-effort-through-the-lane.

use std::path::{Path, PathBuf};
use std::process::Command;

const NODE_TEST: &str = "scripts/lib/review-effort.test.mjs";

/// The workspace root — the parent of `rdm-core/`.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("rdm-core has a parent directory")
        .to_path_buf()
}

/// Resolve a usable `node`: first on `PATH`, else through `mise exec` (the
/// version pinned in `.mise.toml`). Never a silent skip — an unrunnable gate
/// that reports success is worse than a red one.
fn node_command() -> Command {
    if Command::new("node")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return Command::new("node");
    }
    let mise_ok = Command::new("mise")
        .args(["exec", "node", "--", "node", "--version"])
        .current_dir(repo_root())
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if mise_ok {
        let mut cmd = Command::new("mise");
        cmd.args(["exec", "node", "--", "node"]);
        return cmd;
    }
    panic!(
        "no usable `node` found. This test executes {NODE_TEST}, which decides \
         whether the resolved effort reaches every review agent. Install \
         the pinned toolchain with `mise install` (node is pinned in \
         .mise.toml) or put `node` on PATH."
    );
}

#[test]
fn workflow_review_effort_contract() {
    let root = repo_root();
    let output = node_command()
        .args(["--test", NODE_TEST])
        .current_dir(&root)
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn node for {NODE_TEST}: {e}"));

    if !output.status.success() {
        panic!(
            "`node --test {NODE_TEST}` failed ({}).\n--- stdout ---\n{}\n--- stderr ---\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }
}

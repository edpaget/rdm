//! Repo-level gate for the backlog and document engines' environment-arg
//! contract, on `cargo nextest run`.
//!
//! Both engines used to name this repo's own dogfood build path and project in
//! every command they build or hand to an agent, which made them unshippable.
//! `agent-orchestrated-dispatch` phase 26 moved both onto runtime arguments and
//! registered the engines for emission; `scripts/lib/workflow-env-args.test.mjs`
//! decides that contract by EXECUTING the engines' own builders, and this test
//! is what makes `cargo nextest run` the gate for it.
//!
//! The plan-review engine's half of the same contract is driven by
//! `workflow_plan_review_driver.rs`, which already runs its own Node suite.

use std::path::{Path, PathBuf};
use std::process::Command;

const NODE_TEST: &str = "scripts/lib/workflow-env-args.test.mjs";

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
         the backlog and document engines' environment-arg contract. Install \
         the pinned toolchain with `mise install` (node is pinned in \
         .mise.toml) or put `node` on PATH."
    );
}

#[test]
fn workflow_env_args_contract() {
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

//! Runs the code engine's emitted gate ladder against the real `rdm` binary, on
//! `cargo nextest run`.
//!
//! `rdm-wf-review-refute-fix` is the one Workflow engine distributed downstream
//! (`rdm-core/src/templates/workflows/` and `plugins/rdm/workflows/` carry
//! byte-identical copies). Its standalone `mode: 'code'` path no longer writes a
//! status through an agent: it hands the caller `gateCommands` / `gateScript` as
//! text. That makes a wrong flag, a wrong target, or an outcome mapped to the
//! wrong status cheap to emit and silent until some downstream consumer runs the
//! commands for real — so something has to run them for real here.
//!
//! The behavior lives in [`NODE_TEST`], which seeds a plan repo and a source
//! repo with the binary this test builds, registers the worktrees the pinned
//! identity names, drives the real engine under a fake reviewer fleet, executes
//! the ladder it returns, and asserts on the plan state afterwards. This file is
//! the runner: it locates a usable `node`, supplies the freshly built binary and
//! two empty temp directories, and reports the child's output on failure. It
//! greps no source text.

use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

const NODE_TEST: &str = "scripts/lib/review-driver.test.mjs";

/// The workspace root — the parent of `rdm-cli/`.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("rdm-cli has a parent directory")
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
        "no usable `node` found. This test executes {NODE_TEST}, which runs the code engine's \
         emitted gate ladder against the real rdm binary. Install the pinned toolchain with \
         `mise install` (node is pinned in .mise.toml) or put `node` on PATH."
    );
}

#[test]
fn review_driver_gate_commands_do_what_they_claim() {
    let root = repo_root();
    // Rust owns both temp trees, so they are removed even if the node child dies
    // partway through. The source repo gets worktrees placed at
    // `<parent>/<name>__worktrees/`, so it is nested one level inside its own
    // TempDir — otherwise the worktrees land where `TempDir::drop` never reaches
    // (see scripts/verify-worktree-temp-hygiene.sh).
    let plan = TempDir::new().expect("a temp plan repo");
    let src_parent = TempDir::new().expect("a temp source repo parent");
    let src = src_parent.path().join("src");

    let output = node_command()
        .args(["--test", NODE_TEST])
        .current_dir(&root)
        // The binary under test — the one cargo just built for this crate, never
        // an installed `rdm`.
        .env("RDM_REVIEW_TEST_BIN", env!("CARGO_BIN_EXE_rdm"))
        .env("RDM_REVIEW_TEST_ROOT", plan.path())
        .env("RDM_REVIEW_TEST_SRC", &src)
        // A developer's ambient plan repo must not leak in: the emitted commands
        // carry no `--root`, and their project flag is what has to scope them.
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
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

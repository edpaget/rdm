//! End-to-end CLI loops driven through the real `rdm` binary, each in its own
//! sandboxed temp tree: the editor/plugin argv and JSON contract
//! ([`plugin_loop`]), the Claude Code web bootstrap and `Done:` loop
//! ([`claude_code_web`]), backlog grooming ([`backlog_groom`]), the document
//! review revision loop ([`review_revision`]) and the one-worktree-per-roadmap
//! review scoping ([`worktree_review`]).
//!
//! Rust owns every fixture, command sequence, assertion and teardown; the
//! only script executed is the shipped `SessionStart.sh` template. Ported
//! from the retired `scripts/verify-{plugin-loop,claude-code-web-loop,`
//! `backlog-groom-loop,review-revision-loop,worktree-review-loop}.sh`; see
//! `docs/test-migration-inventory.md` § 8.

#[macro_use]
#[path = "../common/workflow_support.rs"]
mod workflow_support;

#[path = "../git_test_support.rs"]
mod git_test_support;

#[path = "../common/plan_fixture.rs"]
mod plan_fixture;

#[path = "../common/seeded_plan.rs"]
mod seeded_plan;

mod backlog_groom;
mod claude_code_web;
mod plugin_loop;
mod review_revision;
mod worktree_review;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;
use tempfile::TempDir;

use crate::plan_fixture::Sandbox;
use crate::workflow_support::Failure;

/// Unwraps a fixture result, panicking with the failure.
pub fn must<T>(r: Result<T, Failure>) -> T {
    r.unwrap_or_else(|f| panic!("{f}"))
}

/// Stdout of `out` as text.
pub fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Stderr of `out` as text.
pub fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// A bare plan repo at `<TempDir>/plan` with a sandbox beside it, driven with
/// `rdm --root <plan>` in a fixed session.
pub struct Plan {
    /// The owning temp directory.
    pub dir: TempDir,
    /// The plan repo root.
    pub root: PathBuf,
    /// The user context every process runs under.
    pub sandbox: Sandbox,
}

impl Plan {
    /// An empty temp tree; the caller runs `init`.
    pub fn empty() -> Self {
        let dir = TempDir::new().expect("tempdir");
        let root = dir.path().join("plan");
        std::fs::create_dir_all(&root).expect("create plan dir");
        let sandbox = must(Sandbox::new(dir.path()));
        Self { dir, root, sandbox }
    }

    /// `rdm --root <plan> <args>` from `cwd`.
    pub fn command_in(&self, cwd: &Path, args: &[&str]) -> Command {
        let mut cmd = self.sandbox.rdm();
        cmd.arg("--root")
            .arg(&self.root)
            .args(args)
            .env("RDM_SESSION", "cli-loop")
            .current_dir(cwd);
        cmd
    }

    /// Runs `rdm <args>` from `cwd`, whatever the status.
    pub fn run_in(&self, cwd: &Path, args: &[&str]) -> Output {
        self.command_in(cwd, args).output().expect("spawn rdm")
    }

    /// Runs `rdm <args>` from `cwd`, requiring success; returns stdout.
    pub fn ok_in(&self, cwd: &Path, args: &[&str]) -> String {
        let out = self.run_in(cwd, args);
        assert!(
            out.status.success(),
            "`rdm {}` failed: {}",
            args.join(" "),
            stderr(&out)
        );
        stdout(&out)
    }

    /// Runs `rdm <args>` from the temp root, requiring success.
    pub fn ok(&self, args: &[&str]) -> String {
        self.ok_in(self.dir.path(), args)
    }

    /// Runs `rdm <args>` and parses stdout as JSON.
    pub fn json(&self, args: &[&str]) -> Value {
        let text = self.ok(args);
        serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("`rdm {}` json: {e}: {text}", args.join(" ")))
    }

    /// Runs `rdm <args>` from `cwd` and parses stdout as JSON.
    pub fn json_in(&self, cwd: &Path, args: &[&str]) -> Value {
        let text = self.ok_in(cwd, args);
        serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("`rdm {}` json: {e}: {text}", args.join(" ")))
    }

    /// Runs `rdm <args>` feeding `body` on stdin, requiring success.
    pub fn ok_stdin(&self, args: &[&str], body: &str) -> String {
        use std::io::Write;
        let mut child = self
            .command_in(self.dir.path(), args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("spawn rdm");
        child
            .stdin
            .take()
            .expect("stdin")
            .write_all(body.as_bytes())
            .expect("write stdin");
        let out = child.wait_with_output().expect("wait rdm");
        assert!(
            out.status.success(),
            "`rdm {}` failed: {}",
            args.join(" "),
            stderr(&out)
        );
        stdout(&out)
    }

    /// `git <args>` in `dir` under the sandbox; returns trimmed stdout.
    pub fn git(&self, dir: &Path, args: &[&str]) -> String {
        must(self.sandbox.git(dir, args)).trim().to_owned()
    }

    /// The plan repo's HEAD.
    pub fn head(&self) -> String {
        self.git(&self.root, &["rev-parse", "HEAD"])
    }
}

/// The objects of a JSON array (or empty).
pub fn items(v: &Value) -> Vec<Value> {
    v.as_array().cloned().unwrap_or_default()
}

/// The string values of `key` across a JSON array.
pub fn field(v: &Value, key: &str) -> Vec<String> {
    items(v)
        .iter()
        .filter_map(|i| i[key].as_str().map(str::to_owned))
        .collect()
}

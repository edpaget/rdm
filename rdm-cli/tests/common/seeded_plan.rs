//! A deterministic, seeded plan repo for the CLI-contract tests: the Rust
//! port of the retired `scripts/lib/rdm-plan-fixture.sh`, used by the
//! `golden_json` and `cli_loops` binaries.
//!
//! [`SeededPlan::new`] builds `<TempDir>/plan` through `CARGO_BIN_EXE_rdm
//! --root <plan>` with one seed commit: roadmap `sample-roadmap` with three
//! phases (done, in-progress, not-started) and the tasks
//! `fixture-task-{open,active,done}` (tagged `bug`/`ui`/`bug`, statuses
//! open/in-progress/done). [`SeededPlan::with_code_repo`] adds a git repo at
//! `<TempDir>/code` on `main` with one empty commit, so `rdm worktree` has a
//! base; its `code__worktrees/` sibling stays inside the same `TempDir`.
//!
//! Every process runs under a [`Sandbox`] rooted in the same `TempDir`
//! (`home`, `xdg-config`, `xdg-data`, `xdg-state`), with every inherited
//! `RDM_*` removed, an explicit `--root`, a fixed `RDM_SESSION` and the fixed
//! git identity `fixture-bot <fixture@example.invalid>`. A seed step that
//! fails returns a [`Failure`] naming the command.
//!
//! Two fixtures built on the same calendar day produce the same
//! `--format json` output once the volatile fields the `golden_json`
//! redactor names are redacted; `golden_json` is where that is asserted.
//!
//! Included with `#[path = "../common/seeded_plan.rs"] mod seeded_plan;`
//! next to `common/plan_fixture.rs`, `common/workflow_support.rs` and
//! `git_test_support.rs`.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;
use tempfile::TempDir;

use crate::plan_fixture::Sandbox;
use crate::workflow_support::{Failure, infra};

/// The project every seeded plan uses unless a test names another.
pub const PROJECT: &str = "fixture-proj";

/// The `RDM_SESSION` every fixture process runs in.
pub const SESSION: &str = "fixture-seed";

/// A seeded plan repo and its sandboxed user context.
pub struct SeededPlan {
    /// The owning temp directory (the fixture root).
    pub dir: TempDir,
    /// `<dir>/plan`, the plan repo root.
    pub plan: PathBuf,
    /// The seeded project.
    pub project: String,
    /// The user context every process runs under.
    pub sandbox: Sandbox,
    /// `<dir>/code`, once [`SeededPlan::with_code_repo`] has run.
    pub code: Option<PathBuf>,
}

impl SeededPlan {
    /// Initializes `<TempDir>/plan` with `project` as its default project,
    /// seeds it and commits the seed.
    pub fn new(project: &str) -> Result<Self, Failure> {
        let dir = TempDir::new().map_err(infra)?;
        let plan = dir.path().join("plan");
        std::fs::create_dir_all(&plan).map_err(infra)?;
        let sandbox = Sandbox::new(dir.path())?;
        let fixture = Self {
            dir,
            plan,
            project: project.to_owned(),
            sandbox,
            code: None,
        };
        fixture.step(&["init", "--default-project", project])?;
        fixture.seed()?;
        fixture.step(&["commit", "-m", "seed: fixture data"])?;
        Ok(fixture)
    }

    fn seed(&self) -> Result<(), Failure> {
        let p = self.project.as_str();
        self.step(&[
            "roadmap",
            "create",
            "sample-roadmap",
            "--title",
            "Sample Roadmap",
            "--body",
            "A sample roadmap for exercising list/show/search/next/tree.",
            "--no-edit",
            "--project",
            p,
        ])?;
        for (slug, title, n, body) in [
            ("seed-one", "Seed One", "1", "First phase: already done."),
            ("seed-two", "Seed Two", "2", "Second phase: in progress."),
            ("seed-three", "Seed Three", "3", "Third phase: not started."),
        ] {
            self.step(&[
                "phase",
                "create",
                slug,
                "--title",
                title,
                "--number",
                n,
                "--body",
                body,
                "--no-edit",
                "--roadmap",
                "sample-roadmap",
                "--project",
                p,
            ])?;
        }
        for (n, status) in [("1", "done"), ("2", "in-progress")] {
            self.step(&[
                "phase",
                "update",
                n,
                "--status",
                status,
                "--no-edit",
                "--roadmap",
                "sample-roadmap",
                "--project",
                p,
            ])?;
        }
        for (slug, title, tag, body) in [
            (
                "fixture-task-open",
                "Fixture Task Open",
                "bug",
                "An open task tagged bug.",
            ),
            (
                "fixture-task-active",
                "Fixture Task Active",
                "ui",
                "An in-progress task tagged ui.",
            ),
            (
                "fixture-task-done",
                "Fixture Task Done",
                "bug",
                "A done task tagged bug.",
            ),
        ] {
            self.step(&[
                "task",
                "create",
                slug,
                "--title",
                title,
                "--tags",
                tag,
                "--body",
                body,
                "--no-edit",
                "--project",
                p,
            ])?;
        }
        for (slug, status) in [
            ("fixture-task-active", "in-progress"),
            ("fixture-task-done", "done"),
        ] {
            self.step(&[
                "task",
                "update",
                slug,
                "--status",
                status,
                "--no-edit",
                "--project",
                p,
            ])?;
        }
        Ok(())
    }

    /// Adds `<dir>/code`: a git repo on `main` with one empty commit.
    pub fn with_code_repo(mut self) -> Result<Self, Failure> {
        let code = self.dir.path().join("code");
        std::fs::create_dir_all(&code).map_err(infra)?;
        self.sandbox
            .git(&code, &["init", "--quiet", "-b", "main"])?;
        self.sandbox.git(
            &code,
            &["commit", "--quiet", "--allow-empty", "-m", "chore: initial"],
        )?;
        self.code = Some(code);
        Ok(self)
    }

    /// The code repo; a [`Failure`] if [`SeededPlan::with_code_repo`] was
    /// not called.
    pub fn code(&self) -> Result<&Path, Failure> {
        self.code
            .as_deref()
            .ok_or_else(|| Failure::Infra("the fixture has no code repo".to_owned()))
    }

    /// `rdm --root <plan> <args>` under the sandbox, run from `cwd`.
    pub fn command_in(&self, cwd: &Path, args: &[&str]) -> Command {
        let mut cmd = self.sandbox.rdm();
        cmd.arg("--root")
            .arg(&self.plan)
            .args(args)
            .env("RDM_SESSION", SESSION)
            .current_dir(cwd);
        cmd
    }

    /// Runs `rdm <args>` from `cwd`, returning its output whatever the
    /// status.
    pub fn run_in(&self, cwd: &Path, args: &[&str]) -> Result<Output, Failure> {
        self.command_in(cwd, args)
            .output()
            .map_err(|e| Failure::Infra(format!("running rdm {args:?}: {e}")))
    }

    /// Runs `rdm <args>` from the fixture root.
    pub fn run(&self, args: &[&str]) -> Result<Output, Failure> {
        self.run_in(self.dir.path(), args)
    }

    /// Runs `rdm <args>` from `cwd`, requiring success; returns stdout. The
    /// failure names the command.
    pub fn ok_in(&self, cwd: &Path, args: &[&str]) -> Result<String, Failure> {
        let out = self.run_in(cwd, args)?;
        if !out.status.success() {
            return Err(Failure::Infra(format!(
                "`rdm {}` failed ({:?}): {}",
                args.join(" "),
                out.status.code(),
                String::from_utf8_lossy(&out.stderr)
            )));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// Runs `rdm <args>` from the fixture root, requiring success.
    pub fn ok(&self, args: &[&str]) -> Result<String, Failure> {
        self.ok_in(self.dir.path(), args)
    }

    fn step(&self, args: &[&str]) -> Result<(), Failure> {
        self.ok(args)
            .map(drop)
            .map_err(|f| Failure::Infra(format!("seed step failed: {f}")))
    }

    /// Runs `rdm <args>` (which must include `--format json`) and parses
    /// stdout.
    pub fn json(&self, args: &[&str]) -> Result<Value, Failure> {
        let text = self.ok(args)?;
        serde_json::from_str(&text)
            .map_err(|e| Failure::Infra(format!("`rdm {}` json: {e}: {text}", args.join(" "))))
    }
}

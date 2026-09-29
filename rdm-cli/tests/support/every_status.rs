//! A plan repo holding one phase and one task in every status, for the
//! `--active` tests of `rdm tree`, `rdm phase list` and `rdm task list`.
//!
//! Project `acme`; roadmap `busy` has one phase per phase status (slug = the
//! status), roadmap `idle` one not-started phase, and there is one task per
//! task status (`task-<status>`). Which of them are "active work" is read
//! from rdm-core's `is_active`, never from a list copied here.
//!
//! Included with `#[path = "support/every_status.rs"] mod every_status;`.

#![allow(dead_code)]

use std::path::Path;

use assert_cmd::Command;
use rdm_core::model::{PhaseStatus, TaskStatus};
use serde_json::Value;
use tempfile::TempDir;

/// The project the fixture seeds.
pub const PROJECT: &str = "acme";

/// Every phase status, in declaration order.
pub const PHASE_STATUSES: [PhaseStatus; 7] = [
    PhaseStatus::NotStarted,
    PhaseStatus::InProgress,
    PhaseStatus::NeedsReview,
    PhaseStatus::Reviewed,
    PhaseStatus::Done,
    PhaseStatus::Blocked,
    PhaseStatus::WontFix,
];

/// Every task status, in declaration order.
pub const TASK_STATUSES: [TaskStatus; 7] = [
    TaskStatus::Open,
    TaskStatus::InProgress,
    TaskStatus::NeedsReview,
    TaskStatus::Reviewed,
    TaskStatus::Done,
    TaskStatus::Blocked,
    TaskStatus::WontFix,
];

/// An `rdm` command against `root`, isolated from the host's config.
pub fn rdm(root: &Path) -> Command {
    let mut cmd = Command::cargo_bin("rdm").unwrap();
    cmd.env("XDG_CONFIG_HOME", "/dev/null/nonexistent")
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .arg("--root")
        .arg(root);
    cmd
}

/// Runs `rdm <args>` against `root` and parses its stdout as JSON.
pub fn json(root: &Path, args: &[&str]) -> Value {
    let out = rdm(root).args(args).assert().success();
    serde_json::from_slice(&out.get_output().stdout).unwrap()
}

fn ok(root: &Path, args: &[&str]) {
    rdm(root).args(args).assert().success();
}

/// The stem `phase create` gives the phase for `status` in roadmap `busy`.
pub fn phase_stem(status: PhaseStatus) -> String {
    let n = PHASE_STATUSES.iter().position(|s| *s == status).unwrap() + 1;
    format!("phase-{n}-{status}")
}

/// Builds the fixture in a fresh temp dir.
pub fn seed() -> TempDir {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    ok(root, &["init"]);
    ok(root, &["project", "create", PROJECT]);
    for roadmap in ["busy", "idle"] {
        ok(
            root,
            &[
                "roadmap",
                "create",
                roadmap,
                "--title",
                roadmap,
                "--project",
                PROJECT,
                "--no-edit",
            ],
        );
    }
    for (i, status) in PHASE_STATUSES.iter().enumerate() {
        let slug = status.to_string();
        let number = (i + 1).to_string();
        ok(
            root,
            &[
                "phase",
                "create",
                &slug,
                "--title",
                &slug,
                "--number",
                &number,
                "--roadmap",
                "busy",
                "--project",
                PROJECT,
                "--no-edit",
            ],
        );
        ok(
            root,
            &[
                "phase",
                "update",
                &phase_stem(*status),
                "--status",
                &slug,
                "--roadmap",
                "busy",
                "--project",
                PROJECT,
                "--no-edit",
            ],
        );
    }
    ok(
        root,
        &[
            "phase",
            "create",
            "later",
            "--title",
            "Later",
            "--roadmap",
            "idle",
            "--project",
            PROJECT,
            "--no-edit",
        ],
    );
    for status in TASK_STATUSES {
        let slug = format!("task-{status}");
        ok(
            root,
            &[
                "task",
                "create",
                &slug,
                "--title",
                &slug,
                "--project",
                PROJECT,
                "--no-edit",
            ],
        );
        ok(
            root,
            &[
                "task",
                "update",
                &slug,
                "--status",
                &status.to_string(),
                "--project",
                PROJECT,
                "--no-edit",
            ],
        );
    }
    dir
}

/// The stems of the fixture's active phases, per rdm-core.
pub fn active_phase_stems() -> Vec<String> {
    PHASE_STATUSES
        .iter()
        .filter(|s| s.is_active())
        .map(|s| phase_stem(*s))
        .collect()
}

/// The slugs of the fixture's active tasks, per rdm-core.
pub fn active_task_slugs() -> Vec<String> {
    TASK_STATUSES
        .iter()
        .filter(|s| s.is_active())
        .map(|s| format!("task-{s}"))
        .collect()
}

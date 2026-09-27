//! Builds the golden fixture and runs the 31-command `--format json`
//! inventory against it.
//!
//! The fixture is [`SeededPlan`] plus a code repo, then, in this order: one
//! submitted `request-changes` review on `task/fixture-task-open` (author
//! `fixture-bot`, one whole-document comment and one comment quoting "open
//! task" — so the review goldens record both a null and a non-null
//! `current` placement), one worktree on the same task
//! (added from the code repo), and the implementation plan `fixture-plan`
//! implementing it, whose `plan create --format json` stdout is itself the
//! `plan-create` golden. `worktree list`/`current` run from inside the
//! worktree (from the bare code-repo root `current` prints `null`).
//!
//! The three `create` goldens come after every read, so no other golden sees
//! what they create. `status`, `commit` and `discard` come last of all: an
//! uncaptured `rdm commit` first lands everything seeded so far (including the
//! review, whose file path is volatile), and each captured command then acts
//! on one deterministic mutation, so its output names only stable paths.
//!
//! One command accepts `--format json` but prints plain text regardless and
//! is not captured: `model resolve` (see `tests/golden/README.md`).

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::Value;

use crate::seeded_plan::{PROJECT, SeededPlan};
use crate::workflow_support::Failure;

/// Every captured golden, by file stem, in capture order.
pub const NAMES: [&str; 31] = [
    "plan-create",
    "info",
    "roadmap-list",
    "roadmap-show",
    "phase-list",
    "phase-show",
    "task-list",
    "task-show",
    "list",
    "search",
    "next",
    "tree",
    "describe",
    "tag-list",
    "backlog-report",
    "model-show",
    "review-list",
    "review-show",
    "review-requests",
    "plan-show",
    "plan-list",
    "verify-resolve",
    "worktree-list",
    "worktree-current",
    "hook-done-line",
    "roadmap-create",
    "phase-create",
    "task-create",
    "status",
    "commit",
    "discard",
];

/// One capture: the fixture it ran against (kept alive so its temp paths
/// can be redacted) and each command's raw stdout by golden name.
pub struct Capture {
    /// The fixture.
    pub fixture: SeededPlan,
    /// Raw stdout by golden name.
    pub raw: BTreeMap<String, String>,
}

/// Builds a fresh fixture and captures every golden command, raw.
pub fn capture() -> Result<Capture, Failure> {
    let fixture = SeededPlan::new(PROJECT)?.with_code_repo()?;
    let p = PROJECT;
    let mut raw = BTreeMap::new();

    let started: Value = fixture.json(&[
        "review",
        "start",
        "--on",
        "task/fixture-task-open",
        "--author",
        "fixture-bot",
        "--no-edit",
        "--project",
        p,
        "--format",
        "json",
    ])?;
    let review = started["id"]
        .as_str()
        .ok_or_else(|| Failure::Infra(format!("`rdm review start` printed no id: {started}")))?
        .to_owned();
    fixture.ok(&[
        "review",
        "comment",
        &review,
        "--body",
        "General feedback.",
        "--no-edit",
        "--project",
        p,
    ])?;
    fixture.ok(&[
        "review",
        "comment",
        &review,
        "--quote",
        "open task",
        "--body",
        "Name the task more precisely.",
        "--no-edit",
        "--project",
        p,
    ])?;
    fixture.ok(&[
        "review",
        "submit",
        &review,
        "--verdict",
        "request-changes",
        "--no-edit",
        "--project",
        p,
    ])?;

    let code = fixture.code()?.to_owned();
    let added = fixture.ok_in(
        &code,
        &[
            "worktree",
            "add",
            "task/fixture-task-open",
            "--project",
            p,
            "--format",
            "json",
        ],
    )?;
    let added: Value = serde_json::from_str(&added)
        .map_err(|e| Failure::Infra(format!("`rdm worktree add` json: {e}: {added}")))?;
    let worktree =
        PathBuf::from(added["path"].as_str().ok_or_else(|| {
            Failure::Infra(format!("`rdm worktree add` printed no path: {added}"))
        })?);
    if !worktree.is_dir() {
        return Err(Failure::Infra(format!(
            "`rdm worktree add` path {} is not a directory",
            worktree.display()
        )));
    }

    raw.insert(
        "plan-create".to_owned(),
        fixture.ok(&[
            "plan",
            "create",
            "fixture-plan",
            "--implements",
            "task/fixture-task-open",
            "--title",
            "Fixture Plan",
            "--body",
            "## Approach\n\nSeeded plan body.",
            "--no-edit",
            "--project",
            p,
            "--format",
            "json",
        ])?,
    );

    let commands: [(&str, Vec<&str>); 21] = [
        ("info", vec!["info", "--format", "json", "--project", p]),
        (
            "roadmap-list",
            vec!["roadmap", "list", "--format", "json", "--project", p],
        ),
        (
            "roadmap-show",
            vec![
                "roadmap",
                "show",
                "sample-roadmap",
                "--format",
                "json",
                "--project",
                p,
            ],
        ),
        (
            "phase-list",
            vec![
                "phase",
                "list",
                "--roadmap",
                "sample-roadmap",
                "--format",
                "json",
                "--project",
                p,
            ],
        ),
        (
            "phase-show",
            vec![
                "phase",
                "show",
                "1",
                "--roadmap",
                "sample-roadmap",
                "--format",
                "json",
                "--project",
                p,
            ],
        ),
        (
            "task-list",
            vec!["task", "list", "--format", "json", "--project", p],
        ),
        (
            "task-show",
            vec![
                "task",
                "show",
                "fixture-task-open",
                "--format",
                "json",
                "--project",
                p,
            ],
        ),
        ("list", vec!["list", "--format", "json", "--project", p]),
        (
            "search",
            vec!["search", "seed", "--format", "json", "--project", p],
        ),
        (
            "next",
            vec![
                "next",
                "--roadmap",
                "sample-roadmap",
                "--format",
                "json",
                "--project",
                p,
            ],
        ),
        ("tree", vec!["tree", "--format", "json", "--project", p]),
        ("describe", vec!["describe", "--format", "json"]),
        (
            "tag-list",
            vec!["tag", "list", "--format", "json", "--project", p],
        ),
        (
            "backlog-report",
            vec!["backlog", "report", "--format", "json", "--project", p],
        ),
        ("model-show", vec!["model", "show", "--format", "json"]),
        (
            "review-list",
            vec!["review", "list", "--format", "json", "--project", p],
        ),
        (
            "review-show",
            vec![
                "review",
                "show",
                &review,
                "--format",
                "json",
                "--project",
                p,
            ],
        ),
        (
            "review-requests",
            vec!["review", "requests", "--format", "json", "--project", p],
        ),
        (
            "plan-show",
            vec![
                "plan",
                "show",
                "fixture-plan",
                "--format",
                "json",
                "--project",
                p,
            ],
        ),
        (
            "plan-list",
            vec!["plan", "list", "--format", "json", "--project", p],
        ),
        (
            "verify-resolve",
            vec!["verify", "resolve", "--format", "json", "--project", p],
        ),
    ];
    for (name, args) in &commands {
        raw.insert((*name).to_owned(), fixture.ok(args)?);
    }
    for (name, sub) in [("worktree-list", "list"), ("worktree-current", "current")] {
        raw.insert(
            name.to_owned(),
            fixture.ok_in(&worktree, &["worktree", sub, "--format", "json"])?,
        );
    }
    raw.insert(
        "hook-done-line".to_owned(),
        fixture.ok(&[
            "hook",
            "done-line",
            "--roadmap",
            "sample-roadmap",
            "--phase",
            "phase-1-seed-one",
            "--format",
            "json",
        ])?,
    );
    let creates: [(&str, Vec<&str>); 3] = [
        (
            "roadmap-create",
            vec![
                "roadmap",
                "create",
                "fixture-created-roadmap",
                "--title",
                "Fixture Created Roadmap",
                "--body",
                "Created by the golden capture.",
                "--no-edit",
                "--project",
                p,
                "--format",
                "json",
            ],
        ),
        (
            "phase-create",
            vec![
                "phase",
                "create",
                "created-phase",
                "--number",
                "1",
                "--roadmap",
                "fixture-created-roadmap",
                "--title",
                "Created Phase",
                "--body",
                "Created by the golden capture.",
                "--no-edit",
                "--project",
                p,
                "--format",
                "json",
            ],
        ),
        (
            "task-create",
            vec![
                "task",
                "create",
                "fixture-created-task",
                "--title",
                "Fixture Created Task",
                "--body",
                "Created by the golden capture.",
                "--no-edit",
                "--project",
                p,
                "--format",
                "json",
            ],
        ),
    ];
    for (name, args) in &creates {
        raw.insert((*name).to_owned(), fixture.ok(args)?);
    }

    // Land everything seeded so far, uncaptured, so the working-tree goldens
    // below see only the one deterministic mutation each makes.
    fixture.ok(&["commit", "-m", "golden: land the seeded fixture"])?;
    let update = |slug: &'static str, status: &'static str| {
        fixture.ok(&[
            "task",
            "update",
            slug,
            "--status",
            status,
            "--no-edit",
            "--project",
            p,
        ])
    };
    update("fixture-task-open", "in-progress")?;
    raw.insert(
        "status".to_owned(),
        fixture.ok(&["status", "--format", "json"])?,
    );
    raw.insert(
        "commit".to_owned(),
        fixture.ok(&["commit", "-m", "golden: capture commit", "--format", "json"])?,
    );
    update("fixture-task-active", "done")?;
    raw.insert(
        "discard".to_owned(),
        fixture.ok(&["discard", "--force", "--format", "json"])?,
    );
    Ok(Capture { fixture, raw })
}

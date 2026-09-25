//! End-to-end coverage of the core-enforced `reviewed` transition gate, driven
//! through the real `rdm` binary against a temp plan repo plus a temp source
//! repo with a real `rdm worktree add` worktree.
//!
//! The core-level branch coverage lives in `rdm-core/tests/gate.rs`; this file
//! proves the CLI actually reaches it — the flag resolution, the worktree
//! probe wiring, the override's argv surface, and the read-back in
//! `phase show`.

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use std::path::Path;
use tempfile::TempDir;

#[path = "git_test_support.rs"]
mod git_test_support;
use git_test_support::git;

fn rdm() -> Command {
    let mut cmd = Command::cargo_bin("rdm").unwrap();
    cmd.env("XDG_CONFIG_HOME", "/dev/null/nonexistent");
    cmd.env_remove("RDM_PROJECT")
        .env_remove("RDM_ROOT")
        .env_remove("RDM_REVIEWED_GATE");
    cmd
}

/// A temp source repo whose git root is a **subdirectory** of the `TempDir`.
///
/// `rdm worktree add` places a worktree at
/// `repo_root.parent()/<name>__worktrees/<item>` — a sibling of the repo. If
/// the repo root were the `TempDir` itself that sibling would land in the
/// system temp directory and outlive `TempDir::drop`, leaking a populated git
/// worktree on every run. Rooting the repo one level down keeps the sibling
/// inside the `TempDir`, so it is removed with everything else.
struct SourceRepo {
    _dir: TempDir,
    root: std::path::PathBuf,
}

impl SourceRepo {
    fn path(&self) -> &Path {
        &self.root
    }
}

/// A source repo on `main` with one commit.
fn init_source_repo() -> SourceRepo {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("repo");
    std::fs::create_dir_all(&root).unwrap();
    let p = root.as_path();
    git(p, &["init", "-b", "main"]);
    std::fs::create_dir_all(p.join("src")).unwrap();
    std::fs::write(p.join("src/lib.rs"), "fn one() {}\n").unwrap();
    git(p, &["add", "."]);
    git(p, &["commit", "-m", "base"]);
    SourceRepo { _dir: dir, root }
}

/// Hand-edits `project.md` to point the project at `repo` as its source —
/// there is no CLI command for it, exactly as `cli_review_change.rs` does.
fn set_project_source(plan: &Path, repo: &str) {
    let path = plan.join("projects").join("demo").join("project.md");
    let content = std::fs::read_to_string(&path).unwrap();
    let rest = content.strip_prefix("---\n").expect("frontmatter open");
    let end = rest.find("\n---").expect("frontmatter close");
    let (frontmatter, tail) = rest.split_at(end);
    std::fs::write(
        &path,
        format!("---\n{frontmatter}\nsource:\n  repo: \"{repo}\"{tail}"),
    )
    .unwrap();
}

/// A plan repo with `demo`, roadmap `auth` + `phase-1-design`, task `solo`,
/// and `gates.reviewed = true`.
fn init_plan_repo(source: &Path) -> TempDir {
    let dir = TempDir::new().unwrap();
    let p = dir.path();
    rdm().arg("--root").arg(p).arg("init").assert().success();
    rdm()
        .arg("--root")
        .arg(p)
        .args(["project", "create", "demo"])
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(p)
        .args([
            "roadmap",
            "create",
            "auth",
            "--title",
            "Auth",
            "--no-edit",
            "--project",
            "demo",
        ])
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(p)
        .args([
            "phase",
            "create",
            "design",
            "--title",
            "Design",
            "--number",
            "1",
            "--no-edit",
            "--roadmap",
            "auth",
            "--project",
            "demo",
        ])
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(p)
        .args([
            "task",
            "create",
            "solo",
            "--title",
            "Solo",
            "--no-edit",
            "--project",
            "demo",
        ])
        .assert()
        .success();
    set_project_source(p, &source.to_string_lossy());
    rdm()
        .arg("--root")
        .arg(p)
        .args(["config", "set", "gates.reviewed", "true"])
        .assert()
        .success();
    dir
}

/// Creates an approved plan implementing `implements`.
fn create_approved_plan(plan: &Path, slug: &str, implements: &str) {
    rdm()
        .arg("--root")
        .arg(plan)
        .args([
            "plan",
            "create",
            slug,
            "--title",
            "A plan",
            "--implements",
            implements,
            "--body",
            "Plan body.\n",
            "--no-edit",
            "--project",
            "demo",
        ])
        .assert()
        .success();
    let id = start_review(plan, None, &format!("plan/{slug}"), &[]);
    submit_approve(plan, &id);
}

fn start_review(plan: &Path, cwd: Option<&Path>, on: &str, extra: &[&str]) -> String {
    let mut cmd = rdm();
    cmd.arg("--root").arg(plan).args([
        "review",
        "start",
        "--on",
        on,
        "--no-edit",
        "--project",
        "demo",
    ]);
    cmd.args(extra);
    if let Some(d) = cwd {
        cmd.current_dir(d);
    }
    let out = cmd.assert().success().get_output().stdout.clone();
    let text = String::from_utf8_lossy(&out).to_string();
    text.split('\'')
        .nth(1)
        .unwrap_or_else(|| panic!("could not read a review id out of: {text}"))
        .to_string()
}

fn submit_approve(plan: &Path, id: &str) {
    rdm()
        .arg("--root")
        .arg(plan)
        .args([
            "review",
            "submit",
            id,
            "--verdict",
            "approve",
            "--body",
            "Looks good.",
            "--no-edit",
            "--project",
            "demo",
        ])
        .assert()
        .success();
}

/// Marks the phase reviewed from `cwd`, returning the assertion so the caller
/// can require success or inspect stderr.
fn mark_reviewed(plan: &Path, cwd: &Path, extra: &[&str]) -> assert_cmd::assert::Assert {
    let mut cmd = rdm();
    cmd.arg("--root")
        .arg(plan)
        .args([
            "phase",
            "update",
            "phase-1-design",
            "--status",
            "reviewed",
            "--no-edit",
            "--roadmap",
            "auth",
            "--project",
            "demo",
        ])
        .args(extra)
        .current_dir(cwd);
    cmd.assert()
}

fn phase_json(plan: &Path) -> Value {
    let out = rdm()
        .arg("--root")
        .arg(plan)
        .args([
            "phase",
            "show",
            "phase-1-design",
            "--format",
            "json",
            "--roadmap",
            "auth",
            "--project",
            "demo",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&out).unwrap()
}

fn stderr_of(a: assert_cmd::assert::Assert) -> String {
    String::from_utf8_lossy(&a.get_output().stderr).to_string()
}

/// Adds an rdm worktree for the `auth` roadmap, commits one change on its
/// branch, and returns its path.
///
/// The commit is load-bearing, not decoration: a freshly created worktree
/// branch points at `main`'s tip, so the merge base of its `HEAD` and `main` IS
/// that `HEAD` and `change/HEAD` names an EMPTY reviewed range —
/// `resolve_change_target` refuses one outright. Every test here reviews a
/// worktree where work has happened, so the fixture makes that true.
fn add_worktree(plan: &Path, source: &Path) -> std::path::PathBuf {
    let out = rdm()
        .arg("--root")
        .arg(plan)
        .args(["worktree", "add", "auth", "--project", "demo"])
        .current_dir(source)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let path = std::path::PathBuf::from(String::from_utf8_lossy(&out).trim().to_string());
    std::fs::write(path.join("src/lib.rs"), "fn one() {}\nfn two() {}\n").unwrap();
    git(&path, &["add", "src/lib.rs"]);
    git(&path, &["commit", "-m", "the work under review"]);
    path
}

// ---------------------------------------------------------------------------
// AC1 — each missing precondition produces a distinct refusal; then success
// ---------------------------------------------------------------------------

#[test]
fn reviewed_gate_refuses_each_precondition_then_succeeds() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    let wt = add_worktree(plan.path(), src.path());

    // Rung 1: no plan at all.
    let err = stderr_of(mark_reviewed(plan.path(), &wt, &[]).failure());
    assert!(err.contains("rdm plan create"), "rung 1 remediation: {err}");
    assert!(
        err.contains("phase/auth/phase-1-design"),
        "rung 1 item label: {err}"
    );
    assert!(
        !err.contains("rdm review start --on change/"),
        "rung 1 must not report rung 2's remediation: {err}"
    );

    // Rung 2: an approved plan exists, but no approving change review.
    create_approved_plan(plan.path(), "design-plan", "phase/auth/phase-1-design");
    let err = stderr_of(mark_reviewed(plan.path(), &wt, &[]).failure());
    assert!(
        err.contains("rdm review start --on change/"),
        "rung 2 remediation: {err}"
    );
    assert!(
        err.contains("plan/design-plan"),
        "rung 2 names the plan: {err}"
    );
    assert!(
        !err.contains("rdm plan create"),
        "rung 2 must not fall back to rung 1's remediation: {err}"
    );

    // Rung 3: records in place, but the worktree is dirty.
    let change_id = start_review(
        plan.path(),
        Some(&wt),
        "change/HEAD",
        &["--implements", "plan/design-plan"],
    );
    submit_approve(plan.path(), &change_id);
    std::fs::write(wt.join("scratch.txt"), "uncommitted\n").unwrap();
    let err = stderr_of(mark_reviewed(plan.path(), &wt, &[]).failure());
    assert!(
        err.contains(&wt.display().to_string()),
        "rung 3 names the worktree: {err}"
    );
    assert!(
        err.contains("scratch.txt"),
        "rung 3 names the dirty path: {err}"
    );
    assert!(
        err.contains("--override-gate` does NOT bypass"),
        "rung 3 must say an override will not help: {err}"
    );

    // Rung 4: committing repairs moves HEAD; the old review cannot approve
    // it — and the refusal names the staleness rather than reporting the
    // approval as absent.
    git(&wt, &["add", "."]);
    git(&wt, &["commit", "-m", "scratch"]);
    let err = stderr_of(mark_reviewed(plan.path(), &wt, &[]).failure());
    assert!(
        err.contains("was recorded at HEAD"),
        "rung 4 must name the head mismatch: {err}"
    );
    assert!(
        err.contains(&change_id),
        "rung 4 must name the stale review: {err}"
    );
    assert!(
        err.contains("rdm review start --on change/HEAD"),
        "rung 4 remediation: {err}"
    );
    assert!(
        !err.contains("no approving change review"),
        "rung 4 must not report an existing approval as absent: {err}"
    );
    let current_review = start_review(
        plan.path(),
        Some(&wt),
        "change/HEAD",
        &["--implements", "plan/design-plan"],
    );
    submit_approve(plan.path(), &current_review);
    mark_reviewed(plan.path(), &wt, &[]).success();
    assert_eq!(phase_json(plan.path())["status"], "reviewed");
}

/// AC5 at the binary boundary: an approval that was *dismissed* no longer
/// satisfies precondition (b), and the refusal reads as "absent" (the
/// review was withdrawn as evidence), not as "stale".
#[test]
fn a_dismissed_approval_no_longer_lets_reviewed_through() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    let wt = add_worktree(plan.path(), src.path());
    create_approved_plan(plan.path(), "design-plan", "phase/auth/phase-1-design");
    let change_id = start_review(
        plan.path(),
        Some(&wt),
        "change/HEAD",
        &["--implements", "plan/design-plan"],
    );
    submit_approve(plan.path(), &change_id);
    // While it stands, the write is allowed.
    mark_reviewed(plan.path(), &wt, &[]).success();
    assert_eq!(phase_json(plan.path())["status"], "reviewed");

    // Dismiss it and go back to needs-review: the same write is now refused.
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "update",
            &change_id,
            "--state",
            "dismissed",
            "--project",
            "demo",
        ])
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "phase",
            "update",
            "phase-1-design",
            "--status",
            "needs-review",
            "--no-edit",
            "--roadmap",
            "auth",
            "--project",
            "demo",
        ])
        .current_dir(&wt)
        .assert()
        .success();
    let err = stderr_of(mark_reviewed(plan.path(), &wt, &[]).failure());
    assert!(
        err.contains("no approving change review"),
        "a dismissed approval must read as absent: {err}"
    );
    assert!(
        !err.contains("was recorded at HEAD"),
        "a dismissed approval is never a stale candidate: {err}"
    );
}

#[test]
fn the_gate_is_off_by_default() {
    // The very property that keeps every existing plan repo and every one of
    // rdm's hermetic harnesses green: without `gates.reviewed`, a `reviewed`
    // write needs no records at all.
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    let wt = add_worktree(plan.path(), src.path());
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args(["config", "set", "gates.reviewed", "false"])
        .assert()
        .success();
    mark_reviewed(plan.path(), &wt, &[]).success();
    assert_eq!(phase_json(plan.path())["status"], "reviewed");
}

#[test]
fn the_env_override_enables_and_disables_the_gate() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    let wt = add_worktree(plan.path(), src.path());

    // Config says on; env says off → off.
    let mut cmd = rdm();
    cmd.env("RDM_REVIEWED_GATE", "false");
    cmd.arg("--root")
        .arg(plan.path())
        .args([
            "phase",
            "update",
            "phase-1-design",
            "--status",
            "reviewed",
            "--no-edit",
            "--roadmap",
            "auth",
            "--project",
            "demo",
        ])
        .current_dir(&wt)
        .assert()
        .success();

    // A junk value is a loud error, never a silent `false`.
    let mut cmd = rdm();
    cmd.env("RDM_REVIEWED_GATE", "yes");
    let err = String::from_utf8_lossy(
        &cmd.arg("--root")
            .arg(plan.path())
            .args([
                "phase",
                "update",
                "phase-1-design",
                "--status",
                "reviewed",
                "--no-edit",
                "--roadmap",
                "auth",
                "--project",
                "demo",
            ])
            .current_dir(&wt)
            .assert()
            .failure()
            .get_output()
            .stderr,
    )
    .to_string();
    assert!(err.contains("RDM_REVIEWED_GATE"), "{err}");
}

#[test]
fn non_reviewed_transitions_are_never_gated() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    let wt = add_worktree(plan.path(), src.path());
    // No plan, no review, and a dirty worktree — none of it matters.
    std::fs::write(wt.join("scratch.txt"), "uncommitted\n").unwrap();
    for status in ["in-progress", "needs-review", "blocked", "done"] {
        rdm()
            .arg("--root")
            .arg(plan.path())
            .args([
                "phase",
                "update",
                "phase-1-design",
                "--status",
                status,
                "--no-edit",
                "--roadmap",
                "auth",
                "--project",
                "demo",
            ])
            .current_dir(&wt)
            .assert()
            .success();
    }
}

// ---------------------------------------------------------------------------
// AC2 — the operator override
// ---------------------------------------------------------------------------

#[test]
fn override_gate_records_reason_and_actor_and_clears_on_exit() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    let wt = add_worktree(plan.path(), src.path());

    // No plan and no change review, but a CLEAN worktree.
    let mut cmd = rdm();
    cmd.env("RDM_REVIEW_AUTHOR", "alice");
    cmd.arg("--root")
        .arg(plan.path())
        .args([
            "phase",
            "update",
            "phase-1-design",
            "--status",
            "reviewed",
            "--override-gate",
            "operator: hotfix, plan filed retroactively",
            "--no-edit",
            "--roadmap",
            "auth",
            "--project",
            "demo",
        ])
        .current_dir(&wt)
        .assert()
        .success();

    let j = phase_json(plan.path());
    assert_eq!(j["status"], "reviewed");
    assert_eq!(
        j["gate_override"]["reason"],
        "operator: hotfix, plan filed retroactively"
    );
    assert_eq!(j["gate_override"]["actor"], "alice");
    assert!(
        j["gate_override"]["at"].as_str().unwrap().len() == 10,
        "a date must be recorded: {j}"
    );

    // The text view surfaces it.
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "phase",
            "show",
            "phase-1-design",
            "--roadmap",
            "auth",
            "--project",
            "demo",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Gate override:"))
        .stdout(predicate::str::contains("by alice"));

    // Leaving `reviewed` clears it...
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "phase",
            "update",
            "phase-1-design",
            "--status",
            "in-progress",
            "--no-edit",
            "--roadmap",
            "auth",
            "--project",
            "demo",
        ])
        .current_dir(&wt)
        .assert()
        .success();
    assert!(
        phase_json(plan.path())["gate_override"].is_null(),
        "a stale override must not survive leaving reviewed"
    );

    // ...so a bare `reviewed` write is refused again.
    mark_reviewed(plan.path(), &wt, &[]).failure();
}

#[test]
fn override_gate_still_refuses_a_dirty_worktree() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    let wt = add_worktree(plan.path(), src.path());
    std::fs::write(wt.join("scratch.txt"), "uncommitted\n").unwrap();

    let mut cmd = rdm();
    cmd.env("RDM_REVIEW_AUTHOR", "alice");
    let err = String::from_utf8_lossy(
        &cmd.arg("--root")
            .arg(plan.path())
            .args([
                "phase",
                "update",
                "phase-1-design",
                "--status",
                "reviewed",
                "--override-gate",
                "operator: hotfix",
                "--no-edit",
                "--roadmap",
                "auth",
                "--project",
                "demo",
            ])
            .current_dir(&wt)
            .assert()
            .failure()
            .get_output()
            .stderr,
    )
    .to_string();
    assert!(err.contains("scratch.txt"), "{err}");
    assert!(err.contains("--override-gate` does NOT bypass"), "{err}");
}

#[test]
fn override_gate_rejects_an_empty_reason_and_a_non_reviewed_status() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    let wt = add_worktree(plan.path(), src.path());

    // Empty reason.
    let mut cmd = rdm();
    cmd.env("RDM_REVIEW_AUTHOR", "alice");
    let err = String::from_utf8_lossy(
        &cmd.arg("--root")
            .arg(plan.path())
            .args([
                "phase",
                "update",
                "phase-1-design",
                "--status",
                "reviewed",
                "--override-gate",
                "   ",
                "--no-edit",
                "--roadmap",
                "auth",
                "--project",
                "demo",
            ])
            .current_dir(&wt)
            .assert()
            .failure()
            .get_output()
            .stderr,
    )
    .to_string();
    assert!(
        err.contains("--override-gate requires a non-empty reason"),
        "{err}"
    );

    // Wrong status: rejected, not silently ignored.
    let err = String::from_utf8_lossy(
        &rdm()
            .arg("--root")
            .arg(plan.path())
            .args([
                "phase",
                "update",
                "phase-1-design",
                "--status",
                "in-progress",
                "--override-gate",
                "why not",
                "--no-edit",
                "--roadmap",
                "auth",
                "--project",
                "demo",
            ])
            .current_dir(&wt)
            .assert()
            .failure()
            .get_output()
            .stderr,
    )
    .to_string();
    assert!(
        err.contains("--override-gate can only be used with --status reviewed"),
        "{err}"
    );
}

#[test]
fn override_gate_is_refused_rather_than_silently_dropped_when_the_gate_is_off() {
    // Regression: `--override-gate` used to exit 0 and record nothing at all
    // when `gates.reviewed` was off (the shipped default) — the reason and
    // actor the operator explicitly supplied were discarded with no feedback,
    // and `phase show`/`task show` silently disagreed with the request. The
    // override exists so that a bypass is an *audited* act, so a bypass that
    // cannot be audited is refused, exactly as one on an ungated transition is.
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    let wt = add_worktree(plan.path(), src.path());
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args(["config", "set", "gates.reviewed", "false"])
        .assert()
        .success();

    // Phase.
    let mut cmd = rdm();
    cmd.env("RDM_REVIEW_AUTHOR", "alice");
    let err = stderr_of(
        cmd.arg("--root")
            .arg(plan.path())
            .args([
                "phase",
                "update",
                "phase-1-design",
                "--status",
                "reviewed",
                "--override-gate",
                "sneaky reason",
                "--no-edit",
                "--roadmap",
                "auth",
                "--project",
                "demo",
            ])
            .current_dir(&wt)
            .assert()
            .failure(),
    );
    assert!(
        err.contains("--override-gate has nothing to bypass"),
        "{err}"
    );
    assert!(
        err.contains("rdm config set gates.reviewed true"),
        "remediation missing: {err}"
    );
    // The refusal is a refusal: the status did not move and nothing was
    // recorded.
    let json = phase_json(plan.path());
    assert_eq!(json["status"], "not-started");
    assert!(json.get("gate_override").is_none(), "{json}");

    // Task, same contract.
    let mut cmd = rdm();
    cmd.env("RDM_REVIEW_AUTHOR", "alice");
    let err = stderr_of(
        cmd.arg("--root")
            .arg(plan.path())
            .args([
                "task",
                "update",
                "solo",
                "--status",
                "reviewed",
                "--override-gate",
                "sneaky reason",
                "--no-edit",
                "--project",
                "demo",
            ])
            .current_dir(&wt)
            .assert()
            .failure(),
    );
    assert!(
        err.contains("--override-gate has nothing to bypass"),
        "{err}"
    );

    // And the same write without the flag still succeeds — the refusal is
    // scoped to the override, not to the disabled gate's write path.
    mark_reviewed(plan.path(), &wt, &[]).success();
    assert_eq!(phase_json(plan.path())["status"], "reviewed");
}

#[test]
fn the_task_flow_gates_and_overrides_identically() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    let wt = add_worktree(plan.path(), src.path());

    // Refused with no records.
    let err = String::from_utf8_lossy(
        &rdm()
            .arg("--root")
            .arg(plan.path())
            .args([
                "task",
                "update",
                "solo",
                "--status",
                "reviewed",
                "--no-edit",
                "--project",
                "demo",
            ])
            .current_dir(&wt)
            .assert()
            .failure()
            .get_output()
            .stderr,
    )
    .to_string();
    assert!(err.contains("rdm plan create"), "{err}");
    assert!(err.contains("task/solo"), "{err}");

    // Overridden, recorded, and readable back.
    let mut cmd = rdm();
    cmd.env("RDM_REVIEW_AUTHOR", "bob");
    cmd.arg("--root")
        .arg(plan.path())
        .args([
            "task",
            "update",
            "solo",
            "--status",
            "reviewed",
            "--override-gate",
            "operator: urgent",
            "--no-edit",
            "--project",
            "demo",
        ])
        .current_dir(&wt)
        .assert()
        .success();
    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "task",
            "show",
            "solo",
            "--format",
            "json",
            "--project",
            "demo",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let j: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(j["gate_override"]["reason"], "operator: urgent");
    assert_eq!(j["gate_override"]["actor"], "bob");
}

#[test]
fn a_clean_item_serializes_without_a_gate_override_key() {
    // `skip_serializing_if` is what keeps the phase-show / task-show goldens
    // byte-identical for every item that was never overridden.
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    let j = phase_json(plan.path());
    assert!(
        j.get("gate_override").is_none(),
        "an un-overridden phase must not carry the key at all: {j}"
    );
}

// ---------------------------------------------------------------------------
// Phase-scoped review base (roadmap agent-orchestrated-dispatch, phase 40):
// each phase's `started_head` is stamped write-once on its first
// `in-progress` transition, `review source` defaults its base to that value
// instead of the merge-base with the default branch, and the gated
// `reviewed` write composes with that default end to end.
// ---------------------------------------------------------------------------

fn rev_parse(dir: &Path, rev: &str) -> String {
    let out = git(dir, &["rev-parse", rev]);
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn current_branch(dir: &Path) -> String {
    let out = git(dir, &["symbolic-ref", "--short", "HEAD"]);
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn phase_json_for(plan: &Path, stem: &str, roadmap: &str) -> Value {
    let out = rdm()
        .arg("--root")
        .arg(plan)
        .args([
            "phase",
            "show",
            stem,
            "--format",
            "json",
            "--roadmap",
            roadmap,
            "--project",
            "demo",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&out).unwrap()
}

fn task_json_for(plan: &Path, slug: &str) -> Value {
    let out = rdm()
        .arg("--root")
        .arg(plan)
        .args([
            "task",
            "show",
            slug,
            "--format",
            "json",
            "--project",
            "demo",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&out).unwrap()
}

fn review_show_json(plan: &Path, cwd: &Path, id: &str) -> Value {
    let out = rdm()
        .arg("--root")
        .arg(plan)
        .args([
            "review",
            "show",
            id,
            "--format",
            "json",
            "--project",
            "demo",
        ])
        .current_dir(cwd)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&out).unwrap()
}

/// AC1 + AC2 + AC3 at the binary boundary: two phases implemented in
/// sequence in one shared roadmap worktree. The second phase's explicit
/// `--start-commit` records `started_head` from OUTSIDE the worktree
/// (the source repo's main checkout), a second `--start-commit` is refused
/// and does not move it, `review source` defaults its base to that recorded
/// value rather than the merge-base — scoping `changedFiles` to the second
/// phase's own commit — and an approving `change/` review persisted at that
/// base lets the gated `reviewed` write through.
#[test]
fn started_head_scopes_the_second_phase_review_and_satisfies_the_gate() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "phase",
            "create",
            "impl",
            "--title",
            "Impl",
            "--number",
            "2",
            "--no-edit",
            "--roadmap",
            "auth",
            "--project",
            "demo",
        ])
        .assert()
        .success();

    // The shared roadmap worktree, with no work committed to it yet.
    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .args(["worktree", "add", "auth", "--project", "demo"])
        .current_dir(src.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let wt = std::path::PathBuf::from(String::from_utf8_lossy(&out).trim().to_string());
    let base_head = rev_parse(&wt, "HEAD");

    // AC1: record phase 1's `--start-commit` from OUTSIDE the worktree (the
    // main source checkout) — resolution goes through the registered
    // worktree, not the caller's cwd. Combined with `--status in-progress`
    // in the same call, matching the dispatch skill's usage.
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "phase",
            "update",
            "phase-1-design",
            "--status",
            "in-progress",
            "--start-commit",
            &base_head,
            "--no-edit",
            "--roadmap",
            "auth",
            "--project",
            "demo",
        ])
        .current_dir(src.path())
        .assert()
        .success();
    assert_eq!(
        phase_json_for(plan.path(), "phase-1-design", "auth")["started_head"],
        base_head
    );

    // Phase 1's own commit.
    std::fs::write(wt.join("src/lib.rs"), "fn one() {}\nfn two() {}\n").unwrap();
    git(&wt, &["add", "."]);
    git(&wt, &["commit", "-m", "phase 1 work"]);
    let phase_1_head = rev_parse(&wt, "HEAD");

    // Write-once: a second `--start-commit` is refused and does not move it.
    // A bare `--status in-progress` re-stamp (no `--start-commit`) is also
    // harmless — it no longer touches the field at all.
    let refused = rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "phase",
            "update",
            "phase-1-design",
            "--start-commit",
            &phase_1_head,
            "--no-edit",
            "--roadmap",
            "auth",
            "--project",
            "demo",
        ])
        .current_dir(src.path())
        .assert()
        .failure();
    let refused_stderr = String::from_utf8_lossy(&refused.get_output().stderr).to_string();
    assert!(
        refused_stderr.contains(&base_head) && refused_stderr.contains("write-once"),
        "the write-once refusal must name the already-recorded SHA and say 'write-once': {refused_stderr}"
    );
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "phase",
            "update",
            "phase-1-design",
            "--status",
            "in-progress",
            "--no-edit",
            "--roadmap",
            "auth",
            "--project",
            "demo",
        ])
        .current_dir(src.path())
        .assert()
        .success();
    assert_eq!(
        phase_json_for(plan.path(), "phase-1-design", "auth")["started_head"],
        base_head,
        "a refused --start-commit, and a plain in-progress re-stamp with none given, must never \
         move an already-recorded started_head"
    );

    // Phase 2 starts here — its started_head is phase 1's HEAD.
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "phase",
            "update",
            "phase-2-impl",
            "--status",
            "in-progress",
            "--start-commit",
            &phase_1_head,
            "--no-edit",
            "--roadmap",
            "auth",
            "--project",
            "demo",
        ])
        .current_dir(src.path())
        .assert()
        .success();
    assert_eq!(
        phase_json_for(plan.path(), "phase-2-impl", "auth")["started_head"],
        phase_1_head
    );

    // Phase 2's own commit.
    std::fs::write(wt.join("src/extra.rs"), "fn three() {}\n").unwrap();
    git(&wt, &["add", "."]);
    git(&wt, &["commit", "-m", "phase 2 work"]);
    let phase_2_head = rev_parse(&wt, "HEAD");

    // AC2: `review source` defaults base to phase 2's started_head, so only
    // phase 2's own file is in scope — phase 1's is excluded.
    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "source",
            "--on",
            "phase/auth/phase-2-impl",
            "--project",
            "demo",
        ])
        .current_dir(src.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let source: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(source["base"], phase_1_head);
    assert_eq!(source["head"], phase_2_head);
    assert_eq!(source["changedFiles"], serde_json::json!(["src/extra.rs"]));
    assert!(
        source.get("baseNote").is_none(),
        "a resolved started_head must not carry a fallback note: {source}"
    );

    // `--base <sha>` still overrides.
    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "source",
            "--on",
            "phase/auth/phase-2-impl",
            "--base",
            &base_head,
            "--project",
            "demo",
        ])
        .current_dir(src.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let overridden: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(overridden["base"], base_head);

    // AC3: an approving `change/` review persisted at that base lets the
    // gated `reviewed` write through.
    create_approved_plan(plan.path(), "impl-plan", "phase/auth/phase-2-impl");
    let branch = current_branch(&wt);
    let change_id = start_review(
        plan.path(),
        Some(&wt),
        "change/HEAD",
        &["--implements", "plan/impl-plan", "--base", &phase_1_head],
    );
    // AC3's first half (review 2026-09-23-0326-b262, finding
    // ac-3-base-unasserted): the persisted review must actually RECORD the
    // base the gate is meant to bind against — passing `--base` at start
    // time isn't proof of that on its own, since the reviewed gate only
    // ever compares `head`.
    let change_review = review_show_json(plan.path(), &wt, &change_id);
    assert_eq!(
        change_review["target"]["base"], phase_1_head,
        "the persisted change review must record the phase's started_head as its base"
    );
    submit_approve(plan.path(), &change_id);

    // Default path: `review start --on change/HEAD` with no explicit
    // `--base` does NOT pick up the item's `started_head` — it resolves
    // through `resolve_change_target`'s plain merge-base-with-default-branch
    // path (rdm-core/src/change.rs), same as any other change review.
    // Documented here rather than changed: teaching `review start` to default
    // to `started_head` is out of this phase's scope. `rdm review source`
    // (and therefore `phase update --status reviewed --source ...`) is the
    // surface that defaults to `started_head`.
    let default_base_id = start_review(
        plan.path(),
        Some(&wt),
        "change/HEAD",
        &["--implements", "plan/impl-plan"],
    );
    let default_base_review = review_show_json(plan.path(), &wt, &default_base_id);
    assert_eq!(
        default_base_review["target"]["base"], base_head,
        "review start with no --base resolves against the merge-base with main, not the \
         phase's started_head"
    );

    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "phase",
            "update",
            "phase-2-impl",
            "--status",
            "reviewed",
            "--no-edit",
            "--roadmap",
            "auth",
            "--project",
            "demo",
            "--source",
            wt.to_str().unwrap(),
            "--base",
            &phase_1_head,
            "--expected-head",
            &phase_2_head,
            "--expected-branch",
            &branch,
        ])
        .current_dir(&wt)
        .assert()
        .success();
    assert_eq!(
        phase_json_for(plan.path(), "phase-2-impl", "auth")["status"],
        "reviewed"
    );
}

/// review 2026-09-23-0326-b262, finding `started-head-no-ancestry-check`,
/// checked at the binary boundary: a phase's recorded `started_head` remains
/// a resolvable commit object, but stops being an ancestor of HEAD once the
/// shared roadmap worktree's branch is rebased onto an advanced default
/// branch. `review source` must fall back to the merge-base with the default
/// branch rather than silently reviewing the wrong, unscoped range.
#[test]
fn review_source_falls_back_when_started_head_is_not_an_ancestor_at_the_binary_boundary() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "phase",
            "create",
            "impl",
            "--title",
            "Impl",
            "--number",
            "2",
            "--no-edit",
            "--roadmap",
            "auth",
            "--project",
            "demo",
        ])
        .assert()
        .success();

    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .args(["worktree", "add", "auth", "--project", "demo"])
        .current_dir(src.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let wt = std::path::PathBuf::from(String::from_utf8_lossy(&out).trim().to_string());
    let stamp_head = rev_parse(&wt, "HEAD");

    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "phase",
            "update",
            "phase-1-design",
            "--status",
            "in-progress",
            "--start-commit",
            &stamp_head,
            "--no-edit",
            "--roadmap",
            "auth",
            "--project",
            "demo",
        ])
        .current_dir(src.path())
        .assert()
        .success();

    std::fs::write(wt.join("src/lib.rs"), "fn one() {}\nfn two() {}\n").unwrap();
    git(&wt, &["add", "."]);
    git(&wt, &["commit", "-m", "phase 1 work"]);
    let phase_1_head = rev_parse(&wt, "HEAD");

    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "phase",
            "update",
            "phase-2-impl",
            "--status",
            "in-progress",
            "--start-commit",
            &phase_1_head,
            "--no-edit",
            "--roadmap",
            "auth",
            "--project",
            "demo",
        ])
        .current_dir(src.path())
        .assert()
        .success();
    assert_eq!(
        phase_json_for(plan.path(), "phase-2-impl", "auth")["started_head"],
        phase_1_head
    );

    std::fs::write(wt.join("src/extra.rs"), "fn three() {}\n").unwrap();
    git(&wt, &["add", "."]);
    git(&wt, &["commit", "-m", "phase 2 work"]);

    // `main` advances independently of the roadmap branch...
    std::fs::write(src.path().join("advance.txt"), "advanced\n").unwrap();
    git(src.path(), &["add", "."]);
    git(src.path(), &["commit", "-m", "advance main"]);
    let main_tip = rev_parse(src.path(), "HEAD");

    // ...and the shared roadmap worktree is rebased onto it, rewriting every
    // commit's SHA, including phase 1's recorded `started_head`.
    git(&wt, &["rebase", "main"]);
    let rebased_head = rev_parse(&wt, "HEAD");

    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "source",
            "--on",
            "phase/auth/phase-2-impl",
            "--project",
            "demo",
        ])
        .current_dir(src.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let source: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(source["head"], rebased_head);
    assert_eq!(
        source["base"], main_tip,
        "a non-ancestor started_head must fall back to the merge-base with main, not the stale \
         recorded value"
    );
    let note = source["baseNote"]
        .as_str()
        .expect("a non-ancestor started_head must explain the fallback via baseNote");
    assert!(
        note.contains(&phase_1_head) && note.contains("ancestor"),
        "note should name the stale started_head and explain the fallback: {note}"
    );
}

/// tests-2 (review 2026-09-23-0309-996c): AC2's merge-base fallback, checked
/// at the binary boundary rather than only at the `rdm-git` struct level. A
/// phase that never had `--start-commit` run against it has no recorded
/// `started_head`, so `review source` must fall back to the merge-base with
/// the project's default branch and say so via `baseNote` — the only prior
/// CLI assertion of the key was `.is_none()`, which would still pass if the
/// field were renamed or dropped outright.
#[test]
fn review_source_with_no_started_head_falls_back_to_merge_base_with_a_note() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    // A shared roadmap worktree with one commit on it, but `--start-commit`
    // was never run against the phase — no `started_head` was ever recorded.
    let wt = add_worktree(plan.path(), src.path());
    let merge_base = rev_parse(src.path(), "HEAD");
    let head = rev_parse(&wt, "HEAD");
    assert_ne!(
        merge_base, head,
        "the fixture's one commit on the worktree branch must make base and head diverge"
    );
    assert!(
        phase_json_for(plan.path(), "phase-1-design", "auth")
            .get("started_head")
            .is_none(),
        "phase-1-design must never have had --start-commit run against it in this test"
    );

    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "source",
            "--on",
            "phase/auth/phase-1-design",
            "--project",
            "demo",
        ])
        .current_dir(src.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let source: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(source["base"], merge_base);
    assert_eq!(source["head"], head);
    let note = source["baseNote"]
        .as_str()
        .unwrap_or_else(|| panic!("baseNote must be a string naming the default branch: {source}"));
    assert!(
        note.contains("main"),
        "baseNote must name the default branch 'main': {note}"
    );
}

/// review 2026-09-23-1257-1982, finding `ac2-pin-empty-range-on-fresh-item`:
/// the dispatch skill's step-4 identity pin runs `rdm review source --on
/// <item> --format json --no-code` — in exactly that form, with no
/// `--started_head` recorded and no commits beyond what `rdm worktree add`
/// itself creates — so `base` (the merge-base with the default branch)
/// equals `head`, and without `--no-code` this refuses with "empty committed
/// range". This is the AC2 case ("the pre-implementation identity pin
/// succeeds on a phase with no commits yet") checked at the binary boundary,
/// for a phase.
#[test]
fn review_source_no_code_succeeds_on_a_freshly_added_roadmap_worktree() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args(["worktree", "add", "auth", "--project", "demo"])
        .current_dir(src.path())
        .assert()
        .success();

    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "source",
            "--on",
            "phase/auth/phase-1-design",
            "--project",
            "demo",
            "--no-code",
            "--format",
            "json",
        ])
        .current_dir(src.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let source: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(
        source["base"], source["head"],
        "a freshly added worktree with no commits beyond the default branch must have base == head: {source}"
    );
    assert_eq!(source["changedFiles"], serde_json::json!([]));
}

/// Task-side twin of the phase test above: the dispatch skill's step-4 pin
/// must also succeed, with `--no-code`, on a freshly added task worktree.
#[test]
fn review_source_no_code_succeeds_on_a_freshly_added_task_worktree() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args(["worktree", "add", "task/solo", "--project", "demo"])
        .current_dir(src.path())
        .assert()
        .success();

    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "source",
            "--on",
            "task/solo",
            "--project",
            "demo",
            "--no-code",
            "--format",
            "json",
        ])
        .current_dir(src.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let source: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(
        source["base"], source["head"],
        "a freshly added worktree with no commits beyond the default branch must have base == head: {source}"
    );
    assert_eq!(source["changedFiles"], serde_json::json!([]));
}

/// Without `--no-code`, the same freshly added worktree refuses with "empty
/// committed range" — the regression the finding above reproduced. This is
/// the negative control proving the two tests above actually exercise the
/// fix, not a scenario that already had a non-empty range for some other
/// reason.
#[test]
fn review_source_without_no_code_refuses_on_a_freshly_added_worktree() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args(["worktree", "add", "auth", "--project", "demo"])
        .current_dir(src.path())
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "source",
            "--on",
            "phase/auth/phase-1-design",
            "--project",
            "demo",
            "--format",
            "json",
        ])
        .current_dir(src.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("empty committed range"));
}

/// The `explicit_source`-skips-full-validation special case for an
/// `in-progress` transition is gone under `explicit-start-commit`: the
/// e6edac4 unconditional behavior is restored, so `--status in-progress
/// --source <fresh worktree>` (HEAD == main, an empty committed range) now
/// refuses exactly like any other explicit source binding does, unless
/// `--no-code` declares the empty range intentional. Recording the start
/// commit is a completely separate, explicit `--start-commit` write that
/// does not go through this validation at all.
#[test]
fn phase_source_binding_on_a_fresh_worktree_requires_no_code() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .args(["worktree", "add", "auth", "--project", "demo"])
        .current_dir(src.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let wt = std::path::PathBuf::from(String::from_utf8_lossy(&out).trim().to_string());
    let wt_head = rev_parse(&wt, "HEAD");

    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "phase",
            "update",
            "phase-1-design",
            "--status",
            "in-progress",
            "--no-edit",
            "--roadmap",
            "auth",
            "--project",
            "demo",
            "--source",
            wt.to_str().unwrap(),
        ])
        .current_dir(src.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("empty committed range"));

    // `--start-commit`, combined with the same explicit `--source` and
    // `--no-code` to satisfy the source binding, records the field from that
    // explicit checkout's HEAD rather than the registry lookup.
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "phase",
            "update",
            "phase-1-design",
            "--status",
            "in-progress",
            "--start-commit",
            &wt_head,
            "--no-code",
            "--no-edit",
            "--roadmap",
            "auth",
            "--project",
            "demo",
            "--source",
            wt.to_str().unwrap(),
        ])
        .current_dir(src.path())
        .assert()
        .success();
    assert_eq!(
        phase_json_for(plan.path(), "phase-1-design", "auth")["started_head"],
        wt_head
    );
}

/// Task-side twin of the phase test above: an explicit `--source <fresh
/// worktree>` on a task no longer bypasses full `review source` validation
/// for an `in-progress` transition — it fails on the task-side rule that an
/// explicit checkout requires `--base` too.
#[test]
fn task_source_binding_on_a_fresh_worktree_requires_base() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .args(["worktree", "add", "task/solo", "--project", "demo"])
        .current_dir(src.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let wt = std::path::PathBuf::from(String::from_utf8_lossy(&out).trim().to_string());
    let wt_head = rev_parse(&wt, "HEAD");

    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "task",
            "update",
            "solo",
            "--status",
            "in-progress",
            "--no-edit",
            "--project",
            "demo",
            "--source",
            wt.to_str().unwrap(),
        ])
        .current_dir(src.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("requires --base"));

    // `--start-commit`, combined with the same explicit `--source`, `--base`
    // and `--no-code` needed to satisfy the task's source binding, records
    // the field from that explicit checkout's HEAD.
    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "task",
            "update",
            "solo",
            "--status",
            "in-progress",
            "--start-commit",
            &wt_head,
            "--base",
            &wt_head,
            "--no-code",
            "--no-edit",
            "--project",
            "demo",
            "--source",
            wt.to_str().unwrap(),
        ])
        .current_dir(src.path())
        .assert()
        .success();
    assert_eq!(task_json_for(plan.path(), "solo")["started_head"], wt_head);
}

/// tests-2 (review 2026-09-23-0249-4ccc): task-side worktree resolution and
/// the task `review source` `started_head` default, exercised at the binary
/// boundary with a real `rdm worktree add task/<slug>` worktree — the task
/// mirror of `started_head_scopes_the_second_phase_review_and_satisfies_the_gate`.
/// Stamps in-progress from the MAIN checkout (outside the worktree, so
/// resolution goes through the registered worktree, not the caller's cwd),
/// then after one commit confirms `review source --on task/<slug>` defaults
/// its base to the recorded `started_head`.
#[test]
fn task_started_head_scopes_review_source_to_the_tasks_own_commit() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .args(["worktree", "add", "task/solo", "--project", "demo"])
        .current_dir(src.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let wt = std::path::PathBuf::from(String::from_utf8_lossy(&out).trim().to_string());
    let base_head = rev_parse(&wt, "HEAD");

    rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "task",
            "update",
            "solo",
            "--status",
            "in-progress",
            "--start-commit",
            &base_head,
            "--no-edit",
            "--project",
            "demo",
        ])
        .current_dir(src.path())
        .assert()
        .success();
    assert_eq!(
        task_json_for(plan.path(), "solo")["started_head"],
        base_head
    );

    std::fs::write(wt.join("src/extra.rs"), "fn extra() {}\n").unwrap();
    git(&wt, &["add", "."]);
    git(&wt, &["commit", "-m", "task work"]);
    let task_head = rev_parse(&wt, "HEAD");

    // Write-once: a second `--start-commit` against the task is refused,
    // naming the already-recorded SHA, and does not move it.
    let refused = rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "task",
            "update",
            "solo",
            "--start-commit",
            &task_head,
            "--no-edit",
            "--project",
            "demo",
        ])
        .current_dir(src.path())
        .assert()
        .failure();
    let refused_stderr = String::from_utf8_lossy(&refused.get_output().stderr).to_string();
    assert!(
        refused_stderr.contains(&base_head) && refused_stderr.contains("write-once"),
        "the write-once refusal must name the already-recorded SHA and say 'write-once': {refused_stderr}"
    );
    assert_eq!(
        task_json_for(plan.path(), "solo")["started_head"],
        base_head,
        "a refused task --start-commit must never move an already-recorded started_head"
    );

    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .args(["review", "source", "--on", "task/solo", "--project", "demo"])
        .current_dir(src.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let source: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(source["base"], base_head);
    assert_eq!(source["head"], task_head);
}

// ---------------------------------------------------------------------------
// The gate resolves per project: `[projects.<p>] gates.reviewed`
// ---------------------------------------------------------------------------

/// Runs `rdm --root <plan>` with every gate env override removed, so only the
/// config decides.
fn rdm_at(plan: &Path) -> Command {
    let mut cmd = rdm();
    cmd.env_remove("RDM_GATES_REVIEWED").arg("--root").arg(plan);
    cmd
}

#[test]
fn a_project_scoped_gate_refuses_only_that_projects_reviewed_write() {
    let dir = TempDir::new().unwrap();
    let p = dir.path();
    rdm_at(p).arg("init").assert().success();
    for project in ["a", "b"] {
        rdm_at(p)
            .args(["project", "create", project])
            .assert()
            .success();
        rdm_at(p)
            .args([
                "roadmap",
                "create",
                "auth",
                "--title",
                "Auth",
                "--no-edit",
                "--project",
                project,
            ])
            .assert()
            .success();
        rdm_at(p)
            .args([
                "phase",
                "create",
                "design",
                "--title",
                "Design",
                "--number",
                "1",
                "--no-edit",
                "--roadmap",
                "auth",
                "--project",
                project,
            ])
            .assert()
            .success();
        rdm_at(p)
            .args([
                "phase",
                "update",
                "phase-1-design",
                "--status",
                "needs-review",
                "--no-edit",
                "--roadmap",
                "auth",
                "--project",
                project,
            ])
            .assert()
            .success();
    }
    // Project `a` opts in; there is no plan-repo-wide `[gates]` table.
    rdm_at(p)
        .args(["config", "set", "gates.reviewed", "true", "--project", "a"])
        .assert()
        .success();

    let mark = |project: &str| {
        rdm_at(p)
            .args([
                "phase",
                "update",
                "phase-1-design",
                "--status",
                "reviewed",
                "--no-edit",
                "--roadmap",
                "auth",
                "--project",
                project,
            ])
            .current_dir(p)
            .assert()
    };

    let err = stderr_of(mark("a").failure());
    assert!(
        err.contains("rdm plan create"),
        "project a's gate refuses with its first precondition: {err}"
    );
    mark("b").success();

    let status = |project: &str| -> Value {
        let out = rdm_at(p)
            .args([
                "phase",
                "show",
                "phase-1-design",
                "--format",
                "json",
                "--roadmap",
                "auth",
                "--project",
                project,
            ])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        serde_json::from_slice::<Value>(&out).unwrap()["status"].clone()
    };
    assert_eq!(status("a"), "needs-review");
    assert_eq!(status("b"), "reviewed");
}

#[test]
fn a_project_scoped_gate_refuses_only_that_projects_task_reviewed_write() {
    let dir = TempDir::new().unwrap();
    let p = dir.path();
    rdm_at(p).arg("init").assert().success();
    for project in ["a", "b"] {
        rdm_at(p)
            .args(["project", "create", project])
            .assert()
            .success();
        rdm_at(p)
            .args([
                "task",
                "create",
                "solo",
                "--title",
                "Solo",
                "--no-edit",
                "--project",
                project,
            ])
            .assert()
            .success();
        rdm_at(p)
            .args([
                "task",
                "update",
                "solo",
                "--status",
                "needs-review",
                "--no-edit",
                "--project",
                project,
            ])
            .assert()
            .success();
    }
    // Project `a` opts in; there is no plan-repo-wide `[gates]` table.
    rdm_at(p)
        .args(["config", "set", "gates.reviewed", "true", "--project", "a"])
        .assert()
        .success();

    let mark = |project: &str| {
        rdm_at(p)
            .args([
                "task",
                "update",
                "solo",
                "--status",
                "reviewed",
                "--no-edit",
                "--project",
                project,
            ])
            .current_dir(p)
            .assert()
    };

    let err = stderr_of(mark("a").failure());
    assert!(
        err.contains("rdm plan create"),
        "project a's gate refuses with its first precondition: {err}"
    );
    mark("b").success();

    let status = |project: &str| -> Value {
        let out = rdm_at(p)
            .args([
                "task",
                "show",
                "solo",
                "--format",
                "json",
                "--project",
                project,
            ])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        serde_json::from_slice::<Value>(&out).unwrap()["status"].clone()
    };
    assert_eq!(status("a"), "needs-review");
    assert_eq!(status("b"), "reviewed");
}

// ---------------------------------------------------------------------------
// `review source` defaults its base to the merge-base with the project's
// `default_branch`: `[projects.<p>] default_branch`
// ---------------------------------------------------------------------------

#[test]
fn review_source_base_follows_the_projects_default_branch() {
    let src = init_source_repo();
    let plan = init_plan_repo(src.path());
    let path = plan.path().join("rdm.toml");
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    std::fs::write(
        &path,
        format!("{existing}\n[projects.demo]\ndefault_branch = \"develop\"\n"),
    )
    .unwrap();

    // `develop` gains a commit and `main` then advances on its own; the
    // roadmap worktree branches from `develop`, so its merge-bases with the
    // two branches differ.
    let p = src.path();
    git(p, &["checkout", "-b", "develop"]);
    std::fs::write(p.join("develop.txt"), "develop\n").unwrap();
    git(p, &["add", "."]);
    git(p, &["commit", "-m", "develop work"]);
    git(p, &["checkout", "main"]);
    std::fs::write(p.join("main.txt"), "main\n").unwrap();
    git(p, &["add", "."]);
    git(p, &["commit", "-m", "main work"]);
    git(p, &["checkout", "develop"]);
    let wt = add_worktree(plan.path(), p);
    let develop_base = rev_parse(&wt, "develop");
    let main_base = rev_parse(&wt, "main~1");
    assert_ne!(develop_base, main_base, "the fixture branches must diverge");

    let out = rdm()
        .arg("--root")
        .arg(plan.path())
        .args([
            "review",
            "source",
            "--on",
            "phase/auth/phase-1-design",
            "--project",
            "demo",
        ])
        .current_dir(p)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let source: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(source["base"], develop_base);
}

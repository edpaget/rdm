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

fn rdm() -> Command {
    let mut cmd = Command::cargo_bin("rdm").unwrap();
    cmd.env("XDG_CONFIG_HOME", "/dev/null/nonexistent");
    cmd.env_remove("RDM_PROJECT")
        .env_remove("RDM_ROOT")
        .env_remove("RDM_REVIEWED_GATE");
    cmd
}

fn git(dir: &Path, args: &[&str]) -> std::process::Output {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@test.com")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@test.com")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out
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

/// Adds an rdm worktree for the `auth` roadmap and returns its path.
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
    std::path::PathBuf::from(String::from_utf8_lossy(&out).trim().to_string())
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

    // Rung 4: committing repairs moves HEAD; the old review cannot approve it.
    git(&wt, &["add", "."]);
    git(&wt, &["commit", "-m", "scratch"]);
    mark_reviewed(plan.path(), &wt, &[]).failure();
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

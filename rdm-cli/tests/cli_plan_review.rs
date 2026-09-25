use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn rdm() -> Command {
    let mut cmd = Command::cargo_bin("rdm").unwrap();
    // Isolate from host global config (e.g. default_format = "json").
    cmd.env("XDG_CONFIG_HOME", "/dev/null/nonexistent");
    cmd
}

fn init_with_project(dir: &TempDir) {
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("init")
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "create", "acme"])
        .assert()
        .success();
}

/// Enables `plan_review` in the repo `rdm.toml` via `rdm config set`.
fn enable_plan_review(dir: &TempDir) {
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["config", "set", "plan_review", "true"])
        .assert()
        .success();
}

fn create_phase(dir: &TempDir, roadmap: &str, slug: &str, title: &str, number: &str) {
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "create",
            slug,
            "--title",
            title,
            "--roadmap",
            roadmap,
            "--project",
            "acme",
            "--number",
            number,
            "--no-edit",
        ])
        .assert()
        .success();
}

#[test]
fn roadmap_create_stamps_tag_when_plan_review_enabled() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);
    enable_plan_review(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "roadmap",
            "create",
            "widget-launch",
            "--title",
            "Widget Launch",
            "--project",
            "acme",
            "--no-edit",
        ])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["roadmap", "show", "widget-launch", "--project", "acme"])
        .assert()
        .success()
        .stdout(predicate::str::contains("needs-plan-review"));
}

#[test]
fn phase_create_stamps_tag_when_plan_review_enabled() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);
    enable_plan_review(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "roadmap",
            "create",
            "widget-launch",
            "--title",
            "Widget Launch",
            "--project",
            "acme",
            "--no-edit",
        ])
        .assert()
        .success();
    create_phase(&dir, "widget-launch", "design", "Design the Widget", "1");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "show",
            "1",
            "--roadmap",
            "widget-launch",
            "--project",
            "acme",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("needs-plan-review"));
}

#[test]
fn task_create_stamps_tag_when_plan_review_enabled() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);
    enable_plan_review(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "task",
            "create",
            "fix-login-bug",
            "--title",
            "Fix Login Bug",
            "--project",
            "acme",
            "--no-edit",
        ])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["task", "show", "fix-login-bug", "--project", "acme"])
        .assert()
        .success()
        .stdout(predicate::str::contains("needs-plan-review"));
}

#[test]
fn create_does_not_stamp_tag_when_plan_review_disabled() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);
    // plan_review left at its default (false).

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "task",
            "create",
            "fix-login-bug",
            "--title",
            "Fix Login Bug",
            "--project",
            "acme",
            "--no-edit",
        ])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["task", "show", "fix-login-bug", "--project", "acme"])
        .assert()
        .success()
        .stdout(predicate::str::contains("needs-plan-review").not());
}

#[test]
fn task_create_no_plan_review_skips_stamp_even_when_config_enabled() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);
    enable_plan_review(&dir);

    // With the flag: no stamp, even though plan_review is on.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "task",
            "create",
            "finding-followup",
            "--title",
            "Finding follow-up",
            "--project",
            "acme",
            "--no-edit",
            "--no-plan-review",
        ])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["task", "show", "finding-followup", "--project", "acme"])
        .assert()
        .success()
        .stdout(predicate::str::contains("needs-plan-review").not());

    // Without the flag, an otherwise-identical create still stamps it.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "task",
            "create",
            "regular-task",
            "--title",
            "Regular task",
            "--project",
            "acme",
            "--no-edit",
        ])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["task", "show", "regular-task", "--project", "acme"])
        .assert()
        .success()
        .stdout(predicate::str::contains("needs-plan-review"));
}

#[test]
fn create_with_plan_review_enabled_preserves_user_supplied_tags() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);
    enable_plan_review(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "task",
            "create",
            "fix-login-bug",
            "--title",
            "Fix Login Bug",
            "--project",
            "acme",
            "--tags",
            "bug,urgent",
            "--no-edit",
        ])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["task", "show", "fix-login-bug", "--project", "acme"])
        .assert()
        .success()
        .stdout(predicate::str::contains("bug"))
        .stdout(predicate::str::contains("urgent"))
        .stdout(predicate::str::contains("needs-plan-review"));
}

#[test]
fn create_stamps_tag_via_rdm_plan_review_env_var() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);
    // No rdm.toml `plan_review` key set — env var alone drives it.

    rdm()
        .arg("--root")
        .arg(dir.path())
        .env("RDM_PLAN_REVIEW", "true")
        .args([
            "task",
            "create",
            "fix-login-bug",
            "--title",
            "Fix Login Bug",
            "--project",
            "acme",
            "--no-edit",
        ])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["task", "show", "fix-login-bug", "--project", "acme"])
        .assert()
        .success()
        .stdout(predicate::str::contains("needs-plan-review"));
}

#[test]
fn search_tag_needs_plan_review_lists_pending_phase_and_task() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);
    enable_plan_review(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "roadmap",
            "create",
            "widget-launch",
            "--title",
            "Widget Launch",
            "--project",
            "acme",
            "--no-edit",
        ])
        .assert()
        .success();
    create_phase(&dir, "widget-launch", "design", "Design the Widget", "1");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "task",
            "create",
            "fix-login-bug",
            "--title",
            "Fix Login Bug",
            "--project",
            "acme",
            "--no-edit",
        ])
        .assert()
        .success();

    // --type phase returns only the phase.
    let output = rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "search",
            "",
            "--tag",
            "needs-plan-review",
            "--type",
            "phase",
            "--format",
            "json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(output).unwrap();
    assert!(
        stdout.contains("design"),
        "expected phase in output: {stdout}"
    );
    assert!(
        !stdout.contains("fix-login-bug"),
        "task should not appear when filtering --type phase: {stdout}"
    );

    // --type task returns only the task.
    let output = rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "search",
            "",
            "--tag",
            "needs-plan-review",
            "--type",
            "task",
            "--format",
            "json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(output).unwrap();
    assert!(
        stdout.contains("fix-login-bug"),
        "expected task in output: {stdout}"
    );
    assert!(
        !stdout.contains("\"design\""),
        "phase should not appear when filtering --type task: {stdout}"
    );
}

#[test]
fn search_tag_needs_plan_review_empty_when_plan_review_disabled() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);
    // plan_review left disabled — nothing should ever get tagged.

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "task",
            "create",
            "fix-login-bug",
            "--title",
            "Fix Login Bug",
            "--project",
            "acme",
            "--no-edit",
        ])
        .assert()
        .success();

    let output = rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "search",
            "",
            "--tag",
            "needs-plan-review",
            "--format",
            "json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(output).unwrap();
    assert_eq!(stdout.trim(), "[]");
}

// ---------------------------------------------------------------------------
// `plan_review` resolves per project: `[projects.<p>] plan_review` overrides
// the plan-repo-wide value, and `RDM_PLAN_REVIEW` overrides both.
// ---------------------------------------------------------------------------

/// A plan repo with projects `a` and `b`.
fn init_two_projects() -> TempDir {
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("init")
        .assert()
        .success();
    for project in ["a", "b"] {
        rdm()
            .arg("--root")
            .arg(dir.path())
            .args(["project", "create", project])
            .assert()
            .success();
    }
    dir
}

fn set_project_plan_review(dir: &TempDir, project: &str, value: &str) {
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["config", "set", "plan_review", value, "--project", project])
        .assert()
        .success();
}

/// Creates task `slug` in `project` (with `env` applied) and reports whether
/// it was stamped with `needs-plan-review`.
fn task_create_stamps(dir: &TempDir, project: &str, slug: &str, env: Option<&str>) -> bool {
    let mut cmd = rdm();
    cmd.env_remove("RDM_PLAN_REVIEW");
    if let Some(v) = env {
        cmd.env("RDM_PLAN_REVIEW", v);
    }
    cmd.arg("--root")
        .arg(dir.path())
        .args([
            "task",
            "create",
            slug,
            "--title",
            "A task",
            "--project",
            project,
            "--no-edit",
        ])
        .assert()
        .success();
    let out = rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["task", "show", slug, "--project", project, "--no-body"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    String::from_utf8_lossy(&out).contains("needs-plan-review")
}

#[test]
fn task_create_honors_a_project_scoped_plan_review() {
    let dir = init_two_projects();
    enable_plan_review(&dir);
    set_project_plan_review(&dir, "b", "false");

    assert!(
        task_create_stamps(&dir, "a", "in-a", None),
        "project a inherits the plan-repo-wide plan_review = true"
    );
    assert!(
        !task_create_stamps(&dir, "b", "in-b", None),
        "project b's override turns the stamp off"
    );
}

/// Runs `rdm --root <dir> <args>` with `RDM_PLAN_REVIEW` unset and returns
/// its stdout.
fn run_in(dir: &TempDir, args: &[&str]) -> String {
    let out = rdm()
        .env_remove("RDM_PLAN_REVIEW")
        .arg("--root")
        .arg(dir.path())
        .args(args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    String::from_utf8_lossy(&out).into_owned()
}

/// Creates roadmap `r` with phase 1 in `project` and reports whether the
/// roadmap and the phase were each stamped with `needs-plan-review`.
fn roadmap_and_phase_create_stamp(dir: &TempDir, project: &str) -> (bool, bool) {
    run_in(
        dir,
        &[
            "roadmap",
            "create",
            "r",
            "--title",
            "R",
            "--project",
            project,
            "--no-edit",
        ],
    );
    run_in(
        dir,
        &[
            "phase",
            "create",
            "p",
            "--title",
            "P",
            "--number",
            "1",
            "--roadmap",
            "r",
            "--project",
            project,
            "--no-edit",
        ],
    );
    let roadmap = run_in(
        dir,
        &["roadmap", "show", "r", "--project", project, "--no-body"],
    );
    let phase = run_in(
        dir,
        &[
            "phase",
            "show",
            "1",
            "--roadmap",
            "r",
            "--project",
            project,
            "--no-body",
        ],
    );
    (
        roadmap.contains("needs-plan-review"),
        phase.contains("needs-plan-review"),
    )
}

#[test]
fn roadmap_and_phase_create_honor_a_project_scoped_plan_review() {
    let dir = init_two_projects();
    enable_plan_review(&dir);
    set_project_plan_review(&dir, "b", "false");

    assert_eq!(
        roadmap_and_phase_create_stamp(&dir, "a"),
        (true, true),
        "project a's roadmap and phase inherit the plan-repo-wide plan_review = true"
    );
    assert_eq!(
        roadmap_and_phase_create_stamp(&dir, "b"),
        (false, false),
        "project b's override turns the roadmap and phase stamps off"
    );
}

#[test]
fn rdm_plan_review_env_overrides_a_project_scoped_value_in_both_directions() {
    let dir = init_two_projects();
    set_project_plan_review(&dir, "a", "true");
    set_project_plan_review(&dir, "b", "false");

    assert!(
        task_create_stamps(&dir, "b", "env-on", Some("true")),
        "RDM_PLAN_REVIEW=true must beat [projects.b] plan_review = false"
    );
    assert!(
        !task_create_stamps(&dir, "a", "env-off", Some("false")),
        "RDM_PLAN_REVIEW=false must beat [projects.a] plan_review = true"
    );
}

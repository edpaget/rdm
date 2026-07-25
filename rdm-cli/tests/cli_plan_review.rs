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

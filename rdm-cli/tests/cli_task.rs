use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn rdm() -> Command {
    Command::cargo_bin("rdm").unwrap()
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
        .args(["project", "create", "fbm"])
        .assert()
        .success();
}

fn create_task(dir: &TempDir, slug: &str, title: &str) {
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["task", "create", slug, "--title", title, "--project", "fbm"])
        .assert()
        .success();
}

#[test]
fn task_create_and_show() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "task",
            "create",
            "fix-bug",
            "--title",
            "Fix the bug",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Created task 'fix-bug'"));

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["task", "show", "fix-bug", "--project", "fbm"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("# Fix the bug")
                .and(predicate::str::contains("Slug: fix-bug"))
                .and(predicate::str::contains("Status: open"))
                .and(predicate::str::contains("Priority: medium")),
        );
}

#[test]
fn task_create_with_tags() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "task",
            "create",
            "fix-bug",
            "--title",
            "Fix the bug",
            "--project",
            "fbm",
            "--tags",
            "bug,urgent",
        ])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["task", "show", "fix-bug", "--project", "fbm"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Tags: bug, urgent"));
}

#[test]
fn task_list_default_filters() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);

    create_task(&dir, "open-task", "Open Task");
    create_task(&dir, "done-task", "Done Task");

    // Mark one as done
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "task",
            "update",
            "done-task",
            "--status",
            "done",
            "--project",
            "fbm",
        ])
        .assert()
        .success();

    // Default list should show only open/in-progress
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["task", "list", "--project", "fbm"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("open-task").and(predicate::str::contains("done-task").not()),
        );
}

#[test]
fn task_list_status_all() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);

    create_task(&dir, "open-task", "Open Task");
    create_task(&dir, "done-task", "Done Task");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "task",
            "update",
            "done-task",
            "--status",
            "done",
            "--project",
            "fbm",
        ])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["task", "list", "--project", "fbm", "--status", "all"])
        .assert()
        .success()
        .stdout(predicate::str::contains("open-task").and(predicate::str::contains("done-task")));
}

#[test]
fn task_list_filter_by_priority() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "task",
            "create",
            "high-task",
            "--title",
            "High",
            "--project",
            "fbm",
            "--priority",
            "high",
        ])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "task",
            "create",
            "low-task",
            "--title",
            "Low",
            "--project",
            "fbm",
            "--priority",
            "low",
        ])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["task", "list", "--project", "fbm", "--priority", "high"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("high-task").and(predicate::str::contains("low-task").not()),
        );
}

#[test]
fn task_list_filter_by_tag() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "task",
            "create",
            "tagged-task",
            "--title",
            "Tagged",
            "--project",
            "fbm",
            "--tags",
            "bug",
        ])
        .assert()
        .success();

    create_task(&dir, "untagged-task", "Untagged");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["task", "list", "--project", "fbm", "--tag", "bug"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("tagged-task")
                .and(predicate::str::contains("untagged-task").not()),
        );
}

#[test]
fn task_update_status() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);
    create_task(&dir, "my-task", "My Task");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "task",
            "update",
            "my-task",
            "--status",
            "done",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("status: done"));

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["task", "show", "my-task", "--project", "fbm"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Status: done"));
}

#[test]
fn task_update_priority() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);
    create_task(&dir, "my-task", "My Task");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "task",
            "update",
            "my-task",
            "--priority",
            "critical",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("priority: critical"));
}

#[test]
fn task_update_tags() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);
    create_task(&dir, "my-task", "My Task");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "task",
            "update",
            "my-task",
            "--tags",
            "new-tag,other",
            "--project",
            "fbm",
        ])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["task", "show", "my-task", "--project", "fbm"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Tags: new-tag, other"));
}

#[test]
fn task_create_missing_project() {
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("init")
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "task",
            "create",
            "my-task",
            "--title",
            "Task",
            "--project",
            "nope",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("project not found"));
}

#[test]
fn task_create_duplicate() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);
    create_task(&dir, "my-task", "My Task");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "task",
            "create",
            "my-task",
            "--title",
            "Dup",
            "--project",
            "fbm",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("already exists"));
}

#[test]
fn promote_task_to_roadmap() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);
    create_task(&dir, "big-feature", "Big Feature");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "promote",
            "big-feature",
            "--roadmap-slug",
            "big-feature-rm",
            "--project",
            "fbm",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Promoted task 'big-feature' → roadmap 'big-feature-rm'",
        ));

    // Verify roadmap was created
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["roadmap", "show", "big-feature-rm", "--project", "fbm"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Big Feature"));

    // Verify task is gone
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["task", "show", "big-feature", "--project", "fbm"])
        .assert()
        .failure();
}

#[test]
fn promote_nonexistent_task() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "promote",
            "nope",
            "--roadmap-slug",
            "rm-slug",
            "--project",
            "fbm",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("task not found"));
}

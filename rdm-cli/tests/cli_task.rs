use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
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

#[test]
fn task_create_with_body_flag() {
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
            "--body",
            "Task body content here.",
        ])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["task", "show", "fix-bug", "--project", "fbm"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Task body content here."));
}

#[test]
fn task_update_with_body_flag() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);
    create_task(&dir, "fix-bug", "Fix the bug");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "task",
            "update",
            "fix-bug",
            "--project",
            "fbm",
            "--body",
            "Updated task body.",
        ])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["task", "show", "fix-bug", "--project", "fbm"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Updated task body."));
}

#[test]
fn task_create_with_stdin_pipe() {
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
        .write_stdin("piped task body")
        .assert()
        .success();

    let task_file = dir.path().join("projects/fbm/tasks/fix-bug.md");
    let content = fs::read_to_string(&task_file).unwrap();
    assert!(
        content.contains("piped task body"),
        "expected piped content in file, got: {content}"
    );
}

#[test]
fn body_flag_beats_stdin() {
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
            "--body",
            "inline body",
        ])
        .write_stdin("piped body")
        .assert()
        .success();

    let task_file = dir.path().join("projects/fbm/tasks/fix-bug.md");
    let content = fs::read_to_string(&task_file).unwrap();
    assert!(
        content.contains("inline body"),
        "expected inline --body content in file, got: {content}"
    );
    assert!(
        !content.contains("piped body"),
        "stdin should be ignored when --body is provided, got: {content}"
    );
}

#[test]
fn task_show_body_and_no_body() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);
    create_task(&dir, "fix-bug", "Fix the bug");

    // Append body text to the task file
    let task_file = dir.path().join("projects/fbm/tasks/fix-bug.md");
    let content = fs::read_to_string(&task_file).unwrap();
    fs::write(
        &task_file,
        format!("{content}\n## Notes\n\nTask body content.\n"),
    )
    .unwrap();

    // show includes body
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["task", "show", "fix-bug", "--project", "fbm"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("# Fix the bug")
                .and(predicate::str::contains("Task body content.")),
        );

    // show --no-body suppresses body
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["task", "show", "fix-bug", "--project", "fbm", "--no-body"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("# Fix the bug")
                .and(predicate::str::contains("Task body content.").not()),
        );
}

#[test]
fn task_create_no_edit_skips_editor() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "task",
            "create",
            "no-edit-task",
            "--title",
            "No Edit",
            "--project",
            "fbm",
            "--no-edit",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Created task 'no-edit-task'"));

    // Verify no body content was written
    let task_file = dir.path().join("projects/fbm/tasks/no-edit-task.md");
    let content = fs::read_to_string(&task_file).unwrap();
    // File should have frontmatter but no body after the closing ---
    let after_frontmatter = content.split("---").nth(2).unwrap_or("");
    assert!(
        after_frontmatter.trim().is_empty(),
        "expected no body, got: {after_frontmatter}"
    );
}

#[test]
fn task_update_no_edit_skips_editor() {
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
            "in-progress",
            "--project",
            "fbm",
            "--no-edit",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("status: in-progress"));
}

#[test]
fn task_create_no_edit_with_body_flag_still_works() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);

    // --no-edit + --body should use the body flag content
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "task",
            "create",
            "both-flags",
            "--title",
            "Both Flags",
            "--project",
            "fbm",
            "--body",
            "Explicit body.",
            "--no-edit",
        ])
        .assert()
        .success();

    let task_file = dir.path().join("projects/fbm/tasks/both-flags.md");
    let content = fs::read_to_string(&task_file).unwrap();
    assert!(
        content.contains("Explicit body."),
        "expected body content, got: {content}"
    );
}

fn head_sha(dir: &std::path::Path) -> String {
    let repo = gix::open(dir).unwrap();
    let mut head = repo.head().unwrap();
    head.peel_to_commit().unwrap().id.to_string()
}

#[test]
fn task_show_at_revision_returns_historical_body() {
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
            "--body",
            "original-task-body",
            "--no-edit",
        ])
        .assert()
        .success();

    let old_sha = head_sha(dir.path());

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "task",
            "update",
            "fix-bug",
            "--project",
            "fbm",
            "--body",
            "new-task-body",
            "--no-edit",
        ])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "task",
            "show",
            "fix-bug",
            "--project",
            "fbm",
            "--at",
            &old_sha,
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("original-task-body")
                .and(predicate::str::contains(format!("Revision: {old_sha}")))
                .and(predicate::str::contains("new-task-body").not()),
        );
}

#[test]
fn task_show_at_unknown_revision_errors() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);
    create_task(&dir, "fix-bug", "Fix Bug");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "task",
            "show",
            "fix-bug",
            "--project",
            "fbm",
            "--at",
            "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("is not known to the store"));
}

#[test]
fn task_show_at_revision_missing_path_errors() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);

    // Anchor before task exists.
    let pre_sha = head_sha(dir.path());

    create_task(&dir, "fix-bug", "Fix Bug");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "task",
            "show",
            "fix-bug",
            "--project",
            "fbm",
            "--at",
            &pre_sha,
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("is not present at revision"));
}

#[test]
fn task_update_body_flag_beats_stdin() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);
    create_task(&dir, "fix-bug", "Fix the bug");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "task",
            "update",
            "fix-bug",
            "--project",
            "fbm",
            "--body",
            "inline body wins",
        ])
        .write_stdin("piped body loses")
        .assert()
        .success();

    let task_file = dir.path().join("projects/fbm/tasks/fix-bug.md");
    let content = fs::read_to_string(&task_file).unwrap();
    assert!(
        content.contains("inline body wins"),
        "expected inline body in file, got: {content}"
    );
    assert!(
        !content.contains("piped body loses"),
        "stdin must be ignored when --body is provided, got: {content}"
    );
}

#[test]
fn task_update_empty_body_refuses_clobber() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);
    create_task(&dir, "fix-bug", "Fix the bug");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "task",
            "update",
            "fix-bug",
            "--project",
            "fbm",
            "--body",
            "existing content",
            "--no-edit",
        ])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "task",
            "update",
            "fix-bug",
            "--project",
            "fbm",
            "--body",
            "",
            "--no-edit",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--clear-body"));

    let task_file = dir.path().join("projects/fbm/tasks/fix-bug.md");
    let content = fs::read_to_string(&task_file).unwrap();
    assert!(
        content.contains("existing content"),
        "body should be unchanged after refused clobber, got: {content}"
    );
}

#[test]
fn task_update_clear_body_succeeds() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);
    create_task(&dir, "fix-bug", "Fix the bug");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "task",
            "update",
            "fix-bug",
            "--project",
            "fbm",
            "--body",
            "existing content",
            "--no-edit",
        ])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "task",
            "update",
            "fix-bug",
            "--project",
            "fbm",
            "--clear-body",
            "--no-edit",
        ])
        .assert()
        .success();

    let task_file = dir.path().join("projects/fbm/tasks/fix-bug.md");
    let content = fs::read_to_string(&task_file).unwrap();
    assert!(
        !content.contains("existing content"),
        "body should be empty after --clear-body, got: {content}"
    );
}

#[test]
fn task_update_empty_body_ok_when_already_empty() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);
    create_task(&dir, "fix-bug", "Fix the bug");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "task",
            "update",
            "fix-bug",
            "--project",
            "fbm",
            "--body",
            "",
            "--no-edit",
        ])
        .assert()
        .success();
}

#[test]
fn task_update_clear_body_conflicts_with_body_flag() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);
    create_task(&dir, "fix-bug", "Fix the bug");

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "task",
            "update",
            "fix-bug",
            "--project",
            "fbm",
            "--body",
            "x",
            "--clear-body",
            "--no-edit",
        ])
        .assert()
        .failure();
}

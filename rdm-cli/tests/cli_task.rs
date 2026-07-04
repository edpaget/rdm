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
fn task_list_default_shows_active_tasks() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);

    create_task(&dir, "open-task", "Open Task");
    create_task(&dir, "in-progress-task", "In Progress Task");
    create_task(&dir, "needs-review-task", "Needs Review Task");
    create_task(&dir, "reviewed-task", "Reviewed Task");
    create_task(&dir, "done-task", "Done Task");
    create_task(&dir, "wont-fix-task", "Wont Fix Task");

    // Set statuses
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "task",
            "update",
            "in-progress-task",
            "--status",
            "in-progress",
            "--project",
            "fbm",
        ])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "task",
            "update",
            "needs-review-task",
            "--status",
            "needs-review",
            "--project",
            "fbm",
        ])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "task",
            "update",
            "reviewed-task",
            "--status",
            "reviewed",
            "--project",
            "fbm",
        ])
        .assert()
        .success();

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
        .args([
            "task",
            "update",
            "wont-fix-task",
            "--status",
            "wont-fix",
            "--project",
            "fbm",
        ])
        .assert()
        .success();

    // Default list should show active tasks (open, in-progress, needs-review, reviewed)
    // but NOT done or wont-fix tasks
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["task", "list", "--project", "fbm"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("open-task")
                .and(predicate::str::contains("in-progress-task"))
                .and(predicate::str::contains("needs-review-task"))
                .and(predicate::str::contains("reviewed-task"))
                .and(predicate::str::contains("done-task").not())
                .and(predicate::str::contains("wont-fix-task").not()),
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

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["commit", "-m", "seed: create task with original body"])
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
        .args(["commit", "-m", "feat: update task body"])
        .assert()
        .success();

    let new_sha = head_sha(dir.path());
    assert_ne!(old_sha, new_sha, "the update commit should have moved HEAD");

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
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["commit", "-m", "seed: init plan repo and project"])
        .assert()
        .success();

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

/// Body content covering the reported hang triggers: backticks, em-dash,
/// curly quotes/ellipsis, shell metacharacters, and a literal `--no-edit`
/// substring embedded in the value (not passed as a separate flag).
const SPECIAL_BODY: &str = r#"backtick `code` em-dash — curly “quotes” ‘single’ ellipsis … shell $!\;|<>*~&& literal --no-edit here"#;

#[test]
fn task_update_with_special_character_body_content() {
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
            SPECIAL_BODY,
            "--no-edit",
        ])
        .assert()
        .success();

    let task_file = dir.path().join("projects/fbm/tasks/fix-bug.md");
    let content = fs::read_to_string(&task_file).unwrap();
    assert!(
        content.contains(SPECIAL_BODY),
        "expected special-character body to round-trip verbatim, got: {content}"
    );
}

#[test]
fn task_create_with_special_character_body_content() {
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
            SPECIAL_BODY,
            "--no-edit",
        ])
        .assert()
        .success();

    let task_file = dir.path().join("projects/fbm/tasks/fix-bug.md");
    let content = fs::read_to_string(&task_file).unwrap();
    assert!(
        content.contains(SPECIAL_BODY),
        "expected special-character body to round-trip verbatim, got: {content}"
    );
}

/// Reproduces the reported hang mechanism directly: an orchestrator that
/// spawns the process with `--body` set but holds stdin open via a pipe
/// that is never written to and never closed. Since `--body` is
/// authoritative, `resolve_body` must never read stdin, so the process
/// must exit promptly regardless of the open pipe.
#[test]
fn task_create_body_flag_no_hang_with_open_stdin_pipe() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);

    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_rdm"))
        .env("XDG_CONFIG_HOME", "/dev/null/nonexistent")
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
            SPECIAL_BODY,
            "--no-edit",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();

    // Keep the child's stdin pipe open (never written, never closed) —
    // dropping it would deliver EOF and defeat the point of this test.
    let _stdin = child.stdin.take();

    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let status = child.wait();
        let _ = tx.send(status);
    });

    let status = rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("rdm task create --body must not hang with stdin held open")
        .unwrap();

    assert!(status.success());
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

#[test]
fn task_update_title_renames_in_place() {
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
            "Old Task Title",
            "--project",
            "fbm",
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
            "--title",
            "New Task Title",
            "--project",
            "fbm",
        ])
        .assert()
        .success();

    // New title reflected via `show`; slug unchanged.
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["task", "show", "fix-bug", "--project", "fbm"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("New Task Title").and(predicate::str::contains("fix-bug")),
        );
}

#[test]
fn task_update_empty_title_rejected() {
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
            "Keep Task Title",
            "--project",
            "fbm",
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
            "--title",
            "\t",
            "--project",
            "fbm",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("title cannot be empty"));

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["task", "show", "fix-bug", "--project", "fbm"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Keep Task Title"));
}

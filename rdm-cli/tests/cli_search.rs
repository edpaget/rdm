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
        .args(["project", "create", "acme"])
        .assert()
        .success();
}

fn create_roadmap(dir: &TempDir, slug: &str, title: &str) {
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "roadmap",
            "create",
            slug,
            "--title",
            title,
            "--project",
            "acme",
            "--no-edit",
        ])
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

fn create_task(dir: &TempDir, slug: &str, title: &str) {
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "task",
            "create",
            slug,
            "--title",
            title,
            "--project",
            "acme",
            "--no-edit",
        ])
        .assert()
        .success();
}

fn setup_test_data(dir: &TempDir) {
    init_with_project(dir);
    create_roadmap(dir, "widget-launch", "Widget Launch");
    create_phase(dir, "widget-launch", "design", "Design the Widget", "1");
    create_phase(
        dir,
        "widget-launch",
        "implementation",
        "Implement the Widget",
        "2",
    );
    // Mark phase 1 as done
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "update",
            "1",
            "--status",
            "done",
            "--roadmap",
            "widget-launch",
            "--project",
            "acme",
            "--no-edit",
        ])
        .assert()
        .success();
    create_task(dir, "fix-login-bug", "Fix Login Bug");
    create_task(dir, "add-search", "Add Search Feature");
    // Mark one task as done
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "task",
            "update",
            "fix-login-bug",
            "--status",
            "done",
            "--project",
            "acme",
            "--no-edit",
        ])
        .assert()
        .success();
}

#[test]
fn search_basic_title_match() {
    let dir = TempDir::new().unwrap();
    setup_test_data(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["search", "Fix Login Bug"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Fix Login Bug"));
}

#[test]
fn search_fuzzy_match() {
    let dir = TempDir::new().unwrap();
    setup_test_data(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["search", "fx logn bg"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Fix Login Bug"));
}

#[test]
fn search_filter_by_type_task() {
    let dir = TempDir::new().unwrap();
    setup_test_data(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["search", "widget", "--type", "task"])
        .assert()
        .success()
        .stdout(predicate::str::contains("roadmap").not());
}

#[test]
fn search_filter_by_status() {
    let dir = TempDir::new().unwrap();
    setup_test_data(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["search", "bug", "--type", "task", "--status", "done"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Fix Login Bug"));
}

#[test]
fn search_filter_by_project() {
    let dir = TempDir::new().unwrap();
    setup_test_data(&dir);

    // Create a second project with a task
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "create", "other"])
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "task",
            "create",
            "other-task",
            "--title",
            "Other Task",
            "--project",
            "other",
            "--no-edit",
        ])
        .assert()
        .success();

    // Search only acme project
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["search", "task", "--project", "acme"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Other Task").not());
}

#[test]
fn search_limit() {
    let dir = TempDir::new().unwrap();
    setup_test_data(&dir);

    let output = rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["search", "widget", "--limit", "1"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(output).unwrap();
    // Count data rows (lines starting with "| " followed by a digit)
    let data_rows = stdout
        .lines()
        .filter(|l| l.starts_with("| ") && l.chars().nth(2).is_some_and(|c| c.is_ascii_digit()))
        .count();
    assert_eq!(data_rows, 1, "Expected exactly 1 result row, got: {stdout}");
}

#[test]
fn search_json_output() {
    let dir = TempDir::new().unwrap();
    setup_test_data(&dir);

    let output = rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["search", "Fix Login", "--format", "json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(output).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("should be valid JSON");
    let arr = parsed.as_array().expect("should be an array");
    assert!(!arr.is_empty(), "Expected results in JSON output");
    let first = &arr[0];
    assert!(first.get("kind").is_some());
    assert!(first.get("title").is_some());
    assert!(first.get("identifier").is_some());
    assert!(first.get("score").is_some());
    assert!(first.get("snippet").is_some());
}

#[test]
fn search_no_results() {
    let dir = TempDir::new().unwrap();
    setup_test_data(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["search", "xyzzy-nonexistent-qqq"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No results found"));
}

#[test]
fn search_help() {
    rdm()
        .args(["search", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Search across roadmaps"));
}

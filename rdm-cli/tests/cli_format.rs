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

fn setup_test_data(dir: &TempDir) {
    init_with_project(dir);
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "roadmap",
            "create",
            "alpha",
            "--title",
            "Alpha Roadmap",
            "--project",
            "acme",
            "--no-edit",
        ])
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "create",
            "setup",
            "--title",
            "Setup Phase",
            "--roadmap",
            "alpha",
            "--project",
            "acme",
            "--no-edit",
        ])
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "task",
            "create",
            "fix-bug",
            "--title",
            "Fix Bug",
            "--project",
            "acme",
            "--no-edit",
        ])
        .assert()
        .success();
}

#[test]
fn format_human_on_roadmap_list() {
    let dir = TempDir::new().unwrap();
    setup_test_data(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["roadmap", "list", "--project", "acme", "--format", "human"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Alpha Roadmap"));
}

#[test]
fn format_default_is_human() {
    let dir = TempDir::new().unwrap();
    setup_test_data(&dir);

    // Without --format, output should be the same as --format human
    let default_output = rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["roadmap", "list", "--project", "acme"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let human_output = rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["roadmap", "list", "--project", "acme", "--format", "human"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    assert_eq!(default_output, human_output);
}

#[test]
fn format_json_on_roadmap_list_returns_error() {
    let dir = TempDir::new().unwrap();
    setup_test_data(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["roadmap", "list", "--project", "acme", "--format", "json"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not yet supported"));
}

#[test]
fn format_json_on_search_still_works() {
    let dir = TempDir::new().unwrap();
    setup_test_data(&dir);

    let output = rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["search", "Alpha", "--format", "json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(output).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("should be valid JSON");
    assert!(parsed.is_array());
}

#[test]
fn format_text_alias_works_on_search() {
    let dir = TempDir::new().unwrap();
    setup_test_data(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["search", "Alpha", "--format", "text"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Alpha Roadmap"));
}

#[test]
fn format_invalid_value_is_rejected() {
    rdm()
        .args(["--format", "xml", "roadmap", "list"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid value 'xml'"));
}

#[test]
fn format_flag_works_after_subcommand_args() {
    let dir = TempDir::new().unwrap();
    setup_test_data(&dir);

    // --format placed after subcommand args should still work (global flag)
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["roadmap", "list", "--project", "acme", "--format", "human"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Alpha Roadmap"));
}

#[test]
fn format_json_on_task_list_returns_error() {
    let dir = TempDir::new().unwrap();
    setup_test_data(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["task", "list", "--project", "acme", "--format", "json"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not yet supported"));
}

#[test]
fn format_json_on_roadmap_show_returns_error() {
    let dir = TempDir::new().unwrap();
    setup_test_data(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "roadmap",
            "show",
            "alpha",
            "--project",
            "acme",
            "--format",
            "json",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not supported"));
}

#[test]
fn format_table_on_roadmap_list() {
    let dir = TempDir::new().unwrap();
    setup_test_data(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["roadmap", "list", "--project", "acme", "--format", "table"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Alpha Roadmap"));
}

#[test]
fn format_table_on_phase_list() {
    let dir = TempDir::new().unwrap();
    setup_test_data(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "list",
            "--roadmap",
            "alpha",
            "--project",
            "acme",
            "--format",
            "table",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Setup Phase"));
}

#[test]
fn format_table_on_task_list() {
    let dir = TempDir::new().unwrap();
    setup_test_data(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["task", "list", "--project", "acme", "--format", "table"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Fix Bug"));
}

#[test]
fn format_table_on_search() {
    let dir = TempDir::new().unwrap();
    setup_test_data(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["search", "Alpha", "--format", "table"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Alpha Roadmap"));
}

#[test]
fn format_table_on_roadmap_show_returns_error() {
    let dir = TempDir::new().unwrap();
    setup_test_data(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "roadmap",
            "show",
            "alpha",
            "--project",
            "acme",
            "--format",
            "table",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not supported"));
}

#[test]
fn format_table_on_phase_show_returns_error() {
    let dir = TempDir::new().unwrap();
    setup_test_data(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "show",
            "1",
            "--roadmap",
            "alpha",
            "--project",
            "acme",
            "--format",
            "table",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not supported"));
}

#[test]
fn format_table_on_task_show_returns_error() {
    let dir = TempDir::new().unwrap();
    setup_test_data(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "task",
            "show",
            "fix-bug",
            "--project",
            "acme",
            "--format",
            "table",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not supported"));
}

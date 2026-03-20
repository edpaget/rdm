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
fn format_json_on_roadmap_list() {
    let dir = TempDir::new().unwrap();
    setup_test_data(&dir);

    let output = rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["roadmap", "list", "--project", "acme", "--format", "json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(output).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("should be valid JSON");
    let arr = parsed.as_array().expect("should be an array");
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["slug"], "alpha");
    assert_eq!(arr[0]["title"], "Alpha Roadmap");
    assert!(arr[0]["total_phases"].is_number());
    assert!(arr[0]["progress"].is_string());
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
fn format_json_on_task_list() {
    let dir = TempDir::new().unwrap();
    setup_test_data(&dir);

    let output = rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["task", "list", "--project", "acme", "--format", "json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(output).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("should be valid JSON");
    let arr = parsed.as_array().expect("should be an array");
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["slug"], "fix-bug");
    assert_eq!(arr[0]["title"], "Fix Bug");
    assert_eq!(arr[0]["status"], "open");
    assert_eq!(arr[0]["priority"], "medium");
    assert!(arr[0]["created"].is_string());
}

#[test]
fn format_json_on_roadmap_show() {
    let dir = TempDir::new().unwrap();
    setup_test_data(&dir);

    let output = rdm()
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
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(output).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("should be valid JSON");
    assert_eq!(parsed["slug"], "alpha");
    assert_eq!(parsed["title"], "Alpha Roadmap");
    assert!(parsed["phases"].is_array());
    assert!(parsed["body"].is_string());
}

#[test]
fn format_json_on_phase_list() {
    let dir = TempDir::new().unwrap();
    setup_test_data(&dir);

    let output = rdm()
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
            "json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(output).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("should be valid JSON");
    let arr = parsed.as_array().expect("should be an array");
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["title"], "Setup Phase");
    assert_eq!(arr[0]["number"], 1);
    assert!(arr[0]["stem"].as_str().unwrap().contains("setup"));
    assert_eq!(arr[0]["status"], "not-started");
}

#[test]
fn format_json_on_phase_show() {
    let dir = TempDir::new().unwrap();
    setup_test_data(&dir);

    let output = rdm()
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
            "json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(output).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("should be valid JSON");
    assert_eq!(parsed["title"], "Setup Phase");
    assert_eq!(parsed["phase"], 1);
    assert_eq!(parsed["status"], "not-started");
    assert_eq!(parsed["roadmap"], "alpha");
    assert!(parsed["body"].is_string());
}

#[test]
fn format_json_on_task_show() {
    let dir = TempDir::new().unwrap();
    setup_test_data(&dir);

    let output = rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "task",
            "show",
            "fix-bug",
            "--project",
            "acme",
            "--format",
            "json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(output).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("should be valid JSON");
    assert_eq!(parsed["slug"], "fix-bug");
    assert_eq!(parsed["title"], "Fix Bug");
    assert_eq!(parsed["status"], "open");
    assert_eq!(parsed["project"], "acme");
    assert!(parsed["body"].is_string());
}

#[test]
fn format_json_on_project_list() {
    let dir = TempDir::new().unwrap();
    setup_test_data(&dir);

    let output = rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "list", "--format", "json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(output).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("should be valid JSON");
    let arr = parsed.as_array().expect("should be an array");
    assert!(arr.contains(&serde_json::Value::String("acme".to_string())));
}

#[test]
fn format_json_on_project_show() {
    let dir = TempDir::new().unwrap();
    setup_test_data(&dir);

    let output = rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "show", "acme", "--format", "json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(output).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("should be valid JSON");
    assert_eq!(parsed["name"], "acme");
    assert_eq!(parsed["title"], "acme");
    assert!(parsed["body"].is_string());
}

#[test]
fn format_human_on_project_show() {
    let dir = TempDir::new().unwrap();
    setup_test_data(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "show", "acme"])
        .assert()
        .success()
        .stdout(predicate::str::contains("acme"));
}

#[test]
fn format_table_on_project_show_returns_error() {
    let dir = TempDir::new().unwrap();
    setup_test_data(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "show", "acme", "--format", "table"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not supported"));
}

#[test]
fn format_json_on_roadmap_show_phases_are_summaries() {
    let dir = TempDir::new().unwrap();
    setup_test_data(&dir);

    let output = rdm()
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
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(output).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("should be valid JSON");
    let phases = parsed["phases"].as_array().expect("phases should be array");
    assert!(!phases.is_empty());
    // Phase summaries should have number, stem, title, status but NOT body
    let phase = &phases[0];
    assert!(phase["number"].is_number());
    assert!(phase["stem"].is_string());
    assert!(phase["title"].is_string());
    assert!(phase["status"].is_string());
    assert!(
        phase.get("body").is_none(),
        "phase summaries in roadmap show should not include body"
    );
}

#[test]
fn format_json_on_search_includes_score() {
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
    let arr = parsed.as_array().expect("should be an array");
    assert!(!arr.is_empty());
    // Search results should have kind, identifier, project, title, snippet, score
    let first = &arr[0];
    assert!(first["kind"].is_string());
    assert!(first["identifier"].is_string());
    assert!(first["title"].is_string());
    assert!(first["score"].is_number());
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

// -- Markdown format tests --

#[test]
fn format_markdown_on_roadmap_list() {
    let dir = TempDir::new().unwrap();
    setup_test_data(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "roadmap",
            "list",
            "--project",
            "acme",
            "--format",
            "markdown",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("| Slug |"))
        .stdout(predicate::str::contains("Alpha Roadmap"));
}

#[test]
fn format_markdown_on_phase_list() {
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
            "markdown",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Setup Phase"))
        .stdout(predicate::str::contains("|---:"));
}

#[test]
fn format_markdown_on_task_list() {
    let dir = TempDir::new().unwrap();
    setup_test_data(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["task", "list", "--project", "acme", "--format", "markdown"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Fix Bug"))
        .stdout(predicate::str::contains("## Tasks"));
}

#[test]
fn format_markdown_on_search() {
    let dir = TempDir::new().unwrap();
    setup_test_data(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["search", "Alpha", "--format", "markdown"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Alpha Roadmap"));
}

#[test]
fn format_markdown_on_roadmap_show() {
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
            "markdown",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("# Alpha Roadmap"))
        .stdout(predicate::str::contains("- **Slug:**"));
}

#[test]
fn format_markdown_on_phase_show() {
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
            "markdown",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("# Phase"))
        .stdout(predicate::str::contains("- **Status:**"));
}

#[test]
fn format_markdown_on_task_show() {
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
            "markdown",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("# Fix Bug"))
        .stdout(predicate::str::contains("- **Status:**"));
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

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;

fn rdm() -> Command {
    let mut cmd = Command::cargo_bin("rdm").unwrap();
    // Isolate from host global config (e.g. default_format = "json").
    cmd.env("XDG_CONFIG_HOME", "/dev/null/nonexistent");
    cmd
}

#[test]
fn describe_lists_all_entities() {
    rdm()
        .arg("describe")
        .assert()
        .success()
        .stdout(predicate::str::contains("project"))
        .stdout(predicate::str::contains("roadmap"))
        .stdout(predicate::str::contains("phase"))
        .stdout(predicate::str::contains("task"));
}

#[test]
fn describe_entity_shows_fields() {
    rdm()
        .arg("describe")
        .arg("task")
        .assert()
        .success()
        .stdout(predicate::str::contains("status"))
        .stdout(predicate::str::contains("priority"))
        .stdout(predicate::str::contains("created"))
        .stdout(predicate::str::contains("tags"));
}

#[test]
fn describe_unknown_entity_errors() {
    rdm()
        .arg("describe")
        .arg("foo")
        .assert()
        .failure()
        .stderr(predicate::str::contains("unknown entity 'foo'"))
        .stderr(predicate::str::contains("project, roadmap, phase, task"));
}

#[test]
fn describe_json_format() {
    let output = rdm()
        .args(["describe", "--format", "json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    let arr = json.as_array().unwrap();
    assert_eq!(arr.len(), 4);
    let names: Vec<&str> = arr.iter().map(|e| e["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["project", "roadmap", "phase", "task"]);
}

#[test]
fn describe_entity_json_format() {
    let output = rdm()
        .args(["describe", "phase", "--format", "json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["name"].as_str().unwrap(), "phase");
    let fields = json["fields"].as_array().unwrap();
    let field_names: Vec<&str> = fields.iter().map(|f| f["name"].as_str().unwrap()).collect();
    assert!(field_names.contains(&"phase"));
    assert!(field_names.contains(&"status"));
    assert!(field_names.contains(&"completed"));
}

#[test]
fn describe_entity_enum_values() {
    let output = rdm().args(["describe", "phase"]).output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("not-started"));
    assert!(stdout.contains("in-progress"));
    assert!(stdout.contains("done"));
    assert!(stdout.contains("blocked"));
}

#[test]
fn describe_markdown_format() {
    rdm()
        .args(["describe", "--format", "markdown"])
        .assert()
        .success()
        .stdout(predicate::str::contains("## Entity Types"))
        .stdout(predicate::str::contains("**project**"));
}

#[test]
fn describe_entity_markdown_format() {
    rdm()
        .args(["describe", "task", "--format", "markdown"])
        .assert()
        .success()
        .stdout(predicate::str::contains("## task"))
        .stdout(predicate::str::contains("| Field |"));
}

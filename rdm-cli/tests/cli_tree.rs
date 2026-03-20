use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn rdm() -> Command {
    let mut cmd = Command::cargo_bin("rdm").unwrap();
    // Isolate from host global config (e.g. default_format = "json").
    cmd.env("XDG_CONFIG_HOME", "/dev/null/nonexistent");
    cmd
}

fn init_with_project_and_roadmap(dir: &TempDir) {
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("init")
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "create", "fbm", "--title", "FBM"])
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "roadmap",
            "create",
            "two-way",
            "--title",
            "Two-Way Players",
            "--project",
            "fbm",
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
            "core",
            "--title",
            "Core Valuation",
            "--roadmap",
            "two-way",
            "--project",
            "fbm",
        ])
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "phase",
            "create",
            "service",
            "--title",
            "Keeper Service",
            "--roadmap",
            "two-way",
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
            "create",
            "fix-bug",
            "--title",
            "Fix Bug",
            "--project",
            "fbm",
            "--no-edit",
        ])
        .assert()
        .success();
}

#[test]
fn tree_shows_project_and_roadmaps() {
    let dir = TempDir::new().unwrap();
    init_with_project_and_roadmap(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["tree", "--project", "fbm"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("fbm")
                .and(predicate::str::contains("two-way"))
                .and(predicate::str::contains("phase-1-core"))
                .and(predicate::str::contains("phase-2-service")),
        );
}

#[test]
fn tree_shows_phases_with_status() {
    let dir = TempDir::new().unwrap();
    init_with_project_and_roadmap(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["tree", "--project", "fbm"])
        .assert()
        .success()
        .stdout(predicate::str::contains("not-started").and(predicate::str::contains("fix-bug")));
}

#[test]
fn tree_json_format() {
    let dir = TempDir::new().unwrap();
    init_with_project_and_roadmap(&dir);

    let assert = rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["--format", "json", "tree", "--project", "fbm"])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let value: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(value["name"], "fbm");
    assert_eq!(value["kind"], "project");
    assert!(value["children"].as_array().unwrap().len() >= 2);

    // Find the roadmap child
    let roadmap = value["children"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["kind"] == "roadmap")
        .expect("should have a roadmap child");
    assert_eq!(roadmap["name"], "two-way");
    assert!(roadmap["children"].as_array().unwrap().len() >= 2);
}

#[test]
fn tree_markdown_format() {
    let dir = TempDir::new().unwrap();
    init_with_project_and_roadmap(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["--format", "markdown", "tree", "--project", "fbm"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("# fbm")
                .and(predicate::str::contains("- **two-way**"))
                .and(predicate::str::contains("- **phase-1-core**")),
        );
}

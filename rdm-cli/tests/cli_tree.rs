use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

#[path = "support/every_status.rs"]
mod every_status;

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

#[test]
fn tree_active_keeps_only_active_work_and_carries_refs() {
    let dir = every_status::seed();
    let tree = every_status::json(
        dir.path(),
        &["tree", "--active", "--project", "acme", "--format", "json"],
    );
    let children = tree["children"].as_array().unwrap();
    let roadmaps: Vec<&serde_json::Value> =
        children.iter().filter(|n| n["kind"] == "roadmap").collect();
    assert_eq!(roadmaps.len(), 1, "idle roadmap is dropped: {tree:#}");
    assert_eq!(roadmaps[0]["name"], "busy");
    assert_eq!(roadmaps[0]["ref"], "busy");
    assert_eq!(roadmaps[0]["review_ref"], "roadmap/busy");
    let stems: Vec<String> = roadmaps[0]["children"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(stems, every_status::active_phase_stems());
    for p in roadmaps[0]["children"].as_array().unwrap() {
        let stem = p["name"].as_str().unwrap();
        assert_eq!(p["ref"], format!("busy/{stem}"));
        assert_eq!(p["review_ref"], format!("phase/busy/{stem}"));
    }
    let mut tasks: Vec<String> = children
        .iter()
        .filter(|n| n["kind"] == "task")
        .map(|t| {
            let slug = t["name"].as_str().unwrap();
            assert_eq!(t["ref"], format!("task/{slug}"));
            slug.to_string()
        })
        .collect();
    tasks.sort();
    let mut expected = every_status::active_task_slugs();
    expected.sort();
    assert_eq!(tasks, expected);
    assert!(tree.get("ref").is_none(), "the project node has no ref");
}

#[test]
fn tree_without_active_keeps_every_item() {
    let dir = every_status::seed();
    let tree = every_status::json(
        dir.path(),
        &["tree", "--project", "acme", "--format", "json"],
    );
    let children = tree["children"].as_array().unwrap();
    assert_eq!(
        children.iter().filter(|n| n["kind"] == "roadmap").count(),
        2
    );
    assert_eq!(
        children.iter().filter(|n| n["kind"] == "task").count(),
        every_status::TASK_STATUSES.len()
    );
}

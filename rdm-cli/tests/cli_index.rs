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
        .arg("--no-index")
        .arg("init")
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("--no-index")
        .args(["project", "create", "fbm"])
        .assert()
        .success();
}

#[test]
fn index_generates_file() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("--no-index")
        .args(["roadmap", "create", "alpha", "--project", "fbm"])
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("--no-index")
        .args([
            "phase",
            "create",
            "core",
            "--roadmap",
            "alpha",
            "--project",
            "fbm",
        ])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("index")
        .assert()
        .success()
        .stdout(predicate::str::contains("Generated INDEX.md"));

    let content = std::fs::read_to_string(dir.path().join("INDEX.md")).unwrap();
    assert!(content.contains("# Plan Index"));
    assert!(content.contains("## Project: fbm"));
    assert!(content.contains("alpha"));
    assert!(content.contains("not started"));
}

#[test]
fn index_idempotent() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("--no-index")
        .args(["roadmap", "create", "alpha", "--project", "fbm"])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("index")
        .assert()
        .success();
    let first = std::fs::read_to_string(dir.path().join("INDEX.md")).unwrap();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("index")
        .assert()
        .success();
    let second = std::fs::read_to_string(dir.path().join("INDEX.md")).unwrap();

    assert_eq!(first, second);
}

#[test]
fn index_deterministic_sorting() {
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("--no-index")
        .arg("init")
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("--no-index")
        .args(["project", "create", "zzz"])
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("--no-index")
        .args(["project", "create", "aaa"])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("index")
        .assert()
        .success();

    let content = std::fs::read_to_string(dir.path().join("INDEX.md")).unwrap();
    let aaa_pos = content.find("## Project: aaa").unwrap();
    let zzz_pos = content.find("## Project: zzz").unwrap();
    assert!(aaa_pos < zzz_pos);
}

#[test]
fn index_task_priority_sorting() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("--no-index")
        .args([
            "task",
            "create",
            "low-task",
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
        .arg("--no-index")
        .args([
            "task",
            "create",
            "crit-task",
            "--project",
            "fbm",
            "--priority",
            "critical",
        ])
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("--no-index")
        .args([
            "task",
            "create",
            "high-task",
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
        .arg("index")
        .assert()
        .success();

    let content = std::fs::read_to_string(dir.path().join("INDEX.md")).unwrap();
    let crit_pos = content.find("crit-task").unwrap();
    let high_pos = content.find("high-task").unwrap();
    let low_pos = content.find("low-task").unwrap();
    assert!(crit_pos < high_pos, "critical should come before high");
    assert!(high_pos < low_pos, "high should come before low");
}

#[test]
fn index_dependency_graph() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);

    // Create roadmap with dependencies by writing the file directly
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("--no-index")
        .args(["roadmap", "create", "alpha", "--project", "fbm"])
        .assert()
        .success();

    // Write a roadmap with dependencies manually
    let roadmap_path = dir.path().join("projects/fbm/roadmaps/beta");
    std::fs::create_dir_all(&roadmap_path).unwrap();
    std::fs::write(
        roadmap_path.join("roadmap.md"),
        "---\nproject: fbm\nroadmap: beta\ntitle: Beta\nphases: []\ndependencies:\n  - alpha\n---\n",
    )
    .unwrap();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("index")
        .assert()
        .success();

    let content = std::fs::read_to_string(dir.path().join("INDEX.md")).unwrap();
    assert!(content.contains("Dependency Graph"));
    assert!(content.contains("**beta** → alpha"));
}

#[test]
fn mutation_auto_generates_index() {
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("--no-index")
        .arg("init")
        .assert()
        .success();

    // project create should auto-generate index
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "create", "fbm"])
        .assert()
        .success();

    let content = std::fs::read_to_string(dir.path().join("INDEX.md")).unwrap();
    assert!(content.contains("# Plan Index"));
    assert!(content.contains("## Project: fbm"));
}

#[test]
fn no_index_flag_suppresses() {
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("--no-index")
        .arg("init")
        .assert()
        .success();

    // Read the init-generated INDEX.md
    let before = std::fs::read_to_string(dir.path().join("INDEX.md")).unwrap();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("--no-index")
        .args(["project", "create", "fbm"])
        .assert()
        .success();

    let after = std::fs::read_to_string(dir.path().join("INDEX.md")).unwrap();
    assert_eq!(
        before, after,
        "--no-index should prevent INDEX.md regeneration"
    );
}

#[test]
fn index_after_phase_update() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("--no-index")
        .args(["roadmap", "create", "alpha", "--project", "fbm"])
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("--no-index")
        .args([
            "phase",
            "create",
            "core",
            "--roadmap",
            "alpha",
            "--project",
            "fbm",
        ])
        .assert()
        .success();

    // Generate initial index
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("index")
        .assert()
        .success();
    let before = std::fs::read_to_string(dir.path().join("INDEX.md")).unwrap();
    assert!(before.contains("not started"));

    // Update phase to done (auto-generates index)
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
            "alpha",
            "--project",
            "fbm",
        ])
        .assert()
        .success();

    let after = std::fs::read_to_string(dir.path().join("INDEX.md")).unwrap();
    assert!(
        after.contains("complete"),
        "index should reflect phase status change"
    );
}

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

#[test]
fn list_empty_project() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["list", "--project", "fbm"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No roadmaps found."));
}

#[test]
fn list_with_progress() {
    let dir = TempDir::new().unwrap();
    init_with_project(&dir);

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
            "Core",
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
            "update",
            "phase-1-core",
            "--status",
            "done",
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
        .args(["list", "--project", "fbm"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "two-way — Two-Way Players (1/1 done)",
        ));
}

#[test]
fn list_all_projects() {
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("init")
        .assert()
        .success();

    // Create two projects with roadmaps
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "create", "alpha", "--title", "Alpha Project"])
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "create", "beta", "--title", "Beta Project"])
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "roadmap",
            "create",
            "r1",
            "--title",
            "Road One",
            "--project",
            "alpha",
        ])
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "roadmap",
            "create",
            "r2",
            "--title",
            "Road Two",
            "--project",
            "beta",
        ])
        .assert()
        .success();

    let assert = rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["list", "--all"])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("Project: alpha"));
    assert!(stdout.contains("Project: beta"));
    assert!(stdout.contains("r1 — Road One"));
    assert!(stdout.contains("r2 — Road Two"));
}

#[test]
fn list_no_project_fails() {
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
        .args(["list"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no project specified"));
}

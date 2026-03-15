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
        .args(["project", "create", "fbm", "--title", "FBM"])
        .assert()
        .success();
}

#[test]
fn roadmap_create_and_show() {
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
        .success()
        .stdout(predicate::str::contains("Created roadmap 'two-way'"));

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["roadmap", "show", "two-way", "--project", "fbm"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Two-Way Players")
                .and(predicate::str::contains("No phases yet.")),
        );
}

#[test]
fn roadmap_show_with_phases() {
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
        .args(["roadmap", "show", "two-way", "--project", "fbm"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("0/1 phases done")
                .and(predicate::str::contains("Core Valuation")),
        );
}

#[test]
fn roadmap_create_missing_project() {
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
        .args(["roadmap", "create", "slug", "--project", "nope"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("project not found"));
}

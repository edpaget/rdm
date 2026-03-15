use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn rdm() -> Command {
    Command::cargo_bin("rdm").unwrap()
}

fn init_repo(dir: &TempDir) {
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("init")
        .assert()
        .success();
}

#[test]
fn project_create_success() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args([
            "project",
            "create",
            "fbm",
            "--title",
            "Fantasy Baseball Manager",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Created project 'fbm'"));

    assert!(dir.path().join("projects/fbm").is_dir());
    assert!(dir.path().join("projects/fbm/roadmaps").is_dir());
    assert!(dir.path().join("projects/fbm/tasks").is_dir());
}

#[test]
fn project_create_duplicate_fails() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "create", "fbm"])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "create", "fbm"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("already exists"));
}

#[test]
fn project_list_empty() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No projects yet."));
}

#[test]
fn project_list_shows_projects() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "create", "aaa"])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "create", "zzz"])
        .assert()
        .success();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("aaa").and(predicate::str::contains("zzz")));
}

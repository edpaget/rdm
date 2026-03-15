use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
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
fn flag_overrides_config_default() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    // Set default_project in config
    fs::write(
        dir.path().join("rdm.toml"),
        "default_project = \"default-proj\"\n",
    )
    .unwrap();

    // Create both projects
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "create", "default-proj"])
        .assert()
        .success();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "create", "other-proj"])
        .assert()
        .success();

    // --project flag should override config
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["list", "--project", "other-proj"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No roadmaps found."));
}

#[test]
fn config_fallback() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    // Set default_project in config
    fs::write(dir.path().join("rdm.toml"), "default_project = \"fbm\"\n").unwrap();

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["project", "create", "fbm"])
        .assert()
        .success();

    // Should use default_project from config
    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No roadmaps found."));
}

#[test]
fn no_project_error() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    rdm()
        .arg("--root")
        .arg(dir.path())
        .args(["list"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no project specified"));
}

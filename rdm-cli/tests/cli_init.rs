use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn rdm() -> Command {
    Command::cargo_bin("rdm").unwrap()
}

#[test]
fn init_creates_plan_repo() {
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("init")
        .assert()
        .success()
        .stdout(predicate::str::contains("Initialized plan repo"));

    assert!(dir.path().join("rdm.toml").exists());
    assert!(dir.path().join("projects").is_dir());
    assert!(dir.path().join("INDEX.md").exists());
}

#[test]
fn init_twice_fails() {
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
        .arg("init")
        .assert()
        .failure()
        .stderr(predicate::str::contains("already initialized"));
}

#[test]
fn no_subcommand_shows_usage() {
    rdm()
        .assert()
        .failure()
        .stderr(predicate::str::contains("Usage"));
}

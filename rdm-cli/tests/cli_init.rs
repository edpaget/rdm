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
fn init_via_rdm_root_env_var() {
    let dir = TempDir::new().unwrap();
    rdm()
        .env("RDM_ROOT", dir.path())
        .arg("init")
        .assert()
        .success()
        .stdout(predicate::str::contains("Initialized plan repo"));

    assert!(dir.path().join("rdm.toml").exists());
}

#[test]
fn root_flag_overrides_rdm_root_env() {
    let env_dir = TempDir::new().unwrap();
    let flag_dir = TempDir::new().unwrap();

    rdm()
        .env("RDM_ROOT", env_dir.path())
        .arg("--root")
        .arg(flag_dir.path())
        .arg("init")
        .assert()
        .success();

    // Flag dir should have the repo, env dir should not
    assert!(flag_dir.path().join("rdm.toml").exists());
    assert!(!env_dir.path().join("rdm.toml").exists());
}

#[test]
fn no_subcommand_shows_usage() {
    rdm()
        .assert()
        .failure()
        .stderr(predicate::str::contains("Usage"));
}

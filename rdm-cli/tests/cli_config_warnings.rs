use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn rdm() -> Command {
    let mut cmd = Command::cargo_bin("rdm").unwrap();
    // Isolate from host environment
    cmd.env_remove("RDM_ROOT");
    cmd.env_remove("RDM_PROJECT");
    cmd.env_remove("RDM_FORMAT");
    cmd
}

#[test]
fn warn_on_global_config_parse_error() {
    let xdg_dir = TempDir::new().unwrap();
    let config_dir = xdg_dir.path().join("rdm");
    std::fs::create_dir_all(&config_dir).unwrap();
    let root_dir = TempDir::new().unwrap();

    // Initialize repo first
    rdm()
        .env("XDG_CONFIG_HOME", xdg_dir.path())
        .arg("--root")
        .arg(root_dir.path())
        .arg("init")
        .assert()
        .success();

    // Create a project so config list succeeds
    rdm()
        .env("XDG_CONFIG_HOME", xdg_dir.path())
        .arg("--root")
        .arg(root_dir.path())
        .args(["project", "create", "test"])
        .assert()
        .success();

    // Now write malformed global config
    std::fs::write(config_dir.join("config.toml"), "invalid = [").unwrap();

    rdm()
        .env("XDG_CONFIG_HOME", xdg_dir.path())
        .arg("--root")
        .arg(root_dir.path())
        .args(["config", "list"])
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "warning: ignoring malformed config at",
        ));
}

#[test]
fn silent_on_missing_global_config() {
    let xdg_dir = TempDir::new().unwrap();
    let root_dir = TempDir::new().unwrap();

    // Initialize repo and create project
    rdm()
        .env("XDG_CONFIG_HOME", xdg_dir.path())
        .arg("--root")
        .arg(root_dir.path())
        .arg("init")
        .assert()
        .success();

    rdm()
        .env("XDG_CONFIG_HOME", xdg_dir.path())
        .arg("--root")
        .arg(root_dir.path())
        .args(["project", "create", "test"])
        .assert()
        .success();

    // Don't create config directory
    rdm()
        .env("XDG_CONFIG_HOME", xdg_dir.path())
        .arg("--root")
        .arg(root_dir.path())
        .args(["config", "list"])
        .assert()
        .success()
        .stderr(predicate::str::contains("warning").not());
}

#[test]
fn warn_on_repo_config_parse_error() {
    let xdg_dir = TempDir::new().unwrap();
    let root_dir = TempDir::new().unwrap();

    // Initialize the repo first
    rdm()
        .env("XDG_CONFIG_HOME", xdg_dir.path())
        .arg("--root")
        .arg(root_dir.path())
        .arg("init")
        .assert()
        .success();

    // Create a project
    rdm()
        .env("XDG_CONFIG_HOME", xdg_dir.path())
        .arg("--root")
        .arg(root_dir.path())
        .args(["project", "create", "test"])
        .assert()
        .success();

    // Now corrupt the rdm.toml
    std::fs::write(root_dir.path().join("rdm.toml"), "invalid = [").unwrap();

    rdm()
        .env("XDG_CONFIG_HOME", xdg_dir.path())
        .arg("--root")
        .arg(root_dir.path())
        .args(["config", "list"])
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "warning: ignoring malformed config at",
        ));
}

#[test]
fn silent_on_missing_repo_config() {
    let xdg_dir = TempDir::new().unwrap();
    let root_dir = TempDir::new().unwrap();

    // Create a minimal plan repo structure without rdm.toml
    std::fs::write(root_dir.path().join("INDEX.md"), "").unwrap();

    // Create a project in a separate location to avoid errors
    let project_root = TempDir::new().unwrap();
    rdm()
        .env("XDG_CONFIG_HOME", xdg_dir.path())
        .arg("--root")
        .arg(project_root.path())
        .arg("init")
        .assert()
        .success();

    rdm()
        .env("XDG_CONFIG_HOME", xdg_dir.path())
        .arg("--root")
        .arg(project_root.path())
        .args(["project", "create", "test"])
        .assert()
        .success();

    // Test loading the first repo (without rdm.toml)
    rdm()
        .env("XDG_CONFIG_HOME", xdg_dir.path())
        .arg("--root")
        .arg(root_dir.path())
        .args(["config", "list"])
        .assert()
        .success()
        .stderr(predicate::str::contains("warning").not());
}

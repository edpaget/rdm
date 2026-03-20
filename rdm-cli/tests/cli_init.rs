use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

/// Creates an rdm command isolated from the host environment.
///
/// `xdg_dir` must be a writable temp directory since `rdm init` always
/// creates the global config file.
fn rdm(xdg_dir: &TempDir) -> Command {
    let mut cmd = Command::cargo_bin("rdm").unwrap();
    cmd.env("XDG_CONFIG_HOME", xdg_dir.path());
    cmd
}

#[test]
fn init_creates_plan_repo() {
    let dir = TempDir::new().unwrap();
    let xdg = TempDir::new().unwrap();
    rdm(&xdg)
        .arg("--root")
        .arg(dir.path())
        .arg("init")
        .assert()
        .success()
        .stdout(predicate::str::contains("Initialized plan repo"));

    assert!(dir.path().join("rdm.toml").exists());
    assert!(dir.path().join("INDEX.md").exists());
}

#[test]
fn init_twice_fails() {
    let dir = TempDir::new().unwrap();
    let xdg = TempDir::new().unwrap();
    rdm(&xdg)
        .arg("--root")
        .arg(dir.path())
        .arg("init")
        .assert()
        .success();

    rdm(&xdg)
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
    let xdg = TempDir::new().unwrap();
    rdm(&xdg)
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
    let xdg = TempDir::new().unwrap();

    rdm(&xdg)
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
fn init_with_default_project() {
    let dir = TempDir::new().unwrap();
    let xdg = TempDir::new().unwrap();
    rdm(&xdg)
        .arg("--root")
        .arg(dir.path())
        .arg("init")
        .arg("--default-project")
        .arg("myproj")
        .assert()
        .success()
        .stdout(predicate::str::contains("default project: myproj"));

    // rdm.toml should contain default_project
    let toml_str = std::fs::read_to_string(dir.path().join("rdm.toml")).unwrap();
    assert!(toml_str.contains("default_project"));
    assert!(toml_str.contains("myproj"));

    // Project directory should exist
    assert!(dir.path().join("projects/myproj").exists());
}

#[test]
fn init_with_default_format() {
    let dir = TempDir::new().unwrap();
    let xdg = TempDir::new().unwrap();
    rdm(&xdg)
        .arg("--root")
        .arg(dir.path())
        .arg("init")
        .arg("--default-format")
        .arg("json")
        .assert()
        .success()
        .stdout(predicate::str::contains("default format: json"));

    // Global config should contain default_format
    let global_path = xdg.path().join("rdm/config.toml");
    let global_str = std::fs::read_to_string(global_path).unwrap();
    assert!(global_str.contains("default_format"));
    assert!(global_str.contains("json"));
}

#[test]
fn init_with_stage() {
    let dir = TempDir::new().unwrap();
    let xdg = TempDir::new().unwrap();
    rdm(&xdg)
        .arg("--root")
        .arg(dir.path())
        .arg("--stage")
        .arg("init")
        .assert()
        .success()
        .stdout(predicate::str::contains("staging mode: enabled"));

    // rdm.toml should contain stage = true
    let toml_str = std::fs::read_to_string(dir.path().join("rdm.toml")).unwrap();
    assert!(toml_str.contains("stage = true"));
}

#[test]
fn init_with_invalid_format_fails() {
    let dir = TempDir::new().unwrap();
    let xdg = TempDir::new().unwrap();
    rdm(&xdg)
        .arg("--root")
        .arg(dir.path())
        .arg("init")
        .arg("--default-format")
        .arg("xml")
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid default_format"));

    // No files should be created
    assert!(!dir.path().join("rdm.toml").exists());
}

#[test]
fn init_creates_parent_dirs() {
    let dir = TempDir::new().unwrap();
    let xdg = TempDir::new().unwrap();
    let nested = dir.path().join("a/b/c");
    rdm(&xdg)
        .arg("--root")
        .arg(&nested)
        .arg("init")
        .assert()
        .success();

    assert!(nested.join("rdm.toml").exists());
    assert!(nested.join("INDEX.md").exists());
}

#[test]
fn init_creates_global_config() {
    let dir = TempDir::new().unwrap();
    let xdg = TempDir::new().unwrap();
    rdm(&xdg)
        .arg("--root")
        .arg(dir.path())
        .arg("init")
        .assert()
        .success();

    // Global config should exist even without --default-format
    let global_path = xdg.path().join("rdm/config.toml");
    assert!(global_path.exists());
}

#[test]
fn init_prints_summary() {
    let dir = TempDir::new().unwrap();
    let xdg = TempDir::new().unwrap();
    rdm(&xdg)
        .arg("--root")
        .arg(dir.path())
        .arg("init")
        .assert()
        .success()
        .stdout(predicate::str::contains("repo config:"))
        .stdout(predicate::str::contains("global config:"))
        .stdout(predicate::str::contains("Next steps:"))
        .stdout(predicate::str::contains("rdm project create"));
}

#[test]
fn init_with_all_flags() {
    let dir = TempDir::new().unwrap();
    let xdg = TempDir::new().unwrap();
    rdm(&xdg)
        .arg("--root")
        .arg(dir.path())
        .arg("--stage")
        .arg("init")
        .arg("--default-project")
        .arg("myproj")
        .arg("--default-format")
        .arg("table")
        .assert()
        .success()
        .stdout(predicate::str::contains("default project: myproj"))
        .stdout(predicate::str::contains("default format: table"))
        .stdout(predicate::str::contains("staging mode: enabled"));

    // Verify repo config
    let toml_str = std::fs::read_to_string(dir.path().join("rdm.toml")).unwrap();
    assert!(toml_str.contains("myproj"));
    assert!(toml_str.contains("stage = true"));

    // Verify global config
    let global_path = xdg.path().join("rdm/config.toml");
    let global_str = std::fs::read_to_string(global_path).unwrap();
    assert!(global_str.contains("table"));

    // Verify project directory exists
    assert!(dir.path().join("projects/myproj").exists());
}

#[test]
fn no_subcommand_shows_usage() {
    let xdg = TempDir::new().unwrap();
    rdm(&xdg)
        .assert()
        .failure()
        .stderr(predicate::str::contains("Usage"));
}

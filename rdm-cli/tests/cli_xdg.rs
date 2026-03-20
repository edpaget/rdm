use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn rdm() -> Command {
    Command::cargo_bin("rdm").unwrap()
}

#[test]
fn init_at_xdg_data_dir() {
    let data_dir = TempDir::new().unwrap();
    // With XDG_DATA_HOME set and no RDM_ROOT, rdm should use $XDG_DATA_HOME/rdm
    rdm()
        .env("XDG_DATA_HOME", data_dir.path())
        .env_remove("RDM_ROOT")
        .arg("init")
        .assert()
        .success()
        .stdout(predicate::str::contains("Initialized plan repo"));

    assert!(data_dir.path().join("rdm").join("rdm.toml").exists());
}

#[test]
fn commands_work_with_xdg_defaults() {
    let data_dir = TempDir::new().unwrap();
    // Init at XDG data dir
    rdm()
        .env("XDG_DATA_HOME", data_dir.path())
        .env_remove("RDM_ROOT")
        .arg("init")
        .assert()
        .success();

    // Create a project so we can list roadmaps
    rdm()
        .env("XDG_DATA_HOME", data_dir.path())
        .env_remove("RDM_ROOT")
        .env("RDM_PROJECT", "test")
        .args(["project", "create", "test"])
        .assert()
        .success();

    // Then listing roadmaps should work without --root
    rdm()
        .env("XDG_DATA_HOME", data_dir.path())
        .env_remove("RDM_ROOT")
        .env("RDM_PROJECT", "test")
        .args(["roadmap", "list"])
        .assert()
        .success();
}

#[test]
fn global_config_root_overrides_xdg() {
    let config_dir = TempDir::new().unwrap();
    let custom_root = TempDir::new().unwrap();
    let data_dir = TempDir::new().unwrap();

    // Write global config pointing to custom_root
    let config_path = config_dir.path().join("rdm");
    std::fs::create_dir_all(&config_path).unwrap();
    std::fs::write(
        config_path.join("config.toml"),
        format!("root = \"{}\"", custom_root.path().display()),
    )
    .unwrap();

    // Init using global config root (not XDG data dir)
    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env("XDG_DATA_HOME", data_dir.path())
        .env_remove("RDM_ROOT")
        .arg("init")
        .assert()
        .success();

    // The repo should be at custom_root, not data_dir/rdm
    assert!(custom_root.path().join("rdm.toml").exists());
    assert!(!data_dir.path().join("rdm").join("rdm.toml").exists());
}

#[test]
fn flag_overrides_global_config() {
    let config_dir = TempDir::new().unwrap();
    let config_root = TempDir::new().unwrap();
    let flag_root = TempDir::new().unwrap();

    // Write global config pointing to config_root
    let config_path = config_dir.path().join("rdm");
    std::fs::create_dir_all(&config_path).unwrap();
    std::fs::write(
        config_path.join("config.toml"),
        format!("root = \"{}\"", config_root.path().display()),
    )
    .unwrap();

    // Use --root to override global config
    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .arg("--root")
        .arg(flag_root.path())
        .arg("init")
        .assert()
        .success();

    // flag_root should have the repo, config_root should not
    assert!(flag_root.path().join("rdm.toml").exists());
    assert!(!config_root.path().join("rdm.toml").exists());
}

#[test]
fn helpful_error_when_no_repo() {
    let data_dir = TempDir::new().unwrap();
    // No repo initialized, trying to list should give helpful error
    rdm()
        .env("XDG_DATA_HOME", data_dir.path())
        .env_remove("RDM_ROOT")
        .env("RDM_PROJECT", "test")
        .args(["roadmap", "list"])
        .assert()
        .failure();
}

#[test]
fn global_config_default_project_fallback() {
    let config_dir = TempDir::new().unwrap();
    let root_dir = TempDir::new().unwrap();

    // Write global config with default_project
    let config_path = config_dir.path().join("rdm");
    std::fs::create_dir_all(&config_path).unwrap();
    std::fs::write(
        config_path.join("config.toml"),
        format!(
            "root = \"{}\"\ndefault_project = \"global-proj\"",
            root_dir.path().display()
        ),
    )
    .unwrap();

    // Init the repo
    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .arg("init")
        .assert()
        .success();

    // Create the project
    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .args(["project", "create", "global-proj"])
        .assert()
        .success();

    // Listing roadmaps should work without --project (uses global config default)
    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .args(["roadmap", "list"])
        .assert()
        .success();
}

#[test]
fn repo_config_overrides_global_for_default_project() {
    let config_dir = TempDir::new().unwrap();
    let root_dir = TempDir::new().unwrap();

    // Write global config with default_project = "global-proj"
    let config_path = config_dir.path().join("rdm");
    std::fs::create_dir_all(&config_path).unwrap();
    std::fs::write(
        config_path.join("config.toml"),
        format!(
            "root = \"{}\"\ndefault_project = \"global-proj\"",
            root_dir.path().display()
        ),
    )
    .unwrap();

    // Init the repo
    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .arg("init")
        .assert()
        .success();

    // Write repo config with default_project = "repo-proj"
    std::fs::write(
        root_dir.path().join("rdm.toml"),
        "default_project = \"repo-proj\"",
    )
    .unwrap();

    // Create the project matching repo config
    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .args(["project", "create", "repo-proj"])
        .assert()
        .success();

    // Listing roadmaps should use repo-proj (not global-proj)
    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .args(["roadmap", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No roadmaps"));
}

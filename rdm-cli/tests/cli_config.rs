use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn rdm() -> Command {
    Command::cargo_bin("rdm").unwrap()
}

/// Helper: create a temp dir with an initialized repo and a project.
fn setup_repo() -> (TempDir, TempDir) {
    let config_dir = TempDir::new().unwrap();
    let root_dir = TempDir::new().unwrap();

    // Write global config pointing to root
    let rdm_config = config_dir.path().join("rdm");
    std::fs::create_dir_all(&rdm_config).unwrap();
    std::fs::write(
        rdm_config.join("config.toml"),
        format!("root = \"{}\"", root_dir.path().display()),
    )
    .unwrap();

    // Init repo
    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .arg("init")
        .assert()
        .success();

    // Create a project
    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args(["project", "create", "test"])
        .assert()
        .success();

    (config_dir, root_dir)
}

#[test]
fn config_list_shows_defaults() {
    let (config_dir, _root_dir) = setup_repo();

    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args(["config", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("default_project"))
        .stdout(predicate::str::contains("default_format"))
        .stdout(predicate::str::contains("stage"))
        .stdout(predicate::str::contains("remote.default"))
        .stdout(predicate::str::contains("root"));
}

#[test]
fn config_set_repo_and_get() {
    let (config_dir, _root_dir) = setup_repo();

    // Set default_project in repo config
    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args(["config", "set", "default_project", "my-proj"])
        .assert()
        .success()
        .stdout(predicate::str::contains("repo config"));

    // Get should show repo config source
    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args(["config", "get", "default_project"])
        .assert()
        .success()
        .stdout(predicate::str::contains("my-proj"))
        .stdout(predicate::str::contains("repo config"));
}

#[test]
fn config_set_global_and_get() {
    let (config_dir, _root_dir) = setup_repo();

    // Set default_project in global config
    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args([
            "config",
            "set",
            "default_project",
            "global-proj",
            "--global",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("global config"));

    // Get should show global config source
    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args(["config", "get", "default_project"])
        .assert()
        .success()
        .stdout(predicate::str::contains("global-proj"))
        .stdout(predicate::str::contains("global config"));
}

#[test]
fn config_set_global_default_format() {
    let config_dir = TempDir::new().unwrap();
    let root_dir = TempDir::new().unwrap();

    // Write minimal global config
    let rdm_config = config_dir.path().join("rdm");
    std::fs::create_dir_all(&rdm_config).unwrap();
    std::fs::write(
        rdm_config.join("config.toml"),
        format!("root = \"{}\"", root_dir.path().display()),
    )
    .unwrap();

    // Set default_format globally
    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args(["config", "set", "default_format", "json", "--global"])
        .assert()
        .success();

    // Verify it was written to the global config file
    let contents = std::fs::read_to_string(rdm_config.join("config.toml")).unwrap();
    assert!(contents.contains("default_format"));
    assert!(contents.contains("json"));
}

#[test]
fn config_repo_overrides_global() {
    let (config_dir, _root_dir) = setup_repo();

    // Set in global
    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args([
            "config",
            "set",
            "default_project",
            "global-proj",
            "--global",
        ])
        .assert()
        .success();

    // Set in repo
    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args(["config", "set", "default_project", "repo-proj"])
        .assert()
        .success();

    // Get should show repo value
    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args(["config", "get", "default_project"])
        .assert()
        .success()
        .stdout(predicate::str::contains("repo-proj"))
        .stdout(predicate::str::contains("repo config"));
}

#[test]
fn config_set_invalid_format_fails() {
    let (config_dir, _root_dir) = setup_repo();

    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args(["config", "set", "default_format", "xml"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid value 'xml'"));
}

#[test]
fn config_set_root_without_global_fails() {
    let (config_dir, _root_dir) = setup_repo();

    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args(["config", "set", "root", "/some/path"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--global"));
}

#[test]
fn config_get_unknown_key_fails() {
    let (config_dir, _root_dir) = setup_repo();

    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args(["config", "get", "nonexistent"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unknown config key"));
}

#[test]
fn default_format_in_global_affects_output() {
    let (config_dir, _root_dir) = setup_repo();

    // Set default_format = "json" globally
    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args(["config", "set", "default_format", "json", "--global"])
        .assert()
        .success();

    // roadmap list should output JSON
    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_FORMAT")
        .args(["roadmap", "list", "--project", "test"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("["));
}

#[test]
fn default_format_in_repo_overrides_global() {
    let (config_dir, _root_dir) = setup_repo();

    // Set global to markdown
    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args(["config", "set", "default_format", "markdown", "--global"])
        .assert()
        .success();

    // Set repo to json
    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args(["config", "set", "default_format", "json"])
        .assert()
        .success();

    // roadmap list should output JSON (repo wins)
    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_FORMAT")
        .args(["roadmap", "list", "--project", "test"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("["));
}

#[test]
fn format_flag_overrides_config() {
    let (config_dir, _root_dir) = setup_repo();

    // Set default_format = "json" in repo
    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args(["config", "set", "default_format", "json"])
        .assert()
        .success();

    // --format human should override
    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_FORMAT")
        .args(["--format", "human", "roadmap", "list", "--project", "test"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No roadmaps"));
}

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn rdm() -> Command {
    let mut cmd = Command::cargo_bin("rdm").unwrap();
    // Isolate from host global config (e.g. default_format = "json").
    cmd.env("XDG_CONFIG_HOME", "/dev/null/nonexistent");
    cmd
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
        .stdout(predicate::str::contains("remote.default"))
        .stdout(predicate::str::contains("root"))
        .stdout(predicate::str::contains("default_branch"))
        .stdout(predicate::str::contains("plan_review"));
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

#[test]
fn config_set_and_get_default_branch() {
    let (config_dir, _root_dir) = setup_repo();

    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args(["config", "set", "default_branch", "develop"])
        .assert()
        .success();

    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args(["config", "get", "default_branch"])
        .assert()
        .success()
        .stdout(predicate::str::contains("develop"));
}

#[test]
fn config_set_and_get_global_default_branch() {
    let (config_dir, _root_dir) = setup_repo();

    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args(["config", "set", "default_branch", "trunk", "--global"])
        .assert()
        .success();

    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args(["config", "get", "default_branch"])
        .assert()
        .success()
        .stdout(predicate::str::contains("trunk"))
        .stdout(predicate::str::contains("global config"));
}

#[test]
fn config_repo_default_branch_overrides_global() {
    let (config_dir, _root_dir) = setup_repo();

    // Set global
    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args(["config", "set", "default_branch", "trunk", "--global"])
        .assert()
        .success();

    // Set repo
    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args(["config", "set", "default_branch", "develop"])
        .assert()
        .success();

    // Get should show repo value
    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args(["config", "get", "default_branch"])
        .assert()
        .success()
        .stdout(predicate::str::contains("develop"))
        .stdout(predicate::str::contains("repo config"));
}

// -- hook_timeout_secs config round-trips --

#[test]
fn config_set_and_get_hook_timeout_secs() {
    let (config_dir, _root_dir) = setup_repo();

    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args(["config", "set", "hook_timeout_secs", "45"])
        .assert()
        .success()
        .stdout(predicate::str::contains("repo config"));

    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args(["config", "get", "hook_timeout_secs"])
        .assert()
        .success()
        .stdout(predicate::str::contains("45"))
        .stdout(predicate::str::contains("repo config"));

    // list shows the key with its value, not "(not set)".
    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args(["config", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("hook_timeout_secs"))
        .stdout(predicate::str::contains("45"));
}

#[test]
fn config_set_and_get_global_hook_timeout_secs() {
    let (config_dir, _root_dir) = setup_repo();

    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args(["config", "set", "hook_timeout_secs", "60", "--global"])
        .assert()
        .success()
        .stdout(predicate::str::contains("global config"));

    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args(["config", "get", "hook_timeout_secs"])
        .assert()
        .success()
        .stdout(predicate::str::contains("60"))
        .stdout(predicate::str::contains("global config"));

    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args(["config", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("hook_timeout_secs"))
        .stdout(predicate::str::contains("60"));
}

#[test]
fn config_repo_hook_timeout_overrides_global() {
    let (config_dir, _root_dir) = setup_repo();

    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args(["config", "set", "hook_timeout_secs", "60", "--global"])
        .assert()
        .success();

    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args(["config", "set", "hook_timeout_secs", "10"])
        .assert()
        .success();

    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args(["config", "get", "hook_timeout_secs"])
        .assert()
        .success()
        .stdout(predicate::str::contains("10"))
        .stdout(predicate::str::contains("repo config"));
}

#[test]
fn config_set_hook_timeout_rejects_non_integer() {
    let (config_dir, _root_dir) = setup_repo();

    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args(["config", "set", "hook_timeout_secs", "soon"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("non-negative integer"));
}

// -- plan_review config round-trips --

#[test]
fn config_set_and_get_plan_review() {
    let (config_dir, _root_dir) = setup_repo();

    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args(["config", "set", "plan_review", "true"])
        .assert()
        .success()
        .stdout(predicate::str::contains("repo config"));

    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args(["config", "get", "plan_review"])
        .assert()
        .success()
        .stdout(predicate::str::contains("true"))
        .stdout(predicate::str::contains("repo config"));
}

#[test]
fn config_set_and_get_global_plan_review() {
    let (config_dir, _root_dir) = setup_repo();

    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args(["config", "set", "plan_review", "true", "--global"])
        .assert()
        .success()
        .stdout(predicate::str::contains("global config"));

    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args(["config", "get", "plan_review"])
        .assert()
        .success()
        .stdout(predicate::str::contains("true"))
        .stdout(predicate::str::contains("global config"));
}

#[test]
fn config_repo_plan_review_overrides_global() {
    let (config_dir, _root_dir) = setup_repo();

    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args(["config", "set", "plan_review", "true", "--global"])
        .assert()
        .success();

    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args(["config", "set", "plan_review", "false"])
        .assert()
        .success();

    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args(["config", "get", "plan_review"])
        .assert()
        .success()
        .stdout(predicate::str::contains("false"))
        .stdout(predicate::str::contains("repo config"));
}

#[test]
fn config_set_plan_review_rejects_invalid() {
    let (config_dir, _root_dir) = setup_repo();

    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args(["config", "set", "plan_review", "yes"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("true").or(predicate::str::contains("false")));
}

// -- server.quick_filters config round-trips --

#[test]
fn config_set_and_get_quick_filters() {
    let (config_dir, root_dir) = setup_repo();

    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .env_remove("RDM_SERVER_QUICK_FILTERS")
        .args([
            "config",
            "set",
            "server.quick_filters",
            "Bug:bug,Refactor:refactor",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("repo config"));

    // rdm.toml on disk contains the array-of-tables form.
    let toml_contents = std::fs::read_to_string(root_dir.path().join("rdm.toml")).unwrap();
    assert!(toml_contents.contains("[[server.quick_filters]]"));
    assert!(toml_contents.contains("label = \"Bug\""));
    assert!(toml_contents.contains("tag = \"bug\""));
    assert!(toml_contents.contains("label = \"Refactor\""));
    assert!(toml_contents.contains("tag = \"refactor\""));

    // get prints the same Label:tag form.
    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .env_remove("RDM_SERVER_QUICK_FILTERS")
        .args(["config", "get", "server.quick_filters"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Bug:bug,Refactor:refactor"))
        .stdout(predicate::str::contains("repo config"));
}

#[test]
fn config_set_quick_filters_global_rejected() {
    let (config_dir, _root_dir) = setup_repo();

    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .env_remove("RDM_SERVER_QUICK_FILTERS")
        .args([
            "config",
            "set",
            "server.quick_filters",
            "Bug:bug",
            "--global",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("repo config"))
        .stderr(predicate::str::contains("--global"));
}

#[test]
fn config_get_quick_filters_env_override() {
    let (config_dir, _root_dir) = setup_repo();

    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env("RDM_SERVER_QUICK_FILTERS", "Env:tag")
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args(["config", "get", "server.quick_filters"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Env:tag"))
        .stdout(predicate::str::contains("environment variable"));
}

#[test]
fn config_list_includes_quick_filters() {
    let (config_dir, _root_dir) = setup_repo();

    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .env_remove("RDM_SERVER_QUICK_FILTERS")
        .args(["config", "set", "server.quick_filters", "Bug:bug"])
        .assert()
        .success();

    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .env_remove("RDM_SERVER_QUICK_FILTERS")
        .args(["config", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("server.quick_filters"))
        .stdout(predicate::str::contains("Bug:bug"))
        .stdout(predicate::str::contains("repo config"));
}

#[test]
fn config_set_empty_quick_filters_clears() {
    let (config_dir, root_dir) = setup_repo();

    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .env_remove("RDM_SERVER_QUICK_FILTERS")
        .args(["config", "set", "server.quick_filters", "Bug:bug"])
        .assert()
        .success();

    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .env_remove("RDM_SERVER_QUICK_FILTERS")
        .args(["config", "set", "server.quick_filters", ""])
        .assert()
        .success();

    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .env_remove("RDM_SERVER_QUICK_FILTERS")
        .args(["config", "get", "server.quick_filters"])
        .assert()
        .success()
        .stdout(predicate::str::contains("repo config"))
        .stdout(predicate::str::contains("Bug:bug").not());

    let toml_contents = std::fs::read_to_string(root_dir.path().join("rdm.toml")).unwrap();
    assert!(!toml_contents.contains("quick_filters"));
}

#[test]
fn config_set_quick_filters_missing_colon_fails() {
    let (config_dir, _root_dir) = setup_repo();

    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .env_remove("RDM_SERVER_QUICK_FILTERS")
        .args(["config", "set", "server.quick_filters", "Bug"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Label:tag"))
        .stderr(predicate::str::contains("server.quick_filters"))
        .stderr(predicate::str::contains("RDM_SERVER_QUICK_FILTERS").not());
}

#[test]
fn config_set_quick_filters_empty_side_fails() {
    let (config_dir, _root_dir) = setup_repo();

    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .env_remove("RDM_SERVER_QUICK_FILTERS")
        .args(["config", "set", "server.quick_filters", "Bug:"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Label:tag"));

    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .env_remove("RDM_SERVER_QUICK_FILTERS")
        .args(["config", "set", "server.quick_filters", ":bug"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Label:tag"));
}

#[test]
fn config_get_raw_prints_the_bare_value() {
    let (config_dir, _root_dir) = setup_repo();

    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args(["config", "set", "dispatch.verify", "bash scripts/ci.sh"])
        .assert()
        .success()
        .stdout(predicate::str::contains("repo config"));

    // The default form is annotated for humans...
    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args(["config", "get", "dispatch.verify"])
        .assert()
        .success()
        .stdout(predicate::str::contains("(source: repo config)"));

    // ...while `--raw` prints a value a caller can run verbatim. Asserted as an
    // exact line, because the whole point of the flag is that nothing else is
    // on it for a consumer to have to strip.
    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args(["config", "get", "dispatch.verify", "--raw"])
        .assert()
        .success()
        .stdout(predicate::eq("bash scripts/ci.sh\n"));
}

#[test]
fn gates_reviewed_round_trips_and_is_repo_only() {
    let (config_dir, _root_dir) = setup_repo();

    // Repo scope: set, then read back both annotated and raw.
    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args(["config", "set", "gates.reviewed", "true"])
        .assert()
        .success()
        .stdout(predicate::str::contains("repo config"));

    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args(["config", "get", "gates.reviewed", "--raw"])
        .assert()
        .success()
        .stdout(predicate::eq("true\n"));

    // A non-boolean value is rejected actionably.
    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args(["config", "set", "gates.reviewed", "yes"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("use 'true' or 'false'"));

    // Global scope: refused — whether a project enforces the gate is a
    // property of the project, never of a user.
    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args(["config", "set", "gates.reviewed", "true", "--global"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("repo config"));
}

#[test]
fn gates_reviewed_is_unset_by_default() {
    // The gate ships OFF. An unset key is what keeps every existing plan repo
    // — and rdm's own harnesses — behaving exactly as before.
    let (config_dir, _root_dir) = setup_repo();
    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args(["config", "get", "gates.reviewed"])
        .assert()
        .success()
        .stdout(predicate::str::contains("(not set)"));
}

#[test]
fn config_get_raw_prints_nothing_when_unset() {
    let (config_dir, _root_dir) = setup_repo();

    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args(["config", "get", "dispatch.verify"])
        .assert()
        .success()
        .stdout(predicate::str::contains("(not set)"));

    // `--raw` emits NO output for an unset key — an empty read is what tells a
    // caller to fall back, and "(not set)" would otherwise be run as a command.
    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args(["config", "get", "dispatch.verify", "--raw"])
        .assert()
        .success()
        .stdout(predicate::eq(""));
}

#[test]
fn config_get_raw_honors_the_env_override() {
    let (config_dir, _root_dir) = setup_repo();

    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .env("RDM_DISPATCH_VERIFY", "bash scripts/from-env.sh")
        .args(["config", "get", "dispatch.verify", "--raw"])
        .assert()
        .success()
        .stdout(predicate::eq("bash scripts/from-env.sh\n"));
}

/// Names of files touched by the most recent commit — clears the outer
/// repo's git env vars so this doesn't inherit them when run from inside a
/// git hook (e.g. the pre-commit hook that runs this very test suite).
fn last_commit_files(dir: &std::path::Path) -> Vec<String> {
    let output = std::process::Command::new("git")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .args(["show", "--stat", "-1", "--pretty=format:"])
        .current_dir(dir)
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|l| {
            let l = l.trim();
            // `git show --stat` lines look like "path/to/file | 3 +++---";
            // the trailing summary line ("N files changed, ...") has no '|'.
            l.split_once('|').map(|(path, _)| path.trim().to_string())
        })
        .collect()
}

#[test]
fn config_set_repo_lands_under_the_callers_changeset() {
    let (config_dir, root_dir) = setup_repo();

    // `config set` stages the write through the Store, under this session's
    // own changeset — not a raw filesystem write that belongs to nobody.
    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args(["config", "set", "default_project", "my-proj"])
        .assert()
        .success()
        .stdout(predicate::str::contains("repo config"));

    // `rdm status` must show rdm.toml as a plain uncommitted change owned by
    // this session — never under an others/unattributed note, which is
    // exactly the bug this phase fixes.
    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args(["status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("rdm.toml"))
        .stdout(predicate::str::contains("belong to other changesets").not())
        .stdout(predicate::str::contains("are not attributed to any changeset").not());

    // A SCOPED `rdm commit` (no --all) must land it.
    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args(["commit", "-m", "chore: set default_project"])
        .assert()
        .success();

    let committed = last_commit_files(root_dir.path());
    assert!(
        committed.iter().any(|p| p == "rdm.toml"),
        "expected rdm.toml among the committed paths, got {committed:?}"
    );

    // And the working tree is now clean.
    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args(["status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No uncommitted changes."));
}

#[test]
fn config_set_against_an_uninitialized_root_fails_actionably() {
    let config_dir = TempDir::new().unwrap();
    let root_dir = TempDir::new().unwrap();

    // Point at root_dir but never run `rdm init` against it.
    let rdm_config = config_dir.path().join("rdm");
    std::fs::create_dir_all(&rdm_config).unwrap();
    std::fs::write(
        rdm_config.join("config.toml"),
        format!("root = \"{}\"", root_dir.path().display()),
    )
    .unwrap();

    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .args(["config", "set", "default_project", "my-proj"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("failed to open git repository"));

    // And no git-less rdm.toml was silently written.
    assert!(!root_dir.path().join("rdm.toml").exists());
}

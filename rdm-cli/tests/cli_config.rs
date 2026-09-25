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

// -- max_refutations config round-trips --

/// An `rdm` command isolated to this test's config dir, with no ambient
/// `RDM_MAX_REFUTATIONS` override.
fn rdm_in(config_dir: &TempDir) -> Command {
    let mut cmd = rdm();
    cmd.env("XDG_CONFIG_HOME", config_dir.path())
        .env_remove("RDM_ROOT")
        .env_remove("RDM_PROJECT")
        .env_remove("RDM_FORMAT")
        .env_remove("RDM_MAX_REFUTATIONS");
    cmd
}

#[test]
fn config_set_and_get_max_refutations() {
    let (config_dir, _root_dir) = setup_repo();

    rdm_in(&config_dir)
        .args(["config", "set", "max_refutations", "8"])
        .assert()
        .success()
        .stdout(predicate::str::contains("repo config"));

    rdm_in(&config_dir)
        .args(["config", "get", "max_refutations"])
        .assert()
        .success()
        .stdout(predicate::str::contains("8  (source: repo config)"));

    rdm_in(&config_dir)
        .args(["config", "get", "max_refutations", "--raw"])
        .assert()
        .success()
        .stdout("8\n");

    rdm_in(&config_dir)
        .args(["config", "list"])
        .assert()
        .success()
        .stdout(
            predicate::str::is_match(r"max_refutations\s+8\s+\(source: repo config\)").unwrap(),
        );
}

#[test]
fn config_set_and_get_global_max_refutations() {
    let (config_dir, _root_dir) = setup_repo();

    rdm_in(&config_dir)
        .args(["config", "set", "max_refutations", "3", "--global"])
        .assert()
        .success()
        .stdout(predicate::str::contains("global config"));

    rdm_in(&config_dir)
        .args(["config", "get", "max_refutations"])
        .assert()
        .success()
        .stdout(predicate::str::contains("3  (source: global config)"));
}

#[test]
fn config_repo_max_refutations_overrides_global() {
    let (config_dir, _root_dir) = setup_repo();

    rdm_in(&config_dir)
        .args(["config", "set", "max_refutations", "3", "--global"])
        .assert()
        .success();
    rdm_in(&config_dir)
        .args(["config", "set", "max_refutations", "1"])
        .assert()
        .success();

    rdm_in(&config_dir)
        .args(["config", "get", "max_refutations"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1  (source: repo config)"));
}

#[test]
fn config_max_refutations_zero_round_trips_as_zero() {
    let (config_dir, _root_dir) = setup_repo();

    rdm_in(&config_dir)
        .args(["config", "set", "max_refutations", "0"])
        .assert()
        .success();

    rdm_in(&config_dir)
        .args(["config", "get", "max_refutations"])
        .assert()
        .success()
        .stdout(predicate::str::contains("0  (source: repo config)"));
}

#[test]
fn config_max_refutations_env_overrides_config() {
    let (config_dir, _root_dir) = setup_repo();

    rdm_in(&config_dir)
        .args(["config", "set", "max_refutations", "8"])
        .assert()
        .success();

    rdm_in(&config_dir)
        .env("RDM_MAX_REFUTATIONS", "0")
        .args(["config", "get", "max_refutations"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "0  (source: environment variable)",
        ));

    rdm_in(&config_dir)
        .env("RDM_MAX_REFUTATIONS", "0")
        .args(["config", "get", "max_refutations", "--raw"])
        .assert()
        .success()
        .stdout("0\n");
}

#[test]
fn config_set_max_refutations_rejects_invalid_values_and_writes_nothing() {
    let (config_dir, root_dir) = setup_repo();
    let rdm_toml = root_dir.path().join("rdm.toml");
    let before = std::fs::read(&rdm_toml).unwrap();

    for bad in ["-1", "soon", ""] {
        rdm_in(&config_dir)
            .args(["config", "set", "max_refutations", "--", bad])
            .assert()
            .failure()
            .stderr(predicate::str::contains("non-negative integer"));
    }

    assert_eq!(std::fs::read(&rdm_toml).unwrap(), before);
    rdm_in(&config_dir)
        .args(["config", "get", "max_refutations"])
        .assert()
        .success()
        .stdout(predicate::str::contains("(not set)"));
}

#[test]
fn config_set_max_refutations_accepts_the_shared_grammar() {
    let (config_dir, _root_dir) = setup_repo();

    for (input, stored) in [("+5", "5"), (" 7 ", "7"), ("007", "7")] {
        rdm_in(&config_dir)
            .args(["config", "set", "max_refutations", "--", input])
            .assert()
            .success();
        rdm_in(&config_dir)
            .args(["config", "get", "max_refutations", "--raw"])
            .assert()
            .success()
            .stdout(format!("{stored}\n"));
    }
}

#[test]
fn config_get_max_refutations_rejects_a_malformed_env_value() {
    let (config_dir, _root_dir) = setup_repo();

    for bad in ["abc", "-1"] {
        for raw in [false, true] {
            let mut cmd = rdm_in(&config_dir);
            cmd.env("RDM_MAX_REFUTATIONS", bad)
                .args(["config", "get", "max_refutations"]);
            if raw {
                cmd.arg("--raw");
            }
            cmd.assert()
                .failure()
                .stdout("")
                .stderr(predicate::str::contains("RDM_MAX_REFUTATIONS"))
                .stderr(predicate::str::contains("non-negative integer"));
        }
    }
}

#[test]
fn config_list_reports_a_malformed_max_refutations_env_in_its_row() {
    let (config_dir, _root_dir) = setup_repo();

    rdm_in(&config_dir)
        .args(["config", "set", "max_refutations", "8"])
        .assert()
        .success();

    rdm_in(&config_dir)
        .env("RDM_MAX_REFUTATIONS", "abc")
        .args(["config", "list"])
        .assert()
        .success()
        .stdout(
            predicate::str::is_match(r"max_refutations\s+\(error: .*RDM_MAX_REFUTATIONS.*\)")
                .unwrap(),
        )
        // The rest of the list still prints.
        .stdout(predicate::str::contains("default_project"))
        .stdout(predicate::str::contains("hook_timeout_secs"));
}

#[test]
fn config_blank_max_refutations_env_falls_through_to_config() {
    let (config_dir, _root_dir) = setup_repo();

    rdm_in(&config_dir)
        .args(["config", "set", "max_refutations", "0"])
        .assert()
        .success();

    for blank in ["", "   "] {
        rdm_in(&config_dir)
            .env("RDM_MAX_REFUTATIONS", blank)
            .args(["config", "get", "max_refutations"])
            .assert()
            .success()
            .stdout(predicate::str::contains("0  (source: repo config)"));

        rdm_in(&config_dir)
            .env("RDM_MAX_REFUTATIONS", blank)
            .args(["config", "get", "max_refutations", "--raw"])
            .assert()
            .success()
            .stdout("0\n");

        rdm_in(&config_dir)
            .env("RDM_MAX_REFUTATIONS", blank)
            .args(["config", "list"])
            .assert()
            .success()
            .stdout(
                predicate::str::is_match(r"max_refutations\s+0\s+\(source: repo config\)").unwrap(),
            );
    }
}

#[test]
fn config_blank_max_refutations_env_with_nothing_set_is_unset() {
    let (config_dir, _root_dir) = setup_repo();

    rdm_in(&config_dir)
        .env("RDM_MAX_REFUTATIONS", " ")
        .args(["config", "get", "max_refutations"])
        .assert()
        .success()
        .stdout(predicate::str::contains("(not set)"));

    rdm_in(&config_dir)
        .env("RDM_MAX_REFUTATIONS", " ")
        .args(["config", "get", "max_refutations", "--raw"])
        .assert()
        .success()
        .stdout("");
}

#[test]
fn config_project_max_refutations_is_reported_for_that_project_only() {
    let (config_dir, _root_dir) = setup_repo();

    rdm_in(&config_dir)
        .args(["config", "set", "max_refutations", "8"])
        .assert()
        .success();
    rdm_in(&config_dir)
        .args(["config", "set", "max_refutations", "3", "--project", "web"])
        .assert()
        .success()
        .stdout(predicate::str::contains("project config for 'web'"));

    rdm_in(&config_dir)
        .args(["config", "get", "max_refutations", "--project", "web"])
        .assert()
        .success()
        .stdout("3  (source: project config)\n");
    rdm_in(&config_dir)
        .args([
            "config",
            "get",
            "max_refutations",
            "--project",
            "web",
            "--raw",
        ])
        .assert()
        .success()
        .stdout("3\n");
    rdm_in(&config_dir)
        .args(["config", "list", "--project", "web"])
        .assert()
        .success()
        .stdout(
            predicate::str::is_match(r"max_refutations\s+3\s+\(source: project config\)").unwrap(),
        );

    // Another project, and no project at all, still see the repo-wide value.
    rdm_in(&config_dir)
        .args(["config", "get", "max_refutations", "--project", "other"])
        .assert()
        .success()
        .stdout("8  (source: repo config)\n");
    rdm_in(&config_dir)
        .args(["config", "get", "max_refutations"])
        .assert()
        .success()
        .stdout("8  (source: repo config)\n");

    // The environment still beats the project override.
    rdm_in(&config_dir)
        .env("RDM_MAX_REFUTATIONS", "1")
        .args([
            "config",
            "get",
            "max_refutations",
            "--project",
            "web",
            "--raw",
        ])
        .assert()
        .success()
        .stdout("1\n");
}

#[test]
fn config_project_max_refutations_zero_is_a_value() {
    let (config_dir, _root_dir) = setup_repo();

    rdm_in(&config_dir)
        .args(["config", "set", "max_refutations", "5"])
        .assert()
        .success();
    rdm_in(&config_dir)
        .args(["config", "set", "max_refutations", "0", "--project", "web"])
        .assert()
        .success();

    rdm_in(&config_dir)
        .args([
            "config",
            "get",
            "max_refutations",
            "--project",
            "web",
            "--raw",
        ])
        .assert()
        .success()
        .stdout("0\n");
}

#[test]
fn config_set_project_max_refutations_rejects_invalid_values_and_writes_nothing() {
    let (config_dir, root_dir) = setup_repo();
    let rdm_toml = root_dir.path().join("rdm.toml");
    let before = std::fs::read(&rdm_toml).unwrap();

    for bad in ["-1", "abc", "", "5x"] {
        rdm_in(&config_dir)
            .args([
                "config",
                "set",
                "max_refutations",
                "--project",
                "web",
                "--",
                bad,
            ])
            .assert()
            .failure()
            .stderr(predicate::str::contains("non-negative integer"));
    }

    assert_eq!(std::fs::read(&rdm_toml).unwrap(), before);
    rdm_in(&config_dir)
        .args(["config", "get", "max_refutations", "--project", "web"])
        .assert()
        .success()
        .stdout("(not set)\n");
}

#[test]
fn config_get_max_refutations_env_reports_the_parsed_number() {
    let (config_dir, _root_dir) = setup_repo();

    rdm_in(&config_dir)
        .env("RDM_MAX_REFUTATIONS", " +4 ")
        .args(["config", "get", "max_refutations", "--raw"])
        .assert()
        .success()
        .stdout("4\n");
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

// --- [projects.<name>] override layer: `config get|set|list --project` ---

/// An `rdm` command against `setup_repo`'s plan repo with every env override
/// of a project-scopable key removed, so each test controls the environment.
fn scoped(config_dir: &TempDir) -> Command {
    let mut cmd = rdm();
    cmd.env("XDG_CONFIG_HOME", config_dir.path());
    for var in [
        "RDM_ROOT",
        "RDM_PROJECT",
        "RDM_FORMAT",
        "RDM_DISPATCH_VERIFY",
        "RDM_GATES_REVIEWED",
        "RDM_REVIEWED_GATE",
        "RDM_PLAN_REVIEW",
        "RDM_DEFAULT_BRANCH",
    ] {
        cmd.env_remove(var);
    }
    cmd
}

fn read_repo_config(root_dir: &TempDir) -> rdm_core::config::Config {
    let text = std::fs::read_to_string(root_dir.path().join("rdm.toml")).unwrap();
    rdm_core::config::Config::from_toml(&text).unwrap()
}

/// Sets `dispatch.verify` plan-repo-wide to `repo-cmd` and for project `a` to `X`.
fn set_repo_and_project_verify(config_dir: &TempDir) {
    scoped(config_dir)
        .args(["config", "set", "dispatch.verify", "repo-cmd"])
        .assert()
        .success();
    scoped(config_dir)
        .args(["config", "set", "dispatch.verify", "X", "--project", "a"])
        .assert()
        .success()
        .stdout(predicate::str::contains("project config for 'a'"));
}

#[test]
fn config_set_project_writes_the_project_table_and_leaves_repo_value() {
    let (config_dir, root_dir) = setup_repo();
    set_repo_and_project_verify(&config_dir);

    let config = read_repo_config(&root_dir);
    assert_eq!(
        config.projects["a"]
            .dispatch
            .as_ref()
            .and_then(|d| d.verify.as_deref()),
        Some("X")
    );
    assert_eq!(
        config.dispatch.as_ref().and_then(|d| d.verify.as_deref()),
        Some("repo-cmd")
    );

    scoped(&config_dir)
        .args(["config", "get", "dispatch.verify"])
        .assert()
        .success()
        .stdout(predicate::eq("repo-cmd  (source: repo config)\n"));
}

#[test]
fn config_get_project_reports_project_then_repo() {
    let (config_dir, _root_dir) = setup_repo();
    set_repo_and_project_verify(&config_dir);

    scoped(&config_dir)
        .args(["config", "get", "dispatch.verify", "--project", "a"])
        .assert()
        .success()
        .stdout(predicate::eq("X  (source: project config)\n"));
    scoped(&config_dir)
        .args(["config", "get", "dispatch.verify", "--project", "b"])
        .assert()
        .success()
        .stdout(predicate::eq("repo-cmd  (source: repo config)\n"));
    scoped(&config_dir)
        .args([
            "config",
            "get",
            "dispatch.verify",
            "--raw",
            "--project",
            "a",
        ])
        .assert()
        .success()
        .stdout(predicate::eq("X\n"));
}

#[test]
fn config_list_project_lists_exactly_the_scopable_keys() {
    let (config_dir, _root_dir) = setup_repo();
    set_repo_and_project_verify(&config_dir);

    let out = scoped(&config_dir)
        .args(["config", "list", "--project", "a"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let out = String::from_utf8(out).unwrap();
    let lines: Vec<&str> = out.lines().collect();
    let keys: Vec<&str> = lines
        .iter()
        .map(|l| l.split_whitespace().next().unwrap())
        .collect();
    assert_eq!(
        keys,
        [
            "dispatch.verify",
            "gates.reviewed",
            "plan_review",
            "default_branch",
            "max_refutations"
        ],
        "{out}"
    );
    let line = |key: &str| {
        *lines
            .iter()
            .find(|l| l.split_whitespace().next() == Some(key))
            .unwrap()
    };
    assert!(
        line("dispatch.verify").contains("X  (source: project config)"),
        "{out}"
    );
    assert!(line("gates.reviewed").contains("(not set)"), "{out}");
    assert!(line("default_branch").contains("(not set)"), "{out}");
}

#[test]
fn config_set_non_scopable_key_with_project_is_refused() {
    let (config_dir, root_dir) = setup_repo();
    let before = std::fs::read_to_string(root_dir.path().join("rdm.toml")).unwrap();

    let assert = scoped(&config_dir)
        .args([
            "config",
            "set",
            "remote.default",
            "origin",
            "--project",
            "a",
        ])
        .assert()
        .failure();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    for key in [
        "dispatch.verify",
        "gates.reviewed",
        "plan_review",
        "default_branch",
    ] {
        assert!(stderr.contains(key), "{key} missing from: {stderr}");
    }
    assert_eq!(
        std::fs::read_to_string(root_dir.path().join("rdm.toml")).unwrap(),
        before
    );
}

#[test]
fn config_get_non_scopable_key_with_project_is_refused() {
    let (config_dir, _root_dir) = setup_repo();

    let assert = scoped(&config_dir)
        .args(["config", "get", "remote.default", "--project", "a"])
        .assert()
        .failure();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    for key in [
        "dispatch.verify",
        "gates.reviewed",
        "plan_review",
        "default_branch",
    ] {
        assert!(stderr.contains(key), "{key} missing from: {stderr}");
    }
}

#[test]
fn config_set_project_with_global_is_refused() {
    let (config_dir, root_dir) = setup_repo();
    let before_repo = std::fs::read_to_string(root_dir.path().join("rdm.toml")).unwrap();
    let global_path = config_dir.path().join("rdm").join("config.toml");
    let before_global = std::fs::read_to_string(&global_path).unwrap();

    scoped(&config_dir)
        .args([
            "config",
            "set",
            "plan_review",
            "true",
            "--project",
            "a",
            "--global",
        ])
        .assert()
        .failure();

    assert_eq!(
        std::fs::read_to_string(root_dir.path().join("rdm.toml")).unwrap(),
        before_repo
    );
    assert_eq!(
        std::fs::read_to_string(&global_path).unwrap(),
        before_global
    );
}

#[test]
fn config_get_project_env_override_wins() {
    let (config_dir, _root_dir) = setup_repo();
    set_repo_and_project_verify(&config_dir);

    scoped(&config_dir)
        .env("RDM_DISPATCH_VERIFY", "E")
        .args(["config", "get", "dispatch.verify", "--project", "a"])
        .assert()
        .success()
        .stdout(predicate::eq("E  (source: environment variable)\n"));
}

#[test]
fn config_set_scope_never_comes_from_rdm_project() {
    let (config_dir, root_dir) = setup_repo();

    scoped(&config_dir)
        .env("RDM_PROJECT", "b")
        .args(["config", "set", "dispatch.verify", "X", "--project", "a"])
        .assert()
        .success();
    scoped(&config_dir)
        .env("RDM_PROJECT", "a")
        .args(["config", "set", "dispatch.verify", "repo-cmd"])
        .assert()
        .success();

    let config = read_repo_config(&root_dir);
    assert_eq!(
        config.projects.keys().collect::<Vec<_>>(),
        ["a"],
        "only the explicit --project may name a project table"
    );
    assert_eq!(
        config.projects["a"]
            .dispatch
            .as_ref()
            .and_then(|d| d.verify.as_deref()),
        Some("X")
    );
    assert_eq!(
        config.dispatch.as_ref().and_then(|d| d.verify.as_deref()),
        Some("repo-cmd")
    );
}

#[test]
fn config_get_gates_reviewed_reports_rdm_reviewed_gate() {
    let (config_dir, _root_dir) = setup_repo();

    scoped(&config_dir)
        .env("RDM_REVIEWED_GATE", "true")
        .args(["config", "get", "gates.reviewed"])
        .assert()
        .success()
        .stdout(predicate::eq("true  (source: environment variable)\n"));
    scoped(&config_dir)
        .env("RDM_REVIEWED_GATE", "yes")
        .args(["config", "get", "gates.reviewed"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("RDM_REVIEWED_GATE"));
}

#[test]
fn config_list_reports_a_bad_key_inline_and_lists_the_rest() {
    let (config_dir, _root_dir) = setup_repo();
    scoped(&config_dir)
        .args(["config", "set", "dispatch.verify", "repo-cmd"])
        .assert()
        .success();

    for extra in [&[][..], &["--project", "a"][..]] {
        let out = scoped(&config_dir)
            .env("RDM_REVIEWED_GATE", "yes")
            .args(["config", "list"])
            .args(extra)
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let out = String::from_utf8(out).unwrap();
        let gate = out
            .lines()
            .find(|l| l.starts_with("gates.reviewed"))
            .unwrap_or_else(|| panic!("no gates.reviewed row: {out}"));
        assert!(
            gate.contains("(error:") && gate.contains("RDM_REVIEWED_GATE"),
            "{out}"
        );
        assert!(
            out.lines()
                .any(|l| l.starts_with("dispatch.verify") && l.contains("repo-cmd")),
            "the other keys must still be listed: {out}"
        );
    }
}

#[test]
fn config_set_project_writes_every_scopable_key_to_the_project_table() {
    let (config_dir, root_dir) = setup_repo();
    for (key, value) in [
        ("plan_review", "true"),
        ("gates.reviewed", "true"),
        ("default_branch", "trunk"),
    ] {
        scoped(&config_dir)
            .args(["config", "set", key, value, "--project", "a"])
            .assert()
            .success()
            .stdout(predicate::str::contains("project config for 'a'"));
    }

    let config = read_repo_config(&root_dir);
    let a = &config.projects["a"];
    assert_eq!(a.plan_review, Some(true));
    assert_eq!(a.gates.as_ref().and_then(|g| g.reviewed), Some(true));
    assert_eq!(a.default_branch.as_deref(), Some("trunk"));
    assert_eq!(config.plan_review, None);
    assert!(config.gates.as_ref().and_then(|g| g.reviewed).is_none());
    assert_eq!(config.default_branch, None);
}

#[test]
fn config_set_invalid_values_are_refused_and_leave_rdm_toml_unchanged() {
    let (config_dir, root_dir) = setup_repo();
    let before = std::fs::read_to_string(root_dir.path().join("rdm.toml")).unwrap();

    for (key, value, project) in [
        ("dispatch.verify", "  ", Some("a")),
        ("plan_review", "maybe", Some("a")),
        ("gates.reviewed", "maybe", Some("a")),
        ("dispatch.verify", "  ", None),
        ("dispatch.verify", "", None),
    ] {
        let mut cmd = scoped(&config_dir);
        cmd.args(["config", "set", key, value]);
        if let Some(p) = project {
            cmd.args(["--project", p]);
        }
        cmd.assert().failure();
        assert_eq!(
            std::fs::read_to_string(root_dir.path().join("rdm.toml")).unwrap(),
            before,
            "{key}={value:?} --project {project:?} changed rdm.toml"
        );
    }
}

#[test]
fn config_get_gates_reviewed_prefers_rdm_reviewed_gate_and_validates_the_generic_name() {
    let (config_dir, _root_dir) = setup_repo();

    scoped(&config_dir)
        .env("RDM_REVIEWED_GATE", "true")
        .env("RDM_GATES_REVIEWED", "false")
        .args(["config", "get", "gates.reviewed"])
        .assert()
        .success()
        .stdout(predicate::eq("true  (source: environment variable)\n"));
    scoped(&config_dir)
        .env("RDM_GATES_REVIEWED", "yes")
        .args(["config", "get", "gates.reviewed"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("RDM_GATES_REVIEWED"));
}

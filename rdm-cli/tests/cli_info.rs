use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn rdm() -> Command {
    let mut cmd = Command::cargo_bin("rdm").unwrap();
    // Isolate from host global config (e.g. default_format = "json") and any
    // env vars a real dev shell might have set.
    cmd.env("XDG_CONFIG_HOME", "/dev/null/nonexistent");
    cmd.env_remove("RDM_ROOT");
    cmd.env_remove("RDM_PROJECT");
    cmd.env_remove("RDM_DEFAULT_PROJECT");
    cmd.env_remove("RDM_FORMAT");
    cmd.env_remove("RDM_DEFAULT_BRANCH");
    cmd
}

/// A bare temp dir with no rdm.toml — `info` must still work here (it is
/// exempt from the "no plan repo found" guard, like `describe`/`model`).
fn bare_dir() -> TempDir {
    TempDir::new().unwrap()
}

/// Writes a repo-level `rdm.toml` with the given raw TOML body.
fn write_repo_config(root: &TempDir, toml: &str) {
    std::fs::write(root.path().join("rdm.toml"), toml).unwrap();
}

/// Writes a global `config.toml` under a fresh XDG config dir, returning
/// that config dir's path (pass it as `XDG_CONFIG_HOME`).
fn write_global_config(toml: &str) -> TempDir {
    let config_dir = TempDir::new().unwrap();
    let rdm_config = config_dir.path().join("rdm");
    std::fs::create_dir_all(&rdm_config).unwrap();
    std::fs::write(rdm_config.join("config.toml"), toml).unwrap();
    config_dir
}

// ---------------------------------------------------------------------------
// Malformed rdm.toml — `info` must read the repo config only once, so a
// parse failure warns exactly once, not once per internal re-read.
// ---------------------------------------------------------------------------

#[test]
fn malformed_repo_config_warns_only_once() {
    let root = bare_dir();
    write_repo_config(&root, "this is not valid toml [[[");

    let output = rdm()
        .arg("--root")
        .arg(root.path())
        .args(["info", "--format", "json"])
        .assert()
        .success()
        .get_output()
        .clone();

    let stderr = String::from_utf8(output.stderr).unwrap();
    let warning_count = stderr
        .matches("warning: ignoring malformed config at")
        .count();
    assert_eq!(
        warning_count, 1,
        "expected exactly one malformed-config warning, got {warning_count}: {stderr}"
    );
}

// ---------------------------------------------------------------------------
// AC1: JSON shape, project omission
// ---------------------------------------------------------------------------

#[test]
fn info_json_omits_project_when_unresolved() {
    let root = bare_dir();

    rdm()
        .arg("--root")
        .arg(root.path())
        .args(["info", "--format", "json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"project\"").not())
        .stdout(predicate::str::contains("\"default_branch\": \"main\""))
        // The --format flag doubles as the reported default_format value —
        // see the design note on the human-format tests below.
        .stdout(predicate::str::contains("\"default_format\": \"json\""));
}

#[test]
fn info_json_includes_project_when_resolved() {
    let root = bare_dir();

    rdm()
        .arg("--root")
        .arg(root.path())
        .args(["info", "--project", "rdm", "--format", "json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"project\": \"rdm\""));
}

#[test]
fn info_json_reports_resolved_root() {
    let root = bare_dir();

    rdm()
        .arg("--root")
        .arg(root.path())
        .args(["info", "--format", "json"])
        .assert()
        .success()
        .stdout(predicate::str::contains(root.path().display().to_string()));
}

// ---------------------------------------------------------------------------
// AC2: project precedence matrix — flag > RDM_PROJECT env > repo > global > absent
// ---------------------------------------------------------------------------

#[test]
fn project_flag_wins_over_everything() {
    let root = bare_dir();
    write_repo_config(&root, "default_project = \"repo-proj\"\n");
    let config_dir = write_global_config("default_project = \"global-proj\"\n");

    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env("RDM_PROJECT", "env-proj")
        .arg("--root")
        .arg(root.path())
        .args(["info", "--project", "flag-proj", "--format", "json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"project\": \"flag-proj\""));
}

#[test]
fn project_env_wins_over_repo_and_global_config() {
    let root = bare_dir();
    write_repo_config(&root, "default_project = \"repo-proj\"\n");
    let config_dir = write_global_config("default_project = \"global-proj\"\n");

    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env("RDM_PROJECT", "env-proj")
        .arg("--root")
        .arg(root.path())
        .args(["info", "--format", "json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"project\": \"env-proj\""));
}

#[test]
fn project_repo_config_wins_over_global_config() {
    let root = bare_dir();
    write_repo_config(&root, "default_project = \"repo-proj\"\n");
    let config_dir = write_global_config("default_project = \"global-proj\"\n");

    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .arg("--root")
        .arg(root.path())
        .args(["info", "--format", "json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"project\": \"repo-proj\""));
}

#[test]
fn project_global_config_used_when_repo_unset() {
    let root = bare_dir();
    let config_dir = write_global_config("default_project = \"global-proj\"\n");

    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .arg("--root")
        .arg(root.path())
        .args(["info", "--format", "json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"project\": \"global-proj\""));
}

#[test]
fn project_absent_when_nothing_resolves() {
    let root = bare_dir();

    rdm()
        .arg("--root")
        .arg(root.path())
        .args(["info", "--format", "json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"project\"").not());
}

/// The env-key-mismatch regression this phase exists to fix: `config
/// list`/`config get` check `RDM_DEFAULT_PROJECT` for the `default_project`
/// key, but real project resolution (`paths::resolve_project`, used by every
/// other command) reads `RDM_PROJECT`. `info` must report what the CLI would
/// really use — so `RDM_DEFAULT_PROJECT` alone must NOT populate `project`.
#[test]
fn rdm_default_project_env_is_ignored_for_info() {
    let root = bare_dir();

    rdm()
        .env("RDM_DEFAULT_PROJECT", "should-not-appear")
        .arg("--root")
        .arg(root.path())
        .args(["info", "--format", "json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"project\"").not())
        .stdout(predicate::str::contains("should-not-appear").not());
}

/// An explicit empty `--project ""` resolves to `Some("")` and is reported
/// as-is, not treated as absent — consistent with every other rdm command,
/// none of which validates this either.
#[test]
fn project_empty_string_flag_is_reported_as_is() {
    let root = bare_dir();

    rdm()
        .arg("--root")
        .arg(root.path())
        .args(["info", "--project", "", "--format", "json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"project\": \"\""));
}

// ---------------------------------------------------------------------------
// AC3: unresolvable project is a non-error
// ---------------------------------------------------------------------------

#[test]
fn unresolvable_project_is_non_error_json() {
    let root = bare_dir();

    rdm()
        .arg("--root")
        .arg(root.path())
        .args(["info", "--format", "json"])
        .assert()
        .success()
        .code(0)
        .stdout(predicate::str::contains("\"project\"").not());
}

#[test]
fn unresolvable_project_human_shows_not_set() {
    let root = bare_dir();

    rdm()
        .arg("--root")
        .arg(root.path())
        .arg("info")
        .assert()
        .success()
        .code(0)
        .stdout(predicate::str::contains("project: (not set)"));
}

// ---------------------------------------------------------------------------
// AC4: human/markdown format with per-key source, --format table rejected
// ---------------------------------------------------------------------------

#[test]
fn human_format_shows_value_and_source() {
    let root = bare_dir();
    write_repo_config(&root, "default_branch = \"trunk\"\n");

    rdm()
        .arg("--root")
        .arg(root.path())
        .args(["info", "--project", "rdm"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("root:").and(predicate::str::contains("(source: CLI flag)")),
        )
        .stdout(predicate::str::contains("project: rdm  (source: CLI flag)"))
        .stdout(predicate::str::contains(
            "default_branch: trunk  (source: repo config)",
        ))
        // No --format flag was passed, so default_format falls all the way
        // through to the built-in default.
        .stdout(predicate::str::contains(
            "default_format: human  (source: default)",
        ));
}

#[test]
fn markdown_format_is_a_table() {
    let root = bare_dir();

    rdm()
        .arg("--root")
        .arg(root.path())
        .args(["info", "--project", "rdm", "--format", "markdown"])
        .assert()
        .success()
        .stdout(predicate::str::contains("| Key | Value | Source |"))
        .stdout(predicate::str::contains("| root |"))
        .stdout(predicate::str::contains("| project | rdm |"))
        .stdout(predicate::str::contains("| default_branch |"))
        .stdout(predicate::str::contains("| default_format |"));
}

#[test]
fn markdown_format_unresolved_project_shows_not_set() {
    let root = bare_dir();

    rdm()
        .arg("--root")
        .arg(root.path())
        .args(["info", "--format", "markdown"])
        .assert()
        .success()
        .stdout(predicate::str::contains("| project | (not set) |"));
}

#[test]
fn table_format_rejected_with_actionable_message() {
    let root = bare_dir();

    rdm()
        .arg("--root")
        .arg(root.path())
        .args(["info", "--format", "table"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "--format table is not supported for 'info'; use --format human, --format json, --format markdown, or omit --format",
        ));
}

// ---------------------------------------------------------------------------
// default_branch precedence — repo > global > built-in default, NO env layer
// ---------------------------------------------------------------------------

#[test]
fn default_branch_repo_config_wins_over_global() {
    let root = bare_dir();
    write_repo_config(&root, "default_branch = \"repo-branch\"\n");
    let config_dir = write_global_config("default_branch = \"global-branch\"\n");

    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .arg("--root")
        .arg(root.path())
        .arg("info")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "default_branch: repo-branch  (source: repo config)",
        ));
}

#[test]
fn default_branch_global_config_used_when_repo_unset() {
    let root = bare_dir();
    let config_dir = write_global_config("default_branch = \"global-branch\"\n");

    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .arg("--root")
        .arg(root.path())
        .arg("info")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "default_branch: global-branch  (source: global config)",
        ));
}

/// `default_branch` has no env-var layer in real resolution (unlike
/// `default_project`/`default_format`) — `RDM_DEFAULT_BRANCH` must be
/// ignored by `rdm info`, mirroring the `RDM_DEFAULT_PROJECT` mismatch this
/// phase fixes for `project`.
#[test]
fn default_branch_ignores_rdm_default_branch_env() {
    let root = bare_dir();

    rdm()
        .env("RDM_DEFAULT_BRANCH", "should-not-win")
        .arg("--root")
        .arg(root.path())
        .arg("info")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "default_branch: main  (source: default)",
        ))
        .stdout(predicate::str::contains("should-not-win").not());
}

// ---------------------------------------------------------------------------
// default_format precedence — flag > RDM_FORMAT env > repo > global > default
// ---------------------------------------------------------------------------

#[test]
fn default_format_flag_wins_over_everything() {
    let root = bare_dir();
    write_repo_config(&root, "default_format = \"json\"\n");
    let config_dir = write_global_config("default_format = \"json\"\n");

    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env("RDM_FORMAT", "json")
        .arg("--root")
        .arg(root.path())
        .args(["info", "--format", "markdown"])
        .assert()
        .success()
        .stdout(predicate::str::contains("| default_format | markdown |"));
}

#[test]
fn default_format_env_wins_over_repo_and_global() {
    let root = bare_dir();
    write_repo_config(&root, "default_format = \"json\"\n");
    let config_dir = write_global_config("default_format = \"json\"\n");

    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .env("RDM_FORMAT", "markdown")
        .arg("--root")
        .arg(root.path())
        .arg("info")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "| default_format | markdown | environment variable |",
        ));
}

#[test]
fn default_format_repo_wins_over_global() {
    let root = bare_dir();
    write_repo_config(&root, "default_format = \"markdown\"\n");
    let config_dir = write_global_config("default_format = \"json\"\n");

    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .arg("--root")
        .arg(root.path())
        .arg("info")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "| default_format | markdown | repo config |",
        ));
}

#[test]
fn default_format_global_used_when_repo_unset() {
    let root = bare_dir();
    let config_dir = write_global_config("default_format = \"markdown\"\n");

    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .arg("--root")
        .arg(root.path())
        .arg("info")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "| default_format | markdown | global config |",
        ));
}

#[test]
fn default_format_defaults_to_human() {
    let root = bare_dir();

    rdm()
        .arg("--root")
        .arg(root.path())
        .arg("info")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "default_format: human  (source: default)",
        ));
}

// ---------------------------------------------------------------------------
// root source disambiguation — clap merges --root and RDM_ROOT into one
// value, so `info` needs ArgMatches::value_source to tell them apart.
// ---------------------------------------------------------------------------

#[test]
fn root_flag_and_env_both_set_flag_wins_and_is_reported() {
    let flag_root = bare_dir();
    let env_root = bare_dir();

    rdm()
        .env("RDM_ROOT", env_root.path())
        .arg("--root")
        .arg(flag_root.path())
        .args(["info", "--format", "json"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            flag_root.path().display().to_string(),
        ))
        .stdout(predicate::str::contains(env_root.path().display().to_string()).not());

    rdm()
        .env("RDM_ROOT", env_root.path())
        .arg("--root")
        .arg(flag_root.path())
        .arg("info")
        .assert()
        .success()
        .stdout(predicate::str::contains("(source: CLI flag)"));
}

#[test]
fn root_source_is_environment_variable_when_only_env_set() {
    let root = bare_dir();

    rdm()
        .env("RDM_ROOT", root.path())
        .arg("info")
        .assert()
        .success()
        .stdout(predicate::str::contains("(source: environment variable)"));
}

#[test]
fn root_source_is_global_config_when_neither_flag_nor_env_set() {
    let root = bare_dir();
    let config_dir = write_global_config(&format!("root = \"{}\"\n", root.path().display()));

    rdm()
        .env("XDG_CONFIG_HOME", config_dir.path())
        .arg("info")
        .assert()
        .success()
        .stdout(predicate::str::contains("(source: global config)"))
        .stdout(predicate::str::contains(root.path().display().to_string()));
}

#[test]
fn root_source_is_default_when_nothing_resolves() {
    // No --root, no RDM_ROOT (removed by `rdm()`), and no `root` in global
    // config (XDG_CONFIG_HOME already points to a nonexistent dir) — root
    // must fall through to rdm-core's XDG-data-dir default, and report it
    // as such rather than mislabeling it Global or leaving it untested.
    let data_dir = TempDir::new().unwrap();

    rdm()
        .env("XDG_DATA_HOME", data_dir.path())
        .arg("info")
        .assert()
        .success()
        .stdout(predicate::str::contains("(source: default)"))
        .stdout(predicate::str::contains(
            data_dir.path().join("rdm").display().to_string(),
        ));
}

// ---------------------------------------------------------------------------
// Works before `rdm init` — like `describe`/`model`, `info` is exempt from
// the "no plan repo found" guard.
// ---------------------------------------------------------------------------

#[test]
fn info_works_before_init() {
    let root = bare_dir();
    // No rdm.toml written at all.

    rdm()
        .arg("--root")
        .arg(root.path())
        .arg("info")
        .assert()
        .success()
        .stdout(predicate::str::contains("no plan repo found").not());
}

// ---------------------------------------------------------------------------
// Edge case: everything empty resolves to the built-in defaults, not a
// misleading source label.
// ---------------------------------------------------------------------------

#[test]
fn defaults_report_default_source_with_no_config_at_all() {
    let root = bare_dir();

    rdm()
        .arg("--root")
        .arg(root.path())
        .arg("info")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "default_branch: main  (source: default)",
        ))
        .stdout(predicate::str::contains(
            "default_format: human  (source: default)",
        ));
}

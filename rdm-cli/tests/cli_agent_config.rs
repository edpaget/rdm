use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn rdm() -> Command {
    Command::cargo_bin("rdm").unwrap()
}

#[test]
fn agent_config_defaults_to_agents_md() {
    rdm()
        .arg("agent-config")
        .assert()
        .success()
        .stdout(predicate::str::contains("# rdm"))
        .stdout(predicate::str::contains("## Discovering work"))
        .stdout(predicate::str::contains("--project <PROJECT>"));
}

#[test]
fn agent_config_claude_platform() {
    rdm()
        .arg("agent-config")
        .arg("claude")
        .assert()
        .success()
        .stdout(predicate::str::contains("# rdm"))
        .stdout(predicate::str::is_match("^[^-]").unwrap()); // does not start with ---
}

#[test]
fn agent_config_cursor_has_mdc_frontmatter() {
    rdm()
        .arg("agent-config")
        .arg("cursor")
        .assert()
        .success()
        .stdout(predicate::str::starts_with("---\n"))
        .stdout(predicate::str::contains("description:"))
        .stdout(predicate::str::contains("# rdm"));
}

#[test]
fn agent_config_copilot_platform() {
    rdm()
        .arg("agent-config")
        .arg("copilot")
        .assert()
        .success()
        .stdout(predicate::str::contains("# rdm"));
}

#[test]
fn agent_config_with_project() {
    rdm()
        .arg("agent-config")
        .arg("agents-md")
        .arg("--project")
        .arg("myproj")
        .assert()
        .success()
        .stdout(predicate::str::contains("--project myproj"))
        .stdout(predicate::str::contains("<PROJECT>").not());
}

#[test]
fn agent_config_invalid_platform() {
    rdm()
        .arg("agent-config")
        .arg("vim")
        .assert()
        .failure()
        .stderr(predicate::str::contains("unknown platform"));
}

#[test]
fn agent_config_out_writes_file() {
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("agent-config")
        .arg("claude")
        .arg("--out")
        .arg(dir.path())
        .assert()
        .success();

    let path = dir.path().join("CLAUDE.md");
    assert!(path.exists());
    let content = std::fs::read_to_string(path).unwrap();
    assert!(content.contains("# rdm"));
}

#[test]
fn agent_config_out_cursor_creates_nested_dirs() {
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("agent-config")
        .arg("cursor")
        .arg("--out")
        .arg(dir.path())
        .assert()
        .success();

    let path = dir.path().join(".cursor/rules/rdm.mdc");
    assert!(path.exists());
    let content = std::fs::read_to_string(path).unwrap();
    assert!(content.starts_with("---\n"));
}

#[test]
fn agent_config_does_not_require_plan_repo() {
    // agent-config should work without --root or RDM_ROOT
    // (it doesn't need a plan repo)
    let dir = TempDir::new().unwrap();
    rdm()
        .current_dir(dir.path())
        .arg("agent-config")
        .assert()
        .success();
}

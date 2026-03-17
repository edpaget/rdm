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

#[test]
fn agent_config_contains_planning_workflow() {
    rdm()
        .arg("agent-config")
        .assert()
        .success()
        .stdout(predicate::str::contains("## Planning workflow"))
        .stdout(predicate::str::contains("Before starting work"))
        .stdout(predicate::str::contains("Implementing a roadmap phase"))
        .stdout(predicate::str::contains("Discovering bugs"))
        .stdout(predicate::str::contains("rdm promote"));
}

#[test]
fn agent_config_contains_status_transitions() {
    rdm()
        .arg("agent-config")
        .assert()
        .success()
        .stdout(predicate::str::contains("## Status transitions"))
        .stdout(predicate::str::contains("### Phase statuses"))
        .stdout(predicate::str::contains("### Task statuses"))
        .stdout(predicate::str::contains("`not-started` → `in-progress`"))
        .stdout(predicate::str::contains("`open` → `wont-fix`"));
}

#[test]
fn agent_config_principles_file_included() {
    rdm()
        .arg("agent-config")
        .arg("--principles-file")
        .arg("docs/principles.md")
        .assert()
        .success()
        .stdout(predicate::str::contains("## Principles"))
        .stdout(predicate::str::contains("docs/principles.md"));
}

#[test]
fn agent_config_no_principles_without_flag() {
    rdm()
        .arg("agent-config")
        .assert()
        .success()
        .stdout(predicate::str::contains("## Principles").not());
}

#[test]
fn agent_config_principles_with_project_and_out() {
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("agent-config")
        .arg("claude")
        .arg("--project")
        .arg("myproj")
        .arg("--principles-file")
        .arg("PRINCIPLES.md")
        .arg("--out")
        .arg(dir.path())
        .assert()
        .success();

    let path = dir.path().join("CLAUDE.md");
    let content = std::fs::read_to_string(path).unwrap();
    assert!(content.contains("--project myproj"));
    assert!(content.contains("## Principles"));
    assert!(content.contains("PRINCIPLES.md"));
}

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
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
fn agent_config_skills_requires_claude_platform() {
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("agent-config")
        .arg("agents-md")
        .arg("--skills")
        .arg("--out")
        .arg(dir.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "--skills is only supported for the claude platform",
        ));
}

#[test]
fn agent_config_skills_requires_out() {
    rdm()
        .arg("agent-config")
        .arg("claude")
        .arg("--skills")
        .assert()
        .failure()
        .stderr(predicate::str::contains("--skills requires --out"));
}

#[test]
fn agent_config_skills_generates_four_files() {
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("agent-config")
        .arg("claude")
        .arg("--skills")
        .arg("--out")
        .arg(dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Wrote").count(4));

    assert!(dir.path().join("rdm-roadmap/SKILL.md").exists());
    assert!(dir.path().join("rdm-implement/SKILL.md").exists());
    assert!(dir.path().join("rdm-tasks/SKILL.md").exists());
    assert!(dir.path().join("rdm-review/SKILL.md").exists());
}

#[test]
fn agent_config_skills_have_valid_frontmatter() {
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("agent-config")
        .arg("claude")
        .arg("--skills")
        .arg("--out")
        .arg(dir.path())
        .assert()
        .success();

    for name in &["rdm-roadmap", "rdm-implement", "rdm-tasks"] {
        let path = dir.path().join(format!("{name}/SKILL.md"));
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(
            content.starts_with("---\n"),
            "{name} missing frontmatter start"
        );
        assert!(content.contains("name:"), "{name} missing name field");
        assert!(
            content.contains("allowed-tools:"),
            "{name} missing allowed-tools"
        );
    }
}

#[test]
fn agent_config_skills_embed_project_flag() {
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("agent-config")
        .arg("claude")
        .arg("--skills")
        .arg("--project")
        .arg("testproj")
        .arg("--out")
        .arg(dir.path())
        .assert()
        .success();

    for name in &["rdm-roadmap", "rdm-implement", "rdm-tasks"] {
        let path = dir.path().join(format!("{name}/SKILL.md"));
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(
            content.contains("--project testproj"),
            "{name} missing project flag"
        );
    }
}

#[test]
fn agent_config_skills_include_principles() {
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("agent-config")
        .arg("claude")
        .arg("--skills")
        .arg("--principles-file")
        .arg("docs/principles.md")
        .arg("--out")
        .arg(dir.path())
        .assert()
        .success();

    let content = std::fs::read_to_string(dir.path().join("rdm-implement/SKILL.md")).unwrap();
    assert!(content.contains("## Principles"));
    assert!(content.contains("docs/principles.md"));
}

#[test]
fn agent_config_skills_does_not_require_plan_repo() {
    let dir = TempDir::new().unwrap();
    let out = TempDir::new().unwrap();
    rdm()
        .current_dir(dir.path())
        .arg("agent-config")
        .arg("claude")
        .arg("--skills")
        .arg("--out")
        .arg(out.path())
        .assert()
        .success();
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

#[test]
fn agent_config_mcp_stdout() {
    let output = rdm()
        .arg("agent-config")
        .arg("--mcp")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let parsed: Value = serde_json::from_slice(&output).expect("should be valid JSON");
    assert!(parsed["mcpServers"]["rdm"]["command"].as_str().is_some());
    assert!(parsed["mcpServers"]["rdm"]["args"].is_array());
}

#[test]
fn agent_config_mcp_with_root() {
    let dir = TempDir::new().unwrap();
    let output = rdm()
        .arg("--root")
        .arg(dir.path())
        .arg("agent-config")
        .arg("--mcp")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let parsed: Value = serde_json::from_slice(&output).expect("should be valid JSON");
    let args = parsed["mcpServers"]["rdm"]["args"]
        .as_array()
        .expect("args should be array");
    assert!(args.contains(&Value::String("--root".to_string())));
    assert!(args.contains(&Value::String("mcp".to_string())));
    // The root path should be in the args
    let root_str = dir.path().to_string_lossy().to_string();
    assert!(args.contains(&Value::String(root_str)));
}

#[test]
fn agent_config_mcp_out_writes_file() {
    let out = TempDir::new().unwrap();
    rdm()
        .arg("agent-config")
        .arg("--mcp")
        .arg("--out")
        .arg(out.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Wrote"));

    let path = out.path().join(".mcp.json");
    assert!(path.exists());
    let content = std::fs::read_to_string(path).unwrap();
    let parsed: Value = serde_json::from_str(&content).expect("should be valid JSON");
    assert!(parsed["mcpServers"]["rdm"]["command"].as_str().is_some());
}

#[test]
fn agent_config_mcp_no_plan_repo_needed() {
    let dir = TempDir::new().unwrap();
    rdm()
        .current_dir(dir.path())
        .arg("agent-config")
        .arg("--mcp")
        .assert()
        .success();
}

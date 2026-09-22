use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use tempfile::TempDir;

fn rdm() -> Command {
    let mut cmd = Command::cargo_bin("rdm").unwrap();
    // Isolate from host global config (e.g. default_format = "json").
    cmd.env("XDG_CONFIG_HOME", "/dev/null/nonexistent");
    cmd
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
fn codex_project_emits_instructions_and_supported_skills() {
    let dir = TempDir::new().unwrap();
    rdm()
        .args(["agent-config", "codex", "--project", "acme", "--out"])
        .arg(dir.path())
        .assert()
        .success();
    assert!(dir.path().join("AGENTS.md").is_file());
    rdm()
        .args([
            "agent-config",
            "codex",
            "--skills",
            "--project",
            "acme",
            "--out",
        ])
        .arg(dir.path())
        .assert()
        .success()
        .stderr(predicate::str::contains("rdm-review"))
        .stderr(predicate::str::contains("needs-review"));
    let root = dir.path().join(".agents/skills");
    let mut names: Vec<_> = std::fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, ["rdm-do", "rdm-land", "rdm-revise", "rdm-roadmap"]);
    for name in names {
        let content = std::fs::read_to_string(root.join(name).join("SKILL.md")).unwrap();
        let frontmatter = content.split("---").nth(1).unwrap();
        assert!(
            frontmatter
                .lines()
                .any(|line| line.starts_with("name: rdm-"))
        );
        assert!(
            frontmatter
                .lines()
                .any(|line| line.starts_with("description: "))
        );
        assert!(content.contains("--project acme"));
        assert!(!content.contains("{proj_flag}"));
        assert!(!content.contains(".claude/"));
        assert!(!content.contains("Workflow("));
    }
    assert!(!dir.path().join(".claude").exists());
    assert!(!dir.path().join(".codex/skills").exists());
}

#[test]
fn codex_user_paths_separate_config_and_skills() {
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("custom-codex-home");
    rdm()
        .env("HOME", dir.path())
        .env("CODEX_HOME", &config)
        .args(["agent-config", "codex", "--user"])
        .assert()
        .success();
    assert!(config.join("AGENTS.md").is_file());
    rdm()
        .env("HOME", dir.path())
        .env("CODEX_HOME", &config)
        .args(["agent-config", "codex", "--skills", "--user"])
        .assert()
        .success();
    assert!(dir.path().join(".agents/skills/rdm-do/SKILL.md").is_file());
    assert!(!config.join("skills").exists());
}

#[test]
fn codex_relative_home_is_resolved_by_the_invoking_process() {
    let dir = TempDir::new().unwrap();
    rdm()
        .current_dir(dir.path())
        .env("HOME", dir.path())
        .env("CODEX_HOME", "codex-config")
        .args(["agent-config", "codex", "--user"])
        .assert()
        .success();
    assert!(dir.path().join("codex-config/AGENTS.md").is_file());
    assert!(!dir.path().join(".codex/AGENTS.md").exists());
}

#[test]
fn codex_default_user_instruction_path() {
    let dir = TempDir::new().unwrap();
    rdm()
        .env("HOME", dir.path())
        .env_remove("CODEX_HOME")
        .args(["agent-config", "codex", "--user"])
        .assert()
        .success();
    assert!(dir.path().join(".codex/AGENTS.md").is_file());
    assert!(!dir.path().join(".claude").exists());
}

#[test]
fn codex_plugin_rejection_names_the_supported_channel() {
    let dir = TempDir::new().unwrap();
    rdm()
        .args(["agent-config", "codex", "--plugin", "--out"])
        .arg(dir.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("Codex"))
        .stderr(predicate::str::contains("--skills --out"));
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
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
fn agent_config_cli_demonstrates_tags() {
    // The CLI instructions template should teach agents to set tags on
    // create/update and to filter list/search by tag for tasks, roadmaps,
    // and phases.
    rdm()
        .arg("agent-config")
        .assert()
        .success()
        // Tags on the create commands.
        .stdout(predicate::str::contains(
            "rdm roadmap create <slug> --title \"Title\" --body \"Summary.\" --tags",
        ))
        .stdout(predicate::str::contains(
            "rdm phase create <slug> --title \"Title\" --number <n>",
        ))
        .stdout(predicate::str::contains(
            "rdm task create <slug> --title \"Title\" --body \"Description.\" --tags",
        ))
        // Tag filter on listing tasks.
        .stdout(
            predicate::str::contains("rdm task list").and(predicate::str::contains("--tag bug")),
        )
        // Tag filter on search (empty query + tag, plus AND across tags).
        .stdout(predicate::str::contains("rdm search \"\" --tag bug"))
        .stdout(predicate::str::contains(
            "rdm search auth --tag bug --tag ui",
        ))
        // Tagging convention guidance present.
        .stdout(predicate::str::contains("Tagging convention"))
        .stdout(predicate::str::contains("kebab-case"));
}

/// The seven suggested default tags, each paired with the leading fragment of
/// its gloss. Assertions anchor to tag+gloss pairs rather than bare tag names —
/// bare `bug`/`cli`/`server` already occur many times in both templates, so a
/// bare `contains` would pass vacuously.
const DEFAULT_TAG_GLOSSES: &[&str] = &[
    "`bug` (defect",
    "`enhancement` (new capability",
    "`cli` (rdm-cli",
    "`core` (rdm-core",
    "`server` (HTTP",
    "`web-ui` (browser",
    "`docs` (documentation",
];

#[test]
fn agent_config_cli_instructs_tagging_on_create() {
    rdm()
        .arg("agent-config")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "**Always pass `--tags` when you create**",
        ))
        .stdout(predicate::str::contains(
            "untagged items are invisible to tag-filtered queries",
        ))
        .stdout(predicate::str::contains("replaces the existing list"));
}

#[test]
fn agent_config_cli_suggests_default_tags() {
    let out = rdm().arg("agent-config").assert().success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    for pair in DEFAULT_TAG_GLOSSES {
        assert!(
            stdout.contains(pair),
            "CLI agent-config output is missing the suggested tag gloss {pair}"
        );
    }
    assert!(stdout.contains("not a closed set"));
    assert!(stdout.contains("--tags bug,cli"));
}

#[test]
fn agent_config_cli_teaches_tag_list_discovery() {
    rdm()
        .arg("agent-config")
        .assert()
        .success()
        .stdout(predicate::str::contains("rdm tag list --project <PROJECT>"))
        .stdout(predicate::str::contains(
            "Tags are compared verbatim — `CLI` and `cli` are different tags.",
        ))
        // The superseded discovery phrasing is gone from this template.
        .stdout(predicate::str::contains("rdm search \"\" --tag <candidate>").not());
}

#[test]
fn agent_config_tag_list_respects_project_flag() {
    rdm()
        .arg("agent-config")
        .arg("--project")
        .arg("myproj")
        .assert()
        .success()
        .stdout(predicate::str::contains("rdm tag list --project myproj"));
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
fn agent_config_skills_rejects_unsupported_platform() {
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
            "--skills is only supported for the claude, codex and pi platforms",
        ));
}

#[test]
fn agent_config_skills_rejects_cursor() {
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("agent-config")
        .arg("cursor")
        .arg("--skills")
        .arg("--out")
        .arg(dir.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("claude, codex and pi"));
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
fn agent_config_skills_generates_ten_files() {
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("agent-config")
        .arg("claude")
        .arg("--skills")
        .arg("--out")
        .arg(dir.path())
        .assert()
        .success()
        // 11 skill files + 5 workflow files + 1 agent definition
        // ("rdm-mechanical.md") emitted for Claude + --out. The workflow half
        // grew from 1 to 5 when agent-orchestrated-dispatch phase 26 shipped
        // the four formerly local-only engines — see
        // agent_config_workflows_written_under_out,
        // agent_config_workflows_are_byte_identical_to_source, and
        // agent_config_agents_written_under_out below.
        .stdout(predicate::str::contains("Wrote").count(17));

    let skills_dir = dir.path().join(".claude/skills");
    assert!(skills_dir.join("rdm-roadmap/SKILL.md").exists());
    assert!(skills_dir.join("rdm-do/SKILL.md").exists());
    assert!(skills_dir.join("rdm-review/SKILL.md").exists());
    assert!(skills_dir.join("rdm-document/SKILL.md").exists());
    assert!(skills_dir.join("rdm-estimate/SKILL.md").exists());
    assert!(skills_dir.join("rdm-dispatch-phase/SKILL.md").exists());
    assert!(skills_dir.join("rdm-autopilot/SKILL.md").exists());
    assert!(skills_dir.join("rdm-land/SKILL.md").exists());
    assert!(skills_dir.join("rdm-revise/SKILL.md").exists());
    assert!(skills_dir.join("rdm-plan-review/SKILL.md").exists());
    assert!(skills_dir.join("rdm-backlog/SKILL.md").exists());
}

#[test]
fn agent_config_workflows_written_under_out() {
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("agent-config")
        .arg("claude")
        .arg("--skills")
        .arg("--out")
        .arg(dir.path())
        .assert()
        .success();

    let workflows_dir = dir.path().join(".claude/workflows");
    assert!(workflows_dir.join("rdm-wf-review-refute-fix.js").exists());
    assert!(workflows_dir.join("rdm-wf-plan-review.js").exists());
    assert!(workflows_dir.join("rdm-wf-estimate.js").exists());
    assert!(workflows_dir.join("rdm-wf-backlog.js").exists());
    assert!(workflows_dir.join("rdm-wf-document.js").exists());
    // The retired dispatch engine is emitted by nothing.
    assert!(!workflows_dir.join("rdm-wf-dispatch-phase.js").exists());
}

#[test]
fn agent_config_workflows_are_byte_identical_to_source() {
    // Intentionally a live comparison against the checked-in
    // `.claude/workflows/*.js` files (not a hardcoded fixture), so a future
    // hand-edit to either side that isn't mirrored to the other fails CI
    // immediately.
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("agent-config")
        .arg("claude")
        .arg("--skills")
        .arg("--out")
        .arg(dir.path())
        .assert()
        .success();

    // rdm-cli's manifest dir's parent is the repo root.
    let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    // Every emitted engine, discovered from the emission rather than from a
    // hardcoded name list, with a floor so the loop cannot pass vacuously.
    let mut checked = 0usize;
    for entry in std::fs::read_dir(dir.path().join(".claude/workflows")).unwrap() {
        let name = entry.unwrap().file_name();
        let emitted = std::fs::read(dir.path().join(".claude/workflows").join(&name)).unwrap();
        let source = std::fs::read(repo_root.join(".claude/workflows").join(&name)).unwrap();
        assert_eq!(
            emitted,
            source,
            "{} drifted from the emitted template",
            name.to_string_lossy()
        );
        checked += 1;
    }
    assert!(checked >= 1, "no emitted workflow script was compared");
}

#[test]
fn agent_config_agents_written_under_out() {
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("agent-config")
        .arg("claude")
        .arg("--skills")
        .arg("--out")
        .arg(dir.path())
        .assert()
        .success();

    let agents_dir = dir.path().join(".claude/agents");
    assert!(agents_dir.join("rdm-mechanical.md").exists());
}

#[test]
fn agent_config_agents_are_byte_identical_to_source() {
    // Intentionally a live comparison against the checked-in
    // `.claude/agents/*.md` files (not a hardcoded fixture), so a future
    // hand-edit to either side that isn't mirrored to the other fails CI
    // immediately.
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("agent-config")
        .arg("claude")
        .arg("--skills")
        .arg("--out")
        .arg(dir.path())
        .assert()
        .success();

    // rdm-cli's manifest dir's parent is the repo root.
    let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    for agent in rdm_core::agent_config::generate_agents() {
        let name = agent.relative_path;
        let emitted = std::fs::read(dir.path().join(".claude/agents").join(name)).unwrap();
        let source = std::fs::read(repo_root.join(".claude/agents").join(name)).unwrap();
        assert_eq!(emitted, source, "{name} drifted from the emitted template");
    }
}

#[test]
fn agent_config_pi_skills_does_not_write_workflows() {
    // Pi has no Workflow-tool runtime, so --skills against Pi must not emit
    // a workflows directory at all. The same gate covers .claude/agents/,
    // which exists only to resolve agentType references from the
    // Workflow-tool scripts.
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("agent-config")
        .arg("pi")
        .arg("--skills")
        .arg("--out")
        .arg(dir.path())
        .assert()
        .success();

    assert!(!dir.path().join(".claude/workflows").exists());
    assert!(!dir.path().join(".pi/workflows").exists());
    assert!(!dir.path().join(".claude/agents").exists());
    assert!(!dir.path().join(".pi/agents").exists());
}

#[test]
fn agent_config_user_skills_does_not_write_workflows() {
    // Workflows are project/repo-scoped (they hardcode a target repo's own
    // ./target/debug/rdm and --project rdm), so --user must not emit them
    // to a user-global location even for Claude. The same gate covers
    // .claude/agents/.
    let home = TempDir::new().unwrap();
    rdm()
        .env("HOME", home.path())
        .arg("agent-config")
        .arg("claude")
        .arg("--skills")
        .arg("--user")
        .assert()
        .success();

    assert!(!home.path().join(".claude/workflows").exists());
    assert!(!home.path().join(".claude/agents").exists());
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

    for name in &[
        "rdm-roadmap",
        "rdm-do",
        "rdm-review",
        "rdm-document",
        "rdm-estimate",
        "rdm-dispatch-phase",
        "rdm-autopilot",
        "rdm-land",
        "rdm-plan-review",
    ] {
        let path = dir.path().join(format!(".claude/skills/{name}/SKILL.md"));
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

    for name in &[
        "rdm-roadmap",
        "rdm-do",
        "rdm-review",
        "rdm-document",
        "rdm-estimate",
        "rdm-dispatch-phase",
        "rdm-autopilot",
        "rdm-land",
        "rdm-plan-review",
    ] {
        let path = dir.path().join(format!(".claude/skills/{name}/SKILL.md"));
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

    let content =
        std::fs::read_to_string(dir.path().join(".claude/skills/rdm-do/SKILL.md")).unwrap();
    assert!(content.contains("## Principles"));
    assert!(content.contains("docs/principles.md"));
}

#[test]
fn agent_config_do_skill_routes_plan_approval_through_a_plan_review() {
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("agent-config")
        .arg("claude")
        .arg("--skills")
        .arg("--out")
        .arg(dir.path())
        .assert()
        .success();

    // The emitted shim must not drop the plan gate. Since the prose orchestrator
    // landed, the plan under review is a persisted `plan/<slug>` document that an
    // approve review releases — not the ephemeral `--implementation-plan` target
    // the old inline flow reviewed.
    let content =
        std::fs::read_to_string(dir.path().join(".claude/skills/rdm-do/SKILL.md")).unwrap();
    assert!(content.contains("review start --on plan/"));
    assert!(content.contains("--verdict approve"));
}

#[test]
fn agent_config_roadmap_skill_notes_plan_review() {
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("agent-config")
        .arg("claude")
        .arg("--skills")
        .arg("--out")
        .arg(dir.path())
        .assert()
        .success();

    let content =
        std::fs::read_to_string(dir.path().join(".claude/skills/rdm-roadmap/SKILL.md")).unwrap();
    assert!(content.contains("needs-plan-review"));
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

// --user flag tests

#[test]
fn agent_config_user_writes_to_claude_dir() {
    let home = TempDir::new().unwrap();
    rdm()
        .env("HOME", home.path())
        .arg("agent-config")
        .arg("claude")
        .arg("--user")
        .assert()
        .success()
        .stdout(predicate::str::contains("Wrote"));

    let path = home.path().join(".claude/CLAUDE.md");
    assert!(path.exists(), "expected {}", path.display());
    let content = std::fs::read_to_string(path).unwrap();
    assert!(content.contains("# rdm"));
}

#[test]
fn agent_config_user_agents_md_writes_to_claude_dir() {
    let home = TempDir::new().unwrap();
    rdm()
        .env("HOME", home.path())
        .arg("agent-config")
        .arg("agents-md")
        .arg("--user")
        .assert()
        .success();

    let path = home.path().join(".claude/AGENTS.md");
    assert!(path.exists(), "expected {}", path.display());
}

#[test]
fn agent_config_user_cursor_writes_to_cursor_rules() {
    let home = TempDir::new().unwrap();
    rdm()
        .env("HOME", home.path())
        .arg("agent-config")
        .arg("cursor")
        .arg("--user")
        .assert()
        .success();

    // conventional_path for cursor is ".cursor/rules/rdm.mdc", base is ~/
    let path = home.path().join(".cursor/rules/rdm.mdc");
    assert!(path.exists(), "expected {}", path.display());
    let content = std::fs::read_to_string(path).unwrap();
    assert!(content.starts_with("---\n"));
}

#[test]
fn agent_config_user_copilot_writes_to_github_dir() {
    let home = TempDir::new().unwrap();
    rdm()
        .env("HOME", home.path())
        .arg("agent-config")
        .arg("copilot")
        .arg("--user")
        .assert()
        .success();

    let path = home.path().join(".github/copilot-instructions.md");
    assert!(path.exists(), "expected {}", path.display());
}

#[test]
fn agent_config_user_and_out_conflict() {
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("agent-config")
        .arg("claude")
        .arg("--user")
        .arg("--out")
        .arg(dir.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot be used with"));
}

#[test]
fn agent_config_user_skills_writes_to_claude_skills() {
    let home = TempDir::new().unwrap();
    rdm()
        .env("HOME", home.path())
        .arg("agent-config")
        .arg("claude")
        .arg("--skills")
        .arg("--user")
        .assert()
        .success()
        .stdout(predicate::str::contains("Wrote").count(11));

    let skills_dir = home.path().join(".claude/skills");
    assert!(skills_dir.join("rdm-roadmap/SKILL.md").exists());
    assert!(skills_dir.join("rdm-do/SKILL.md").exists());
    assert!(skills_dir.join("rdm-review/SKILL.md").exists());
    assert!(skills_dir.join("rdm-document/SKILL.md").exists());
    assert!(skills_dir.join("rdm-estimate/SKILL.md").exists());
    assert!(skills_dir.join("rdm-dispatch-phase/SKILL.md").exists());
    assert!(skills_dir.join("rdm-autopilot/SKILL.md").exists());
    assert!(skills_dir.join("rdm-land/SKILL.md").exists());
    assert!(skills_dir.join("rdm-revise/SKILL.md").exists());
    assert!(skills_dir.join("rdm-plan-review/SKILL.md").exists());
    assert!(skills_dir.join("rdm-backlog/SKILL.md").exists());
}

#[test]
fn agent_config_hooks_flag_is_unrecognized() {
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("agent-config")
        .arg("claude")
        .arg("--skills")
        .arg("--hooks")
        .arg("--out")
        .arg(dir.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("unexpected argument"))
        .stderr(predicate::str::contains("--hooks"));
}

#[test]
fn agent_config_pi_platform() {
    rdm()
        .arg("agent-config")
        .arg("pi")
        .assert()
        .success()
        .stdout(predicate::str::contains("# rdm"))
        .stdout(predicate::str::contains("--project <PROJECT>"));
}

#[test]
fn agent_config_pi_out_writes_nested_file() {
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("agent-config")
        .arg("pi")
        .arg("--out")
        .arg(dir.path())
        .assert()
        .success();

    let path = dir.path().join(".pi/AGENTS.md");
    assert!(path.exists(), "expected {}", path.display());
    let content = std::fs::read_to_string(path).unwrap();
    assert!(content.contains("# rdm"));
}

#[test]
fn agent_config_pi_with_project_substitutes() {
    rdm()
        .arg("agent-config")
        .arg("pi")
        .arg("--project")
        .arg("myproj")
        .assert()
        .success()
        .stdout(predicate::str::contains("--project myproj"))
        .stdout(predicate::str::contains("<PROJECT>").not());
}

#[test]
fn agent_config_pi_in_help_text() {
    rdm()
        .arg("agent-config")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("pi"));
}

#[test]
fn agent_config_pi_user_writes_to_dot_pi_agent() {
    let home = TempDir::new().unwrap();
    rdm()
        .env("HOME", home.path())
        .arg("agent-config")
        .arg("pi")
        .arg("--user")
        .assert()
        .success()
        .stdout(predicate::str::contains("Wrote"));

    let path = home.path().join(".pi/agent/AGENTS.md");
    assert!(path.exists(), "expected {}", path.display());
    let content = std::fs::read_to_string(path).unwrap();
    assert!(content.contains("# rdm"));
}

#[test]
fn agent_config_pi_skills_out_writes_ten_files() {
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("agent-config")
        .arg("pi")
        .arg("--skills")
        .arg("--out")
        .arg(dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Wrote").count(11));

    let skills_dir = dir.path().join(".pi/skills");
    assert!(skills_dir.join("rdm-roadmap/SKILL.md").exists());
    assert!(skills_dir.join("rdm-do/SKILL.md").exists());
    assert!(skills_dir.join("rdm-review/SKILL.md").exists());
    assert!(skills_dir.join("rdm-document/SKILL.md").exists());
    assert!(skills_dir.join("rdm-estimate/SKILL.md").exists());
    assert!(skills_dir.join("rdm-dispatch-phase/SKILL.md").exists());
    assert!(skills_dir.join("rdm-autopilot/SKILL.md").exists());
    assert!(skills_dir.join("rdm-land/SKILL.md").exists());
    assert!(skills_dir.join("rdm-revise/SKILL.md").exists());
    assert!(skills_dir.join("rdm-plan-review/SKILL.md").exists());
    assert!(skills_dir.join("rdm-backlog/SKILL.md").exists());
}

#[test]
fn agent_config_pi_skills_have_valid_frontmatter() {
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("agent-config")
        .arg("pi")
        .arg("--skills")
        .arg("--out")
        .arg(dir.path())
        .assert()
        .success();

    for name in &[
        "rdm-roadmap",
        "rdm-do",
        "rdm-review",
        "rdm-document",
        "rdm-estimate",
        "rdm-dispatch-phase",
        "rdm-autopilot",
        "rdm-land",
        "rdm-plan-review",
    ] {
        let path = dir.path().join(format!(".pi/skills/{name}/SKILL.md"));
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
fn agent_config_pi_skills_embed_project_flag() {
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("agent-config")
        .arg("pi")
        .arg("--skills")
        .arg("--project")
        .arg("myproj")
        .arg("--out")
        .arg(dir.path())
        .assert()
        .success();

    for name in &[
        "rdm-roadmap",
        "rdm-do",
        "rdm-review",
        "rdm-document",
        "rdm-estimate",
        "rdm-dispatch-phase",
        "rdm-autopilot",
        "rdm-land",
        "rdm-plan-review",
    ] {
        let path = dir.path().join(format!(".pi/skills/{name}/SKILL.md"));
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(
            content.contains("--project myproj"),
            "{name} missing project flag"
        );
    }
}

#[test]
fn agent_config_pi_skills_user_writes_to_pi_agent_skills() {
    let home = TempDir::new().unwrap();
    rdm()
        .env("HOME", home.path())
        .arg("agent-config")
        .arg("pi")
        .arg("--skills")
        .arg("--user")
        .assert()
        .success()
        .stdout(predicate::str::contains("Wrote").count(11));

    let skills_dir = home.path().join(".pi/agent/skills");
    assert!(skills_dir.join("rdm-roadmap/SKILL.md").exists());
    assert!(skills_dir.join("rdm-do/SKILL.md").exists());
    assert!(skills_dir.join("rdm-review/SKILL.md").exists());
    assert!(skills_dir.join("rdm-document/SKILL.md").exists());
    assert!(skills_dir.join("rdm-estimate/SKILL.md").exists());
    assert!(skills_dir.join("rdm-dispatch-phase/SKILL.md").exists());
    assert!(skills_dir.join("rdm-autopilot/SKILL.md").exists());
    assert!(skills_dir.join("rdm-land/SKILL.md").exists());
    assert!(skills_dir.join("rdm-revise/SKILL.md").exists());
    assert!(skills_dir.join("rdm-plan-review/SKILL.md").exists());
    assert!(skills_dir.join("rdm-backlog/SKILL.md").exists());
}

// --- --plugin: the new emission mode this phase adds ------------------------

#[test]
fn agent_config_plugin_writes_manifest_skills_and_workflows() {
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("agent-config")
        .arg("claude")
        .arg("--plugin")
        .arg("--out")
        .arg(dir.path())
        .arg("--project")
        .arg("distro-check")
        .assert()
        .success()
        .stdout(predicate::str::contains("Wrote").count(17));

    let manifest_path = dir.path().join(".claude-plugin/plugin.json");
    assert!(
        manifest_path.exists(),
        "expected {}",
        manifest_path.display()
    );
    let manifest: Value =
        serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();
    assert_eq!(manifest["name"], "rdm");
    assert!(manifest["version"].is_string());
    assert!(manifest["workflows"].is_null());

    let skills_dir = dir.path().join("skills");
    for name in [
        "roadmap",
        "do",
        "review",
        "document",
        "estimate",
        "dispatch-phase",
        "autopilot",
        "land",
        "revise",
        "plan-review",
        "backlog",
    ] {
        let path = skills_dir.join(name).join("SKILL.md");
        assert!(path.exists(), "expected {}", path.display());
    }

    let workflows_dir = dir.path().join("workflows");
    assert!(workflows_dir.join("rdm-wf-review-refute-fix.js").exists());
    assert!(workflows_dir.join("rdm-wf-plan-review.js").exists());
    assert!(workflows_dir.join("rdm-wf-estimate.js").exists());
    assert!(workflows_dir.join("rdm-wf-backlog.js").exists());
    assert!(workflows_dir.join("rdm-wf-document.js").exists());
    // The retired dispatch engine is emitted by nothing.
    assert!(!workflows_dir.join("rdm-wf-dispatch-phase.js").exists());

    // Plugin skill directory names never carry the raw `rdm-` prefix.
    assert!(!skills_dir.join("rdm-roadmap").exists());
}

#[test]
fn agent_config_plugin_and_skills_conflict() {
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("agent-config")
        .arg("claude")
        .arg("--plugin")
        .arg("--skills")
        .arg("--out")
        .arg(dir.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot be used with"));
}

#[test]
fn agent_config_plugin_requires_out() {
    rdm()
        .arg("agent-config")
        .arg("claude")
        .arg("--plugin")
        .assert()
        .failure()
        .stderr(predicate::str::contains("--plugin requires --out"));
}

#[test]
fn agent_config_plugin_and_user_rejected_with_distinct_message() {
    let home = TempDir::new().unwrap();
    rdm()
        .env("HOME", home.path())
        .arg("agent-config")
        .arg("claude")
        .arg("--plugin")
        .arg("--user")
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "--plugin cannot be combined with --user",
        ))
        .stderr(predicate::str::contains("claude plugin marketplace add"));
}

#[test]
fn agent_config_plugin_rejected_on_agents_md() {
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("agent-config")
        .arg("agents-md")
        .arg("--plugin")
        .arg("--out")
        .arg(dir.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "--plugin is only supported for the claude platform",
        ));
}

#[test]
fn agent_config_plugin_rejected_on_cursor() {
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("agent-config")
        .arg("cursor")
        .arg("--plugin")
        .arg("--out")
        .arg(dir.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "--plugin is only supported for the claude platform",
        ));
}

#[test]
fn agent_config_plugin_rejected_on_copilot() {
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("agent-config")
        .arg("copilot")
        .arg("--plugin")
        .arg("--out")
        .arg(dir.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "--plugin is only supported for the claude platform",
        ));
}

#[test]
fn agent_config_plugin_rejected_on_pi() {
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("agent-config")
        .arg("pi")
        .arg("--plugin")
        .arg("--out")
        .arg(dir.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "--plugin is only supported for the claude platform",
        ));
}

#[test]
fn agent_config_skills_and_user_still_works_unaffected_by_plugin() {
    // Positive control: --skills --user's pre-existing --out-only exclusivity
    // is unchanged by adding --plugin.
    let home = TempDir::new().unwrap();
    rdm()
        .env("HOME", home.path())
        .arg("agent-config")
        .arg("claude")
        .arg("--skills")
        .arg("--user")
        .assert()
        .success()
        .stdout(predicate::str::contains("Wrote").count(11));

    assert!(!home.path().join(".claude/workflows").exists());
}

// --- --mcp is gone: clap rejects it, no hand-written message survives ------

#[test]
fn agent_config_mcp_flag_is_unknown_argument() {
    let dir = TempDir::new().unwrap();
    rdm()
        .arg("agent-config")
        .arg("claude")
        .arg("--skills")
        .arg("--mcp")
        .arg("--out")
        .arg(dir.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("unexpected argument '--mcp'"))
        .stderr(predicate::str::contains("Pi does not support MCP").not());
}

#[test]
fn agent_config_pi_mcp_is_unknown_argument() {
    // The bespoke Pi rejection was DELETED, not merely shadowed by an
    // earlier check: Pi + --mcp now fails the same way every other unknown
    // flag does.
    rdm()
        .arg("agent-config")
        .arg("pi")
        .arg("--mcp")
        .assert()
        .failure()
        .stderr(predicate::str::contains("unexpected argument '--mcp'"))
        .stderr(predicate::str::contains("Pi does not support MCP").not());
}

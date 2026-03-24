//! Agent configuration generation for AI coding assistants.
//!
//! Generates platform-specific instruction files that teach AI agents
//! how to interact with `rdm` via its CLI.

use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

/// Target platform for agent configuration output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    /// Claude Code (`CLAUDE.md`)
    Claude,
    /// Cross-agent standard (`AGENTS.md`)
    AgentsMd,
    /// Cursor IDE (`.cursor/rules/rdm.mdc`)
    Cursor,
    /// GitHub Copilot (`.github/copilot-instructions.md`)
    Copilot,
}

impl Platform {
    /// Returns the conventional file path for this platform's instruction file.
    pub fn conventional_path(&self) -> &'static str {
        match self {
            Platform::Claude => "CLAUDE.md",
            Platform::AgentsMd => "AGENTS.md",
            Platform::Cursor => ".cursor/rules/rdm.mdc",
            Platform::Copilot => ".github/copilot-instructions.md",
        }
    }

    /// Returns the user-level base directory for this platform.
    ///
    /// This is the directory that plays the same role as `--out` but for
    /// user-global configuration. The instruction file will be written at
    /// `user_level_dir() / conventional_path()`, just as `--out` writes to
    /// `out / conventional_path()`.
    ///
    /// | Platform   | Directory        |
    /// |------------|------------------|
    /// | Claude     | `~/.claude/`     |
    /// | AgentsMd   | `~/.claude/`     |
    /// | Cursor     | `~/`             |
    /// | Copilot    | `~/`             |
    ///
    /// # Errors
    ///
    /// Returns an error if the home directory cannot be determined.
    pub fn user_level_dir(&self) -> Result<PathBuf, String> {
        let home = home_dir()?;
        let dir = match self {
            // conventional_path is "CLAUDE.md" / "AGENTS.md" — flat file, so base is ~/.claude/
            Platform::Claude | Platform::AgentsMd => home.join(".claude"),
            // conventional_path is ".cursor/rules/rdm.mdc" — includes subdirs
            Platform::Cursor => home,
            // conventional_path is ".github/copilot-instructions.md" — includes subdir
            Platform::Copilot => home,
        };
        Ok(dir)
    }

    /// Returns the user-level directory for Claude Code skills (`~/.claude/skills/`).
    ///
    /// Skills are always under `~/.claude/skills/` regardless of platform
    /// (skills are a Claude Code concept), so this is an associated function
    /// rather than a method on `&self`.
    ///
    /// # Errors
    ///
    /// Returns an error if the home directory cannot be determined.
    pub fn user_level_skills_dir() -> Result<PathBuf, String> {
        let home = home_dir()?;
        Ok(home.join(".claude").join("skills"))
    }
}

impl fmt::Display for Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Platform::Claude => write!(f, "claude"),
            Platform::AgentsMd => write!(f, "agents-md"),
            Platform::Cursor => write!(f, "cursor"),
            Platform::Copilot => write!(f, "copilot"),
        }
    }
}

impl FromStr for Platform {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "claude" => Ok(Platform::Claude),
            "agents-md" => Ok(Platform::AgentsMd),
            "cursor" => Ok(Platform::Cursor),
            "copilot" => Ok(Platform::Copilot),
            other => Err(format!(
                "unknown platform '{other}'; expected one of: claude, agents-md, cursor, copilot"
            )),
        }
    }
}

fn home_dir() -> Result<PathBuf, String> {
    std::env::var("HOME")
        .map(PathBuf::from)
        .map_err(|_| "cannot determine home directory (HOME not set)".to_string())
}

/// Options for generating agent configuration.
pub struct AgentConfigOptions {
    /// Target platform.
    pub platform: Platform,
    /// Project name to embed in examples. If `None`, uses `<PROJECT>` placeholder.
    pub project: Option<String>,
    /// Optional path to a principles file to reference in generated output.
    pub principles_file: Option<String>,
    /// When `true`, generate instructions referencing MCP tool calls instead of CLI commands.
    pub mcp: bool,
}

/// Generates agent configuration content for the given options.
///
/// Returns a string containing platform-formatted instructions for
/// interacting with `rdm` via its CLI.
///
/// # Examples
///
/// ```
/// use rdm_core::agent_config::{AgentConfigOptions, Platform, generate_agent_config};
///
/// let content = generate_agent_config(&AgentConfigOptions {
///     platform: Platform::AgentsMd,
///     project: Some("myproj".to_string()),
///     principles_file: None,
///     mcp: false,
/// });
/// assert!(content.contains("--project myproj"));
/// ```
pub fn generate_agent_config(opts: &AgentConfigOptions) -> String {
    let instructions = if opts.mcp {
        agent_instructions_mcp(opts.project.as_deref(), opts.principles_file.as_deref())
    } else {
        agent_instructions(opts.project.as_deref(), opts.principles_file.as_deref())
    };

    match opts.platform {
        Platform::Cursor => {
            format!(
                "---\ndescription: Instructions for using rdm to manage project roadmaps\nglobs:\n---\n\n{instructions}"
            )
        }
        _ => instructions,
    }
}

fn proj_flag_str(project: Option<&str>) -> String {
    match project {
        Some(name) => format!("--project {name}"),
        None => "--project <PROJECT>".to_string(),
    }
}

/// Generates the core instruction content shared across all platforms.
fn agent_instructions(project: Option<&str>, principles_file: Option<&str>) -> String {
    let proj_flag = proj_flag_str(project);
    let principles = principles_file
        .map(|p| format!("\n\n{}", section_principles(p)))
        .unwrap_or_default();
    include_str!("templates/instructions-cli.md")
        .replace("{proj_flag}", &proj_flag)
        .replace("\n{principles}", &principles)
}

/// A generated skill file with its relative path and content.
pub struct SkillFile {
    /// Relative path within the output directory (e.g., "rdm-roadmap/SKILL.md").
    pub relative_path: &'static str,
    /// The full content of the skill file.
    pub content: String,
}

/// Options for generating skill definition files.
pub struct SkillOptions {
    /// Project name to embed in skill CLI invocations.
    pub project: Option<String>,
    /// Optional path to a principles file to reference.
    pub principles_file: Option<String>,
    /// When `true`, generate skills referencing MCP tool calls instead of CLI commands.
    pub mcp: bool,
}

/// Generates Claude Code skill definition files.
///
/// Returns a vector of [`SkillFile`]s, each containing the relative path
/// and content for a skill definition. Skills are reusable agent behaviors
/// triggered by slash commands in Claude Code.
///
/// # Examples
///
/// ```
/// use rdm_core::agent_config::{SkillOptions, generate_skills};
///
/// let skills = generate_skills(&SkillOptions {
///     project: Some("myproj".to_string()),
///     principles_file: None,
///     mcp: false,
/// });
/// assert_eq!(skills.len(), 5);
/// assert!(skills[0].content.contains("--project myproj"));
/// ```
pub fn generate_skills(opts: &SkillOptions) -> Vec<SkillFile> {
    let principles_note = opts.principles_file.as_deref().map(skill_principles_note);
    if opts.mcp {
        let proj = proj_param_str(opts.project.as_deref());
        vec![
            skill_roadmap_mcp(&proj, principles_note.as_deref()),
            skill_implement_mcp(&proj, principles_note.as_deref()),
            skill_tasks_mcp(&proj, principles_note.as_deref()),
            skill_review_mcp(&proj, principles_note.as_deref()),
            skill_document_mcp(&proj, principles_note.as_deref()),
        ]
    } else {
        let proj_flag = proj_flag_str(opts.project.as_deref());
        vec![
            skill_roadmap(&proj_flag, principles_note.as_deref()),
            skill_implement(&proj_flag, principles_note.as_deref()),
            skill_tasks(&proj_flag, principles_note.as_deref()),
            skill_review(&proj_flag, principles_note.as_deref()),
            skill_document(&proj_flag, principles_note.as_deref()),
        ]
    }
}

fn skill_principles_note(path: &str) -> String {
    format!(
        "\n## Principles\n\nRead `{path}` before starting. It contains project conventions that should guide your work."
    )
}

fn render_skill(
    template: &str,
    proj_placeholder: &str,
    proj_value: &str,
    principles_note: Option<&str>,
) -> String {
    let principles = principles_note.unwrap_or("");
    template
        .replace(proj_placeholder, proj_value)
        .replace("{principles}", principles)
}

fn skill_roadmap(proj_flag: &str, principles_note: Option<&str>) -> SkillFile {
    SkillFile {
        relative_path: "rdm-roadmap/SKILL.md",
        content: render_skill(
            include_str!("templates/skill-roadmap-cli.md"),
            "{proj_flag}",
            proj_flag,
            principles_note,
        ),
    }
}

fn skill_implement(proj_flag: &str, principles_note: Option<&str>) -> SkillFile {
    SkillFile {
        relative_path: "rdm-implement/SKILL.md",
        content: render_skill(
            include_str!("templates/skill-implement-cli.md"),
            "{proj_flag}",
            proj_flag,
            principles_note,
        ),
    }
}

fn skill_tasks(proj_flag: &str, principles_note: Option<&str>) -> SkillFile {
    SkillFile {
        relative_path: "rdm-tasks/SKILL.md",
        content: render_skill(
            include_str!("templates/skill-tasks-cli.md"),
            "{proj_flag}",
            proj_flag,
            principles_note,
        ),
    }
}

fn skill_document(proj_flag: &str, principles_note: Option<&str>) -> SkillFile {
    SkillFile {
        relative_path: "rdm-document/SKILL.md",
        content: render_skill(
            include_str!("templates/skill-document-cli.md"),
            "{proj_flag}",
            proj_flag,
            principles_note,
        ),
    }
}

fn skill_review(proj_flag: &str, principles_note: Option<&str>) -> SkillFile {
    SkillFile {
        relative_path: "rdm-review/SKILL.md",
        content: render_skill(
            include_str!("templates/skill-review-cli.md"),
            "{proj_flag}",
            proj_flag,
            principles_note,
        ),
    }
}

/// Options for generating MCP server configuration.
pub struct McpConfigOptions {
    /// Plan repo root path. When `Some`, the generated config includes `--root <path>`.
    pub root: Option<String>,
}

/// Generates a `.mcp.json` configuration for the rdm MCP server.
///
/// The output is a JSON object with an `mcpServers.rdm` entry that tells
/// MCP-aware clients how to launch the rdm MCP server.
///
/// # Examples
///
/// ```
/// use rdm_core::agent_config::{McpConfigOptions, generate_mcp_config};
///
/// let json = generate_mcp_config(&McpConfigOptions { root: None });
/// assert!(json.contains("mcpServers"));
/// ```
pub fn generate_mcp_config(opts: &McpConfigOptions) -> String {
    let args: Vec<serde_json::Value> = match &opts.root {
        Some(root) => vec![
            serde_json::Value::String("--root".to_string()),
            serde_json::Value::String(root.clone()),
            serde_json::Value::String("mcp".to_string()),
        ],
        None => vec![serde_json::Value::String("mcp".to_string())],
    };

    let config = serde_json::json!({
        "mcpServers": {
            "rdm": {
                "command": "rdm",
                "args": args
            }
        }
    });

    serde_json::to_string_pretty(&config).expect("JSON serialization cannot fail")
}

/// Returns a quoted project name for use in MCP tool call examples.
fn proj_param_str(project: Option<&str>) -> String {
    match project {
        Some(name) => format!("\"{name}\""),
        None => "\"<PROJECT>\"".to_string(),
    }
}

/// Generates MCP-oriented instruction content referencing MCP tool calls.
fn agent_instructions_mcp(project: Option<&str>, principles_file: Option<&str>) -> String {
    let proj_param = proj_param_str(project);
    let principles = principles_file
        .map(|p| format!("\n\n{}", section_principles(p)))
        .unwrap_or_default();
    include_str!("templates/instructions-mcp.md")
        .replace("{proj_param}", &proj_param)
        .replace("\n{principles}", &principles)
}

// ---------- MCP skill generators ----------

fn mcp_tool_name(tool: &str) -> String {
    format!("mcp__rdm__{tool}")
}

fn skill_roadmap_mcp(proj: &str, principles_note: Option<&str>) -> SkillFile {
    SkillFile {
        relative_path: "rdm-roadmap/SKILL.md",
        content: render_mcp_skill(
            include_str!("templates/skill-roadmap-mcp.md"),
            proj,
            principles_note,
            &[
                ("t_roadmap_create", "rdm_roadmap_create"),
                ("t_phase_create", "rdm_phase_create"),
                ("t_roadmap_show", "rdm_roadmap_show"),
            ],
        ),
    }
}

fn skill_implement_mcp(proj: &str, principles_note: Option<&str>) -> SkillFile {
    SkillFile {
        relative_path: "rdm-implement/SKILL.md",
        content: render_mcp_skill(
            include_str!("templates/skill-implement-mcp.md"),
            proj,
            principles_note,
            &[
                ("t_phase_list", "rdm_phase_list"),
                ("t_phase_show", "rdm_phase_show"),
                ("t_phase_update", "rdm_phase_update"),
                ("t_task_create", "rdm_task_create"),
            ],
        ),
    }
}

fn skill_tasks_mcp(proj: &str, principles_note: Option<&str>) -> SkillFile {
    SkillFile {
        relative_path: "rdm-tasks/SKILL.md",
        content: render_mcp_skill(
            include_str!("templates/skill-tasks-mcp.md"),
            proj,
            principles_note,
            &[
                ("t_task_list", "rdm_task_list"),
                ("t_task_show", "rdm_task_show"),
                ("t_task_update", "rdm_task_update"),
            ],
        ),
    }
}

fn skill_document_mcp(proj: &str, principles_note: Option<&str>) -> SkillFile {
    SkillFile {
        relative_path: "rdm-document/SKILL.md",
        content: render_mcp_skill(
            include_str!("templates/skill-document-mcp.md"),
            proj,
            principles_note,
            &[
                ("t_roadmap_show", "rdm_roadmap_show"),
                ("t_phase_show", "rdm_phase_show"),
            ],
        ),
    }
}

fn skill_review_mcp(proj: &str, principles_note: Option<&str>) -> SkillFile {
    SkillFile {
        relative_path: "rdm-review/SKILL.md",
        content: render_mcp_skill(
            include_str!("templates/skill-review-mcp.md"),
            proj,
            principles_note,
            &[
                ("t_phase_show", "rdm_phase_show"),
                ("t_task_show", "rdm_task_show"),
                ("t_task_create", "rdm_task_create"),
            ],
        ),
    }
}

fn render_mcp_skill(
    template: &str,
    proj: &str,
    principles_note: Option<&str>,
    tools: &[(&str, &str)],
) -> String {
    let principles = principles_note.unwrap_or("");
    let mut result = template
        .replace("{proj_param}", proj)
        .replace("{principles}", principles);
    for (placeholder, tool) in tools {
        result = result.replace(&format!("{{{placeholder}}}"), &mcp_tool_name(tool));
    }
    result
}

fn section_principles(path: &str) -> String {
    format!(
        r#"## Principles

Read `{path}` before starting implementation work. It contains project conventions and design principles that should guide your decisions."#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_from_str_valid() {
        assert_eq!("claude".parse::<Platform>().unwrap(), Platform::Claude);
        assert_eq!("agents-md".parse::<Platform>().unwrap(), Platform::AgentsMd);
        assert_eq!("cursor".parse::<Platform>().unwrap(), Platform::Cursor);
        assert_eq!("copilot".parse::<Platform>().unwrap(), Platform::Copilot);
    }

    #[test]
    fn platform_from_str_case_insensitive() {
        assert_eq!("Claude".parse::<Platform>().unwrap(), Platform::Claude);
        assert_eq!("CURSOR".parse::<Platform>().unwrap(), Platform::Cursor);
    }

    #[test]
    fn platform_from_str_invalid() {
        let err = "vim".parse::<Platform>().unwrap_err();
        assert!(err.contains("unknown platform"));
        assert!(err.contains("vim"));
    }

    #[test]
    fn platform_display() {
        assert_eq!(Platform::Claude.to_string(), "claude");
        assert_eq!(Platform::AgentsMd.to_string(), "agents-md");
        assert_eq!(Platform::Cursor.to_string(), "cursor");
        assert_eq!(Platform::Copilot.to_string(), "copilot");
    }

    #[test]
    fn conventional_path() {
        assert_eq!(Platform::Claude.conventional_path(), "CLAUDE.md");
        assert_eq!(Platform::AgentsMd.conventional_path(), "AGENTS.md");
        assert_eq!(
            Platform::Cursor.conventional_path(),
            ".cursor/rules/rdm.mdc"
        );
        assert_eq!(
            Platform::Copilot.conventional_path(),
            ".github/copilot-instructions.md"
        );
    }

    #[test]
    fn generate_with_project_name() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::AgentsMd,
            project: Some("myproj".to_string()),
            principles_file: None,
            mcp: false,
        });
        assert!(content.contains("--project myproj"));
        assert!(!content.contains("<PROJECT>"));
    }

    #[test]
    fn generate_without_project_name() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::AgentsMd,
            project: None,
            principles_file: None,
            mcp: false,
        });
        assert!(content.contains("--project <PROJECT>"));
    }

    #[test]
    fn generate_contains_key_sections() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::AgentsMd,
            project: None,
            principles_file: None,
            mcp: false,
        });
        assert!(content.contains("# rdm"));
        assert!(content.contains("## Setup"));
        assert!(content.contains("## Discovering work"));
        assert!(content.contains("## Reading details"));
        assert!(content.contains("## Updating status"));
        assert!(content.contains("## Creating items"));
        assert!(content.contains("## Body content"));
        assert!(content.contains("## Planning workflow"));
        assert!(content.contains("## Status transitions"));
    }

    #[test]
    fn generate_contains_key_commands() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::AgentsMd,
            project: None,
            principles_file: None,
            mcp: false,
        });
        assert!(content.contains("rdm roadmap list"));
        assert!(content.contains("rdm task list"));
        assert!(content.contains("rdm roadmap show"));
        assert!(content.contains("rdm phase show"));
        assert!(content.contains("rdm task show"));
        assert!(content.contains("rdm phase update"));
        assert!(content.contains("rdm task update"));
        assert!(content.contains("rdm roadmap create"));
        assert!(content.contains("rdm phase create"));
        assert!(content.contains("rdm task create"));
        assert!(content.contains("--no-edit"));
        assert!(content.contains("--no-body"));
    }

    #[test]
    fn cursor_has_mdc_frontmatter() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::Cursor,
            project: None,
            principles_file: None,
            mcp: false,
        });
        assert!(content.starts_with("---\n"));
        assert!(content.contains("description:"));
        assert!(content.contains("globs:"));
        // Should still have the instructions after frontmatter
        assert!(content.contains("# rdm"));
    }

    #[test]
    fn claude_no_mdc_frontmatter() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::Claude,
            project: None,
            principles_file: None,
            mcp: false,
        });
        assert!(!content.starts_with("---"));
    }

    #[test]
    fn copilot_no_mdc_frontmatter() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::Copilot,
            project: None,
            principles_file: None,
            mcp: false,
        });
        assert!(!content.starts_with("---"));
    }

    #[test]
    fn planning_workflow_section_contains_key_steps() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::AgentsMd,
            project: Some("myproj".to_string()),
            principles_file: None,
            mcp: false,
        });
        assert!(content.contains("### Before starting work"));
        assert!(content.contains("### Implementing a roadmap phase"));
        assert!(content.contains("### Discovering bugs or side-work"));
        assert!(content.contains("### When a task grows too complex"));
        assert!(content.contains("rdm promote"));
    }

    #[test]
    fn planning_workflow_uses_project_flag() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::AgentsMd,
            project: Some("myproj".to_string()),
            principles_file: None,
            mcp: false,
        });
        // Workflow section should embed the project flag
        assert!(content.contains("rdm roadmap list --project myproj"));
        assert!(content.contains("rdm task list --project myproj"));
    }

    #[test]
    fn status_transitions_documents_phase_statuses() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::AgentsMd,
            project: None,
            principles_file: None,
            mcp: false,
        });
        assert!(content.contains("### Phase statuses"));
        assert!(content.contains("`not-started` → `in-progress`"));
        assert!(content.contains("`in-progress` → `done`"));
        assert!(content.contains("`in-progress` → `blocked`"));
        assert!(content.contains("`blocked` → `in-progress`"));
    }

    #[test]
    fn status_transitions_documents_task_statuses() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::AgentsMd,
            project: None,
            principles_file: None,
            mcp: false,
        });
        assert!(content.contains("### Task statuses"));
        assert!(content.contains("`open` → `in-progress`"));
        assert!(content.contains("`in-progress` → `done`"));
        assert!(content.contains("`in-progress` → `wont-fix`"));
        assert!(content.contains("`open` → `wont-fix`"));
    }

    #[test]
    fn principles_section_included_when_file_specified() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::AgentsMd,
            project: None,
            principles_file: Some("docs/principles.md".to_string()),
            mcp: false,
        });
        assert!(content.contains("## Principles"));
        assert!(content.contains("docs/principles.md"));
    }

    #[test]
    fn principles_section_excluded_when_no_file() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::AgentsMd,
            project: None,
            principles_file: None,
            mcp: false,
        });
        assert!(!content.contains("## Principles"));
    }

    // --- Skill generation tests ---

    #[test]
    fn generate_skills_returns_five_files() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        assert_eq!(skills.len(), 5);
    }

    #[test]
    fn generate_skills_correct_paths() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        assert_eq!(skills[0].relative_path, "rdm-roadmap/SKILL.md");
        assert_eq!(skills[1].relative_path, "rdm-implement/SKILL.md");
        assert_eq!(skills[2].relative_path, "rdm-tasks/SKILL.md");
        assert_eq!(skills[3].relative_path, "rdm-review/SKILL.md");
        assert_eq!(skills[4].relative_path, "rdm-document/SKILL.md");
    }

    #[test]
    fn skills_have_yaml_frontmatter() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        for skill in &skills {
            assert!(
                skill.content.starts_with("---\n"),
                "skill {} missing frontmatter",
                skill.relative_path
            );
            assert!(
                skill.content.contains("name:"),
                "skill {} missing name",
                skill.relative_path
            );
            assert!(
                skill.content.contains("allowed-tools:"),
                "skill {} missing allowed-tools",
                skill.relative_path
            );
        }
    }

    #[test]
    fn skills_have_correct_names() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        assert!(skills[0].content.contains("name: rdm-roadmap"));
        assert!(skills[1].content.contains("name: rdm-implement"));
        assert!(skills[2].content.contains("name: rdm-tasks"));
        assert!(skills[3].content.contains("name: rdm-review"));
        assert!(skills[4].content.contains("name: rdm-document"));
    }

    #[test]
    fn skills_use_project_flag() {
        let skills = generate_skills(&SkillOptions {
            project: Some("myproj".to_string()),
            principles_file: None,
            mcp: false,
        });
        for skill in &skills {
            assert!(
                skill.content.contains("--project myproj"),
                "skill {} missing project flag",
                skill.relative_path
            );
            assert!(
                !skill.content.contains("<PROJECT>"),
                "skill {} has placeholder",
                skill.relative_path
            );
        }
    }

    #[test]
    fn skills_use_placeholder_without_project() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        for skill in &skills {
            assert!(
                skill.content.contains("--project <PROJECT>"),
                "skill {} missing placeholder",
                skill.relative_path
            );
        }
    }

    #[test]
    fn skills_contain_arguments_variable() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        for skill in &skills {
            assert!(
                skill.content.contains("$ARGUMENTS"),
                "skill {} missing $ARGUMENTS",
                skill.relative_path
            );
        }
    }

    #[test]
    fn skill_roadmap_contains_rdm_commands() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[0].content;
        assert!(content.contains("rdm roadmap create"));
        assert!(content.contains("rdm phase create"));
        assert!(content.contains("rdm roadmap show"));
    }

    #[test]
    fn skill_implement_contains_rdm_commands() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[1].content;
        assert!(content.contains("rdm phase list"));
        assert!(content.contains("rdm phase show"));
        assert!(content.contains("rdm phase update"));
        assert!(content.contains("rdm task create"));
    }

    #[test]
    fn skill_tasks_contains_rdm_commands() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[2].content;
        assert!(content.contains("rdm task list"));
        assert!(content.contains("rdm task show"));
        assert!(content.contains("rdm task update"));
    }

    #[test]
    fn skills_include_principles_when_specified() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: Some("docs/principles.md".to_string()),
            mcp: false,
        });
        for skill in &skills {
            assert!(
                skill.content.contains("## Principles"),
                "skill {} missing principles",
                skill.relative_path
            );
            assert!(
                skill.content.contains("docs/principles.md"),
                "skill {} missing principles path",
                skill.relative_path
            );
        }
    }

    #[test]
    fn skills_exclude_principles_when_not_specified() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        for skill in &skills {
            assert!(
                !skill.content.contains("## Principles"),
                "skill {} has unexpected principles",
                skill.relative_path
            );
        }
    }

    #[test]
    fn skill_implement_has_write_edit_tools() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[1].content;
        assert!(content.contains("Write"));
        assert!(content.contains("Edit"));
    }

    #[test]
    fn skill_implement_has_plan_mode_tools() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[1].content;
        assert!(content.contains("EnterPlanMode"));
        assert!(content.contains("ExitPlanMode"));
    }

    #[test]
    fn skill_tasks_has_plan_mode_tools() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[2].content;
        assert!(content.contains("EnterPlanMode"));
        assert!(content.contains("ExitPlanMode"));
    }

    #[test]
    fn skill_implement_uses_plan_mode_workflow() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[1].content;
        assert!(content.contains("Enter plan mode"));
        assert!(content.contains("Exit plan mode"));
        assert!(content.contains("implementation plan"));
    }

    #[test]
    fn skill_tasks_uses_plan_mode_workflow() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[2].content;
        assert!(content.contains("Enter plan mode"));
        assert!(content.contains("Exit plan mode"));
        assert!(content.contains("implementation plan"));
    }

    #[test]
    fn skill_roadmap_no_write_edit_tools() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        // Roadmap skill should only have Read, Bash, Glob, Grep
        let frontmatter = skills[0]
            .content
            .split("---")
            .nth(1)
            .expect("missing frontmatter");
        assert!(!frontmatter.contains("Write"));
        assert!(!frontmatter.contains("Edit"));
    }

    #[test]
    fn skill_review_contains_rdm_commands() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[3].content;
        assert!(content.contains("rdm phase show"));
        assert!(content.contains("rdm task show"));
    }

    #[test]
    fn skill_review_has_correct_name() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        assert!(skills[3].content.contains("name: rdm-review"));
    }

    #[test]
    fn skill_review_has_agent_tool() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[3].content;
        assert!(content.contains("Agent"));
    }

    #[test]
    fn skill_review_contains_arguments_variable() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        assert!(skills[3].content.contains("$ARGUMENTS"));
    }

    #[test]
    fn skill_document_contains_rdm_commands() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[4].content;
        assert!(content.contains("rdm roadmap show"));
        assert!(content.contains("rdm phase show"));
        assert!(content.contains("--format json"));
        assert!(content.contains("git log"));
        assert!(content.contains("git diff"));
    }

    #[test]
    fn skill_document_has_write_edit_tools() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[4].content;
        assert!(content.contains("Write"));
        assert!(content.contains("Edit"));
    }

    #[test]
    fn skill_document_no_plan_mode_tools() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let frontmatter = skills[4]
            .content
            .split("---")
            .nth(1)
            .expect("missing frontmatter");
        assert!(!frontmatter.contains("EnterPlanMode"));
        assert!(!frontmatter.contains("ExitPlanMode"));
    }

    #[test]
    fn skill_implement_includes_done_convention() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[1].content;
        assert!(content.contains("Done:"));
        assert!(content.contains("<roadmap-slug>/<phase-stem>"));
    }

    #[test]
    fn skill_tasks_includes_done_convention() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[2].content;
        assert!(content.contains("Done:"));
        assert!(content.contains("<roadmap-slug>/<phase-stem>"));
    }

    #[test]
    fn planning_workflow_includes_done_convention() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::AgentsMd,
            project: None,
            principles_file: None,
            mcp: false,
        });
        assert!(content.contains("Done:"));
        assert!(content.contains("<roadmap-slug>/<phase-stem>"));
    }

    // --- MCP config generation tests ---

    #[test]
    fn mcp_config_is_valid_json() {
        let output = generate_mcp_config(&McpConfigOptions { root: None });
        let parsed: serde_json::Value = serde_json::from_str(&output).expect("invalid JSON");
        assert!(parsed.is_object());
    }

    #[test]
    fn mcp_config_without_root() {
        let output = generate_mcp_config(&McpConfigOptions { root: None });
        let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
        let args = parsed["mcpServers"]["rdm"]["args"]
            .as_array()
            .expect("args should be array");
        assert_eq!(args.len(), 1);
        assert_eq!(args[0], "mcp");
    }

    #[test]
    fn mcp_config_with_root() {
        let output = generate_mcp_config(&McpConfigOptions {
            root: Some("/home/user/plans".to_string()),
        });
        let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
        let args = parsed["mcpServers"]["rdm"]["args"]
            .as_array()
            .expect("args should be array");
        assert_eq!(args.len(), 3);
        assert_eq!(args[0], "--root");
        assert_eq!(args[1], "/home/user/plans");
        assert_eq!(args[2], "mcp");
    }

    #[test]
    fn mcp_config_has_correct_structure() {
        let output = generate_mcp_config(&McpConfigOptions { root: None });
        let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(parsed["mcpServers"]["rdm"]["command"], "rdm");
        assert!(parsed["mcpServers"]["rdm"]["args"].is_array());
    }

    // --- MCP agent instructions tests ---

    #[test]
    fn mcp_agent_config_references_mcp_tools() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::AgentsMd,
            project: None,
            principles_file: None,
            mcp: true,
        });
        assert!(content.contains("rdm_roadmap_list"));
        assert!(content.contains("rdm_task_list"));
        assert!(content.contains("rdm_roadmap_show"));
        assert!(content.contains("rdm_phase_show"));
        assert!(content.contains("rdm_task_show"));
        assert!(content.contains("rdm_phase_update"));
        assert!(content.contains("rdm_task_update"));
        assert!(content.contains("rdm_roadmap_create"));
        assert!(content.contains("rdm_phase_create"));
        assert!(content.contains("rdm_task_create"));
        assert!(content.contains("rdm_search"));
    }

    #[test]
    fn mcp_agent_config_no_bash_blocks() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::AgentsMd,
            project: None,
            principles_file: None,
            mcp: true,
        });
        assert!(
            !content.contains("```bash"),
            "MCP instructions should not contain bash code blocks"
        );
    }

    #[test]
    fn mcp_agent_config_has_key_sections() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::AgentsMd,
            project: None,
            principles_file: None,
            mcp: true,
        });
        assert!(content.contains("# rdm"));
        assert!(content.contains("## Setup"));
        assert!(content.contains("## Discovering work"));
        assert!(content.contains("## Reading details"));
        assert!(content.contains("## Searching"));
        assert!(content.contains("## Updating status"));
        assert!(content.contains("## Creating items"));
        assert!(content.contains("## Planning workflow"));
        assert!(content.contains("## Status transitions"));
    }

    #[test]
    fn mcp_agent_config_with_project() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::AgentsMd,
            project: Some("myproj".to_string()),
            principles_file: None,
            mcp: true,
        });
        assert!(content.contains("\"myproj\""));
        assert!(!content.contains("<PROJECT>"));
    }

    #[test]
    fn mcp_agent_config_without_project() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::AgentsMd,
            project: None,
            principles_file: None,
            mcp: true,
        });
        assert!(content.contains("\"<PROJECT>\""));
    }

    #[test]
    fn mcp_agent_config_no_no_edit() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::AgentsMd,
            project: None,
            principles_file: None,
            mcp: true,
        });
        assert!(
            !content.contains("--no-edit"),
            "MCP instructions should not mention --no-edit"
        );
    }

    #[test]
    fn mcp_agent_config_includes_done_convention() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::AgentsMd,
            project: None,
            principles_file: None,
            mcp: true,
        });
        assert!(content.contains("Done:"));
        assert!(content.contains("<roadmap-slug>/<phase-stem>"));
    }

    #[test]
    fn mcp_agent_config_includes_promote() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::AgentsMd,
            project: None,
            principles_file: None,
            mcp: true,
        });
        assert!(content.contains("rdm_task_promote"));
    }

    #[test]
    fn mcp_agent_config_cursor_has_frontmatter() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::Cursor,
            project: None,
            principles_file: None,
            mcp: true,
        });
        assert!(content.starts_with("---\n"));
        assert!(content.contains("rdm_roadmap_list"));
    }

    #[test]
    fn mcp_agent_config_principles_included() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::AgentsMd,
            project: None,
            principles_file: Some("docs/principles.md".to_string()),
            mcp: true,
        });
        assert!(content.contains("## Principles"));
        assert!(content.contains("docs/principles.md"));
    }

    // --- MCP skill generation tests ---

    #[test]
    fn mcp_skills_returns_five_files() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        assert_eq!(skills.len(), 5);
    }

    #[test]
    fn mcp_skills_correct_paths() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        assert_eq!(skills[0].relative_path, "rdm-roadmap/SKILL.md");
        assert_eq!(skills[1].relative_path, "rdm-implement/SKILL.md");
        assert_eq!(skills[2].relative_path, "rdm-tasks/SKILL.md");
        assert_eq!(skills[3].relative_path, "rdm-review/SKILL.md");
        assert_eq!(skills[4].relative_path, "rdm-document/SKILL.md");
    }

    #[test]
    fn mcp_skills_no_bash_in_allowed_tools() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        for skill in &skills {
            // Extract frontmatter (between first and second ---)
            let parts: Vec<&str> = skill.content.splitn(3, "---").collect();
            let frontmatter = parts[1];
            assert!(
                !frontmatter.contains("  - Bash"),
                "MCP skill {} should not list Bash in allowed-tools",
                skill.relative_path
            );
        }
    }

    #[test]
    fn mcp_skills_have_mcp_tools_in_allowed_tools() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        for skill in &skills {
            assert!(
                skill.content.contains("mcp__rdm__"),
                "MCP skill {} should list mcp__rdm__ tools in allowed-tools",
                skill.relative_path
            );
        }
    }

    #[test]
    fn mcp_skills_reference_mcp_tool_calls() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        // Roadmap skill should reference MCP create tools
        assert!(skills[0].content.contains("rdm_roadmap_create"));
        assert!(skills[0].content.contains("rdm_phase_create"));
        // Implement skill should reference MCP phase tools
        assert!(skills[1].content.contains("rdm_phase_list"));
        assert!(skills[1].content.contains("rdm_phase_show"));
        assert!(skills[1].content.contains("rdm_phase_update"));
        // Tasks skill should reference MCP task tools
        assert!(skills[2].content.contains("rdm_task_list"));
        assert!(skills[2].content.contains("rdm_task_show"));
        assert!(skills[2].content.contains("rdm_task_update"));
    }

    #[test]
    fn mcp_skills_have_correct_names() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        assert!(skills[0].content.contains("name: rdm-roadmap"));
        assert!(skills[1].content.contains("name: rdm-implement"));
        assert!(skills[2].content.contains("name: rdm-tasks"));
        assert!(skills[3].content.contains("name: rdm-review"));
        assert!(skills[4].content.contains("name: rdm-document"));
    }

    #[test]
    fn mcp_skills_use_project_param() {
        let skills = generate_skills(&SkillOptions {
            project: Some("myproj".to_string()),
            principles_file: None,
            mcp: true,
        });
        for skill in &skills {
            assert!(
                skill.content.contains("\"myproj\""),
                "MCP skill {} should use project param",
                skill.relative_path
            );
        }
    }

    #[test]
    fn mcp_skills_contain_arguments_variable() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        for skill in &skills {
            assert!(
                skill.content.contains("$ARGUMENTS"),
                "MCP skill {} missing $ARGUMENTS",
                skill.relative_path
            );
        }
    }

    #[test]
    fn mcp_skill_implement_includes_done_convention() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let content = &skills[1].content;
        assert!(content.contains("Done:"));
        assert!(content.contains("<roadmap-slug>/<phase-stem>"));
    }

    #[test]
    fn mcp_skill_tasks_includes_done_convention() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let content = &skills[2].content;
        assert!(content.contains("Done:"));
        assert!(content.contains("<roadmap-slug>/<phase-stem>"));
    }
}

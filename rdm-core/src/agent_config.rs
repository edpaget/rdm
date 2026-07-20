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
    /// Pi coding agent (`.pi/AGENTS.md`)
    Pi,
}

impl Platform {
    /// Returns the conventional file path for this platform's instruction file.
    pub fn conventional_path(&self) -> &'static str {
        match self {
            Platform::Claude => "CLAUDE.md",
            Platform::AgentsMd => "AGENTS.md",
            Platform::Cursor => ".cursor/rules/rdm.mdc",
            Platform::Copilot => ".github/copilot-instructions.md",
            Platform::Pi => ".pi/AGENTS.md",
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
    /// | Pi         | `~/.pi/agent`    |
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
            // Pi loads AGENTS.md from ~/.pi/agent/ (note: not symmetric with project path)
            Platform::Pi => home.join(".pi/agent"),
        };
        Ok(dir)
    }

    /// Returns the absolute path to the user-level instruction file for this platform.
    ///
    /// For most platforms this is `user_level_dir().join(conventional_path())`,
    /// but Pi breaks the symmetry: its project-local path is `.pi/AGENTS.md`
    /// while its user-global path is `~/.pi/agent/AGENTS.md` (note the extra
    /// `agent/` segment). This method hides that asymmetry so callers don't
    /// need to special-case Pi.
    ///
    /// # Errors
    ///
    /// Returns an error if the home directory cannot be determined.
    pub fn user_level_instruction_path(&self) -> Result<PathBuf, String> {
        match self {
            Platform::Pi => Ok(home_dir()?.join(".pi/agent/AGENTS.md")),
            _ => Ok(self.user_level_dir()?.join(self.conventional_path())),
        }
    }

    /// Returns the relative skills directory inside a project root.
    ///
    /// | Platform   | Subdir          |
    /// |------------|-----------------|
    /// | Claude     | `.claude/skills`|
    /// | Pi         | `.pi/skills`    |
    /// | others     | `None`          |
    ///
    /// Returns `None` for platforms that don't follow the Agent Skills standard.
    pub fn project_skills_subdir(&self) -> Option<&'static str> {
        match self {
            Platform::Claude => Some(".claude/skills"),
            Platform::Pi => Some(".pi/skills"),
            _ => None,
        }
    }

    /// Returns the user-level skills directory for this platform.
    ///
    /// | Platform   | Directory                |
    /// |------------|--------------------------|
    /// | Claude     | `~/.claude/skills`       |
    /// | Pi         | `~/.pi/agent/skills`     |
    ///
    /// # Errors
    ///
    /// Returns an error if the platform doesn't support skills, or if the
    /// home directory cannot be determined.
    pub fn user_level_skills_dir(&self) -> Result<PathBuf, String> {
        let home = home_dir()?;
        match self {
            Platform::Claude => Ok(home.join(".claude").join("skills")),
            Platform::Pi => Ok(home.join(".pi").join("agent").join("skills")),
            other => Err(format!("--skills is not supported for platform '{other}'")),
        }
    }
}

impl fmt::Display for Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Platform::Claude => write!(f, "claude"),
            Platform::AgentsMd => write!(f, "agents-md"),
            Platform::Cursor => write!(f, "cursor"),
            Platform::Copilot => write!(f, "copilot"),
            Platform::Pi => write!(f, "pi"),
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
            "pi" => Ok(Platform::Pi),
            other => Err(format!(
                "unknown platform '{other}'; expected one of: claude, agents-md, cursor, copilot, pi"
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
/// assert_eq!(skills.len(), 11);
/// assert!(skills[0].content.contains("--project myproj"));
/// ```
pub fn generate_skills(opts: &SkillOptions) -> Vec<SkillFile> {
    let principles_note = opts.principles_file.as_deref().map(skill_principles_note);
    if opts.mcp {
        let proj = proj_param_str(opts.project.as_deref());
        vec![
            skill_roadmap_mcp(&proj, principles_note.as_deref()),
            skill_do_mcp(&proj, principles_note.as_deref()),
            skill_review_mcp(&proj, principles_note.as_deref()),
            skill_document_mcp(&proj, principles_note.as_deref()),
            skill_estimate_mcp(&proj, principles_note.as_deref()),
            skill_dispatch_phase_mcp(&proj, principles_note.as_deref()),
            skill_autopilot_mcp(&proj, principles_note.as_deref()),
            skill_land_mcp(&proj, principles_note.as_deref()),
            skill_revise_mcp(&proj, principles_note.as_deref()),
            skill_plan_review_mcp(&proj, principles_note.as_deref()),
        ]
    } else {
        let proj_flag = proj_flag_str(opts.project.as_deref());
        vec![
            skill_roadmap(&proj_flag, principles_note.as_deref()),
            skill_do(&proj_flag, principles_note.as_deref()),
            skill_review(&proj_flag, principles_note.as_deref()),
            skill_document(&proj_flag, principles_note.as_deref()),
            skill_estimate(&proj_flag, principles_note.as_deref()),
            skill_dispatch_phase(&proj_flag, principles_note.as_deref()),
            skill_autopilot(&proj_flag, principles_note.as_deref()),
            skill_land(&proj_flag, principles_note.as_deref()),
            skill_revise(&proj_flag, principles_note.as_deref()),
            skill_plan_review(&proj_flag, principles_note.as_deref()),
            skill_backlog(&proj_flag, principles_note.as_deref()),
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

fn skill_do(proj_flag: &str, principles_note: Option<&str>) -> SkillFile {
    SkillFile {
        relative_path: "rdm-do/SKILL.md",
        content: render_skill(
            include_str!("templates/skill-do-cli.md"),
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

fn skill_estimate(proj_flag: &str, principles_note: Option<&str>) -> SkillFile {
    SkillFile {
        relative_path: "rdm-estimate/SKILL.md",
        content: render_skill(
            include_str!("templates/skill-estimate-cli.md"),
            "{proj_flag}",
            proj_flag,
            principles_note,
        ),
    }
}

fn skill_dispatch_phase(proj_flag: &str, principles_note: Option<&str>) -> SkillFile {
    SkillFile {
        relative_path: "rdm-dispatch-phase/SKILL.md",
        content: render_skill(
            include_str!("templates/skill-dispatch-phase-cli.md"),
            "{proj_flag}",
            proj_flag,
            principles_note,
        ),
    }
}

fn skill_autopilot(proj_flag: &str, principles_note: Option<&str>) -> SkillFile {
    SkillFile {
        relative_path: "rdm-autopilot/SKILL.md",
        content: render_skill(
            include_str!("templates/skill-autopilot-cli.md"),
            "{proj_flag}",
            proj_flag,
            principles_note,
        ),
    }
}

fn skill_land(proj_flag: &str, principles_note: Option<&str>) -> SkillFile {
    SkillFile {
        relative_path: "rdm-land/SKILL.md",
        content: render_skill(
            include_str!("templates/skill-land-cli.md"),
            "{proj_flag}",
            proj_flag,
            principles_note,
        ),
    }
}

fn skill_revise(proj_flag: &str, principles_note: Option<&str>) -> SkillFile {
    SkillFile {
        relative_path: "rdm-revise/SKILL.md",
        content: render_skill(
            include_str!("templates/skill-revise-cli.md"),
            "{proj_flag}",
            proj_flag,
            principles_note,
        ),
    }
}

fn skill_backlog(proj_flag: &str, principles_note: Option<&str>) -> SkillFile {
    SkillFile {
        relative_path: "rdm-backlog/SKILL.md",
        content: render_skill(
            include_str!("templates/skill-backlog-cli.md"),
            "{proj_flag}",
            proj_flag,
            principles_note,
        ),
    }
}

fn skill_plan_review(proj_flag: &str, principles_note: Option<&str>) -> SkillFile {
    SkillFile {
        relative_path: "rdm-plan-review/SKILL.md",
        content: render_skill(
            include_str!("templates/skill-plan-review-cli.md"),
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
                ("t_commit", "rdm_commit"),
            ],
        ),
    }
}

fn skill_do_mcp(proj: &str, principles_note: Option<&str>) -> SkillFile {
    SkillFile {
        relative_path: "rdm-do/SKILL.md",
        content: render_mcp_skill(
            include_str!("templates/skill-do-mcp.md"),
            proj,
            principles_note,
            &[
                ("t_phase_list", "rdm_phase_list"),
                ("t_phase_show", "rdm_phase_show"),
                ("t_phase_update", "rdm_phase_update"),
                ("t_task_list", "rdm_task_list"),
                ("t_task_show", "rdm_task_show"),
                ("t_task_update", "rdm_task_update"),
                ("t_task_create", "rdm_task_create"),
                ("t_worktree_current", "rdm_worktree_current"),
                ("t_worktree_add", "rdm_worktree_add"),
                ("t_commit", "rdm_commit"),
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
                ("t_phase_update", "rdm_phase_update"),
                ("t_task_show", "rdm_task_show"),
                ("t_task_update", "rdm_task_update"),
                ("t_task_create", "rdm_task_create"),
                ("t_commit", "rdm_commit"),
            ],
        ),
    }
}

fn skill_estimate_mcp(proj: &str, principles_note: Option<&str>) -> SkillFile {
    SkillFile {
        relative_path: "rdm-estimate/SKILL.md",
        content: render_mcp_skill(
            include_str!("templates/skill-estimate-mcp.md"),
            proj,
            principles_note,
            &[
                ("t_phase_list", "rdm_phase_list"),
                ("t_phase_show", "rdm_phase_show"),
                ("t_phase_update", "rdm_phase_update"),
                ("t_roadmap_show", "rdm_roadmap_show"),
            ],
        ),
    }
}

fn skill_dispatch_phase_mcp(proj: &str, principles_note: Option<&str>) -> SkillFile {
    SkillFile {
        relative_path: "rdm-dispatch-phase/SKILL.md",
        content: render_mcp_skill(
            include_str!("templates/skill-dispatch-phase-mcp.md"),
            proj,
            principles_note,
            &[
                ("t_phase_list", "rdm_phase_list"),
                ("t_phase_show", "rdm_phase_show"),
                ("t_phase_update", "rdm_phase_update"),
                ("t_worktree_add", "rdm_worktree_add"),
                ("t_task_create", "rdm_task_create"),
            ],
        ),
    }
}

fn skill_autopilot_mcp(proj: &str, principles_note: Option<&str>) -> SkillFile {
    SkillFile {
        relative_path: "rdm-autopilot/SKILL.md",
        content: render_mcp_skill(
            include_str!("templates/skill-autopilot-mcp.md"),
            proj,
            principles_note,
            &[
                ("t_next", "rdm_next"),
                ("t_phase_list", "rdm_phase_list"),
                ("t_phase_show", "rdm_phase_show"),
                ("t_phase_update", "rdm_phase_update"),
            ],
        ),
    }
}

fn skill_land_mcp(proj: &str, principles_note: Option<&str>) -> SkillFile {
    SkillFile {
        relative_path: "rdm-land/SKILL.md",
        content: render_mcp_skill(
            include_str!("templates/skill-land-mcp.md"),
            proj,
            principles_note,
            &[
                ("t_phase_show", "rdm_phase_show"),
                ("t_phase_update", "rdm_phase_update"),
                ("t_task_show", "rdm_task_show"),
                ("t_task_update", "rdm_task_update"),
                ("t_worktree_current", "rdm_worktree_current"),
                ("t_worktree_remove", "rdm_worktree_remove"),
            ],
        ),
    }
}

fn skill_revise_mcp(proj: &str, principles_note: Option<&str>) -> SkillFile {
    SkillFile {
        relative_path: "rdm-revise/SKILL.md",
        content: render_mcp_skill(
            include_str!("templates/skill-revise-mcp.md"),
            proj,
            principles_note,
            &[
                ("t_review_requests", "rdm_review_requests"),
                ("t_review_show", "rdm_review_show"),
                ("t_review_address_comment", "rdm_review_address_comment"),
                ("t_review_complete", "rdm_review_complete"),
                ("t_phase_update", "rdm_phase_update"),
                ("t_task_update", "rdm_task_update"),
                ("t_roadmap_update", "rdm_roadmap_update"),
                ("t_commit", "rdm_commit"),
            ],
        ),
    }
}

fn skill_plan_review_mcp(proj: &str, principles_note: Option<&str>) -> SkillFile {
    SkillFile {
        relative_path: "rdm-plan-review/SKILL.md",
        content: render_mcp_skill(
            include_str!("templates/skill-plan-review-mcp.md"),
            proj,
            principles_note,
            &[
                ("t_phase_show", "rdm_phase_show"),
                ("t_phase_update", "rdm_phase_update"),
                ("t_task_show", "rdm_task_show"),
                ("t_task_update", "rdm_task_update"),
                ("t_task_create", "rdm_task_create"),
                ("t_roadmap_show", "rdm_roadmap_show"),
                ("t_roadmap_update", "rdm_roadmap_update"),
                ("t_commit", "rdm_commit"),
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
        assert_eq!("pi".parse::<Platform>().unwrap(), Platform::Pi);
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
        assert_eq!(Platform::Pi.to_string(), "pi");
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
        assert_eq!(Platform::Pi.conventional_path(), ".pi/AGENTS.md");
    }

    #[test]
    fn pi_user_level_instruction_path() {
        let path = Platform::Pi.user_level_instruction_path().unwrap();
        assert!(
            path.ends_with(".pi/agent/AGENTS.md"),
            "unexpected path: {}",
            path.display()
        );
    }

    #[test]
    fn project_skills_subdir_for_claude_and_pi() {
        assert_eq!(
            Platform::Claude.project_skills_subdir(),
            Some(".claude/skills")
        );
        assert_eq!(Platform::Pi.project_skills_subdir(), Some(".pi/skills"));
        assert_eq!(Platform::AgentsMd.project_skills_subdir(), None);
        assert_eq!(Platform::Cursor.project_skills_subdir(), None);
        assert_eq!(Platform::Copilot.project_skills_subdir(), None);
    }

    #[test]
    fn user_level_skills_dir_for_claude() {
        let path = Platform::Claude.user_level_skills_dir().unwrap();
        assert!(
            path.ends_with(".claude/skills"),
            "unexpected path: {}",
            path.display()
        );
    }

    #[test]
    fn user_level_skills_dir_for_pi() {
        let path = Platform::Pi.user_level_skills_dir().unwrap();
        assert!(
            path.ends_with(".pi/agent/skills"),
            "unexpected path: {}",
            path.display()
        );
    }

    #[test]
    fn user_level_skills_dir_rejects_unsupported() {
        assert!(Platform::Cursor.user_level_skills_dir().is_err());
        assert!(Platform::AgentsMd.user_level_skills_dir().is_err());
        assert!(Platform::Copilot.user_level_skills_dir().is_err());
    }

    #[test]
    fn claude_user_level_instruction_path_default() {
        let path = Platform::Claude.user_level_instruction_path().unwrap();
        assert!(
            path.ends_with(".claude/CLAUDE.md"),
            "unexpected path: {}",
            path.display()
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
    fn cli_instructions_teach_document_reviews() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::AgentsMd,
            project: Some("myproj".to_string()),
            principles_file: None,
            mcp: false,
        });
        assert!(content.contains("## Document reviews"));
        assert!(content.contains("rdm review start --on task/<slug>"));
        assert!(content.contains("rdm review submit <review-id> --verdict request-changes"));
        assert!(content.contains("--occurrence <n>"));
        assert!(content.contains("--type review"));
        // Drifted-range semantics are spelled out for agents (JSON ranges
        // index the created_commit body).
        assert!(content.contains("drifted"));
        assert!(content.contains("created_commit"));
        // The project flag is substituted into the review commands too.
        assert!(content.contains("rdm review requests --project myproj"));
        // The agent loop is automated by the rdm-revise skill.
        assert!(content.contains("rdm-revise"));
    }

    #[test]
    fn mcp_instructions_teach_document_reviews() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::AgentsMd,
            project: Some("myproj".to_string()),
            principles_file: None,
            mcp: true,
        });
        assert!(content.contains("## Document reviews"));
        // The four review tools and the loop.
        assert!(content.contains("rdm_review_requests"));
        assert!(content.contains("rdm_review_show"));
        assert!(content.contains("rdm_review_address_comment"));
        assert!(content.contains("rdm_review_complete"));
        assert!(content.contains("rdm-revise"));
        // Anchor/resolution semantics an agent must know.
        assert!(content.contains("anchor_type"));
        assert!(content.contains("drifted"));
        assert!(content.contains("body_at_created_commit"));
        // Commit provenance threading from update responses.
        assert!(content.contains("Commit: <sha>"));
        assert!(content.contains("applied_commit"));
        // The project param is substituted.
        assert!(content.contains("review_id: \"<id>\""));
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
        assert!(content.contains("`in-progress` → `needs-review`"));
        assert!(content.contains("`needs-review` → `reviewed`"));
        assert!(content.contains("`reviewed` → `done`"));
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
        assert!(content.contains("`in-progress` → `needs-review`"));
        assert!(content.contains("`needs-review` → `reviewed`"));
        assert!(content.contains("`reviewed` → `done`"));
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
    fn generate_skills_returns_eleven_files() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        assert_eq!(skills.len(), 11);
    }

    #[test]
    fn generate_skills_correct_paths() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        assert_eq!(skills[0].relative_path, "rdm-roadmap/SKILL.md");
        assert_eq!(skills[1].relative_path, "rdm-do/SKILL.md");
        assert_eq!(skills[2].relative_path, "rdm-review/SKILL.md");
        assert_eq!(skills[3].relative_path, "rdm-document/SKILL.md");
        assert_eq!(skills[4].relative_path, "rdm-estimate/SKILL.md");
        assert_eq!(skills[5].relative_path, "rdm-dispatch-phase/SKILL.md");
        assert_eq!(skills[6].relative_path, "rdm-autopilot/SKILL.md");
        assert_eq!(skills[7].relative_path, "rdm-land/SKILL.md");
        assert_eq!(skills[8].relative_path, "rdm-revise/SKILL.md");
        assert_eq!(skills[9].relative_path, "rdm-plan-review/SKILL.md");
        assert_eq!(skills[10].relative_path, "rdm-backlog/SKILL.md");
    }

    #[test]
    fn skill_backlog_documents_the_grooming_plan() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[10].content;
        assert!(content.contains("name: rdm-backlog"));
        assert!(content.contains("$ARGUMENTS"));
        // Read-and-propose: runs backlog report, mutates nothing.
        assert!(content.contains("rdm backlog report --format json"));
        assert!(content.contains("Non-mutation guarantee"));
        assert!(content.contains("never runs"));
        // Each category maps to a literal proposed command.
        assert!(content.contains("task update <slug> --status wont-fix"));
        assert!(content.contains("task merge <survivor> --from"));
        assert!(content.contains("promote <slug> --into <roadmap>"));
        assert!(content.contains("roadmap archive <roadmap>"));
        // Never force-archive; the candidates never need it.
        assert!(content.contains("Never** add `--force`"));
        // Ambiguity degrades to open questions, not blind actions.
        assert!(content.contains("## Open questions"));
        assert!(content.contains("file it as an open question instead"));
        // Empty case is handled explicitly.
        assert!(content.contains("Nothing to groom"));
        // Autopilot-ready framing: proposed phase bodies carry the standard headings.
        assert!(content.contains("## Context` / `## Steps` / `## Acceptance Criteria"));
        assert!(content.contains("rdm-autopilot"));
    }

    #[test]
    fn skill_revise_documents_the_revision_loop() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[8].content;
        assert!(content.contains("name: rdm-revise"));
        assert!(content.contains("$ARGUMENTS"));
        // Distinct from rdm-review: acts on *document* reviews.
        assert!(content.contains("rdm-review"));
        assert!(content.contains("document"));
        // The queue → read → dispatch → apply → record → close loop.
        assert!(content.contains("rdm review requests --format json"));
        assert!(content.contains("rdm review show <review-id> --format json"));
        // Summary-first: comments are instances of the reviewer's intent.
        assert!(content.contains("summary"));
        assert!(content.contains("overall intent"));
        // Anchor dispatch: resolved / drifted / whole-document (incl. Unknown).
        assert!(content.contains("anchor_type"));
        assert!(content.contains("drifted"));
        assert!(content.contains("whole-document"));
        assert!(content.contains("unrecognized `anchor_type`"));
        assert!(content.contains("created_commit"));
        // Edits go through rdm update commands, never direct file edits.
        assert!(content.contains("rdm phase update"));
        assert!(content.contains("rdm task update"));
        assert!(content.contains("rdm roadmap update"));
        // SHA capture immediately after the edit, threaded explicitly.
        assert!(content.contains("rev-parse HEAD"));
        assert!(content.contains("--applied-commit <sha>"));
        assert!(content.contains("before any review update"));
        // Clarification leaves the comment open; wont-fix carries reasoning.
        assert!(content.contains("leave the comment **open** (no `--status`)"));
        assert!(content.contains("--status wont-fix"));
        // Close only when nothing is open; otherwise stay submitted.
        assert!(content.contains("--state addressed"));
        assert!(content.contains("leave the review submitted"));
    }

    #[test]
    fn mcp_skill_revise_uses_review_tools_and_commit_threading() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let content = &skills[8].content;
        assert!(content.contains("name: rdm-revise"));
        assert!(content.contains("$ARGUMENTS"));
        // Drives the loop through the four review tools plus the doc updates.
        assert!(content.contains("rdm_review_requests"));
        assert!(content.contains("rdm_review_show"));
        assert!(content.contains("rdm_review_address_comment"));
        assert!(content.contains("rdm_review_complete"));
        assert!(content.contains("rdm_phase_update"));
        assert!(content.contains("rdm_task_update"));
        assert!(content.contains("rdm_roadmap_update"));
        // Commit provenance is threaded from the update tool's response —
        // the defaulting fallback is best-effort and never fires on wont-fix.
        assert!(content.contains("Commit: <sha>"));
        assert!(content.contains("applied_commit"));
        assert!(content.contains("best-effort"));
        assert!(content.contains("never defaults for `wont-fix`"));
        // Clarification: no status leaves the comment open; complete refuses
        // and lists open ids while clarification is pending.
        assert!(content.contains("no `status`"));
        assert!(content.contains("leave the review submitted"));
        // Anchor dispatch against the inlined document bodies.
        assert!(content.contains("body_at_created_commit"));
        assert!(content.contains("current_body"));
        assert!(content.contains("anchor_type"));
        // MCP variant: Bash-free frontmatter, resolved mcp__rdm__ tool names.
        let frontmatter = content.split("---").nth(1).expect("missing frontmatter");
        assert!(!frontmatter.contains("  - Bash"));
        assert!(frontmatter.contains("mcp__rdm__rdm_review_requests"));
        assert!(frontmatter.contains("mcp__rdm__rdm_review_address_comment"));
        assert!(frontmatter.contains("mcp__rdm__rdm_task_update"));
    }

    // --- rdm-plan-review skill tests ---

    #[test]
    fn skill_plan_review_has_correct_name() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        assert!(skills[9].content.contains("name: rdm-plan-review"));
    }

    #[test]
    fn skill_plan_review_has_agent_tool() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        assert!(skills[9].content.contains("Agent"));
    }

    #[test]
    fn skill_plan_review_contains_arguments_variable() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        assert!(skills[9].content.contains("$ARGUMENTS"));
    }

    #[test]
    fn mcp_skill_plan_review_has_correct_name() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        assert!(skills[9].content.contains("name: rdm-plan-review"));
    }

    #[test]
    fn mcp_skill_plan_review_contains_arguments_variable() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        assert!(skills[9].content.contains("$ARGUMENTS"));
    }

    #[test]
    fn skill_plan_review_covers_three_dimensions() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[9].content;
        assert!(content.contains("**coherence** — *always.*"));
        assert!(content.contains("**architectural-fit** — *always.*"));
        // The unit-of-work dimension is triggered by the target type, not by
        // diff shape: it runs only for a phase.
        assert!(content.contains("**unit-of-work** — *trigger: the target is a phase.*"));
        assert!(
            content.contains("add `unit-of-work` only when the target type from step 1 is a phase")
        );
    }

    #[test]
    fn mcp_skill_plan_review_covers_three_dimensions() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let content = &skills[9].content;
        assert!(content.contains("**coherence** — *always.*"));
        assert!(content.contains("**architectural-fit** — *always.*"));
        assert!(content.contains("**unit-of-work** — *trigger: the target is a phase.*"));
        assert!(
            content.contains("add `unit-of-work` only when the target type from step 1 is a phase")
        );
    }

    #[test]
    fn skill_plan_review_architecture_reviewer_falls_back_to_claude_md() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[9].content;
        assert!(content.contains("CLAUDE.md"));
        assert!(content.contains("AGENTS.md"));
    }

    #[test]
    fn skill_plan_review_dispatches_read_only_reviewers() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[9].content;
        assert!(content.contains("read-only"));
        assert!(content.contains("parallel"));
    }

    #[test]
    fn mcp_skill_plan_review_dispatches_read_only_reviewers() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let content = &skills[9].content;
        assert!(content.contains("read-only"));
        assert!(content.contains("parallel"));
    }

    #[test]
    fn skill_plan_review_consolidates_single_verdict() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[9].content;
        assert!(content.contains("**reviewed**"));
        assert!(content.contains("**rework**"));
        assert!(content.contains("**escalated**"));
        assert!(content.contains("the first matching rule wins"));
        // The retired vocabulary survives only as the explicit mapping note.
        assert!(!content.contains("**PASS WITH CONCERNS**"));
        assert!(!content.contains("**REWORK**"));
    }

    #[test]
    fn mcp_skill_plan_review_consolidates_single_verdict() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let content = &skills[9].content;
        assert!(content.contains("**reviewed**"));
        assert!(content.contains("**rework**"));
        assert!(content.contains("**escalated**"));
        assert!(content.contains("the first matching rule wins"));
        assert!(!content.contains("**PASS WITH CONCERNS**"));
        assert!(!content.contains("**REWORK**"));
    }

    #[test]
    fn skill_plan_review_categorizes_findings() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[9].content;
        assert!(content.contains("Small"));
        assert!(content.contains("Large"));
        // Large findings are filed as tasks; small findings are applied via
        // the plan document's own update command.
        assert!(content.contains("rdm task create"));
        assert!(content.contains("rdm phase update"));
        assert!(content.contains("--body"));
    }

    #[test]
    fn skill_plan_review_orchestrator_applies_fixes_not_subagents() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[9].content;
        assert!(content.contains("never edits"));
        assert!(content.contains("orchestrator"));
        assert!(content.contains("only the orchestrator edits"));
    }

    /// Suggested default tags paired with the leading fragment of each gloss.
    /// Bare tag names would match vacuously (`bug` already occurs ~10x per
    /// template), so assertions anchor to tag+gloss pairs.
    const DEFAULT_TAG_GLOSSES: &[&str] = &[
        "`bug` (defect",
        "`enhancement` (new capability",
        "`cli` (rdm-cli",
        "`core` (rdm-core",
        "`server` (HTTP/MCP",
        "`web-ui` (browser",
        "`docs` (documentation",
    ];

    #[test]
    fn cli_instructions_instruct_tagging_on_create() {
        let content = agent_instructions(None, None);
        assert!(content.contains("**Always pass `--tags` when you create**"));
        assert!(content.contains("untagged items are invisible to tag-filtered queries"));
        assert!(content.contains("replaces the existing list"));
    }

    #[test]
    fn mcp_instructions_instruct_tagging_on_create() {
        let content = agent_instructions_mcp(None, None);
        assert!(content.contains("**Always pass `tags` when you create**"));
        assert!(content.contains("untagged items are invisible to tag-filtered queries"));
        assert!(content.contains("replaces the existing list"));
    }

    #[test]
    fn instructions_suggest_default_tag_vocabulary() {
        let cli = agent_instructions(None, None);
        let mcp = agent_instructions_mcp(None, None);
        for pair in DEFAULT_TAG_GLOSSES {
            assert!(cli.contains(pair), "CLI instructions missing gloss {pair}");
            assert!(mcp.contains(pair), "MCP instructions missing gloss {pair}");
        }
        assert!(cli.contains("not a closed set"));
        assert!(mcp.contains("not a closed set"));
        assert!(cli.contains("--tags bug,cli"));
        assert!(mcp.contains("tags: [\"bug\", \"cli\"]"));
    }

    #[test]
    fn cli_instructions_teach_tag_list_discovery() {
        let content = agent_instructions(None, None);
        assert!(content.contains("rdm tag list --project <PROJECT>"));
        assert!(
            content.contains("Tags are compared verbatim — `CLI` and `cli` are different tags.")
        );
        // Superseded discovery phrasing is gone.
        assert!(!content.contains("rdm search \"\" --tag <candidate>"));

        let scoped = agent_instructions(Some("myproj"), None);
        assert!(scoped.contains("rdm tag list --project myproj"));
    }

    #[test]
    fn mcp_instructions_teach_tag_list_discovery() {
        let content = agent_instructions_mcp(None, None);
        assert!(content.contains("rdm tag list"));
        // No such MCP tool exists — don't invent one.
        assert!(!content.contains("rdm_tag_list"));
    }

    #[test]
    fn instructions_leave_no_unsubstituted_placeholders() {
        for content in [
            agent_instructions(None, None),
            agent_instructions(Some("myproj"), None),
        ] {
            assert!(!content.contains("{proj_flag}"));
            assert!(!content.contains("{proj_param}"));
        }
        for content in [
            agent_instructions_mcp(None, None),
            agent_instructions_mcp(Some("myproj"), None),
        ] {
            // `agent_instructions_mcp` only substitutes `{proj_param}`; a
            // `{proj_flag}` in the MCP template would leak verbatim.
            assert!(!content.contains("{proj_flag}"));
            assert!(!content.contains("{proj_param}"));
        }
    }

    #[test]
    fn skill_plan_review_clears_tag_on_pass() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[9].content;
        assert!(content.contains("needs-plan-review"));
        assert!(content.contains("--format json"));
        assert!(content.contains("--tags"));
        assert!(content.contains("On **reviewed**, read the target's current tags"));
        assert!(content.contains("| **reviewed** | cleared | none |"));
        assert!(
            content.contains("rdm commit -m \"chore(plan): clear needs-plan-review on <target>\"")
        );
    }

    #[test]
    fn mcp_skill_plan_review_clears_tag_on_pass() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let content = &skills[9].content;
        assert!(content.contains("needs-plan-review"));
        assert!(content.contains("On **reviewed**, read the target's current tags"));
        assert!(content.contains("| **reviewed** | cleared | none |"));
    }

    #[test]
    fn mcp_skill_plan_review_uses_array_tags_not_string() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let content = &skills[9].content;
        // The MCP tags parameter is a JSON array (Option<Vec<String>>), never
        // the CLI's comma-joined string convention.
        assert!(content.contains("tags: [\"<remaining-tag-1>\", \"<remaining-tag-2>\"]"));
        assert!(content.contains("tags: []"));
        assert!(!content.contains("tags: \"<comma-joined-remaining-tags>\""));
        assert!(!content.contains("tags: \"\""));
    }

    #[test]
    fn skill_plan_review_implementation_plan_mode_skips_gate() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[9].content;
        assert!(content.contains("--implementation-plan"));
        assert!(content.contains("no tag-gate step"));
        assert!(content.contains("skip the Gate step entirely for this mode"));
        assert!(content.contains("Skip this step entirely in `--implementation-plan` mode"));
        // The carve-out also survives in the generated gate spec, so it cannot
        // be lost on regeneration.
        assert!(content.contains("**`--implementation-plan`** — **no gate at all.**"));
    }

    #[test]
    fn mcp_skill_plan_review_implementation_plan_mode_skips_gate() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let content = &skills[9].content;
        assert!(content.contains("--implementation-plan"));
        assert!(content.contains("no tag-gate step"));
        assert!(content.contains("skip the Gate step entirely for this mode"));
        assert!(content.contains("Skip this step entirely in `--implementation-plan` mode"));
        // The carve-out also survives in the generated gate spec, so it cannot
        // be lost on regeneration.
        assert!(content.contains("**`--implementation-plan`** — **no gate at all.**"));
    }

    #[test]
    fn skill_plan_review_implementation_plan_mode_skips_step4_mutations() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[9].content;
        assert!(content.contains("the *act* half is skipped entirely"));
        assert!(content.contains("folding them back into the plan text is left to the caller"));
        assert!(content.contains("skips the Act step's fix-application half the same way"));
        assert!(content.contains(
            "Skip this step's fix-application half entirely in `--implementation-plan` mode"
        ));
    }

    #[test]
    fn mcp_skill_plan_review_implementation_plan_mode_skips_step4_mutations() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let content = &skills[9].content;
        assert!(content.contains("the *act* half is skipped entirely"));
        assert!(content.contains("folding them back into the plan text is left to the caller"));
        assert!(content.contains("skips the Act step's fix-application half the same way"));
        assert!(content.contains(
            "Skip this step's fix-application half entirely in `--implementation-plan` mode"
        ));
    }

    #[test]
    fn skill_plan_review_unit_of_work_reviewer_skips_implementation_plan() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[9].content;
        // The phases-only trigger and its skip list are rendered from the
        // canonical source, so assert the (line-wrapped) rendered fragments.
        assert!(content.contains("*trigger: the target is a phase.* Skipped for"));
        assert!(content.contains("tasks, standalone roadmap bodies, and `--implementation-plan`"));
    }

    #[test]
    fn mcp_skill_plan_review_unit_of_work_reviewer_skips_implementation_plan() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let content = &skills[9].content;
        // The phases-only trigger and its skip list are rendered from the
        // canonical source, so assert the (line-wrapped) rendered fragments.
        assert!(content.contains("*trigger: the target is a phase.* Skipped for"));
        assert!(content.contains("tasks, standalone roadmap bodies, and `--implementation-plan`"));
    }

    #[test]
    fn skill_plan_review_leaves_tag_on_rework() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[9].content;
        assert!(content.contains(
            "On **rework** and **escalated**, do **not** call `update --tags`. `needs-plan-review` is left unchanged in place."
        ));
        // Backed by the generated gate spec so it survives regeneration.
        assert!(content.contains("| **rework** | left in place | none |"));
        assert!(content.contains("| **escalated** | left in place | none |"));
    }

    #[test]
    fn mcp_skill_plan_review_leaves_tag_on_rework() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let content = &skills[9].content;
        assert!(content.contains(
            "On **rework** and **escalated**, do **not** call the update tool with `tags`. `needs-plan-review` is left unchanged in place."
        ));
        assert!(content.contains("| **rework** | left in place | none |"));
        assert!(content.contains("| **escalated** | left in place | none |"));
    }

    #[test]
    fn skill_plan_review_explains_tags_replace_semantics() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[9].content;
        assert!(content.contains("`--tags` replaces the whole list"));
    }

    #[test]
    fn skill_plan_review_gates_each_phase_individually_under_roadmap() {
        for mcp in [false, true] {
            let skills = generate_skills(&SkillOptions {
                project: None,
                principles_file: None,
                mcp,
            });
            let content = &skills[9].content;
            assert!(
                content.contains("Under `--roadmap <slug>`, gate each phase **individually**"),
                "mcp={mcp}: per-phase roadmap gating missing from the hand-authored step"
            );
            // Backed by the generated gate spec so it survives regeneration.
            assert!(
                content.contains("gate each phase **individually**, and the roadmap"),
                "mcp={mcp}: per-phase roadmap gating missing from the generated gate spec"
            );
        }
    }

    #[test]
    fn skill_plan_review_spec_confined_to_generated_block() {
        const BEGIN: &str = "<!-- rdm:review-spec:begin";
        const END: &str = "<!-- rdm:review-spec:end -->";
        // Definitional phrases lifted verbatim from the generated plan block.
        // Bare words like "rework" recur legitimately in the hand-authored
        // CLI/MCP mechanics, so only full definitions are listed.
        const DEFINITIONAL_PHRASES: &[&str] = &[
            "the work must not advance as-is",
            "recorded but non-gating",
            "minor optional improvement",
            "Scale the fleet to what the change actually touches",
            "Determine the outcome in this strict order",
            "needs a *human decision*",
            "**Plan-stage severity calibration.**",
            "*trigger: the target is a phase.*",
        ];

        for mcp in [false, true] {
            let skills = generate_skills(&SkillOptions {
                project: None,
                principles_file: None,
                mcp,
            });
            let content = &skills[9].content;
            let begin = content
                .find(BEGIN)
                .unwrap_or_else(|| panic!("mcp={mcp}: missing begin marker"));
            let end = content
                .find(END)
                .unwrap_or_else(|| panic!("mcp={mcp}: missing end marker"));
            assert!(
                begin < end,
                "mcp={mcp}: begin marker must precede end marker"
            );

            let before = &content[..begin];
            let after = &content[end + END.len()..];

            for phrase in DEFINITIONAL_PHRASES {
                assert!(
                    !before.contains(phrase),
                    "mcp={mcp}: spec-definition phrase found before the generated block: {phrase:?}"
                );
                assert!(
                    !after.contains(phrase),
                    "mcp={mcp}: spec-definition phrase found after the generated block: {phrase:?}"
                );
            }
        }
    }

    #[test]
    fn skill_land_documents_landing_and_safety() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[7].content;
        assert!(content.contains("name: rdm-land"));
        // Item ref comes from $ARGUMENTS.
        assert!(content.contains("$ARGUMENTS"));
        assert!(content.contains("item ref"));
        // Linear history via rebase + fast-forward merge, no merge commit.
        assert!(content.contains("merge --ff-only"));
        assert!(content.contains("linear history"));
        assert!(content.contains("rebase"));
        assert!(content.contains("no merge commit"));
        // Preconditions: reviewed, the Done: line, the CI-equivalent checks.
        assert!(content.contains("reviewed"));
        assert!(content.contains("Done:"));
        assert!(content.contains("cargo fmt --check"));
        assert!(content.contains("cargo clippy -- -D warnings"));
        assert!(content.contains("cargo nextest run"));
        // Abort-and-escalate on conflict/failure per the shared protocol; never force.
        assert!(content.contains("git rebase --abort"));
        assert!(content.contains("docs/escalation-protocol.md"));
        assert!(content.contains("never force"));
        // Post-commit flips reviewed -> done; idempotent fallback exists.
        assert!(content.contains("reviewed → done"));
        assert!(content.contains("post-commit"));
        assert!(content.contains("--status done --commit"));
        // Cleanup: single remove + batch prune.
        assert!(content.contains("rdm worktree remove <item> --delete-branch"));
        assert!(content.contains("rdm worktree prune"));
        // Safety posture: explicit/opt-in only, never auto-lands.
        assert!(content.contains("never auto-lands"));
        assert!(content.contains("--land"));
    }

    #[test]
    fn skill_land_synthesizes_the_completion_trailer_before_the_rebase() {
        for mcp in [false, true] {
            let skills = generate_skills(&SkillOptions {
                project: None,
                principles_file: None,
                mcp,
            });
            let content = &skills[7].content;
            // Precondition 2 reads the completion policy off the autonomous
            // OUTCOME rather than inferring it from a missing trailer...
            assert!(
                content.contains("`writesCompletion: true` on `reviewed`"),
                "skill-land (mcp={mcp}) must state the OUTCOME carries writesCompletion on reviewed"
            );
            assert!(
                content.contains("Read the policy off the outcome, do not infer it"),
                "skill-land (mcp={mcp}) must instruct the lander to read the policy, not infer it"
            );
            // ...synthesizes the line from rdm (one home for the format)...
            assert!(
                content.contains("rdm hook done-line"),
                "skill-land (mcp={mcp}) must source the trailer from rdm hook done-line"
            );
            assert!(
                content.contains("git commit --amend"),
                "skill-land (mcp={mcp}) must amend the synthesized trailer onto the branch tip"
            );
            // ...BEFORE the rebase/fast-forward, so landing needs no manual rebase.
            assert!(
                content.contains("**before** the rebase and fast-forward below"),
                "skill-land (mcp={mcp}) must amend BEFORE the rebase so no manual rebase is ever needed"
            );
            assert!(
                content.contains("never needs a manual rebase"),
                "skill-land (mcp={mcp}) must state that an autonomous branch never needs a manual rebase"
            );
            // A failed done-line is an abort, not an empty amend.
            assert!(
                content.contains("never amend an empty trailer"),
                "skill-land (mcp={mcp}) must abort rather than amend an empty trailer"
            );
        }
    }

    #[test]
    fn skill_autopilot_documents_loop_and_budgets() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[6].content;
        assert!(content.contains("name: rdm-autopilot"));
        // Drives one named roadmap; the slug is required and the loop never roams.
        assert!(content.contains("required roadmap slug"));
        assert!(content.contains("never roams to another roadmap"));
        // The loop driver / termination oracle is `rdm next`.
        assert!(content.contains("rdm next --roadmap <slug> --format json"));
        assert!(content.contains("blocked-on-dependencies"));
        // Composes the per-phase skills rather than re-implementing them.
        assert!(content.contains("rdm-dispatch-phase"));
        assert!(content.contains("rdm-estimate"));
        // Interprets the three dispatch outcomes.
        assert!(content.contains("reviewed | rework | escalated"));
        assert!(content.contains("**reviewed**"));
        assert!(content.contains("**rework**"));
        assert!(content.contains("**escalated**"));
        // Rework budget exhaustion parks the phase as a `code`-stage escalation.
        assert!(content.contains("--status blocked"));
        assert!(content.contains("[code]"));
        // Bounded run: global step budget + stop conditions + always-on summary.
        assert!(content.contains("Global step budget"));
        assert!(content.contains("Stop the loop"));
        assert!(content.contains("Summary"));
        // Escalation rule is owned by the shared protocol, not redefined here;
        // the batch queue is surfaced via `rdm review blocked`.
        assert!(content.contains("docs/escalation-protocol.md"));
        assert!(content.contains("rdm review blocked"));
        // Opt-in landing leaves `main` untouched by default.
        assert!(content.contains("--land"));
        assert!(content.contains("Default OFF"));
        assert!(content.contains("never touched"));
        // Dry-run / bounded modes.
        assert!(content.contains("--plan-only"));
        assert!(content.contains("--max-phases"));
        // Active driver: every dispatched phase actively runs review.
        assert!(content.contains("active driver"));
        // Never writes a Done: line by hand.
        assert!(!content.contains("Done: <roadmap-slug>/<phase-stem>"));
        // Unattended-permission guidance.
        assert!(content.contains("--permission-mode auto"));
        // Mandatory dispatch enforcement: non-skippable MUST, inline-collapse
        // negative example, self-check gate, and consistency with phase 1's
        // synchronous-dispatch contract.
        assert!(content.contains("Mandatory dispatch"));
        assert!(content.contains("inline-collapse"));
        assert!(content.contains("MUST NOT"));
        assert!(content.contains("Self-check before proceeding"));
        assert!(content.contains("dispatched synchronously"));
        assert!(content.contains("SendMessage"));
    }

    #[test]
    fn skill_dispatch_phase_documents_contract_and_plan_gate() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[5].content;
        assert!(content.contains("name: rdm-dispatch-phase"));
        // One worktree per roadmap, reused across phases — created with the bare
        // roadmap ref, not a per-phase `<slug>/<phase-stem>` worktree.
        assert!(content.contains("rdm worktree add <slug>"));
        assert!(content.contains("roadmap worktree"));
        assert!(!content.contains("worktree add <slug>/<phase-stem>"));
        assert!(content.contains("model tier"));
        // Structured outcome with the three documented values.
        assert!(content.contains("reviewed | rework | escalated"));
        // A *separate* plan reviewer gates the plan before code is written,
        // returning approve / revise / escalate, and the gate is bounded.
        assert!(content.contains("Plan gate"));
        assert!(content.contains("separate"));
        assert!(content.contains("approve"));
        assert!(content.contains("revise"));
        assert!(content.contains("escalate"));
        assert!(content.contains("at most one revise round"));
        // Plan gate rigor scales with the phase's difficulty tier.
        assert!(content.contains("Scale the reviewer's rigor to the phase's difficulty tier"));
        assert!(content.contains("Trivial/easy"));
        assert!(content.contains("Moderate"));
        assert!(content.contains("per-finding evidence"));
        assert!(content.contains("refute pass"));
        // Sharpened checklist: AC->step, AC->test, edge cases, and declared
        // cross-phase/cross-crate dependencies, in addition to scope/architecture.
        assert!(content.contains("Acceptance criteria → steps"));
        assert!(content.contains("Acceptance criteria → tests"));
        assert!(content.contains("test-per-AC"));
        assert!(content.contains("Edge cases / error paths"));
        assert!(content.contains("Cross-phase / cross-crate dependencies"));
        // Revise round re-checks the revised plan; an unconverged revise escalates
        // rather than silently proceeding with a deficient plan.
        assert!(content.contains("re-checks the revised plan against the same checklist"));
        assert!(content.contains("exhausted plan-revise budget"));
        assert!(!content.contains("a single time, then proceeds"));
        // Delegates code review to rdm-review (which owns the Done: line).
        assert!(content.contains("rdm-review"));
        // Escalation parks the phase as blocked; it never writes a Done: line.
        assert!(content.contains("--status blocked"));
        assert!(!content.contains("Done: <roadmap-slug>/<phase-stem>"));
        // Escalation follows the shared protocol: it references the protocol
        // doc and records a stage-tagged reason via --reason when parking.
        assert!(content.contains("docs/escalation-protocol.md"));
        assert!(content.contains("--reason"));
        assert!(content.contains("[plan]"));
        assert!(content.contains("[code]"));
        // Dispatches subagents and isolates their context.
        assert!(content.contains("Agent"));
        assert!(content.contains("Context isolation"));
        // Mandatory dispatch enforcement: non-skippable MUST, inline-collapse
        // negative example, self-check gate, and consistency with phase 1's
        // synchronous-dispatch contract.
        assert!(content.contains("Mandatory dispatch"));
        assert!(content.contains("inline-collapse"));
        assert!(content.contains("MUST NOT"));
        assert!(content.contains("Self-check before proceeding"));
        assert!(content.contains("dispatched synchronously"));
        assert!(content.contains("SendMessage"));
    }

    #[test]
    fn skill_estimate_names_itself_and_uses_phase_update() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[4].content;
        assert!(content.contains("name: rdm-estimate"));
        assert!(content.contains("rdm phase update"));
        assert!(content.contains("--difficulty"));
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
        assert!(skills[1].content.contains("name: rdm-do"));
        assert!(skills[2].content.contains("name: rdm-review"));
        assert!(skills[3].content.contains("name: rdm-document"));
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
    fn skill_do_contains_rdm_commands() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[1].content;
        // Phase flow commands.
        assert!(content.contains("rdm phase list"));
        assert!(content.contains("rdm phase show"));
        assert!(content.contains("rdm phase update"));
        // Task flow commands.
        assert!(content.contains("rdm task list"));
        assert!(content.contains("rdm task show"));
        assert!(content.contains("rdm task update"));
        assert!(content.contains("rdm task create"));
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
    fn skill_do_has_write_edit_tools() {
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
    fn skill_do_has_plan_mode_tools() {
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
    fn skill_do_uses_plan_mode_workflow() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[1].content;
        assert!(content.contains("Enter plan mode"));
        assert!(content.contains("implementation plan"));
    }

    #[test]
    fn skill_do_runs_implementation_plan_review() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[1].content;
        assert!(content.contains("rdm-plan-review"));
        assert!(content.contains("--implementation-plan"));
    }

    #[test]
    fn mcp_skill_do_runs_implementation_plan_review() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let content = &skills[1].content;
        assert!(content.contains("rdm-plan-review"));
        assert!(content.contains("--implementation-plan"));
    }

    #[test]
    fn skill_do_implementation_plan_review_precedes_approval_gate() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[1].content;
        let plan_pos = content
            .find("Enter plan mode")
            .expect("missing Enter plan mode step");
        let review_pos = content
            .find("rdm-plan-review")
            .expect("missing rdm-plan-review reference");
        let approval_pos = content
            .find("Wait for user approval")
            .expect("missing Wait for user approval step");
        assert!(
            plan_pos < review_pos,
            "implementation-plan review step should come after drafting the plan"
        );
        assert!(
            review_pos < approval_pos,
            "implementation-plan review step should come before the approval gate"
        );
    }

    #[test]
    fn skill_do_auto_mode_folds_blocking_findings() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[1].content;
        assert!(content.contains("--auto"));
        assert!(content.contains("blocking"));
        assert!(content.contains("fold every surviving"));
        assert!(content.contains("plan-review"));
    }

    #[test]
    fn mcp_skill_do_auto_mode_folds_blocking_findings() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let content = &skills[1].content;
        assert!(content.contains("--auto"));
        assert!(content.contains("blocking"));
        assert!(content.contains("fold every surviving"));
        assert!(content.contains("plan-review"));
    }

    #[test]
    fn skill_do_has_agent_tool() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let frontmatter = skills[1]
            .content
            .split("---")
            .nth(1)
            .expect("missing frontmatter");
        assert!(frontmatter.contains("Agent"));
    }

    #[test]
    fn mcp_skill_do_has_agent_tool() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let frontmatter = skills[1]
            .content
            .split("---")
            .nth(1)
            .expect("missing frontmatter");
        assert!(frontmatter.contains("Agent"));
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
    fn skill_roadmap_notes_plan_review_gate() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[0].content;
        assert!(content.contains("needs-plan-review"));
        assert!(content.contains("plan_review"));
    }

    #[test]
    fn skill_review_contains_rdm_commands() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[2].content;
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
        assert!(skills[2].content.contains("name: rdm-review"));
    }

    #[test]
    fn skill_review_has_agent_tool() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[2].content;
        assert!(content.contains("Agent"));
    }

    #[test]
    fn skill_review_contains_arguments_variable() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        assert!(skills[2].content.contains("$ARGUMENTS"));
    }

    #[test]
    fn skill_review_categorizes_findings() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[2].content;
        // Small findings are fixed inline and amended; large findings are filed as tasks.
        assert!(content.contains("git commit --amend"));
        assert!(content.contains("rdm task create"));
        assert!(content.contains("Small"));
        assert!(content.contains("Large"));
    }

    #[test]
    fn skill_review_transitions_to_reviewed() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[2].content;
        // The three canonical outcomes and the statuses they map to.
        assert!(content.contains("reviewed"));
        assert!(content.contains("rework"));
        assert!(content.contains("escalated"));
        assert!(content.contains("`in-progress`"));
        assert!(content.contains("`blocked`"));
        // The completion trailer is never hand-typed: it is sourced from rdm.
        assert!(content.contains("rdm hook done-line"));
        assert!(content.contains("git commit --amend"));
    }

    #[test]
    fn skill_review_dispatches_adaptive_fleet() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[2].content;
        assert!(content.contains("fleet"));
        // Every canonical dimension is documented, including the new security one.
        for dim in [
            "**ac**",
            "**correctness**",
            "**tests**",
            "**architecture**",
            "**api-docs**",
            "**changelog**",
            "**security**",
        ] {
            assert!(content.contains(dim), "missing dimension: {dim}");
        }
        // Conditional dimensions are gated on per-dimension triggers.
        assert!(content.contains("trigger"));
        // The fleet agents review without editing.
        assert!(content.contains("read-only"));
    }

    #[test]
    fn skill_review_has_refute_pass() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[2].content;
        assert!(content.contains("Refute"));
        // The refuter returns a refuted boolean plus a corrected confidence.
        assert!(content.contains("`refuted` (boolean)"));
        // The finder is never the refuter.
        assert!(content.contains("never the agent that confirms it"));
        // Post-refutation confidence floor.
        assert!(content.contains("below **70**"));
    }

    #[test]
    fn skill_review_acts_only_on_verified() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[2].content;
        assert!(content.contains("only on verified"));
        assert!(content.contains("Never fix or file an unverified"));
    }

    #[test]
    fn skill_review_mcp_dispatches_adaptive_fleet() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let content = &skills[2].content;
        assert!(content.contains("fleet"));
        for dim in [
            "**ac**",
            "**correctness**",
            "**tests**",
            "**architecture**",
            "**api-docs**",
            "**changelog**",
            "**security**",
        ] {
            assert!(content.contains(dim), "missing dimension: {dim}");
        }
        assert!(content.contains("trigger"));
        assert!(content.contains("read-only"));
    }

    #[test]
    fn skill_review_mcp_has_refute_pass() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let content = &skills[2].content;
        assert!(content.contains("Refute"));
        assert!(content.contains("`refuted` (boolean)"));
        assert!(content.contains("never the agent that confirms it"));
        assert!(content.contains("below **70**"));
    }

    #[test]
    fn skill_review_mcp_acts_only_on_verified() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let content = &skills[2].content;
        assert!(content.contains("only on verified"));
        assert!(content.contains("Never fix or file an unverified"));
    }

    #[test]
    fn skill_review_has_blocked_verdict() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[2].content;
        // Severity scale drives the verdict.
        assert!(content.contains("Severity scale"));
        // The retired quartet is gone; the canonical trio has a strict order.
        assert!(!content.contains("PASS WITH CONCERNS"));
        assert!(!content.contains("**BLOCKED**"));
        assert!(content.contains("**escalated**"));
        assert!(content.contains("**rework**"));
        assert!(content.contains("**reviewed**"));
        assert!(content.contains("the first matching rule wins"));
        // Escalation maps to `blocked` for BOTH item kinds, with a [code] reason.
        assert!(content.contains("| **escalated** |"));
        assert!(!content.contains("tasks have no `blocked` status"));
        assert!(content.contains("`blocked` is a valid task status"));
        assert!(content.contains("[code]"));
    }

    #[test]
    fn skill_review_mcp_has_blocked_verdict() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let content = &skills[2].content;
        assert!(content.contains("Severity scale"));
        assert!(!content.contains("PASS WITH CONCERNS"));
        assert!(!content.contains("**BLOCKED**"));
        assert!(content.contains("**escalated**"));
        assert!(content.contains("**rework**"));
        assert!(content.contains("**reviewed**"));
        assert!(content.contains("the first matching rule wins"));
        // MCP gate uses the tool call with status "blocked" for an escalation.
        assert!(content.contains("status: \"blocked\""));
        assert!(!content.contains("tasks have no `blocked` status"));
        assert!(content.contains("`blocked` is a valid task status"));
    }

    #[test]
    fn skill_review_sizes_the_fleet_via_model_policy() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[2].content;
        assert!(content.contains("rdm model resolve review-find"));
        assert!(content.contains("rdm model resolve review-verify"));
        assert!(content.contains("tier hint"));
        assert!(content.contains("small"));
        assert!(content.contains("large"));
        assert!(content.contains("never the inherited session model"));
    }

    #[test]
    fn skill_review_mcp_sizes_the_fleet_via_model_policy() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let content = &skills[2].content;
        assert!(content.contains("rdm model resolve review-find"));
        assert!(content.contains("rdm model resolve review-verify"));
        assert!(content.contains("tier hint"));
        assert!(content.contains("never the inherited session model"));
    }

    #[test]
    fn skill_document_contains_rdm_commands() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[3].content;
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
        let content = &skills[3].content;
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
        let frontmatter = skills[3]
            .content
            .split("---")
            .nth(1)
            .expect("missing frontmatter");
        assert!(!frontmatter.contains("EnterPlanMode"));
        assert!(!frontmatter.contains("ExitPlanMode"));
    }

    #[test]
    fn skill_do_finalize_runs_canonical_review() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[1].content;
        // Finalize still stamps the transient needs-review marker via the update
        // command...
        assert!(content.contains("--status needs-review"));
        // ...but it no longer PARKS there: it actively invokes the canonical
        // review (the `rdm-review` skill, the projection of the one review
        // source) as part of finalizing.
        assert!(content.contains("Immediately invoke the `rdm-review` skill"));
        // The review runs in BOTH lanes — interactive and --auto — not just one.
        assert!(content.contains("This runs in **both** modes"));
        assert!(
            content
                .contains("`--auto` (which skips only the human confirmation, never the review)")
        );
        // The completion trailer is sourced from rdm, never hand-typed...
        assert!(content.contains("rdm hook done-line"));
        assert!(content.contains("Never hand-type the completion trailer"));
        // ...so the raw format string never appears in the shipped skill.
        assert!(!content.contains("<roadmap-slug>/<phase-stem>"));
        // The stale "park it and let a hook pick it up later" framing is gone.
        assert!(!content.contains("deferred two-stage"));
        assert!(!content.contains("the sentinel that signals a review is pending"));
    }

    #[test]
    fn skill_do_uses_worktree_and_run_modes() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[1].content;
        // Work happens in one worktree per roadmap, created via the roadmap-scoped
        // worktree command (not a per-phase ref)...
        assert!(content.contains("worktree add <slug>"));
        // ...detecting the current worktree's roadmap via `worktree current`...
        assert!(content.contains("worktree current"));
        // ...and following the Match/None/Mismatch in-place flow.
        assert!(content.contains("**Match**"));
        assert!(content.contains("**None**"));
        assert!(content.contains("**Mismatch**"));
        assert!(content.contains("work in place"));
        // EnterWorktree is a one-time convenience, NOT a correctness dependency:
        // non-Claude hosts have a working cd/launch entry path.
        assert!(content.contains("EnterWorktree"));
        assert!(content.contains("not** a correctness dependency"));
        assert!(content.contains("cd`/launch"));
        // Tasks keep their own per-task worktree.
        assert!(content.contains("worktree add task/<slug>"));
        // The Mismatch branch pins the relaunch entry path and the reason
        // EnterWorktree cannot be used from inside another worktree — so a
        // regression that dropped/inverted this caveat would fail, not pass.
        assert!(content.contains("relaunch"));
        assert!(content.contains(".claude/worktrees/"));
        // ...and the skill supports a non-interactive run mode...
        assert!(content.contains("--auto"));
        // ...with unattended-permission guidance for Claude Code.
        assert!(content.contains("--permission-mode auto"));
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
    fn mcp_skills_returns_ten_files() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        assert_eq!(skills.len(), 10);
    }

    #[test]
    fn mcp_skills_correct_paths() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        assert_eq!(skills[0].relative_path, "rdm-roadmap/SKILL.md");
        assert_eq!(skills[1].relative_path, "rdm-do/SKILL.md");
        assert_eq!(skills[2].relative_path, "rdm-review/SKILL.md");
        assert_eq!(skills[3].relative_path, "rdm-document/SKILL.md");
        assert_eq!(skills[4].relative_path, "rdm-estimate/SKILL.md");
        assert_eq!(skills[5].relative_path, "rdm-dispatch-phase/SKILL.md");
        assert_eq!(skills[6].relative_path, "rdm-autopilot/SKILL.md");
        assert_eq!(skills[7].relative_path, "rdm-land/SKILL.md");
        assert_eq!(skills[8].relative_path, "rdm-revise/SKILL.md");
        assert_eq!(skills[9].relative_path, "rdm-plan-review/SKILL.md");
    }

    #[test]
    fn mcp_skill_land_uses_mcp_tools() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let content = &skills[7].content;
        assert!(content.contains("name: rdm-land"));
        assert!(content.contains("$ARGUMENTS"));
        // Same landing contract as the CLI variant.
        assert!(content.contains("merge --ff-only"));
        assert!(content.contains("linear history"));
        assert!(content.contains("reviewed"));
        assert!(content.contains("Done:"));
        assert!(content.contains("reviewed → done"));
        assert!(content.contains("git rebase --abort"));
        assert!(content.contains("docs/escalation-protocol.md"));
        assert!(content.contains("never auto-lands"));
        // Status reads/updates and single-worktree cleanup go through MCP tools.
        assert!(content.contains("rdm_phase_show"));
        assert!(content.contains("rdm_phase_update"));
        assert!(content.contains("rdm_worktree_remove"));
        // Batch prune has no MCP tool this phase — delegated as the CLI command.
        assert!(content.contains("rdm worktree prune"));
        // The git landing is delegated to a Bash-capable subagent via Agent.
        assert!(content.contains("Agent"));
        assert!(content.contains("subagent"));
        // MCP variant: Bash-free frontmatter, mcp__rdm__ tools resolved.
        let frontmatter = content.split("---").nth(1).expect("missing frontmatter");
        assert!(!frontmatter.contains("  - Bash"));
        assert!(frontmatter.contains("mcp__rdm__rdm_phase_show"));
        assert!(frontmatter.contains("mcp__rdm__rdm_worktree_remove"));
    }

    #[test]
    fn mcp_skill_autopilot_uses_mcp_tools() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let content = &skills[6].content;
        assert!(content.contains("name: rdm-autopilot"));
        // Drives one named roadmap; the slug is required and the loop never roams.
        assert!(content.contains("required roadmap slug"));
        assert!(content.contains("never roams to another roadmap"));
        // The loop driver is the MCP `rdm_next` tool, named in body and frontmatter.
        assert!(content.contains("rdm_next"));
        assert!(content.contains("blocked-on-dependencies"));
        // Composes the per-phase skills.
        assert!(content.contains("rdm-dispatch-phase"));
        assert!(content.contains("rdm-estimate"));
        // Interprets the three dispatch outcomes.
        assert!(content.contains("reviewed | rework | escalated"));
        assert!(content.contains("**reviewed**"));
        assert!(content.contains("**rework**"));
        assert!(content.contains("**escalated**"));
        // Rework exhaustion parks the phase blocked via the MCP update tool.
        assert!(content.contains("rdm_phase_update"));
        assert!(content.contains("status: \"blocked\""));
        assert!(content.contains("[code]"));
        // Bounded run + summary + shared escalation protocol + batch queue.
        assert!(content.contains("Global step budget"));
        assert!(content.contains("Summary"));
        assert!(content.contains("docs/escalation-protocol.md"));
        assert!(content.contains("rdm review blocked"));
        // Opt-in landing, dry-run / bounded modes.
        assert!(content.contains("--land"));
        assert!(content.contains("Default OFF"));
        assert!(content.contains("--plan-only"));
        assert!(content.contains("--max-phases"));
        // MCP variant: Bash-free frontmatter, mcp__rdm__ tools resolved.
        let frontmatter = content.split("---").nth(1).expect("missing frontmatter");
        assert!(!frontmatter.contains("  - Bash"));
        assert!(frontmatter.contains("mcp__rdm__rdm_next"));
        assert!(frontmatter.contains("mcp__rdm__rdm_phase_update"));
        // Mandatory dispatch enforcement: non-skippable MUST, inline-collapse
        // negative example, self-check gate, and consistency with phase 1's
        // synchronous-dispatch contract.
        assert!(content.contains("Mandatory dispatch"));
        assert!(content.contains("inline-collapse"));
        assert!(content.contains("MUST NOT"));
        assert!(content.contains("Self-check before proceeding"));
        assert!(content.contains("dispatched synchronously"));
        assert!(content.contains("SendMessage"));
    }

    #[test]
    fn mcp_skill_dispatch_phase_uses_mcp_tools_and_plan_gate() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let content = &skills[5].content;
        assert!(content.contains("name: rdm-dispatch-phase"));
        // The MCP variant drives the worktree and phase status via MCP tools,
        // not Bash — body references them and frontmatter lists the resolved names.
        assert!(content.contains("rdm_worktree_add"));
        assert!(content.contains("rdm_phase_show"));
        assert!(content.contains("rdm_phase_update"));
        // One worktree per roadmap: the worktree-add tool gets the bare roadmap ref,
        // not a per-phase `<slug>/<phase-stem>` item.
        assert!(content.contains("item: \"<slug>\""));
        assert!(!content.contains("item: \"<slug>/<phase-stem>\""));
        let frontmatter = content.split("---").nth(1).expect("missing frontmatter");
        assert!(frontmatter.contains("mcp__rdm__rdm_worktree_add"));
        assert!(frontmatter.contains("mcp__rdm__rdm_phase_update"));
        assert!(!frontmatter.contains("  - Bash"));
        // Same bounded, independent plan gate and structured outcome as the CLI variant.
        assert!(content.contains("Plan gate"));
        assert!(content.contains("at most one revise round"));
        assert!(content.contains("reviewed | rework | escalated"));
        assert!(content.contains("\"blocked\""));
        assert!(content.contains("rdm-review"));
        // Plan gate rigor scales with the phase's difficulty tier.
        assert!(content.contains("Scale the reviewer's rigor to the phase's difficulty tier"));
        assert!(content.contains("Trivial/easy"));
        assert!(content.contains("Moderate"));
        assert!(content.contains("per-finding evidence"));
        assert!(content.contains("refute pass"));
        // Sharpened checklist: AC->step, AC->test, edge cases, and declared
        // cross-phase/cross-crate dependencies, in addition to scope/architecture.
        assert!(content.contains("Acceptance criteria → steps"));
        assert!(content.contains("Acceptance criteria → tests"));
        assert!(content.contains("test-per-AC"));
        assert!(content.contains("Edge cases / error paths"));
        assert!(content.contains("Cross-phase / cross-crate dependencies"));
        // Revise round re-checks the revised plan; an unconverged revise escalates
        // rather than silently proceeding with a deficient plan.
        assert!(content.contains("re-checks the revised plan against the same checklist"));
        assert!(content.contains("exhausted plan-revise budget"));
        assert!(!content.contains("a single time, then proceeds"));
        // Escalation follows the shared protocol: references the protocol doc
        // and records a stage-tagged reason via the MCP tool's `reason` param.
        assert!(content.contains("docs/escalation-protocol.md"));
        assert!(content.contains("reason:"));
        assert!(content.contains("[plan]"));
        assert!(content.contains("[code]"));
        // Mandatory dispatch enforcement: non-skippable MUST, inline-collapse
        // negative example, self-check gate, and consistency with phase 1's
        // synchronous-dispatch contract.
        assert!(content.contains("Mandatory dispatch"));
        assert!(content.contains("inline-collapse"));
        assert!(content.contains("MUST NOT"));
        assert!(content.contains("Self-check before proceeding"));
        assert!(content.contains("dispatched synchronously"));
        assert!(content.contains("SendMessage"));
    }

    #[test]
    fn mcp_skill_estimate_names_itself_and_uses_phase_update() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let content = &skills[4].content;
        assert!(content.contains("name: rdm-estimate"));
        assert!(content.contains("rdm_phase_update"));
        // MCP skill must not list Bash in allowed-tools.
        let frontmatter = content.split("---").nth(1).expect("missing frontmatter");
        assert!(!frontmatter.contains("Bash"));
        assert!(frontmatter.contains("mcp__rdm__rdm_phase_update"));
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
        // Do skill should reference both MCP phase and task tools
        assert!(skills[1].content.contains("rdm_phase_list"));
        assert!(skills[1].content.contains("rdm_phase_show"));
        assert!(skills[1].content.contains("rdm_phase_update"));
        assert!(skills[1].content.contains("rdm_task_list"));
        assert!(skills[1].content.contains("rdm_task_show"));
        assert!(skills[1].content.contains("rdm_task_update"));
        // ...and the worktree tool that isolates its work.
        assert!(skills[1].content.contains("rdm_worktree_add"));
    }

    #[test]
    fn mcp_skills_have_correct_names() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        assert!(skills[0].content.contains("name: rdm-roadmap"));
        assert!(skills[1].content.contains("name: rdm-do"));
        assert!(skills[2].content.contains("name: rdm-review"));
        assert!(skills[3].content.contains("name: rdm-document"));
    }

    #[test]
    fn mcp_skill_roadmap_notes_plan_review_gate() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let content = &skills[0].content;
        assert!(content.contains("needs-plan-review"));
        assert!(content.contains("plan_review"));
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
    fn mcp_skill_do_finalize_runs_canonical_review() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let content = &skills[1].content;
        // Finalize still stamps the transient needs-review marker via the update
        // tool...
        assert!(content.contains("status: \"needs-review\""));
        // ...but it no longer PARKS there: it actively invokes the canonical
        // review as part of finalizing.
        assert!(content.contains("Immediately invoke the `rdm-review` skill"));
        // The review runs in BOTH lanes — interactive and --auto — not just one.
        assert!(content.contains("This runs in **both** modes"));
        assert!(
            content
                .contains("`--auto` (which skips only the human confirmation, never the review)")
        );
        // The completion trailer is sourced from rdm, never hand-typed...
        assert!(content.contains("rdm hook done-line"));
        assert!(content.contains("Never hand-type the completion trailer"));
        // ...so the raw format string never appears in the shipped skill.
        assert!(!content.contains("<roadmap-slug>/<phase-stem>"));
        // The stale "park it and let a hook pick it up later" framing is gone.
        assert!(!content.contains("deferred two-stage"));
        assert!(!content.contains("the sentinel that signals a review is pending"));
    }

    #[test]
    fn mcp_skill_do_supports_run_modes_and_worktree() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let content = &skills[1].content;
        // The MCP variant supports the interactive/non-interactive run modes...
        assert!(content.contains("--auto"));
        assert!(content.contains("Run modes"));
        // ...and drives the one-worktree-per-roadmap, work-in-place flow via the
        // MCP worktree tools (not Bash, no EnterWorktree): it detects the current
        // worktree with rdm_worktree_current and creates/reuses the roadmap
        // worktree with rdm_worktree_add, following Match/None/Mismatch.
        assert!(content.contains("rdm_worktree_current"));
        assert!(content.contains("rdm_worktree_add"));
        assert!(content.contains("**Match**"));
        assert!(content.contains("**None**"));
        assert!(content.contains("**Mismatch**"));
        assert!(content.contains("work in place"));
        // The roadmap-scoped item ref is used, plus the per-task ref for tasks.
        assert!(content.contains("item: \"<slug>\""));
        assert!(content.contains("item: \"task/<slug>\""));
        // MCP hosts cd/open the returned path — EnterWorktree is not used here.
        assert!(!content.contains("EnterWorktree"));
        let frontmatter = content.split("---").nth(1).expect("missing frontmatter");
        assert!(frontmatter.contains("mcp__rdm__rdm_worktree_current"));
        assert!(frontmatter.contains("mcp__rdm__rdm_worktree_add"));
    }

    #[test]
    fn mcp_skill_do_implementation_plan_review_precedes_approval_gate() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let content = &skills[1].content;
        let plan_pos = content
            .find("Enter plan mode")
            .expect("missing Enter plan mode step");
        let review_pos = content
            .find("rdm-plan-review")
            .expect("missing rdm-plan-review reference");
        let approval_pos = content
            .find("Wait for user approval")
            .expect("missing Wait for user approval step");
        assert!(
            plan_pos < review_pos,
            "implementation-plan review step should come after drafting the plan"
        );
        assert!(
            review_pos < approval_pos,
            "implementation-plan review step should come before the approval gate"
        );
    }

    #[test]
    fn mcp_skill_review_categorizes_findings() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let content = &skills[2].content;
        // Large findings are filed as tasks via the MCP create tool.
        assert!(content.contains("rdm_task_create"));
        assert!(content.contains("Small"));
        assert!(content.contains("Large"));
        assert!(content.contains("git commit --amend"));
    }

    #[test]
    fn mcp_skill_review_drives_transition() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let content = &skills[2].content;
        // Transition is driven via the MCP update tools; the status comes from
        // the generated outcome->status mapping (reviewed|in-progress|blocked).
        assert!(content.contains("rdm_phase_update"));
        assert!(content.contains("rdm_task_update"));
        assert!(content.contains("status: \"<status>\""));
        assert!(content.contains("`reviewed`, `in-progress`, or `blocked`"));
        // allowed-tools frontmatter lists the new MCP tools.
        let frontmatter = content.split("---").nth(1).expect("missing frontmatter");
        assert!(frontmatter.contains("mcp__rdm__rdm_phase_update"));
        assert!(frontmatter.contains("mcp__rdm__rdm_task_update"));
    }

    /// Both review skill variants share the generated review specification:
    /// the canonical outcome vocabulary, the seven-dimension fleet including
    /// the new `security` dimension, and a completion trailer sourced from
    /// `rdm hook done-line` rather than a hand-typed format string.
    #[test]
    fn skill_review_shares_the_generated_spec_across_variants() {
        for mcp in [false, true] {
            let skills = generate_skills(&SkillOptions {
                project: None,
                principles_file: None,
                mcp,
            });
            let content = &skills[2].content;
            for needle in [
                "**security**",
                "**reviewed**",
                "**rework**",
                "**escalated**",
                "rdm hook done-line",
                "`blocked` is a valid task status",
            ] {
                assert!(content.contains(needle), "mcp={mcp}: missing {needle}");
            }
            for retired in ["PASS WITH CONCERNS", "**BLOCKED**", "**FAIL**"] {
                assert!(
                    !content.contains(retired),
                    "mcp={mcp}: retired verdict word still present: {retired}"
                );
            }
        }
    }

    /// Every review skill variant carries the dimension/severity/verdict
    /// *definitions* solely inside `scripts/gen-skill-review.sh`'s generated
    /// `<!-- rdm:review-spec:begin -->` / `<!-- rdm:review-spec:end -->` span.
    /// Definitional sentences drawn from the generated block must not also be
    /// hand-duplicated in the surrounding Setup/Report/Act/Gate narrative —
    /// that narrative may still *reference* a term (e.g. a `--status
    /// reviewed` command literal, or "per the verdict below"), which is why
    /// this checks full defining phrases rather than the bare dimension /
    /// severity / verdict words, which legitimately recur outside the block.
    #[test]
    fn skill_review_spec_confined_to_generated_block() {
        const BEGIN: &str = "<!-- rdm:review-spec:begin";
        const END: &str = "<!-- rdm:review-spec:end -->";
        // Full definitional phrases lifted verbatim from the generated block:
        // the severity scale's three definitions, the dimensions intro, and
        // the verdict decision rule. A bare word like "blocking" or
        // "escalated" is *not* included here — those legitimately recur as
        // command literals / status values in the Gate section outside the
        // block.
        const DEFINITIONAL_PHRASES: &[&str] = &[
            // Severity scale definitions.
            "the work must not advance as-is",
            "recorded but non-gating",
            "minor optional improvement",
            // Dimensions intro (defines what "always-on"/"triggered" mean).
            "Scale the fleet to what the change actually touches",
            // Verdict decision rule.
            "Determine the outcome in this strict order",
            "needs a *human decision*",
        ];

        for mcp in [false, true] {
            let skills = generate_skills(&SkillOptions {
                project: None,
                principles_file: None,
                mcp,
            });
            let content = &skills[2].content;
            let begin = content
                .find(BEGIN)
                .unwrap_or_else(|| panic!("mcp={mcp}: missing begin marker"));
            let end = content
                .find(END)
                .unwrap_or_else(|| panic!("mcp={mcp}: missing end marker"));
            assert!(
                begin < end,
                "mcp={mcp}: begin marker must precede end marker"
            );

            let before = &content[..begin];
            let after = &content[end + END.len()..];

            for phrase in DEFINITIONAL_PHRASES {
                assert!(
                    !before.contains(phrase),
                    "mcp={mcp}: spec-definition phrase found before the generated block: {phrase:?}"
                );
                assert!(
                    !after.contains(phrase),
                    "mcp={mcp}: spec-definition phrase found after the generated block: {phrase:?}"
                );
            }
        }
    }
}

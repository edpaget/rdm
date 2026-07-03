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
/// assert_eq!(skills.len(), 8);
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

// ---------- Claude auto-review Stop hook ----------

/// The command registered in `.claude/settings.json` for the auto-review Stop hook.
///
/// This is the exact `command` string the hook entry carries; it is used both when
/// writing the registration and when checking (for idempotency) whether the hook is
/// already present.
const CLAUDE_STOP_HOOK_COMMAND: &str =
    "$CLAUDE_PROJECT_DIR/.claude/hooks/rdm-review-on-finalize.sh";

/// Returns the generalized auto-review Stop hook script for Claude Code.
///
/// The script re-prompts the agent to run `rdm-review` while any rdm item is in
/// `needs-review`. It calls `rdm` on `PATH` and relies on the standard project
/// resolution chain (`RDM_PROJECT` / `default_project`), so it is portable across
/// end-user plan repos.
///
/// # Examples
///
/// ```
/// use rdm_core::agent_config::generate_claude_stop_hook_script;
///
/// let script = generate_claude_stop_hook_script();
/// assert!(script.contains("needs-review"));
/// ```
pub fn generate_claude_stop_hook_script() -> &'static str {
    include_str!("templates/hook-review-on-finalize.sh")
}

/// The files emitted by `rdm agent-config claude --hooks`.
///
/// Describes the Stop hook script and the settings file it must be registered in,
/// each as a relative path under the target `.claude/` directory.
pub struct ClaudeHookFiles {
    /// Relative path for the hook script (`hooks/rdm-review-on-finalize.sh`).
    pub script_relative_path: &'static str,
    /// The full content of the hook script.
    pub script_content: &'static str,
    /// Relative path for the Claude Code settings file (`settings.json`).
    pub settings_relative_path: &'static str,
}

/// Returns the file plan for the Claude auto-review Stop hook.
///
/// The caller writes [`ClaudeHookFiles::script_content`] to
/// [`ClaudeHookFiles::script_relative_path`] (setting the executable bit), then merges
/// the Stop hook registration into the file at
/// [`ClaudeHookFiles::settings_relative_path`] via [`merge_stop_hook_into_settings`].
///
/// # Examples
///
/// ```
/// use rdm_core::agent_config::generate_claude_hook;
///
/// let files = generate_claude_hook();
/// assert_eq!(files.script_relative_path, "hooks/rdm-review-on-finalize.sh");
/// assert_eq!(files.settings_relative_path, "settings.json");
/// ```
pub fn generate_claude_hook() -> ClaudeHookFiles {
    ClaudeHookFiles {
        script_relative_path: "hooks/rdm-review-on-finalize.sh",
        script_content: generate_claude_stop_hook_script(),
        settings_relative_path: "settings.json",
    }
}

/// Errors that can occur while merging the Stop hook into Claude Code settings.
#[derive(Debug)]
pub enum AgentConfigError {
    /// The existing settings content was not valid JSON.
    InvalidJson(serde_json::Error),
    /// The existing settings content was valid JSON but not a JSON object.
    NotAnObject,
}

impl fmt::Display for AgentConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AgentConfigError::InvalidJson(e) => {
                write!(f, "settings.json is not valid JSON: {e}")
            }
            AgentConfigError::NotAnObject => {
                write!(f, "settings.json must be a JSON object")
            }
        }
    }
}

impl std::error::Error for AgentConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            AgentConfigError::InvalidJson(e) => Some(e),
            AgentConfigError::NotAnObject => None,
        }
    }
}

/// Merges the auto-review Stop hook registration into Claude Code settings JSON.
///
/// Given the existing `settings.json` content (or `None` to start from `{}`), returns
/// pretty-printed JSON with a `hooks.Stop` entry registering
/// `$CLAUDE_PROJECT_DIR/.claude/hooks/rdm-review-on-finalize.sh`. All other keys are
/// preserved.
///
/// The merge is idempotent: if the Stop hook command is already registered, the input
/// is returned unchanged so re-running `--hooks` never duplicates the entry.
///
/// Only top-level objecthood is enforced as an invariant. If `hooks` exists but is not
/// an object, or `hooks.Stop` exists but is not an array, that wrong-typed value is
/// rebuilt rather than preserved — such shapes are invalid under Claude Code's settings
/// schema, so no usable data is lost. Sibling keys are always preserved.
///
/// # Errors
///
/// Returns [`AgentConfigError::InvalidJson`] if `existing` is not parseable JSON, and
/// [`AgentConfigError::NotAnObject`] if it parses to a non-object value (e.g. an array
/// or scalar).
///
/// # Examples
///
/// ```
/// use rdm_core::agent_config::merge_stop_hook_into_settings;
///
/// let merged = merge_stop_hook_into_settings(None).unwrap();
/// assert!(merged.contains("rdm-review-on-finalize.sh"));
/// // Idempotent: merging again changes nothing.
/// assert_eq!(merge_stop_hook_into_settings(Some(&merged)).unwrap(), merged);
/// ```
pub fn merge_stop_hook_into_settings(existing: Option<&str>) -> Result<String, AgentConfigError> {
    let mut root: serde_json::Value = match existing {
        Some(s) if !s.trim().is_empty() => {
            serde_json::from_str(s).map_err(AgentConfigError::InvalidJson)?
        }
        _ => serde_json::json!({}),
    };

    let obj = root.as_object_mut().ok_or(AgentConfigError::NotAnObject)?;

    // hooks must be an object; tolerate a pre-existing non-object by replacing? No —
    // per the error policy we only reject the top-level non-object case. Nested shapes
    // that don't match are treated as "not present" and rebuilt.
    let hooks = obj.entry("hooks").or_insert_with(|| serde_json::json!({}));
    if !hooks.is_object() {
        *hooks = serde_json::json!({});
    }
    let hooks = hooks.as_object_mut().expect("hooks is an object");

    let stop = hooks.entry("Stop").or_insert_with(|| serde_json::json!([]));
    if !stop.is_array() {
        *stop = serde_json::json!([]);
    }
    let stop = stop.as_array_mut().expect("Stop is an array");

    // Idempotency: bail out unchanged if the command is already registered anywhere
    // in the Stop matcher entries.
    let already_present = stop.iter().any(|entry| {
        entry
            .get("hooks")
            .and_then(|h| h.as_array())
            .map(|hooks| {
                hooks.iter().any(|h| {
                    h.get("command").and_then(|c| c.as_str()) == Some(CLAUDE_STOP_HOOK_COMMAND)
                })
            })
            .unwrap_or(false)
    });

    if already_present {
        // Return the input verbatim so re-running `--hooks` is a true no-op. (When
        // `existing` is None the command can never already be present, so this branch
        // only fires for real, non-empty inputs.)
        if let Some(s) = existing {
            return Ok(s.to_string());
        }
    } else {
        stop.push(serde_json::json!({
            "hooks": [
                {
                    "type": "command",
                    "command": CLAUDE_STOP_HOOK_COMMAND,
                }
            ]
        }));
    }

    serde_json::to_string_pretty(&root).map_err(AgentConfigError::InvalidJson)
}

// ---------- Pi auto-review extension ----------

/// Returns the auto-review extension script for the Pi coding agent.
///
/// The extension subscribes to Pi's `agent_end` lifecycle event and re-prompts the agent
/// to run `rdm-review` while any rdm item is in `needs-review`. It calls `rdm` on `PATH`
/// and relies on the standard project resolution chain (`RDM_PROJECT` / `default_project`),
/// so it is portable across end-user plan repos. It is the Pi analog of
/// [`generate_claude_stop_hook_script`].
///
/// # Examples
///
/// ```
/// use rdm_core::agent_config::generate_pi_extension_script;
///
/// let script = generate_pi_extension_script();
/// assert!(script.contains("agent_end"));
/// assert!(script.contains("needs-review"));
/// ```
pub fn generate_pi_extension_script() -> &'static str {
    include_str!("templates/extension-review-on-finalize.ts")
}

/// The files emitted by `rdm agent-config pi --hooks`.
///
/// Describes the auto-review extension as a relative path under the target `.pi/`
/// directory. Unlike Claude, Pi auto-discovers extensions from its `extensions/`
/// directory, so there is no settings file to register.
pub struct PiExtensionFiles {
    /// Relative path for the extension (`extensions/rdm-review.ts`).
    pub extension_relative_path: &'static str,
    /// The full content of the extension.
    pub extension_content: &'static str,
}

/// Returns the file plan for the Pi auto-review extension.
///
/// The caller writes [`PiExtensionFiles::extension_content`] to
/// [`PiExtensionFiles::extension_relative_path`]. No executable bit and no settings merge
/// are needed — the file is a TypeScript module auto-discovered by Pi.
///
/// # Examples
///
/// ```
/// use rdm_core::agent_config::generate_pi_extension;
///
/// let files = generate_pi_extension();
/// assert_eq!(files.extension_relative_path, "extensions/rdm-review.ts");
/// ```
pub fn generate_pi_extension() -> PiExtensionFiles {
    PiExtensionFiles {
        extension_relative_path: "extensions/rdm-review.ts",
        extension_content: generate_pi_extension_script(),
    }
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
    fn generate_skills_returns_eight_files() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        assert_eq!(skills.len(), 8);
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
        // Active driver vs the passive needs-review safety net.
        assert!(content.contains("active driver"));
        assert!(content.contains("passive safety net"));
        // Never writes a Done: line by hand.
        assert!(!content.contains("Done: <roadmap-slug>/<phase-stem>"));
        // Unattended-permission guidance.
        assert!(content.contains("--permission-mode auto"));
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
        assert!(content.contains("--status reviewed"));
        assert!(content.contains("Done: <roadmap-slug>/<phase-stem>"));
        assert!(content.contains("--status in-progress"));
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
        assert!(content.contains("AC Compliance"));
        assert!(content.contains("Correctness"));
        // Conditional agents are gated on per-agent triggers.
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
        assert!(content.contains("refute"));
        assert!(content.contains("confirmed | refuted | uncertain"));
        // Finder is never the verifier.
        assert!(content.contains("finder is never the verifier"));
        // Post-verification confidence filter threshold.
        assert!(content.contains("below 70"));
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
        assert!(content.contains("AC Compliance"));
        assert!(content.contains("Correctness"));
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
        assert!(content.contains("refute"));
        assert!(content.contains("confirmed | refuted | uncertain"));
        assert!(content.contains("finder is never the verifier"));
        assert!(content.contains("below 70"));
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
        // BLOCKED is one of the four verdicts and has a strict determination order.
        assert!(content.contains("**BLOCKED**"));
        assert!(content.contains("the first matching rule wins"));
        // Gate handles BLOCKED per item kind (phase → blocked, task → in-progress).
        assert!(content.contains("--status blocked"));
        assert!(content.contains("tasks have no `blocked` status"));
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
        assert!(content.contains("**BLOCKED**"));
        assert!(content.contains("the first matching rule wins"));
        // MCP gate uses the tool call with status "blocked" for phases.
        assert!(content.contains("status: \"blocked\""));
        assert!(content.contains("tasks have no `blocked` status"));
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
    fn skill_do_finalizes_into_needs_review() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: false,
        });
        let content = &skills[1].content;
        // Finalize transitions the item to needs-review via the update command...
        assert!(content.contains("--status needs-review"));
        // ...and defers the Done: line to the rdm-review skill on a passing review.
        assert!(content.contains("rdm-review"));
        assert!(!content.contains("<roadmap-slug>/<phase-stem>"));
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
    fn mcp_skills_returns_eight_files() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        assert_eq!(skills.len(), 8);
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
    fn mcp_skill_do_finalizes_into_needs_review() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
            mcp: true,
        });
        let content = &skills[1].content;
        // Finalize transitions the item to needs-review via the update tool...
        assert!(content.contains("status: \"needs-review\""));
        // ...and defers the Done: line to the rdm-review skill on a passing review.
        assert!(content.contains("rdm-review"));
        assert!(!content.contains("<roadmap-slug>/<phase-stem>"));
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
        // Transition is driven via the MCP update tools to reviewed / in-progress.
        assert!(content.contains("rdm_phase_update"));
        assert!(content.contains("rdm_task_update"));
        assert!(content.contains("\"reviewed\""));
        assert!(content.contains("\"in-progress\""));
        // allowed-tools frontmatter lists the new MCP tools.
        let frontmatter = content.split("---").nth(1).expect("missing frontmatter");
        assert!(frontmatter.contains("mcp__rdm__rdm_phase_update"));
        assert!(frontmatter.contains("mcp__rdm__rdm_task_update"));
    }

    // --- Claude auto-review Stop hook tests ---

    #[test]
    fn stop_hook_script_is_generalized() {
        let script = generate_claude_stop_hook_script();
        // Keeps the loop guard and the needs-review query approach.
        assert!(script.contains("stop_hook_active"));
        assert!(script.contains("needs-review"));
        assert!(script.contains("rdm review pending"));
        // Refreshes stale stamps before checking scope.
        assert!(script.contains("rdm review restamp"));
        // Generalized: no dev-binary path and no hard-coded project.
        assert!(
            !script.contains("--project"),
            "script should not hard-code a project"
        );
        assert!(
            !script.contains("target/debug/rdm"),
            "script should call rdm on PATH, not the dev binary"
        );
    }

    #[test]
    fn generate_claude_hook_paths() {
        let files = generate_claude_hook();
        assert_eq!(
            files.script_relative_path,
            "hooks/rdm-review-on-finalize.sh"
        );
        assert_eq!(files.settings_relative_path, "settings.json");
        assert!(files.script_content.contains("needs-review"));
    }

    #[test]
    fn merge_into_none_produces_stop_hook() {
        let merged = merge_stop_hook_into_settings(None).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&merged).unwrap();
        let cmd = parsed["hooks"]["Stop"][0]["hooks"][0]["command"]
            .as_str()
            .expect("command should be a string");
        assert_eq!(cmd, CLAUDE_STOP_HOOK_COMMAND);
    }

    #[test]
    fn merge_into_empty_object_produces_stop_hook() {
        let merged = merge_stop_hook_into_settings(Some("{}")).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&merged).unwrap();
        assert_eq!(
            parsed["hooks"]["Stop"][0]["hooks"][0]["command"],
            CLAUDE_STOP_HOOK_COMMAND
        );
    }

    #[test]
    fn merge_preserves_unrelated_keys() {
        let existing = r#"{"model":"x","hooks":{"PreToolUse":[{"matcher":"Bash"}]}}"#;
        let merged = merge_stop_hook_into_settings(Some(existing)).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&merged).unwrap();
        // Unrelated top-level key survives.
        assert_eq!(parsed["model"], "x");
        // Existing hooks bucket survives alongside the new Stop entry.
        assert_eq!(parsed["hooks"]["PreToolUse"][0]["matcher"], "Bash");
        assert_eq!(
            parsed["hooks"]["Stop"][0]["hooks"][0]["command"],
            CLAUDE_STOP_HOOK_COMMAND
        );
    }

    #[test]
    fn merge_is_idempotent() {
        let once = merge_stop_hook_into_settings(None).unwrap();
        let twice = merge_stop_hook_into_settings(Some(&once)).unwrap();
        // Re-running returns the input verbatim — no duplicate entry.
        assert_eq!(once, twice);
        let parsed: serde_json::Value = serde_json::from_str(&twice).unwrap();
        let stop = parsed["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 1, "Stop should not be duplicated");
    }

    #[test]
    fn merge_replaces_wrong_typed_nested_shapes() {
        // A top-level object with `hooks` of the wrong type: the nested shape is
        // rebuilt (not preserved), but the merge still succeeds and registers Stop.
        // Top-level objecthood is the only structural invariant we enforce.
        let merged = merge_stop_hook_into_settings(Some(r#"{"hooks":"oops"}"#)).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&merged).unwrap();
        assert_eq!(
            parsed["hooks"]["Stop"][0]["hooks"][0]["command"],
            CLAUDE_STOP_HOOK_COMMAND
        );

        // `Stop` present but not an array is likewise rebuilt into a single-entry array.
        let merged = merge_stop_hook_into_settings(Some(r#"{"hooks":{"Stop":{}}}"#)).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&merged).unwrap();
        let stop = parsed["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 1);
        assert_eq!(stop[0]["hooks"][0]["command"], CLAUDE_STOP_HOOK_COMMAND);
    }

    #[test]
    fn merge_rejects_non_object_json() {
        let err = merge_stop_hook_into_settings(Some("[1, 2, 3]")).unwrap_err();
        assert!(matches!(err, AgentConfigError::NotAnObject));
    }

    #[test]
    fn merge_rejects_invalid_json() {
        let err = merge_stop_hook_into_settings(Some("{not json")).unwrap_err();
        assert!(matches!(err, AgentConfigError::InvalidJson(_)));
    }

    // --- Pi auto-review extension tests ---

    #[test]
    fn pi_extension_script_is_generalized() {
        let script = generate_pi_extension_script();
        // Subscribes to the agent_end lifecycle event and queries needs-review via rdm.
        assert!(script.contains("agent_end"));
        assert!(script.contains("needs-review"));
        assert!(script.contains("pi.exec"));
        assert!(script.contains("sendUserMessage"));
        assert!(script.contains("export default"));
        // Refreshes stale stamps before checking scope.
        assert!(script.contains("review"));
        assert!(script.contains("restamp"));
        // Generalized: no dev-binary path and no hard-coded project.
        assert!(
            !script.contains("--project"),
            "extension should not hard-code a project"
        );
        assert!(
            !script.contains("target/debug"),
            "extension should call rdm on PATH, not the dev binary"
        );
    }

    #[test]
    fn generate_pi_extension_paths() {
        let files = generate_pi_extension();
        assert_eq!(files.extension_relative_path, "extensions/rdm-review.ts");
        assert!(files.extension_content.contains("agent_end"));
    }
}

//! Agent configuration generation for AI coding assistants.
//!
//! Generates platform-specific instruction files that teach AI agents
//! how to interact with `rdm` via its CLI.

use std::fmt;
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

/// Options for generating agent configuration.
pub struct AgentConfigOptions {
    /// Target platform.
    pub platform: Platform,
    /// Project name to embed in examples. If `None`, uses `<PROJECT>` placeholder.
    pub project: Option<String>,
    /// Optional path to a principles file to reference in generated output.
    pub principles_file: Option<String>,
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
/// });
/// assert!(content.contains("--project myproj"));
/// ```
pub fn generate_agent_config(opts: &AgentConfigOptions) -> String {
    let instructions = agent_instructions(opts.project.as_deref(), opts.principles_file.as_deref());

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
    let mut sections = vec![
        section_header(),
        section_setup(&proj_flag),
        section_discovering_work(&proj_flag),
        section_reading_details(&proj_flag),
        section_updating_status(&proj_flag),
        section_creating_items(&proj_flag),
        section_body_content(&proj_flag),
        section_planning_workflow(&proj_flag),
        section_status_transitions(),
    ];
    if let Some(path) = principles_file {
        sections.push(section_principles(path));
    }
    sections.join("\n\n")
}

fn section_header() -> String {
    "# rdm\n\nrdm is a CLI for managing project roadmaps, phases, and tasks. Use these instructions to interact with plan data exclusively through the rdm CLI.".to_string()
}

fn section_setup(proj_flag: &str) -> String {
    format!(
        "## Setup\n\nThe plan repo location is set via `RDM_ROOT` environment variable or `--root` flag. The project is specified with `{proj_flag}` (or set `RDM_PROJECT` env var, or configure `default_project` in `rdm.toml`)."
    )
}

fn section_discovering_work(proj_flag: &str) -> String {
    format!(
        r#"## Discovering work

```bash
rdm roadmap list {proj_flag}       # list all roadmaps with progress
rdm task list {proj_flag}           # list open/in-progress tasks
rdm task list {proj_flag} --status all  # list all tasks including done
```"#
    )
}

fn section_reading_details(proj_flag: &str) -> String {
    format!(
        r#"## Reading details

```bash
rdm roadmap show <slug> {proj_flag}          # show roadmap with phases and body
rdm phase list --roadmap <slug> {proj_flag}  # list phases with numbers and statuses
rdm phase show <stem-or-number> --roadmap <slug> {proj_flag}  # show phase details
rdm task show <slug> {proj_flag}             # show task details
```

Add `--no-body` to any `show` command to suppress body content when you only need metadata."#
    )
}

fn section_updating_status(proj_flag: &str) -> String {
    format!(
        r#"## Updating status

Always pass `--no-edit` to prevent the CLI from opening an interactive editor.

```bash
rdm phase update <stem-or-number> --status done --no-edit --roadmap <slug> {proj_flag}
rdm task update <slug> --status done --no-edit {proj_flag}
```"#
    )
}

fn section_creating_items(proj_flag: &str) -> String {
    format!(
        r#"## Creating items

Always pass `--no-edit` to suppress the interactive editor.

```bash
rdm roadmap create <slug> --title "Title" --body "Summary." --no-edit {proj_flag}
rdm phase create <slug> --title "Title" --number <n> --body "Details." --no-edit --roadmap <slug> {proj_flag}
rdm task create <slug> --title "Title" --body "Description." --no-edit {proj_flag}
```"#
    )
}

fn section_body_content(proj_flag: &str) -> String {
    format!(
        r#"## Body content

Use `--body` for short inline content. For multiline content, pipe via stdin:

```bash
rdm task create <slug> --title "Title" --no-edit {proj_flag} <<'EOF'
Multi-line body content goes here.

It supports full Markdown.
EOF
```

Do **not** use `--body` and stdin together — the CLI will error."#
    )
}

fn section_planning_workflow(proj_flag: &str) -> String {
    format!(
        r#"## Planning workflow

### Before starting work

Run `rdm roadmap list {proj_flag}` to see all roadmaps and their progress. Check `rdm task list {proj_flag}` for open tasks. Identify what is in-progress and what comes next before writing any code.

### Implementing a roadmap phase

1. Read the phase: `rdm phase show <stem-or-number> --roadmap <slug> {proj_flag}`
2. Plan your approach and get approval before starting
3. Implement the work described in the phase
4. Mark it done: `rdm phase update <stem-or-number> --status done --no-edit --roadmap <slug> {proj_flag}`
5. Check the next phase: `rdm phase list --roadmap <slug> {proj_flag}`

### Discovering bugs or side-work

If you encounter a bug or unrelated improvement while working on a phase, do not fix it inline. Create a task instead:

```bash
rdm task create <slug> --title "Description of the issue" --body "Details." --no-edit {proj_flag}
```

This keeps the current phase focused and ensures nothing is forgotten.

### When a task grows too complex

If a task becomes large enough to warrant multiple phases, promote it to a roadmap:

```bash
rdm promote <task-slug> --roadmap-slug <new-roadmap-slug> {proj_flag}
```"#
    )
}

fn section_status_transitions() -> String {
    r#"## Status transitions

### Phase statuses

- `not-started` → `in-progress` — work begins
- `in-progress` → `done` — work is complete
- `in-progress` → `blocked` — waiting on an external dependency
- `blocked` → `in-progress` — blocker resolved
- `done` is terminal (can be manually reverted if needed)

### Task statuses

- `open` → `in-progress` — work begins
- `in-progress` → `done` — work is complete
- `in-progress` → `wont-fix` — decided not to do
- `open` → `wont-fix` — decided not to do before starting
- `done` and `wont-fix` are terminal"#
        .to_string()
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
/// });
/// assert_eq!(skills.len(), 3);
/// assert!(skills[0].content.contains("--project myproj"));
/// ```
pub fn generate_skills(opts: &SkillOptions) -> Vec<SkillFile> {
    let proj_flag = proj_flag_str(opts.project.as_deref());
    let principles_note = opts.principles_file.as_deref().map(skill_principles_note);
    vec![
        skill_roadmap(&proj_flag, principles_note.as_deref()),
        skill_implement(&proj_flag, principles_note.as_deref()),
        skill_tasks(&proj_flag, principles_note.as_deref()),
    ]
}

fn skill_principles_note(path: &str) -> String {
    format!(
        "\n## Principles\n\nRead `{path}` before starting. It contains project conventions that should guide your work."
    )
}

fn skill_roadmap(proj_flag: &str, principles_note: Option<&str>) -> SkillFile {
    let principles = principles_note.unwrap_or("");
    SkillFile {
        relative_path: "rdm-roadmap/SKILL.md",
        content: format!(
            r#"---
name: rdm-roadmap
description: Create an rdm roadmap with phases for a topic
allowed-tools:
  - Read
  - Bash
  - Glob
  - Grep
---

Create an rdm roadmap with phases for the topic described in `$ARGUMENTS`.
{principles}
## Steps

1. **Explore the codebase** to understand the current state relevant to `$ARGUMENTS`. Read key files, search for related code, and build context.
2. **Design phases** that break the work into independently deliverable increments. Each phase should produce a working, testable result.
3. **Create the roadmap**: `rdm roadmap create <slug> --title "Title" --body "Summary." --no-edit {proj_flag}`
4. **Create each phase** with context, steps, and acceptance criteria in the body:
   ```bash
   rdm phase create <slug> --title "Phase title" --number <n> --no-edit --roadmap <roadmap-slug> {proj_flag} <<'EOF'
   ## Context
   Why this phase exists and what it builds on.

   ## Steps
   1. First step
   2. Second step

   ## Acceptance Criteria
   - [ ] Criterion one
   - [ ] Criterion two
   EOF
   ```
5. **Verify** the roadmap looks correct: `rdm roadmap show <slug> {proj_flag}`

## Guidelines

- Aim for 2–6 phases per roadmap
- Each phase should be independently deliverable and testable
- Include Context, Steps, and Acceptance Criteria in every phase body
- Order phases so each builds on the previous one
- Use clear, descriptive slugs (e.g., `add-caching`, `migrate-auth`)
"#
        ),
    }
}

fn skill_implement(proj_flag: &str, principles_note: Option<&str>) -> SkillFile {
    let principles = principles_note.unwrap_or("");
    SkillFile {
        relative_path: "rdm-implement/SKILL.md",
        content: format!(
            r#"---
name: rdm-implement
description: Implement the next phase of an rdm roadmap
allowed-tools:
  - Read
  - Bash
  - Glob
  - Grep
  - Write
  - Edit
  - EnterPlanMode
  - ExitPlanMode
---

Implement a phase from an rdm roadmap. `$ARGUMENTS` should be `<roadmap-slug> [phase-number]`.
{principles}
## Steps

1. **Parse arguments**: extract the roadmap slug and optional phase number from `$ARGUMENTS`.
2. **Find the phase**: if no phase number was given, run `rdm phase list --roadmap <slug> {proj_flag}` and pick the first `not-started` or `in-progress` phase.
3. **Read the phase**: `rdm phase show <phase> --roadmap <slug> {proj_flag}` to get full context, steps, and acceptance criteria.
4. **Mark in-progress**: `rdm phase update <phase> --status in-progress --no-edit --roadmap <slug> {proj_flag}`
5. **Enter plan mode**: use the `EnterPlanMode` tool to switch into planning mode.
6. **Create an implementation plan** using the planning tool. The plan should:
   - Break the phase into concrete implementation steps based on the phase description and acceptance criteria
   - Include a final step: "Review changes with user, commit, and mark phase done"
7. **Wait for user approval**: the user will review the plan and either accept or request changes. Do not proceed until the plan is accepted.
8. **Exit plan mode**: use the `ExitPlanMode` tool to switch back to execution mode.
9. **Execute the plan**: implement each step, following the plan and the phase's acceptance criteria.
10. **Review with user**: present a summary of the changes and ask the user to confirm they are ready to finalize.
11. **Finalize**: on user acceptance:
    - Commit the implementation changes
    - Mark the phase done: `rdm phase update <phase> --status done --no-edit --roadmap <slug> {proj_flag}`
12. **Handle side-work**: if you discover bugs or unrelated improvements, create tasks instead of fixing them inline:
    ```bash
    rdm task create <slug> --title "Description" --body "Details." --no-edit {proj_flag}
    ```
"#
        ),
    }
}

fn skill_tasks(proj_flag: &str, principles_note: Option<&str>) -> SkillFile {
    let principles = principles_note.unwrap_or("");
    SkillFile {
        relative_path: "rdm-tasks/SKILL.md",
        content: format!(
            r#"---
name: rdm-tasks
description: Work on rdm tasks
allowed-tools:
  - Read
  - Bash
  - Glob
  - Grep
  - Write
  - Edit
  - EnterPlanMode
  - ExitPlanMode
---

Work on rdm tasks. `$ARGUMENTS` is an optional task slug.
{principles}
## Steps

1. **List tasks**: `rdm task list {proj_flag}` to see open and in-progress tasks.
2. **Show details**: if a task slug was provided in `$ARGUMENTS`, run `rdm task show <slug> {proj_flag}`. Otherwise, present the task list and ask the user which task to work on.
3. **Mark in-progress**: `rdm task update <slug> --status in-progress --no-edit {proj_flag}`
4. **Enter plan mode**: use the `EnterPlanMode` tool to switch into planning mode.
5. **Create an implementation plan** using the planning tool. The plan should:
   - Break the task into concrete implementation steps based on the task description
   - Include a final step: "Review changes with user, commit, and mark task done"
6. **Wait for user approval**: the user will review the plan and either accept or request changes. Do not proceed until the plan is accepted.
7. **Exit plan mode**: use the `ExitPlanMode` tool to switch back to execution mode.
8. **Execute the plan**: implement each step, following the plan.
9. **Review with user**: present a summary of the changes and ask the user to confirm they are ready to finalize.
10. **Finalize**: on user acceptance:
    - Commit the implementation changes
    - Mark the task done: `rdm task update <slug> --status done --no-edit {proj_flag}`
"#
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
///
/// # Errors
///
/// This function does not return errors; it always produces valid JSON.
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
        });
        assert!(content.contains("--project <PROJECT>"));
    }

    #[test]
    fn generate_contains_key_sections() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::AgentsMd,
            project: None,
            principles_file: None,
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
        });
        assert!(!content.starts_with("---"));
    }

    #[test]
    fn copilot_no_mdc_frontmatter() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::Copilot,
            project: None,
            principles_file: None,
        });
        assert!(!content.starts_with("---"));
    }

    #[test]
    fn planning_workflow_section_contains_key_steps() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::AgentsMd,
            project: Some("myproj".to_string()),
            principles_file: None,
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
        });
        assert!(!content.contains("## Principles"));
    }

    // --- Skill generation tests ---

    #[test]
    fn generate_skills_returns_three_files() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
        });
        assert_eq!(skills.len(), 3);
    }

    #[test]
    fn generate_skills_correct_paths() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
        });
        assert_eq!(skills[0].relative_path, "rdm-roadmap/SKILL.md");
        assert_eq!(skills[1].relative_path, "rdm-implement/SKILL.md");
        assert_eq!(skills[2].relative_path, "rdm-tasks/SKILL.md");
    }

    #[test]
    fn skills_have_yaml_frontmatter() {
        let skills = generate_skills(&SkillOptions {
            project: None,
            principles_file: None,
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
        });
        assert!(skills[0].content.contains("name: rdm-roadmap"));
        assert!(skills[1].content.contains("name: rdm-implement"));
        assert!(skills[2].content.contains("name: rdm-tasks"));
    }

    #[test]
    fn skills_use_project_flag() {
        let skills = generate_skills(&SkillOptions {
            project: Some("myproj".to_string()),
            principles_file: None,
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
}

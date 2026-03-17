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
/// });
/// assert!(content.contains("--project myproj"));
/// ```
pub fn generate_agent_config(opts: &AgentConfigOptions) -> String {
    let instructions = agent_instructions(opts.project.as_deref());

    match opts.platform {
        Platform::Cursor => {
            format!(
                "---\ndescription: Instructions for using rdm to manage project roadmaps\nglobs:\n---\n\n{instructions}"
            )
        }
        _ => instructions,
    }
}

/// Generates the core instruction content shared across all platforms.
fn agent_instructions(project: Option<&str>) -> String {
    let proj_flag = match project {
        Some(name) => format!("--project {name}"),
        None => "--project <PROJECT>".to_string(),
    };

    format!(
        r#"# rdm

rdm is a CLI for managing project roadmaps, phases, and tasks. Use these instructions to interact with plan data exclusively through the rdm CLI.

## Setup

The plan repo location is set via `RDM_ROOT` environment variable or `--root` flag. The project is specified with `{proj_flag}` (or set `RDM_PROJECT` env var, or configure `default_project` in `rdm.toml`).

## Discovering work

```bash
rdm roadmap list {proj_flag}       # list all roadmaps with progress
rdm task list {proj_flag}           # list open/in-progress tasks
rdm task list {proj_flag} --status all  # list all tasks including done
```

## Reading details

```bash
rdm roadmap show <slug> {proj_flag}          # show roadmap with phases and body
rdm phase list --roadmap <slug> {proj_flag}  # list phases with numbers and statuses
rdm phase show <stem-or-number> --roadmap <slug> {proj_flag}  # show phase details
rdm task show <slug> {proj_flag}             # show task details
```

Add `--no-body` to any `show` command to suppress body content when you only need metadata.

## Updating status

Always pass `--no-edit` to prevent the CLI from opening an interactive editor.

```bash
rdm phase update <stem-or-number> --status done --no-edit --roadmap <slug> {proj_flag}
rdm task update <slug> --status done --no-edit {proj_flag}
```

## Creating items

Always pass `--no-edit` to suppress the interactive editor.

```bash
rdm roadmap create <slug> --title "Title" --body "Summary." --no-edit {proj_flag}
rdm phase create <slug> --title "Title" --number <n> --body "Details." --no-edit --roadmap <slug> {proj_flag}
rdm task create <slug> --title "Title" --body "Description." --no-edit {proj_flag}
```

## Body content

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
        });
        assert!(content.contains("--project myproj"));
        assert!(!content.contains("<PROJECT>"));
    }

    #[test]
    fn generate_without_project_name() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::AgentsMd,
            project: None,
        });
        assert!(content.contains("--project <PROJECT>"));
    }

    #[test]
    fn generate_contains_key_sections() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::AgentsMd,
            project: None,
        });
        assert!(content.contains("# rdm"));
        assert!(content.contains("## Setup"));
        assert!(content.contains("## Discovering work"));
        assert!(content.contains("## Reading details"));
        assert!(content.contains("## Updating status"));
        assert!(content.contains("## Creating items"));
        assert!(content.contains("## Body content"));
    }

    #[test]
    fn generate_contains_key_commands() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::AgentsMd,
            project: None,
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
        });
        assert!(!content.starts_with("---"));
    }

    #[test]
    fn copilot_no_mdc_frontmatter() {
        let content = generate_agent_config(&AgentConfigOptions {
            platform: Platform::Copilot,
            project: None,
        });
        assert!(!content.starts_with("---"));
    }
}

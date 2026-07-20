use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use rdm_core::agent_config::{self, AgentConfigOptions, McpConfigOptions, Platform, SkillOptions};

/// Generates agent configuration for AI coding assistants.
///
/// # Errors
///
/// Returns an error if the platform is unknown, an unsupported flag
/// combination is given, or any output file cannot be written.
#[allow(clippy::too_many_arguments)]
pub fn run(
    root: &Path,
    platform: String,
    project: Option<String>,
    out: Option<PathBuf>,
    principles_file: Option<String>,
    skills: bool,
    mcp: bool,
    user: bool,
) -> Result<()> {
    let platform: Platform = platform.parse().map_err(|e: String| anyhow!(e))?;

    if platform == Platform::Pi && mcp {
        bail!(
            "Pi does not support MCP natively. Use `--skills` for skill-based \
             integration, or omit `--mcp` for an AGENTS.md integration."
        );
    }

    if skills {
        write_skills(platform, project, principles_file, mcp, user, &out, root)?;
    } else {
        write_instruction(platform, project, principles_file, mcp, user, &out, root)?;
    }

    Ok(())
}

/// Creates `path`'s parent directories, writes `contents`, and prints the
/// conventional `Wrote <path>` line.
///
/// # Errors
///
/// Returns an error if creating the parent directory or writing the file
/// fails.
fn write_output(path: &Path, contents: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create directory {}", parent.display()))?;
    }
    std::fs::write(path, contents)
        .with_context(|| format!("failed to write {}", path.display()))?;
    println!("Wrote {}", path.display());
    Ok(())
}

/// Writes the `.mcp.json` file into `base_dir`, pointing at the plan `root`.
///
/// # Errors
///
/// Returns an error if the file cannot be written.
fn write_mcp_json(base_dir: &Path, root: &Path) -> Result<()> {
    let root_str = root.to_string_lossy().to_string();
    let mcp_content = agent_config::generate_mcp_config(&McpConfigOptions {
        root: Some(root_str),
    });
    let mcp_path = base_dir.join(".mcp.json");
    write_output(&mcp_path, mcp_content.as_bytes())
}

/// Writes the Claude Code / Pi skill files (and `.mcp.json` when `--mcp`).
///
/// # Errors
///
/// Returns an error if neither `--out` nor `--user` resolves an output
/// directory, the platform does not support skills, or a file write fails.
fn write_skills(
    platform: Platform,
    project: Option<String>,
    principles_file: Option<String>,
    mcp: bool,
    user: bool,
    out: &Option<PathBuf>,
    root: &Path,
) -> Result<()> {
    // Resolve the skills root and the base dir for .mcp.json.
    // --user → ~/.claude/skills or ~/.pi/agent/skills (base dir is
    // platform.user_level_dir()). --out <dir> → <dir>/<project_skills_subdir>
    // (base dir is <dir>). Other platforms reject --skills.
    let (skills_root, base_dir): (PathBuf, PathBuf) = if user {
        let root_path = platform.user_level_skills_dir().map_err(|e| anyhow!(e))?;
        let base = platform.user_level_dir().map_err(|e| anyhow!(e))?;
        (root_path, base)
    } else {
        let dir = out.as_ref().ok_or_else(|| {
            anyhow!("--skills requires --out or --user to specify the output directory")
        })?;
        let sub = platform
            .project_skills_subdir()
            .ok_or_else(|| anyhow!("--skills is only supported for the claude and pi platforms"))?;
        (dir.join(sub), dir.clone())
    };
    let skill_files = agent_config::generate_skills(&SkillOptions {
        project,
        principles_file,
        mcp,
    });
    for skill in &skill_files {
        let path = skills_root.join(skill.relative_path);
        write_output(&path, skill.content.as_bytes())?;
    }
    // When --mcp, also write .mcp.json at the project (or user-level) root.
    if mcp {
        write_mcp_json(&base_dir, root)?;
    }
    Ok(())
}

/// Writes the instruction file (and `.mcp.json` when `--mcp`), or prints the
/// instruction content to stdout when no output directory is resolved.
///
/// # Errors
///
/// Returns an error if a user-level path cannot be resolved or a file write
/// fails.
fn write_instruction(
    platform: Platform,
    project: Option<String>,
    principles_file: Option<String>,
    mcp: bool,
    user: bool,
    out: &Option<PathBuf>,
    root: &Path,
) -> Result<()> {
    // Resolve the output base dir and instruction file path. --user
    // uses the platform-aware user-level path (which handles Pi's
    // asymmetric layout); --out joins the conventional path under the
    // supplied directory. `base_dir` is where .mcp.json lands.
    let resolved: Option<(PathBuf, PathBuf)> = if user {
        let base = platform.user_level_dir().map_err(|e| anyhow!(e))?;
        let path = platform
            .user_level_instruction_path()
            .map_err(|e| anyhow!(e))?;
        Some((base, path))
    } else {
        out.as_ref()
            .map(|dir| (dir.clone(), dir.join(platform.conventional_path())))
    };

    let content = agent_config::generate_agent_config(&AgentConfigOptions {
        platform,
        project,
        principles_file,
        mcp,
    });
    if let Some((base_dir, path)) = resolved {
        write_output(&path, content.as_bytes())?;
        // When --mcp, also write .mcp.json in the output base dir
        if mcp {
            write_mcp_json(&base_dir, root)?;
        }
    } else {
        print!("{content}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_output_creates_parent_dirs_and_writes_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/deep/file.txt");
        write_output(&path, b"hello world").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"hello world");
    }

    #[test]
    fn write_output_overwrites_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file.txt");
        write_output(&path, b"first").unwrap();
        write_output(&path, b"second").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second");
    }
}

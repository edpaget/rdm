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
    hooks: bool,
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

    // Validate --hooks up front, before any files are written, so an invalid
    // combination never leaves a partially-written instruction/skills tree.
    if hooks {
        if platform != Platform::Claude && platform != Platform::Pi {
            bail!(
                "--hooks is only supported for the claude and pi platforms (claude \
                 writes a Stop hook registered in .claude/settings.json; pi writes a \
                 .pi/extensions/rdm-plan-review.ts extension)"
            );
        }
        if out.is_none() && !user {
            bail!("--hooks requires --out or --user to specify the output directory");
        }
    }

    if skills {
        write_skills(platform, project, principles_file, mcp, user, &out, root)?;
    } else {
        write_instruction(platform, project, principles_file, mcp, user, &out, root)?;
    }

    if hooks && platform == Platform::Claude {
        write_claude_hook(platform, user, &out)?;
    } else if hooks && platform == Platform::Pi {
        write_pi_extension(platform, user, &out)?;
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

/// Writes the Claude Stop-hook scripts (executable) and merges the hook
/// registration into `settings.json`.
///
/// Iterates over the plan-review hook ([`agent_config::generate_claude_plan_review_hook`]):
/// its script is written and marked executable, then its Stop hook command is folded
/// into the in-memory settings JSON via [`agent_config::merge_stop_hook_into_settings`].
/// The settings file is read from disk only once (before the first merge) and written
/// once at the end. The loop shape is kept even though there is currently only one hook
/// set, so a future additional Claude Stop hook composes into the same `settings.json`
/// non-destructively and idempotently without restructuring this function.
///
/// # Errors
///
/// Returns an error if the output directory cannot be resolved, a file
/// write/permission change fails, or either settings merge fails.
fn write_claude_hook(platform: Platform, user: bool, out: &Option<PathBuf>) -> Result<()> {
    // Platform and --out/--user were validated up front. Resolve the
    // `.claude/` directory that hosts the hook scripts and settings file.
    // --user writes under ~/.claude (which user_level_dir already resolves
    // to for claude); --out <dir> writes under <dir>/.claude so it lands
    // alongside any skills written above.
    let claude_dir: PathBuf = if user {
        platform.user_level_dir().map_err(|e| anyhow!(e))?
    } else {
        let dir = out
            .as_ref()
            .ok_or_else(|| anyhow!("internal error: --out required when not --user"))?;
        dir.join(".claude")
    };

    // Invariant: every hook set below must share the same
    // `settings_relative_path` — the loop threads one in-memory merged
    // settings string across all sets and writes a single settings file at
    // the end. A future hook targeting a different settings file would need
    // the merges grouped per path instead.
    let hook_file_sets = [agent_config::generate_claude_plan_review_hook()];
    debug_assert!(
        hook_file_sets
            .iter()
            .all(|h| h.settings_relative_path == hook_file_sets[0].settings_relative_path),
        "all Claude hook sets must merge into the same settings file"
    );

    let mut settings_path: Option<PathBuf> = None;
    let mut merged: Option<String> = None;

    for hook_files in &hook_file_sets {
        // Write the hook script and mark it executable (unix only).
        let script_path = claude_dir.join(hook_files.script_relative_path);
        if let Some(parent) = script_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create directory {}", parent.display()))?;
        }
        std::fs::write(&script_path, hook_files.script_content)
            .with_context(|| format!("failed to write {}", script_path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&script_path)
                .with_context(|| format!("failed to stat {}", script_path.display()))?
                .permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&script_path, perms).with_context(|| {
                format!("failed to set permissions on {}", script_path.display())
            })?;
        }
        println!("Wrote {}", script_path.display());

        // Merge this hook's Stop registration into settings.json (non-destructive).
        // Read the existing settings file from disk only on the first iteration;
        // subsequent iterations thread the in-memory merged string through so both
        // commands land in the same write.
        let this_settings_path = claude_dir.join(hook_files.settings_relative_path);
        let existing = match &merged {
            Some(s) => Some(s.clone()),
            None => match std::fs::read_to_string(&this_settings_path) {
                Ok(s) => Some(s),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => {
                    return Err(e).with_context(|| {
                        format!("failed to read {}", this_settings_path.display())
                    });
                }
            },
        };
        merged = Some(
            agent_config::merge_stop_hook_into_settings(
                existing.as_deref(),
                hook_files.stop_hook_command,
            )
            .with_context(|| {
                format!(
                    "failed to merge Stop hook into {}",
                    this_settings_path.display()
                )
            })?,
        );
        settings_path = Some(this_settings_path);
    }

    let settings_path = settings_path.expect("hook_file_sets is non-empty");
    let merged = merged.expect("hook_file_sets is non-empty");
    write_output(&settings_path, merged.as_bytes())
}

/// Writes the Pi `rdm-plan-review.ts` extension file.
///
/// # Errors
///
/// Returns an error if the output directory cannot be resolved or a file
/// write fails.
fn write_pi_extension(platform: Platform, user: bool, out: &Option<PathBuf>) -> Result<()> {
    // Pi auto-discovers extensions from its `extensions/` directory, so there
    // is no settings file to register and no executable bit to set — the files
    // are TypeScript modules, not directly-executed scripts. --user writes
    // under ~/.pi/agent; --out <dir> writes under <dir>/.pi alongside any
    // skills written above.
    let pi_base: PathBuf = if user {
        platform.user_level_dir().map_err(|e| anyhow!(e))?
    } else {
        let dir = out
            .as_ref()
            .ok_or_else(|| anyhow!("internal error: --out required when not --user"))?;
        dir.join(".pi")
    };

    let ext_file_sets = [agent_config::generate_pi_plan_review_extension()];
    for ext_files in &ext_file_sets {
        let ext_path = pi_base.join(ext_files.extension_relative_path);
        write_output(&ext_path, ext_files.extension_content.as_bytes())?;
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

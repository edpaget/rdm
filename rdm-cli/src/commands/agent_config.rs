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
        write_skills(
            platform,
            project,
            principles_file,
            mcp,
            user,
            &out,
            root,
            agent_config::SUPERSEDED_WORKFLOWS,
        )?;
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
/// When the target is `Platform::Claude` and the output is a project
/// directory (`--out`, not `--user`), this also emits the autonomous-lane
/// Workflow-tool scripts under `<base_dir>/.claude/workflows/`, then runs
/// superseded-workflow cleanup over that same directory
/// (`agent_config::resolve_superseded_workflows` against the shipped
/// `agent_config::SUPERSEDED_WORKFLOWS` table) and reports one line per
/// removed or skipped-as-modified file. This is intentionally narrower than
/// the skills surface:
///
/// - **Claude-only**: Pi has no Workflow-tool runtime, so `--skills` against
///   `Platform::Pi` writes only `.pi/skills`, never a workflows directory,
///   and never runs cleanup.
/// - **`--out`-only, not `--user`**: the shipped scripts hardcode this
///   repo's own `./target/debug/rdm` binary path and `--project rdm`
///   invocation (they are not yet parameterized for a downstream target
///   repo). Those values only make sense relative to a specific checked-out
///   project, so they are never written to a user-global location like
///   `~/.claude/workflows`, and cleanup never runs against `--user` either.
///
/// # Errors
///
/// Returns an error if neither `--out` nor `--user` resolves an output
/// directory, the platform does not support skills, or a file write fails.
/// A superseded-workflow removal failure is never surfaced as an error here
/// — it is reported on stdout and otherwise ignored, per
/// [`agent_config::resolve_superseded_workflows`]'s contract.
///
/// `superseded_table` is threaded through (rather than reading
/// [`agent_config::SUPERSEDED_WORKFLOWS`] directly) so tests can inject a
/// synthetic non-empty table and observe that a `Failed` cleanup outcome
/// never aborts the emit; `run` always passes the shipped production table.
#[allow(clippy::too_many_arguments)]
fn write_skills(
    platform: Platform,
    project: Option<String>,
    principles_file: Option<String>,
    mcp: bool,
    user: bool,
    out: &Option<PathBuf>,
    root: &Path,
    superseded_table: &[agent_config::SupersededWorkflow],
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
    // Workflow-tool scripts are Claude-only (Pi has no Workflow-tool runtime)
    // and --out-only (not --user; see the doc comment above for why).
    if platform == Platform::Claude && !user {
        let workflows_dir = base_dir.join(".claude/workflows");
        for workflow in agent_config::generate_workflows() {
            let path = workflows_dir.join(workflow.relative_path);
            write_output(&path, workflow.content.as_bytes())?;
        }
        // Clean up files superseded by an earlier emission of this same
        // lane. Reported, never fatal: a removal failure must not abort an
        // otherwise-successful emit (see the decision function's contract).
        for outcome in agent_config::resolve_superseded_workflows(&workflows_dir, superseded_table)
        {
            match outcome {
                agent_config::SupersededOutcome::Removed { path } => {
                    println!("Removed {}", path.display());
                }
                agent_config::SupersededOutcome::SkippedModified { path } => {
                    println!(
                        "Skipped {} (content modified since emission; left in place)",
                        path.display()
                    );
                }
                agent_config::SupersededOutcome::Failed { path, error } => {
                    println!("Failed to remove {}: {error}", path.display());
                }
                agent_config::SupersededOutcome::InvalidName { name } => {
                    println!(
                        "Skipped {name} (invalid superseded-workflow table entry name; not a bare filename)"
                    );
                }
            }
        }
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

    /// A `Failed` superseded-workflow cleanup outcome (e.g. a permission
    /// error on the unlink) must never abort the emit: `write_skills` still
    /// returns `Ok(())`, and the newly emitted workflow files still land on
    /// disk with their generated content. This exercises `write_skills`
    /// end-to-end (not just `resolve_superseded_workflows` in isolation) by
    /// injecting a synthetic table via the `superseded_table` parameter, per
    /// this phase's code-review finding that the prior pass only ever
    /// verified the failure-capture behavior of the decision function on its
    /// own.
    #[test]
    #[cfg(unix)]
    fn write_skills_failed_cleanup_does_not_abort_emit() {
        use std::os::unix::fs::PermissionsExt;

        let out_dir = tempfile::tempdir().unwrap();
        let out = out_dir.path().to_path_buf();
        let workflows_dir = out.join(".claude/workflows");
        std::fs::create_dir_all(&workflows_dir).unwrap();

        // A stale file whose content matches a known fingerprint in the
        // synthetic table below (SHA-256 of the exact bytes written here,
        // precomputed offline).
        let stale_content = b"stale-old-content-for-cli-failed-cleanup-test\n";
        let stale_path = workflows_dir.join("stale-old.js");
        std::fs::write(&stale_path, stale_content).unwrap();
        const STALE_DIGEST: &str =
            "817fb92cc9636eafeca8496fbceee4c9523cda22174939637ee7067b01370298";

        // Pre-seed the conventionally-named workflow files too (arbitrary
        // placeholder content) so `write_skills`'s overwrite of them below
        // only needs write permission on each *file*, not on the directory
        // — matching how a real re-emission encounters files that already
        // exist. Only a brand-new file name would need directory write
        // permission to create.
        for workflow in agent_config::generate_workflows() {
            std::fs::write(workflows_dir.join(workflow.relative_path), b"placeholder").unwrap();
        }

        // Removing a file requires write+execute on its parent directory;
        // make workflows_dir read-only-and-traversable so the cleanup's
        // unlink fails with EACCES/EPERM regardless of the file's own mode
        // (same technique as rdm-core's
        // resolve_superseded_workflows_reports_failed_removal_without_erroring).
        std::fs::set_permissions(&workflows_dir, std::fs::Permissions::from_mode(0o555)).unwrap();

        let table = [agent_config::SupersededWorkflow {
            name: "stale-old.js",
            fingerprints: &[STALE_DIGEST],
            successor: None,
        }];

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            write_skills(
                Platform::Claude,
                Some("distro-check".to_string()),
                None,
                false,
                false,
                &Some(out.clone()),
                Path::new("."),
                &table,
            )
        }));

        // Restore permissions unconditionally so the tempdir can be cleaned
        // up regardless of whether the assertions below pass or fail.
        std::fs::set_permissions(&workflows_dir, std::fs::Permissions::from_mode(0o755)).unwrap();

        let write_result = result.expect("write_skills must not panic");
        assert!(
            write_result.is_ok(),
            "a Failed cleanup outcome must not abort the emit: {write_result:?}"
        );

        // The stale file survives (removal failed) ...
        assert!(
            stale_path.exists(),
            "unlink failed, so the stale file must still be present"
        );
        // ... and the newly emitted workflow files still landed with their
        // generated content, proving the primary emit is unaffected.
        for workflow in agent_config::generate_workflows() {
            let written =
                std::fs::read(workflows_dir.join(workflow.relative_path)).unwrap_or_else(|e| {
                    panic!("expected {} to be written: {e}", workflow.relative_path)
                });
            assert_eq!(
                written,
                workflow.content.as_bytes(),
                "{} must be rewritten with generated content despite the Failed cleanup outcome",
                workflow.relative_path
            );
        }
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

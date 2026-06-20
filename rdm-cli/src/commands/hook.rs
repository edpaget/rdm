use std::path::Path;

use anyhow::{Context, Result, bail};

use super::{run_post_commit_hook, run_post_merge_hook};
use crate::HookCommand;

pub fn run(command: HookCommand, root: &Path, staging: bool) -> Result<()> {
    // Hooks are automated plumbing: the PostMerge / PostCommit arms below force
    // staging off so the Done: status updates they apply always land as a
    // commit, regardless of the user's resolved staging preference (--stage /
    // RDM_STAGE / stage = true). Staging is a human "batch my edits" affordance
    // that makes no sense for an automated hook — honoring it would leave the
    // update written-but-uncommitted, silently losing the Done: directive.
    let _ = staging;
    match command {
        HookCommand::Install { force } => {
            let cwd = std::env::current_dir().context("cannot determine current directory")?;
            let hooks_dir = rdm_store_git::discover_hooks_dir(&cwd)
                .context("current directory is not inside a git repository")?;
            std::fs::create_dir_all(&hooks_dir).context("failed to create hooks directory")?;

            let hooks: &[(&str, &str)] = &[
                (
                    "post-merge",
                    "#!/usr/bin/env bash\nGIT_DIR_PATH=$(git rev-parse --git-dir 2>/dev/null)\nLOG=\"${GIT_DIR_PATH:-.git}/rdm-hook.log\"\n{ rdm hook post-merge; } >>\"$LOG\" 2>&1 || true\n",
                ),
                (
                    "post-commit",
                    "#!/usr/bin/env bash\nGIT_DIR_PATH=$(git rev-parse --git-dir 2>/dev/null)\nLOG=\"${GIT_DIR_PATH:-.git}/rdm-hook.log\"\n{ rdm hook post-commit; } >>\"$LOG\" 2>&1 || true\n",
                ),
            ];
            for (name, shim) in hooks {
                let hook_path = hooks_dir.join(name);
                if hook_path.exists() && !force {
                    bail!(
                        "{name} hook already exists at {}; use --force to overwrite",
                        hook_path.display()
                    );
                }
                std::fs::write(&hook_path, shim)
                    .with_context(|| format!("failed to write {name} hook"))?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(&hook_path, std::fs::Permissions::from_mode(0o755))
                        .with_context(|| format!("failed to set {name} hook permissions"))?;
                }
                println!("Installed {name} hook at {}", hook_path.display());
            }
            println!(
                "Hook diagnostics will be appended to <git_dir>/rdm-hook.log on every invocation."
            );
        }
        HookCommand::Uninstall => {
            let cwd = std::env::current_dir().context("cannot determine current directory")?;
            let hooks_dir = rdm_store_git::discover_hooks_dir(&cwd)
                .context("current directory is not inside a git repository")?;

            let mut removed_any = false;
            for name in &["post-merge", "post-commit"] {
                let hook_path = hooks_dir.join(name);
                if !hook_path.exists() {
                    continue;
                }
                let contents = std::fs::read_to_string(&hook_path)
                    .with_context(|| format!("failed to read {name} hook"))?;
                let marker = format!("rdm hook {name}");
                if !contents.contains(&marker) {
                    bail!(
                        "{name} hook at {} was not installed by rdm; refusing to remove",
                        hook_path.display()
                    );
                }
                std::fs::remove_file(&hook_path)
                    .with_context(|| format!("failed to remove {name} hook"))?;
                println!("Removed {name} hook at {}", hook_path.display());
                removed_any = true;
            }
            if !removed_any {
                bail!("no rdm hooks found in {}", hooks_dir.display());
            }
        }
        HookCommand::PostMerge { since } => {
            // Capture errors so we can log them, but never propagate — the hook
            // must always exit 0 to avoid blocking git.
            if let Err(err) = run_post_merge_hook(root, false, since.as_deref()) {
                let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
                let logger = crate::hook_log::HookLogger::new(&cwd);
                let msg = format!("{err:#}");
                logger.log("post-merge", "wrapper-error", &[("error", msg.as_str())]);
            }
        }
        HookCommand::PostCommit => {
            if let Err(err) = run_post_commit_hook(root, false) {
                let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
                let logger = crate::hook_log::HookLogger::new(&cwd);
                let msg = format!("{err:#}");
                logger.log("post-commit", "wrapper-error", &[("error", msg.as_str())]);
            }
        }
    }
    Ok(())
}

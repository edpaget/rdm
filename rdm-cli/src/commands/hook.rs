use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail};

use super::{
    resolve_hook_timeout_secs, run_hook_with_timeout, run_post_commit_hook, run_post_merge_hook,
};
use crate::HookCommand;

pub fn run(command: HookCommand, root: &Path) -> Result<()> {
    match command {
        HookCommand::Install { force } => {
            let cwd = std::env::current_dir().context("cannot determine current directory")?;
            let hooks_dir = rdm_git::discover_hooks_dir(&cwd)
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
            let hooks_dir = rdm_git::discover_hooks_dir(&cwd)
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
            // must always exit 0 to avoid blocking git. Execution is also
            // bounded by `run_hook_with_timeout` (AC1): a hook body that hangs
            // past its configured deadline is abandoned rather than allowed
            // to block the invoking `git merge` indefinitely.
            let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            let logger = crate::hook_log::HookLogger::new(&cwd);
            let timeout = Duration::from_secs(resolve_hook_timeout_secs(root));
            let root = root.to_path_buf();
            if let Err(err) = run_hook_with_timeout(timeout, &logger, "post-merge", move || {
                run_post_merge_hook(&root, since.as_deref())
            }) {
                let msg = format!("{err:#}");
                logger.log("post-merge", "wrapper-error", &[("error", msg.as_str())]);
            }
        }
        HookCommand::PostCommit => {
            let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            let logger = crate::hook_log::HookLogger::new(&cwd);
            let timeout = Duration::from_secs(resolve_hook_timeout_secs(root));
            let root = root.to_path_buf();
            if let Err(err) = run_hook_with_timeout(timeout, &logger, "post-commit", move || {
                run_post_commit_hook(&root)
            }) {
                let msg = format!("{err:#}");
                logger.log("post-commit", "wrapper-error", &[("error", msg.as_str())]);
            }
        }
    }
    Ok(())
}

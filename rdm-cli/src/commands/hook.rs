use std::path::Path;

use anyhow::{Context, Result, bail};

use super::{run_post_commit_hook, run_post_merge_hook};
use crate::HookCommand;

pub fn run(command: HookCommand, root: &Path, staging: bool) -> Result<()> {
    match command {
        HookCommand::Install { force } => {
            let cwd = std::env::current_dir().context("cannot determine current directory")?;
            let hooks_dir = rdm_store_git::discover_hooks_dir(&cwd)
                .context("current directory is not inside a git repository")?;
            std::fs::create_dir_all(&hooks_dir).context("failed to create hooks directory")?;

            let hooks: &[(&str, &str)] = &[
                (
                    "post-merge",
                    "#!/usr/bin/env bash\nrdm hook post-merge 2>/dev/null || true\n",
                ),
                (
                    "post-commit",
                    "#!/usr/bin/env bash\nrdm hook post-commit 2>/dev/null || true\n",
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
            // Silently swallow all errors — hook must never fail.
            let _ = run_post_merge_hook(root, staging, since.as_deref());
        }
        HookCommand::PostCommit => {
            // Silently swallow all errors — hook must never fail.
            let _ = run_post_commit_hook(root, staging);
        }
    }
    Ok(())
}

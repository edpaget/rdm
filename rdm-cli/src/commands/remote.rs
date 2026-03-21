use std::path::Path;
use std::process;

use anyhow::{Context, Result};
use rdm_core::config::Config;
use rdm_core::repo::PlanRepo;

use super::make_store;
use crate::paths;
use crate::{AppStore, RemoteCommand};

pub fn run(
    command: RemoteCommand,
    store: &mut AppStore,
    root: &Path,
    repo_config: &Config,
    staging: bool,
) -> Result<()> {
    match command {
        RemoteCommand::Add { name, url } => {
            store
                .git_remote_add(&name, &url)
                .context("failed to add remote")?;
            println!("Added remote '{name}' ({url})");
        }
        RemoteCommand::Remove { name } => {
            store
                .git_remote_remove(&name)
                .context("failed to remove remote")?;
            println!("Removed remote '{name}'");
        }
        RemoteCommand::List => {
            let remotes = store.git_remote_list().context("failed to list remotes")?;
            if remotes.is_empty() {
                println!("No remotes configured.");
            } else {
                for r in &remotes {
                    println!("{}\t{}", r.name, r.url);
                }
            }
        }
        RemoteCommand::Fetch { name } => {
            let remote_name = paths::resolve_remote_name(name, repo_config)?;
            store
                .git_fetch(&remote_name)
                .context("failed to fetch from remote")?;
            println!("Fetched from '{remote_name}'.");
        }
        RemoteCommand::Push { name, force } => {
            let remote_name = paths::resolve_remote_name(name, repo_config)?;
            let result = store.git_push(&remote_name, force)?;
            if result.commits_pushed == 0 {
                println!("Already up to date.");
            } else {
                println!(
                    "Pushed {} commit(s) to {}/{}.",
                    result.commits_pushed, result.remote, result.branch
                );
            }
        }
        RemoteCommand::Pull { name } => {
            let remote_name = paths::resolve_remote_name(name, repo_config)?;
            let outcome = store.git_pull(&remote_name)?;
            match outcome {
                rdm_store_git::PullOutcome::Success(result) => {
                    if !result.changed {
                        println!("Already up to date.");
                    } else {
                        println!(
                            "Pulled {} commit(s) from {}/{}.",
                            result.commits_merged, result.remote, result.branch
                        );
                        // Regenerate INDEX.md after pulling new content
                        let new_store = make_store(root, staging)?;
                        let mut repo = PlanRepo::new(new_store);
                        repo.generate_index()
                            .context("failed to regenerate INDEX.md after pull")?;
                    }
                }
                rdm_store_git::PullOutcome::Conflict(conflict) => {
                    eprintln!(
                        "Merge conflict: {} file(s) need resolution.",
                        conflict.conflicted_files.len()
                    );
                    eprintln!();
                    for item in &conflict.conflicted_files {
                        eprintln!("  conflict: {} — {item}", item.path);
                    }
                    eprintln!();
                    eprintln!("Edit the conflicted files, then run `rdm resolve <file>` for each.");
                    eprintln!("Run `rdm conflicts` to see the full list.");
                    eprintln!("Run `rdm discard --force` to abort the merge and discard changes.");
                    process::exit(1);
                }
            }
        }
    }
    Ok(())
}

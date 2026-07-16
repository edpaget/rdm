use std::process;

use anyhow::Result;
use clap::Parser;
#[cfg(not(feature = "git"))]
use rdm_store_fs::FsStore;

mod cli;
mod commands;
#[cfg(feature = "git")]
mod hook_log;
mod paths;
mod table;

pub(crate) use cli::*;

#[cfg(feature = "git")]
pub(crate) type AppStore = rdm_store_git::GitStore;
#[cfg(not(feature = "git"))]
pub(crate) type AppStore = FsStore;

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err:#}");
        process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let global_config = paths::load_global_config();

    // Handle config commands early — some don't need a repo.
    if let Command::Config { command } = cli.command {
        return commands::config::run(command, &cli.root, &global_config);
    }

    let root = paths::resolve_root(cli.root, &global_config)?;
    let root = paths::expand_root(root)?;
    let repo_config = paths::load_repo_config(&root).with_global_defaults(&global_config);
    let format_str = paths::resolve_format(cli.format.map(|f| f.to_string()), &repo_config);
    let format: OutputFormat = format_str
        .parse::<OutputFormat>()
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    // Guard: non-init commands require rdm.toml to exist at the resolved root.
    // Exempt commands (Init, Bootstrap, Describe, AgentConfig, Hook, Model, Mcp)
    // are allowed to proceed without rdm.toml.
    let rdm_toml_exists = root.join("rdm.toml").exists();
    if !rdm_toml_exists {
        match &cli.command {
            Command::Init { .. }
            | Command::Describe { .. }
            | Command::AgentConfig { .. }
            | Command::Model { .. } => {
                // These commands are exempt; proceed.
            }
            #[cfg(feature = "git")]
            Command::Bootstrap { .. } | Command::Hook { .. } => {
                // These commands are exempt; proceed.
            }
            #[cfg(feature = "mcp")]
            Command::Mcp => {
                // This command is exempt; proceed.
            }
            _ => {
                // All other commands require rdm.toml.
                anyhow::bail!(
                    "no plan repo found at {} — run `rdm init` to create one",
                    root.display()
                );
            }
        }
    }

    match cli.command {
        Command::Config { .. } => unreachable!("handled above"),
        Command::Init {
            default_project,
            default_format,
            #[cfg(feature = "git")]
            remote,
        } => commands::init::run(
            &root,
            &global_config,
            default_project,
            default_format,
            #[cfg(feature = "git")]
            remote,
        )?,

        #[cfg(feature = "git")]
        Command::Bootstrap {
            plan_repo,
            path,
            branch,
            init,
            token,
            print_root,
            command,
        } => commands::bootstrap::run_command(
            plan_repo, path, branch, init, token, command, print_root, cli.format,
        )?,

        Command::Index => commands::index::run(&root)?,

        Command::Project { command } => {
            let mut store = commands::make_store(&root)?;
            commands::project::run(command, &mut store, format, cli.no_index)?;
        }

        Command::Roadmap { command } => {
            let mut store = commands::make_store(&root)?;
            commands::roadmap::run(command, &mut store, &repo_config, format, cli.no_index)?;
        }

        Command::Phase { command } => {
            let mut store = commands::make_store(&root)?;
            commands::phase::run(command, &mut store, &repo_config, format, cli.no_index)?;
        }

        Command::Task { command } => {
            let mut store = commands::make_store(&root)?;
            commands::task::run(command, &mut store, &repo_config, format, cli.no_index)?;
        }

        Command::Promote {
            task_slug,
            roadmap_slug,
            project,
        } => commands::promote::run(
            &root,
            &repo_config,
            cli.no_index,
            task_slug,
            roadmap_slug,
            project,
        )?,

        Command::Tree { project } => commands::tree::run(&root, &repo_config, format, project)?,

        Command::Describe { entity } => commands::describe::run(format, entity)?,

        Command::AgentConfig {
            platform,
            project,
            out,
            principles_file,
            skills,
            hooks,
            mcp,
            user,
        } => commands::agent_config::run(
            &root,
            platform,
            project,
            out,
            principles_file,
            skills,
            hooks,
            mcp,
            user,
        )?,

        Command::Search {
            query,
            kind,
            status,
            project,
            tags,
            limit,
            min_score_ratio,
        } => commands::search::run(
            &root,
            format,
            query,
            kind,
            status,
            project,
            tags,
            limit,
            min_score_ratio,
        )?,

        #[cfg(feature = "mcp")]
        Command::Mcp => commands::mcp::run(root, &global_config)?,

        #[cfg(feature = "server")]
        Command::Serve {
            port,
            bind,
            quick_filter,
        } => commands::serve::run(root, &repo_config, port, bind, quick_filter)?,

        #[cfg(feature = "git")]
        Command::Status { fetch } => commands::status::run(&root, fetch)?,

        #[cfg(feature = "git")]
        Command::Commit { message } => commands::commit::run(&root, message)?,

        #[cfg(feature = "git")]
        Command::Discard { force } => commands::discard::run(&root, force)?,

        #[cfg(feature = "git")]
        Command::Conflicts => commands::conflicts::run(&root)?,

        #[cfg(feature = "git")]
        Command::Resolve { file } => commands::resolve::run(&root, file)?,

        #[cfg(feature = "git")]
        Command::Remote { command } => {
            let mut store = commands::make_store(&root)?;
            commands::remote::run(command, &mut store, &root, &repo_config)?;
        }

        #[cfg(feature = "git")]
        Command::Hook { command } => {
            commands::hook::run(command, &root)?;
        }

        #[cfg(feature = "git")]
        Command::Worktree { command } => {
            commands::worktree::run(command, &root, &repo_config, format)?;
        }

        #[cfg(feature = "git")]
        Command::Review { command } => {
            let mut store = commands::make_store(&root)?;
            commands::review::run(command, &mut store, &repo_config, format, cli.no_index)?;
        }

        Command::List { project, all } => {
            commands::list::run(&root, &repo_config, format, project, all)?
        }

        Command::Next { roadmap, project } => {
            let mut store = commands::make_store(&root)?;
            commands::next::run(&mut store, &repo_config, format, roadmap, project)?;
        }

        Command::Model { command } => {
            commands::model::run(command, &repo_config, format)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::path::PathBuf;

    #[test]
    fn expand_root_tilde_expands_to_home() {
        let home = env::var("HOME").unwrap();
        let result = paths::expand_root(PathBuf::from("~")).unwrap();
        assert_eq!(result, PathBuf::from(&home));
    }

    #[test]
    fn expand_root_tilde_slash_expands_to_home_subpath() {
        let home = env::var("HOME").unwrap();
        let result = paths::expand_root(PathBuf::from("~/foo/bar")).unwrap();
        assert_eq!(result, PathBuf::from(format!("{home}/foo/bar")));
    }

    #[test]
    fn expand_root_dot_resolves_to_cwd() {
        let cwd = env::current_dir().unwrap();
        let result = paths::expand_root(PathBuf::from(".")).unwrap();
        assert_eq!(result, cwd);
    }

    #[test]
    fn expand_root_dotdot_resolves_relative_to_cwd() {
        let cwd = env::current_dir().unwrap();
        let result = paths::expand_root(PathBuf::from("../foo")).unwrap();
        let expected = cwd.parent().unwrap().join("foo");
        assert_eq!(result, expected);
    }

    #[test]
    fn expand_root_absolute_path_unchanged() {
        let result = paths::expand_root(PathBuf::from("/tmp/plans")).unwrap();
        assert_eq!(result, PathBuf::from("/tmp/plans"));
    }
}

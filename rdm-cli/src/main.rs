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
    let staging = paths::resolve_staging(cli.stage, &repo_config);
    let format_str = paths::resolve_format(cli.format.map(|f| f.to_string()), &repo_config);
    let format: OutputFormat = format_str
        .parse::<OutputFormat>()
        .map_err(|e| anyhow::anyhow!("{e}"))?;

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
            cli.stage,
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
            command,
        } => commands::bootstrap::run_command(plan_repo, path, branch, init, token, command)?,

        Command::Index => commands::index::run(&root, staging)?,

        Command::Project { command } => {
            let mut store = commands::make_store(&root, staging)?;
            commands::project::run(command, &mut store, format, cli.no_index, staging)?;
        }

        Command::Roadmap { command } => {
            let mut store = commands::make_store(&root, staging)?;
            commands::roadmap::run(
                command,
                &mut store,
                &repo_config,
                format,
                cli.no_index,
                staging,
            )?;
        }

        Command::Phase { command } => {
            let mut store = commands::make_store(&root, staging)?;
            commands::phase::run(
                command,
                &mut store,
                &repo_config,
                format,
                cli.no_index,
                staging,
            )?;
        }

        Command::Task { command } => {
            let mut store = commands::make_store(&root, staging)?;
            commands::task::run(
                command,
                &mut store,
                &repo_config,
                format,
                cli.no_index,
                staging,
            )?;
        }

        Command::Promote {
            task_slug,
            roadmap_slug,
            project,
        } => commands::promote::run(
            &root,
            &repo_config,
            cli.no_index,
            staging,
            task_slug,
            roadmap_slug,
            project,
        )?,

        Command::Tree { project } => {
            commands::tree::run(&root, &repo_config, staging, format, project)?
        }

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
            staging,
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
        Command::Mcp => commands::mcp::run(root, &global_config, staging)?,

        #[cfg(feature = "server")]
        Command::Serve {
            port,
            bind,
            quick_filter,
        } => commands::serve::run(root, &repo_config, port, bind, quick_filter)?,

        #[cfg(feature = "git")]
        Command::Status { fetch } => commands::status::run(&root, staging, fetch)?,

        #[cfg(feature = "git")]
        Command::Commit { message } => commands::commit::run(&root, staging, message)?,

        #[cfg(feature = "git")]
        Command::Discard { force } => commands::discard::run(&root, staging, force)?,

        #[cfg(feature = "git")]
        Command::Conflicts => commands::conflicts::run(&root, staging)?,

        #[cfg(feature = "git")]
        Command::Resolve { file } => commands::resolve::run(&root, staging, file)?,

        #[cfg(feature = "git")]
        Command::Remote { command } => {
            let mut store = commands::make_store(&root, staging)?;
            commands::remote::run(command, &mut store, &root, &repo_config, staging)?;
        }

        #[cfg(feature = "git")]
        Command::Hook { command } => {
            commands::hook::run(command, &root, staging)?;
        }

        #[cfg(feature = "git")]
        Command::Worktree { command } => {
            commands::worktree::run(command, &root, &repo_config, staging, format)?;
        }

        Command::List { project, all } => {
            commands::list::run(&root, &repo_config, staging, format, project, all)?
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
